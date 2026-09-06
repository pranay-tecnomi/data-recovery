use recovery_core::{ByteRange, RecoveryResult};
use storage_io::BlockDevice;

use crate::{read_volume_filesystem_index, recover_regular_files, ApfsContainer, ApfsDiscoveredVolume, ApfsRecoveredFile};

/// Build the catalog index and recover regular files from one discovered APFS volume.
///
/// The discovered volume carries the checkpoint XID that must be used for the
/// volume OMAP. Keeping that XID attached to the discovery result prevents the
/// caller from accidentally mixing a volume superblock from one transaction
/// with an object-map view from another transaction.
pub fn recover_discovered_volume_files<D: BlockDevice>(
    device: &D,
    range: ByteRange,
    container: &ApfsContainer,
    discovered: &ApfsDiscoveredVolume,
) -> RecoveryResult<Vec<ApfsRecoveredFile>> {
    let index = read_volume_filesystem_index(
        device,
        range,
        container,
        &discovered.volume,
        discovered.xid,
    )?;
    recover_regular_files(&index, device, range, container.block_size)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_result_carries_transaction_context() {
        let volume = crate::ApfsVolume {
            fs_index: 0,
            features: 0,
            read_only_compatible_features: 0,
            incompatible_features: 0,
            unmount_time: 0,
            reserve_blocks: 0,
            quota_blocks: 0,
            allocated_blocks: 0,
            fs_reserve_blocks: 0,
            omap_oid: 7,
            root_tree_oid: 8,
            extentref_tree_oid: 0,
            snap_meta_tree_oid: 0,
        };
        let discovered = ApfsDiscoveredVolume { object_id: 9, xid: 10, volume };
        assert_eq!(discovered.xid, 10);
        assert_eq!(discovered.object_id, 9);
    }
}
