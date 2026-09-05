use recovery_core::{ByteRange, RecoveryError, RecoveryResult};
use storage_io::BlockDevice;

use crate::{lookup_object_map, parse_object_header, parse_object_map, parse_volume_superblock, read_object, ApfsContainer, ApfsObjectMapKey, ApfsObjectMapValue, ApfsVolume};

pub fn lookup_volume_object<D: BlockDevice>(device: &D, range: ByteRange, container: &ApfsContainer, volume: &ApfsVolume, target: ApfsObjectMapKey) -> RecoveryResult<ApfsObjectMapValue> {
    if volume.omap_oid == 0 { return Err(RecoveryError::IoFailure("APFS volume has no object-map OID".into())); }
    let omap_block = read_object(device, range, container, volume.omap_oid)?;
    let omap = parse_object_map(&omap_block)?;
    lookup_object_map(device, range, container, &omap, target)
}

pub fn resolve_volume_root<D: BlockDevice>(device: &D, range: ByteRange, container: &ApfsContainer, volume: &ApfsVolume, xid: u64) -> RecoveryResult<u64> {
    if volume.root_tree_oid == 0 { return Err(RecoveryError::IoFailure("APFS volume has no root-tree OID".into())); }
    let mapping = lookup_volume_object(device, range, container, volume, ApfsObjectMapKey { oid: volume.root_tree_oid, xid })?;
    if mapping.is_deleted() { return Err(RecoveryError::IoFailure("APFS volume root-tree mapping is deleted".into())); }
    if mapping.physical_address >= container.block_count { return Err(RecoveryError::OutOfRange { offset: mapping.physical_address, length: 1, capacity: container.block_count }); }
    Ok(mapping.physical_address)
}

pub(crate) fn volume_superblock_xid(superblock: &[u8]) -> RecoveryResult<u64> {
    parse_volume_superblock(superblock)?;
    let header = parse_object_header(superblock)?;
    if header.oid == 0 || header.xid == 0 { return Err(RecoveryError::IoFailure("APFS volume superblock has invalid object identity".into())); }
    Ok(header.xid)
}

pub(crate) fn resolve_volume_root_from_superblock<D: BlockDevice>(device: &D, range: ByteRange, container: &ApfsContainer, superblock: &[u8]) -> RecoveryResult<u64> {
    let volume = parse_volume_superblock(superblock)?;
    let xid = volume_superblock_xid(superblock)?;
    resolve_volume_root(device, range, container, &volume, xid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_missing_volume_omap_before_device_access() {
        let volume = ApfsVolume { fs_index: 0, features: 0, read_only_compatible_features: 0, incompatible_features: 0, unmount_time: 0, reserve_blocks: 0, quota_blocks: 0, allocated_blocks: 0, fs_reserve_blocks: 0, omap_oid: 0, root_tree_oid: 1, extentref_tree_oid: 0, snap_meta_tree_oid: 0 };
        assert_eq!(volume.omap_oid, 0);
    }

    #[test]
    fn extracts_volume_superblock_xid() {
        let mut block = vec![0u8; 512];
        block[32..36].copy_from_slice(&0x4253_5041u32.to_le_bytes());
        block[8..16].copy_from_slice(&55u64.to_le_bytes());
        block[16..24].copy_from_slice(&77u64.to_le_bytes());
        block[128..136].copy_from_slice(&1u64.to_le_bytes());
        block[136..144].copy_from_slice(&2u64.to_le_bytes());
        assert_eq!(volume_superblock_xid(&block).unwrap(), 77);
    }

    #[test]
    fn rejects_zero_volume_superblock_xid() {
        let mut block = vec![0u8; 512];
        block[32..36].copy_from_slice(&0x4253_5041u32.to_le_bytes());
        block[8..16].copy_from_slice(&55u64.to_le_bytes());
        block[128..136].copy_from_slice(&1u64.to_le_bytes());
        block[136..144].copy_from_slice(&2u64.to_le_bytes());
        assert!(volume_superblock_xid(&block).is_err());
    }
}
