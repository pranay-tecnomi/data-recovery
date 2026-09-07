use recovery_core::{ByteRange, RecoveryError, RecoveryResult};
use storage_io::BlockDevice;

use crate::{
    ApfsContainer, parse_container_superblock, parse_object_header, read_object, verify_fletcher64,
};

const NX_SUPERBLOCK_TYPE: u32 = 0x0000_0001;
const XP_DESC_FRAGMENTED: u32 = 0x8000_0000;
const MAX_CHECKPOINT_BLOCKS: u32 = 1_048_576;
const NX_XP_DESC_BLOCKS: usize = 0x68;
const NX_XP_DESC_BASE: usize = 0x70;
const NX_XP_DESC_INDEX: usize = 0x88;
const NX_XP_DESC_LEN: usize = 0x8c;

fn u32_at(block: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        block[offset..offset + 4]
            .try_into()
            .expect("fixed APFS integer"),
    )
}

fn u64_at(block: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(
        block[offset..offset + 8]
            .try_into()
            .expect("fixed APFS integer"),
    )
}

/// Locate the newest valid container superblock and return its physical block
/// together with the parsed container metadata. The physical block is needed
/// by discovery because the selected checkpoint may contain newer filesystem
/// object identifiers than block zero.
pub(crate) fn read_latest_container_superblock_with_block<D: BlockDevice>(
    device: &D,
    range: ByteRange,
) -> RecoveryResult<(ApfsContainer, Vec<u8>)> {
    range.validate_within(device.capacity())?;
    let initial_len = range.length.min(65_536) as usize;
    let mut initial = vec![0u8; initial_len];
    let initial_range = ByteRange::new(range.offset, initial_len as u64)?;
    if device.read(initial_range, &mut initial)? != initial.len() {
        return Err(RecoveryError::IoFailure(
            "short APFS container superblock read".into(),
        ));
    }
    let base = parse_container_superblock(&initial)?;
    let block_size = base.block_size as usize;
    if block_size > initial.len() {
        return Err(RecoveryError::IoFailure(
            "APFS container block exceeds initial read".into(),
        ));
    }
    verify_fletcher64(&initial[..block_size])?;
    if u64::from(base.block_size) > range.length {
        return Err(RecoveryError::IoFailure(
            "APFS container block exceeds supplied range".into(),
        ));
    }

    let desc_blocks_raw = u32_at(&initial, NX_XP_DESC_BLOCKS);
    if desc_blocks_raw & XP_DESC_FRAGMENTED != 0 {
        return Err(RecoveryError::IoFailure(
            "APFS checkpoint descriptor area is fragmented and is not yet supported".into(),
        ));
    }
    let desc_blocks = desc_blocks_raw;
    if desc_blocks == 0 {
        return Ok((base, initial[..block_size].to_vec()));
    }
    if desc_blocks > MAX_CHECKPOINT_BLOCKS {
        return Err(RecoveryError::LengthTooLarge {
            length: desc_blocks as u64,
        });
    }

    let desc_base = u64_at(&initial, NX_XP_DESC_BASE);
    let desc_end = desc_base
        .checked_add(desc_blocks as u64)
        .ok_or(RecoveryError::RangeOverflow)?;
    if desc_end > base.block_count {
        return Err(RecoveryError::OutOfRange {
            offset: desc_base,
            length: desc_blocks as u64,
            capacity: base.block_count,
        });
    }

    let mut best: Option<(u64, ApfsContainer, Vec<u8>)> = None;
    for index in 0..desc_blocks {
        let oid = desc_base
            .checked_add(index as u64)
            .ok_or(RecoveryError::RangeOverflow)?;
        let block = read_object(device, range, &base, oid)?;
        if verify_fletcher64(&block).is_err() {
            continue;
        }
        let header = match parse_object_header(&block) {
            Ok(header) if header.object_type & 0x0000_ffff == NX_SUPERBLOCK_TYPE => header,
            _ => continue,
        };
        let candidate = match parse_container_superblock(&block) {
            Ok(candidate) => candidate,
            Err(_) => continue,
        };

        // A checkpoint superblock is the final block in its descriptor window.
        // Require its self-described ring position to agree with the physical
        // position we scanned; otherwise stale/corrupt NXSB-like data can win
        // merely because it has a large transaction identifier.
        let candidate_desc_blocks =
            u64::from(u32_at(&block, NX_XP_DESC_BLOCKS) & !XP_DESC_FRAGMENTED);
        let candidate_desc_index = u64::from(u32_at(&block, NX_XP_DESC_INDEX));
        let candidate_desc_len = u64::from(u32_at(&block, NX_XP_DESC_LEN));
        if candidate_desc_blocks == 0
            || candidate_desc_blocks > u64::from(MAX_CHECKPOINT_BLOCKS)
            || candidate_desc_index >= candidate_desc_blocks
            || candidate_desc_len == 0
            || candidate_desc_len > candidate_desc_blocks
        {
            continue;
        }
        let expected_index =
            (candidate_desc_index + candidate_desc_len - 1) % candidate_desc_blocks;
        if expected_index != u64::from(index) {
            continue;
        }

        let candidate_bytes = u64::from(candidate.block_size)
            .checked_mul(candidate.block_count)
            .ok_or(RecoveryError::RangeOverflow)?;
        if candidate_bytes > range.length {
            continue;
        }
        if best
            .as_ref()
            .map(|(xid, _, _)| header.xid > *xid)
            .unwrap_or(true)
        {
            best = Some((header.xid, candidate, block));
        }
    }

    Ok(best
        .map(|(_, container, block)| (container, block))
        .unwrap_or_else(|| {
            let block = initial[..block_size].to_vec();
            (base, block)
        }))
}

/// Locate the newest valid container superblock stored in the checkpoint
/// descriptor area. Falls back to block zero when no newer valid checkpoint
/// superblock is present.
pub fn read_latest_container_superblock<D: BlockDevice>(
    device: &D,
    range: ByteRange,
) -> RecoveryResult<ApfsContainer> {
    Ok(read_latest_container_superblock_with_block(device, range)?.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checkpoint_geometry_offsets_match_nx_superblock_layout() {
        let mut block = vec![0u8; 512];
        block[NX_XP_DESC_BLOCKS..NX_XP_DESC_BLOCKS + 4].copy_from_slice(&16u32.to_le_bytes());
        block[NX_XP_DESC_BASE..NX_XP_DESC_BASE + 8].copy_from_slice(&8u64.to_le_bytes());
        block[NX_XP_DESC_INDEX..NX_XP_DESC_INDEX + 4].copy_from_slice(&4u32.to_le_bytes());
        block[NX_XP_DESC_LEN..NX_XP_DESC_LEN + 4].copy_from_slice(&12u32.to_le_bytes());
        assert_eq!(u32_at(&block, NX_XP_DESC_BLOCKS), 16);
        assert_eq!(u64_at(&block, NX_XP_DESC_BASE), 8);
        assert_eq!(u32_at(&block, NX_XP_DESC_INDEX), 4);
        assert_eq!(u32_at(&block, NX_XP_DESC_LEN), 12);
    }

    #[test]
    fn checkpoint_superblock_position_is_final_block_of_window() {
        let desc_blocks = 64u64;
        let desc_index = 50u64;
        let desc_len = 15u64;
        let expected = (desc_index + desc_len - 1) % desc_blocks;
        assert_eq!(expected, 0);
    }

    #[test]
    fn checkpoint_superblock_position_rejects_wrong_index() {
        let desc_blocks = 64u64;
        let desc_index = 10u64;
        let desc_len = 8u64;
        let scanned_index = 20u64;
        let expected = (desc_index + desc_len - 1) % desc_blocks;
        assert_ne!(expected, scanned_index);
    }

    #[test]
    fn fragmented_flag_is_detected() {
        let mut block = vec![0u8; 512];
        block[NX_XP_DESC_BLOCKS..NX_XP_DESC_BLOCKS + 4]
            .copy_from_slice(&XP_DESC_FRAGMENTED.to_le_bytes());
        assert_ne!(u32_at(&block, NX_XP_DESC_BLOCKS) & XP_DESC_FRAGMENTED, 0);
    }
}
