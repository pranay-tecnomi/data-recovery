use recovery_core::{ByteRange, RecoveryError, RecoveryResult};
use storage_io::BlockDevice;

use crate::{parse_container_superblock, parse_object_header, read_object, ApfsContainer};

const NX_SUPERBLOCK_TYPE: u32 = 0x0000_0001;
const XP_DESC_FRAGMENTED: u32 = 0x8000_0000;
const MAX_CHECKPOINT_BLOCKS: u32 = 1_048_576;

fn u32_at(block: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(block[offset..offset + 4].try_into().expect("fixed APFS integer"))
}

fn u64_at(block: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(block[offset..offset + 8].try_into().expect("fixed APFS integer"))
}

/// Locate the newest valid container superblock stored in the contiguous
/// checkpoint descriptor area. APFS keeps block zero as a copy, while the
/// checkpoint ring contains newer NX superblocks identified by their XID.
/// Fragmented checkpoint descriptor areas are rejected until their extent-list
/// tree can be resolved safely.
pub fn read_latest_container_superblock<D: BlockDevice>(
    device: &D,
    range: ByteRange,
) -> RecoveryResult<ApfsContainer> {
    range.validate_within(device.capacity())?;
    let initial_len = range.length.min(65_536) as usize;
    let mut initial = vec![0u8; initial_len];
    let initial_range = ByteRange::new(range.offset, initial_len as u64)?;
    if device.read(initial_range, &mut initial)? != initial.len() {
        return Err(RecoveryError::IoFailure("short APFS container superblock read".into()));
    }
    let base = parse_container_superblock(&initial)?;

    let desc_blocks_raw = u32_at(&initial, 0x68);
    if desc_blocks_raw & XP_DESC_FRAGMENTED != 0 {
        return Err(RecoveryError::IoFailure(
            "APFS checkpoint descriptor area is fragmented and is not yet supported".into(),
        ));
    }
    let desc_blocks = desc_blocks_raw;
    if desc_blocks == 0 {
        return Ok(base);
    }
    if desc_blocks > MAX_CHECKPOINT_BLOCKS {
        return Err(RecoveryError::LengthTooLarge { length: desc_blocks as u64 });
    }

    let desc_base = u64_at(&initial, 0x70);
    let desc_end = desc_base.checked_add(desc_blocks as u64).ok_or(RecoveryError::RangeOverflow)?;
    if desc_end > base.block_count {
        return Err(RecoveryError::OutOfRange { offset: desc_base, length: desc_blocks as u64, capacity: base.block_count });
    }

    let mut best: Option<(u64, ApfsContainer)> = None;
    for index in 0..desc_blocks {
        let oid = desc_base.checked_add(index as u64).ok_or(RecoveryError::RangeOverflow)?;
        let block = read_object(device, range, &base, oid)?;
        let header = match parse_object_header(&block) {
            Ok(header) if header.object_type & 0x0000_ffff == NX_SUPERBLOCK_TYPE => header,
            _ => continue,
        };
        let candidate = match parse_container_superblock(&block) {
            Ok(candidate) => candidate,
            Err(_) => continue,
        };
        let candidate_bytes = u64::from(candidate.block_size)
            .checked_mul(candidate.block_count)
            .ok_or(RecoveryError::RangeOverflow)?;
        if candidate_bytes > range.length {
            continue;
        }
        if best.as_ref().map(|(xid, _)| header.xid > *xid).unwrap_or(true) {
            best = Some((header.xid, candidate));
        }
    }

    Ok(best.map(|(_, container)| container).unwrap_or(base))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checkpoint_geometry_offsets_match_nx_superblock_layout() {
        let mut block = vec![0u8; 512];
        block[0x68..0x6c].copy_from_slice(&16u32.to_le_bytes());
        block[0x70..0x78].copy_from_slice(&8u64.to_le_bytes());
        assert_eq!(u32_at(&block, 0x68), 16);
        assert_eq!(u64_at(&block, 0x70), 8);
    }

    #[test]
    fn fragmented_flag_is_detected() {
        let mut block = vec![0u8; 512];
        block[0x68..0x6c].copy_from_slice(&XP_DESC_FRAGMENTED.to_le_bytes());
        assert_ne!(u32_at(&block, 0x68) & XP_DESC_FRAGMENTED, 0);
    }
}
