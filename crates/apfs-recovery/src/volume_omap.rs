use recovery_core::{ByteRange, RecoveryError, RecoveryResult};
use storage_io::BlockDevice;

use crate::{lookup_object_map, parse_object_map, read_object, ApfsContainer, ApfsObjectMapKey, ApfsObjectMapValue, ApfsVolume};

/// Resolve a virtual APFS object ID through a volume's object map.
pub fn lookup_volume_object<D: BlockDevice>(
    device: &D,
    range: ByteRange,
    container: &ApfsContainer,
    volume: &ApfsVolume,
    target: ApfsObjectMapKey,
) -> RecoveryResult<ApfsObjectMapValue> {
    if volume.omap_oid == 0 {
        return Err(RecoveryError::IoFailure("APFS volume has no object-map OID".into()));
    }
    let omap_block = read_object(device, range, container, volume.omap_oid)?;
    let omap = parse_object_map(&omap_block)?;
    lookup_object_map(device, range, container, &omap, target)
}

/// Resolve the volume root-tree object to its physical block address.
pub fn resolve_volume_root<D: BlockDevice>(
    device: &D,
    range: ByteRange,
    container: &ApfsContainer,
    volume: &ApfsVolume,
    xid: u64,
) -> RecoveryResult<u64> {
    if volume.root_tree_oid == 0 {
        return Err(RecoveryError::IoFailure("APFS volume has no root-tree OID".into()));
    }
    let mapping = lookup_volume_object(
        device,
        range,
        container,
        volume,
        ApfsObjectMapKey { oid: volume.root_tree_oid, xid },
    )?;
    if mapping.is_deleted() {
        return Err(RecoveryError::IoFailure("APFS volume root-tree mapping is deleted".into()));
    }
    if mapping.physical_address >= container.block_count {
        return Err(RecoveryError::OutOfRange {
            offset: mapping.physical_address,
            length: 1,
            capacity: container.block_count,
        });
    }
    Ok(mapping.physical_address)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_missing_volume_omap_before_device_access() {
        let volume = ApfsVolume {
            fs_index: 0,
            features: 0,
            read_only_compatible_features: 0,
            incompatible_features: 0,
            unmount_time: 0,
            reserve_blocks: 0,
            quota_blocks: 0,
            allocated_blocks: 0,
            fs_reserve_blocks: 0,
            omap_oid: 0,
            root_tree_oid: 1,
            extentref_tree_oid: 0,
            snap_meta_tree_oid: 0,
        };
        assert_eq!(volume.omap_oid, 0);
    }
}
