#![forbid(unsafe_code)]

use recovery_core::{ByteRange, RecoveryError, RecoveryResult};
use storage_io::BlockDevice;

const NXSB_MAGIC: u32 = 0x4253_584e;
const APSB_MAGIC: u32 = 0x4253_5041;
const MIN_BLOCK_SIZE: u32 = 512;
const MAX_BLOCK_SIZE: u32 = 65_536;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApfsContainer {
    pub block_size: u32,
    pub block_count: u64,
    pub features: u64,
    pub read_only_compatible_features: u64,
    pub incompatible_features: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApfsVolume {
    pub fs_index: u32,
    pub features: u64,
    pub read_only_compatible_features: u64,
    pub incompatible_features: u64,
    pub unmount_time: u64,
    pub allocated_blocks: u64,
    pub reserve_blocks: u64,
    pub quota_blocks: u64,
}

fn u32_at(block: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(block[offset..offset + 4].try_into().expect("fixed APFS field"))
}

fn u64_at(block: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(block[offset..offset + 8].try_into().expect("fixed APFS field"))
}

pub fn parse_container_superblock(block: &[u8]) -> RecoveryResult<ApfsContainer> {
    if block.len() < 80 {
        return Err(RecoveryError::LengthTooLarge { length: block.len() as u64 });
    }
    if u32_at(block, 32) != NXSB_MAGIC {
        return Err(RecoveryError::IoFailure("not an APFS container superblock".into()));
    }
    let block_size = u32_at(block, 40);
    if !(MIN_BLOCK_SIZE..=MAX_BLOCK_SIZE).contains(&block_size) || !block_size.is_power_of_two() {
        return Err(RecoveryError::IoFailure("invalid APFS block size".into()));
    }
    let block_count = u64_at(block, 48);
    if block_count == 0 {
        return Err(RecoveryError::IoFailure("APFS container has zero blocks".into()));
    }
    Ok(ApfsContainer {
        block_size,
        block_count,
        features: u64_at(block, 56),
        read_only_compatible_features: u64_at(block, 64),
        incompatible_features: u64_at(block, 72),
    })
}

pub fn parse_volume_superblock(block: &[u8]) -> RecoveryResult<ApfsVolume> {
    if block.len() < 104 {
        return Err(RecoveryError::LengthTooLarge { length: block.len() as u64 });
    }
    if u32_at(block, 32) != APSB_MAGIC {
        return Err(RecoveryError::IoFailure("not an APFS volume superblock".into()));
    }
    let fs_index = u32_at(block, 40);
    let features = u64_at(block, 48);
    let read_only_compatible_features = u64_at(block, 56);
    let incompatible_features = u64_at(block, 64);
    let unmount_time = u64_at(block, 72);
    let reserve_blocks = u64_at(block, 80);
    let quota_blocks = u64_at(block, 88);
    let allocated_blocks = u64_at(block, 96);
    Ok(ApfsVolume {
        fs_index,
        features,
        read_only_compatible_features,
        incompatible_features,
        unmount_time,
        allocated_blocks,
        reserve_blocks,
        quota_blocks,
    })
}

pub fn open<D: BlockDevice>(device: &D, range: ByteRange) -> RecoveryResult<ApfsContainer> {
    range.validate_within(device.capacity())?;
    if range.length < 512 {
        return Err(RecoveryError::LengthTooLarge { length: range.length });
    }
    let read_len = range.length.min(4096) as usize;
    let mut block = vec![0u8; read_len];
    let read_range = ByteRange::new(range.offset, read_len as u64)?;
    if device.read(read_range, &mut block)? != read_len {
        return Err(RecoveryError::IoFailure("short APFS superblock read".into()));
    }
    let container = parse_container_superblock(&block)?;
    let bytes = u64::from(container.block_size)
        .checked_mul(container.block_count)
        .ok_or(RecoveryError::RangeOverflow)?;
    if bytes > range.length {
        return Err(RecoveryError::IoFailure("APFS container exceeds supplied range".into()));
    }
    Ok(container)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_container_superblock_geometry() {
        let mut block = vec![0u8; 512];
        block[32..36].copy_from_slice(&NXSB_MAGIC.to_le_bytes());
        block[40..44].copy_from_slice(&4096u32.to_le_bytes());
        block[48..56].copy_from_slice(&1024u64.to_le_bytes());
        block[56..64].copy_from_slice(&7u64.to_le_bytes());
        block[64..72].copy_from_slice(&8u64.to_le_bytes());
        block[72..80].copy_from_slice(&9u64.to_le_bytes());
        let parsed = parse_container_superblock(&block).unwrap();
        assert_eq!(parsed.block_size, 4096);
        assert_eq!(parsed.block_count, 1024);
        assert_eq!(parsed.features, 7);
        assert_eq!(parsed.read_only_compatible_features, 8);
        assert_eq!(parsed.incompatible_features, 9);
    }

    #[test]
    fn parses_volume_superblock_metadata() {
        let mut block = vec![0u8; 512];
        block[32..36].copy_from_slice(&APSB_MAGIC.to_le_bytes());
        block[40..44].copy_from_slice(&3u32.to_le_bytes());
        block[48..56].copy_from_slice(&7u64.to_le_bytes());
        block[56..64].copy_from_slice(&8u64.to_le_bytes());
        block[64..72].copy_from_slice(&9u64.to_le_bytes());
        block[72..80].copy_from_slice(&10u64.to_le_bytes());
        block[80..88].copy_from_slice(&11u64.to_le_bytes());
        block[88..96].copy_from_slice(&12u64.to_le_bytes());
        block[96..104].copy_from_slice(&13u64.to_le_bytes());
        let parsed = parse_volume_superblock(&block).unwrap();
        assert_eq!(parsed.fs_index, 3);
        assert_eq!(parsed.features, 7);
        assert_eq!(parsed.read_only_compatible_features, 8);
        assert_eq!(parsed.incompatible_features, 9);
        assert_eq!(parsed.unmount_time, 10);
        assert_eq!(parsed.reserve_blocks, 11);
        assert_eq!(parsed.quota_blocks, 12);
        assert_eq!(parsed.allocated_blocks, 13);
    }

    #[test]
    fn rejects_wrong_volume_magic() {
        let mut block = vec![0u8; 512];
        block[32..36].copy_from_slice(&NXSB_MAGIC.to_le_bytes());
        assert!(parse_volume_superblock(&block).is_err());
    }

    #[test]
    fn rejects_truncated_volume_superblock() {
        assert!(parse_volume_superblock(&[0u8; 103]).is_err());
    }

    #[test]
    fn rejects_invalid_block_size() {
        let mut block = vec![0u8; 512];
        block[32..36].copy_from_slice(&NXSB_MAGIC.to_le_bytes());
        block[40..44].copy_from_slice(&3000u32.to_le_bytes());
        block[48..56].copy_from_slice(&1u64.to_le_bytes());
        assert!(parse_container_superblock(&block).is_err());
    }
}
