use recovery_core::{ByteRange, RecoveryError, RecoveryResult};
use storage_io::BlockDevice;

use crate::{btree_variable_entries, lookup_volume_object, read_object, ApfsContainer, ApfsObjectMapKey, ApfsVariableBtreeEntry, ApfsVolume};

const MAX_DEPTH: usize = 64;

/// An owned raw catalog record. Catalog keys are record-type dependent, so
/// decoding remains the responsibility of the filesystem-record layer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApfsCatalogRecord {
    pub key: Vec<u8>,
    pub value: Vec<u8>,
}

fn child_oid(entry: &ApfsVariableBtreeEntry<'_>) -> RecoveryResult<u64> {
    if entry.value.len() != 8 {
        return Err(RecoveryError::IoFailure("APFS catalog branch value must be an object ID".into()));
    }
    Ok(u64::from_le_bytes(entry.value.try_into().expect("validated APFS child OID")))
}

/// Traverse every reachable catalog B-tree child when node references are
/// already physical block numbers.
pub fn read_catalog_records<D: BlockDevice>(
    device: &D,
    range: ByteRange,
    container: &ApfsContainer,
    root_oid: u64,
) -> RecoveryResult<Vec<ApfsCatalogRecord>> {
    fn walk<D: BlockDevice>(
        device: &D,
        range: ByteRange,
        container: &ApfsContainer,
        node_oid: u64,
        depth: usize,
        visited: &mut Vec<u64>,
        out: &mut Vec<ApfsCatalogRecord>,
    ) -> RecoveryResult<()> {
        if depth >= MAX_DEPTH {
            return Err(RecoveryError::IoFailure("APFS catalog tree exceeds depth limit".into()));
        }
        if node_oid >= container.block_count {
            return Err(RecoveryError::OutOfRange { offset: node_oid, length: 1, capacity: container.block_count });
        }
        if visited.contains(&node_oid) {
            return Err(RecoveryError::IoFailure("APFS catalog tree contains a cycle".into()));
        }
        visited.push(node_oid);
        let block = read_object(device, range, container, node_oid)?;
        let node = crate::parse_btree_node(&block)?;
        let entries = btree_variable_entries(&block)?;
        if entries.is_empty() {
            return Err(RecoveryError::IoFailure("APFS catalog node has no entries".into()));
        }
        if node.is_leaf() {
            out.extend(entries.into_iter().map(|entry| ApfsCatalogRecord {
                key: entry.key.to_vec(),
                value: entry.value.to_vec(),
            }));
        } else {
            for entry in entries {
                let child = child_oid(&entry)?;
                walk(device, range, container, child, depth + 1, visited, out)?;
            }
        }
        visited.pop();
        Ok(())
    }

    let mut visited = Vec::new();
    let mut records = Vec::new();
    walk(device, range, container, root_oid, 0, &mut visited, &mut records)?;
    Ok(records)
}

/// Traverse a volume catalog while resolving every virtual B-tree node
/// reference through the volume object map. APFS catalog branch values are
/// object IDs, not guaranteed physical block addresses.
pub fn read_volume_catalog_records<D: BlockDevice>(
    device: &D,
    range: ByteRange,
    container: &ApfsContainer,
    volume: &ApfsVolume,
    xid: u64,
) -> RecoveryResult<Vec<ApfsCatalogRecord>> {
    fn resolve_node<D: BlockDevice>(
        device: &D,
        range: ByteRange,
        container: &ApfsContainer,
        volume: &ApfsVolume,
        oid: u64,
        xid: u64,
    ) -> RecoveryResult<u64> {
        let mapping = lookup_volume_object(device, range, container, volume, ApfsObjectMapKey { oid, xid })?;
        if mapping.is_deleted() {
            return Err(RecoveryError::IoFailure("APFS catalog node mapping is deleted".into()));
        }
        if mapping.physical_address >= container.block_count {
            return Err(RecoveryError::OutOfRange { offset: mapping.physical_address, length: 1, capacity: container.block_count });
        }
        Ok(mapping.physical_address)
    }

    fn walk<D: BlockDevice>(
        device: &D,
        range: ByteRange,
        container: &ApfsContainer,
        volume: &ApfsVolume,
        node_oid: u64,
        xid: u64,
        depth: usize,
        visited: &mut Vec<u64>,
        out: &mut Vec<ApfsCatalogRecord>,
    ) -> RecoveryResult<()> {
        if depth >= MAX_DEPTH {
            return Err(RecoveryError::IoFailure("APFS catalog tree exceeds depth limit".into()));
        }
        if visited.contains(&node_oid) {
            return Err(RecoveryError::IoFailure("APFS catalog tree contains a cycle".into()));
        }
        visited.push(node_oid);
        let physical = resolve_node(device, range, container, volume, node_oid, xid)?;
        let block = read_object(device, range, container, physical)?;
        let node = crate::parse_btree_node(&block)?;
        let entries = btree_variable_entries(&block)?;
        if entries.is_empty() {
            return Err(RecoveryError::IoFailure("APFS catalog node has no entries".into()));
        }
        if node.is_leaf() {
            out.extend(entries.into_iter().map(|entry| ApfsCatalogRecord {
                key: entry.key.to_vec(),
                value: entry.value.to_vec(),
            }));
        } else {
            for entry in entries {
                walk(device, range, container, volume, child_oid(&entry)?, xid, depth + 1, visited, out)?;
            }
        }
        visited.pop();
        Ok(())
    }

    if volume.root_tree_oid == 0 {
        return Err(RecoveryError::IoFailure("APFS volume has no root-tree OID".into()));
    }
    let mut visited = Vec::new();
    let mut records = Vec::new();
    walk(device, range, container, volume, volume.root_tree_oid, xid, 0, &mut visited, &mut records)?;
    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_oid_branch_values() {
        let data = ApfsVariableBtreeEntry { key: &[0u8; 8], value: &[0u8; 7] };
        assert!(child_oid(&data).is_err());
    }

    #[test]
    fn decodes_little_endian_child_oid() {
        let oid = 0x1122_3344_5566_7788u64;
        let bytes = oid.to_le_bytes();
        let data = ApfsVariableBtreeEntry { key: &[0u8; 8], value: &bytes };
        assert_eq!(child_oid(&data).unwrap(), oid);
    }
}
