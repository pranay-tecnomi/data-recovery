use recovery_core::{ByteRange, RecoveryError, RecoveryResult};
use storage_io::BlockDevice;

use crate::checkpoint::read_latest_container_superblock_with_block;
use crate::{
    ApfsContainer, ApfsObjectMapKey, ApfsVolume, lookup_object_map, parse_container_superblock,
    parse_object_header, parse_object_map, parse_volume_superblock, read_object,
};

const NX_FS_OID_OFFSET: usize = 0xC8;
const NX_MAX_FILE_SYSTEMS_OFFSET: usize = 0xC4;
const NX_FS_OID_SLOTS: usize = 100;

/// Extract the volume object identifiers recorded in an NX superblock.
pub fn container_volume_oids(superblock: &[u8]) -> RecoveryResult<Vec<u64>> {
    parse_container_superblock(superblock)?;
    if superblock.len() < NX_FS_OID_OFFSET + NX_FS_OID_SLOTS * 8 {
        return Err(RecoveryError::LengthTooLarge {
            length: superblock.len() as u64,
        });
    }
    let advertised = u32::from_le_bytes(
        superblock[NX_MAX_FILE_SYSTEMS_OFFSET..NX_MAX_FILE_SYSTEMS_OFFSET + 4]
            .try_into()
            .expect("fixed APFS field"),
    ) as usize;
    let count = advertised.min(NX_FS_OID_SLOTS);
    let mut result = Vec::with_capacity(count);
    for index in 0..count {
        let offset = NX_FS_OID_OFFSET + index * 8;
        let oid = u64::from_le_bytes(
            superblock[offset..offset + 8]
                .try_into()
                .expect("fixed APFS field"),
        );
        if oid != 0 {
            result.push(oid);
        }
    }
    Ok(result)
}

/// Resolve a volume superblock OID through the container object map.
pub fn read_volume_superblock<D: BlockDevice>(
    device: &D,
    range: ByteRange,
    container: &ApfsContainer,
    volume_oid: u64,
    xid: u64,
) -> RecoveryResult<ApfsVolume> {
    if container.omap_oid == 0 {
        return Err(RecoveryError::IoFailure(
            "APFS container has no object-map OID".into(),
        ));
    }
    let omap_block = read_object(device, range, container, container.omap_oid)?;
    let omap = parse_object_map(&omap_block)?;
    let mapping = lookup_object_map(
        device,
        range,
        container,
        &omap,
        ApfsObjectMapKey {
            oid: volume_oid,
            xid,
        },
    )?;
    if mapping.is_deleted() {
        return Err(RecoveryError::IoFailure(
            "APFS volume object-map mapping is deleted".into(),
        ));
    }
    if mapping.physical_address >= container.block_count {
        return Err(RecoveryError::OutOfRange {
            offset: mapping.physical_address,
            length: 1,
            capacity: container.block_count,
        });
    }
    let block = read_object(device, range, container, mapping.physical_address)?;
    let header = parse_object_header(&block)?;
    if header.oid != volume_oid {
        return Err(RecoveryError::IoFailure(
            "resolved APFS volume object has unexpected OID".into(),
        ));
    }
    parse_volume_superblock(&block)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApfsDiscoveredVolume {
    pub object_id: u64,
    pub xid: u64,
    pub volume: ApfsVolume,
}

/// Discover all live APFS volumes from the newest checkpoint superblock.
/// The checkpoint's transaction ID is used for container OMAP version lookup,
/// so callers no longer need to guess an XID. A stale/deleted volume mapping
/// is skipped rather than aborting discovery of other volumes.
pub fn discover_volumes<D: BlockDevice>(
    device: &D,
    range: ByteRange,
) -> RecoveryResult<(ApfsContainer, Vec<ApfsDiscoveredVolume>)> {
    let (container, superblock) = read_latest_container_superblock_with_block(device, range)?;
    let header = parse_object_header(&superblock)?;
    if header.oid == 0 || header.xid == 0 {
        return Err(RecoveryError::IoFailure(
            "selected APFS container superblock has invalid object identity".into(),
        ));
    }
    let volume_oids = container_volume_oids(&superblock)?;
    let mut volumes = Vec::with_capacity(volume_oids.len());
    for object_id in volume_oids {
        match read_volume_superblock(device, range, &container, object_id, header.xid) {
            Ok(volume) => volumes.push(ApfsDiscoveredVolume {
                object_id,
                xid: header.xid,
                volume,
            }),
            Err(RecoveryError::IoFailure(message)) if message.contains("mapping is deleted") => {
                continue;
            }
            Err(RecoveryError::OutOfRange { .. }) => continue,
            Err(error) => return Err(error),
        }
    }
    Ok((container, volumes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_advertised_volume_oids_without_treating_them_as_addresses() {
        let mut block = vec![0u8; 1024];
        block[32..36].copy_from_slice(&0x4253_584eu32.to_le_bytes());
        block[36..40].copy_from_slice(&4096u32.to_le_bytes());
        block[40..48].copy_from_slice(&1024u64.to_le_bytes());
        block[176..184].copy_from_slice(&10u64.to_le_bytes());
        block[NX_MAX_FILE_SYSTEMS_OFFSET..NX_MAX_FILE_SYSTEMS_OFFSET + 4]
            .copy_from_slice(&3u32.to_le_bytes());
        block[NX_FS_OID_OFFSET..NX_FS_OID_OFFSET + 8].copy_from_slice(&101u64.to_le_bytes());
        block[NX_FS_OID_OFFSET + 8..NX_FS_OID_OFFSET + 16].copy_from_slice(&202u64.to_le_bytes());
        block[NX_FS_OID_OFFSET + 16..NX_FS_OID_OFFSET + 24].copy_from_slice(&303u64.to_le_bytes());
        assert_eq!(container_volume_oids(&block).unwrap(), vec![101, 202, 303]);
    }

    #[test]
    fn zero_volume_slots_are_ignored() {
        let mut block = vec![0u8; 1024];
        block[32..36].copy_from_slice(&0x4253_584eu32.to_le_bytes());
        block[36..40].copy_from_slice(&4096u32.to_le_bytes());
        block[40..48].copy_from_slice(&1024u64.to_le_bytes());
        block[176..184].copy_from_slice(&10u64.to_le_bytes());
        block[NX_MAX_FILE_SYSTEMS_OFFSET..NX_MAX_FILE_SYSTEMS_OFFSET + 4]
            .copy_from_slice(&2u32.to_le_bytes());
        block[NX_FS_OID_OFFSET + 8..NX_FS_OID_OFFSET + 16].copy_from_slice(&55u64.to_le_bytes());
        assert_eq!(container_volume_oids(&block).unwrap(), vec![55]);
    }
}
