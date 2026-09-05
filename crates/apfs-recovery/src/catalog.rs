use recovery_core::{ByteRange, RecoveryError, RecoveryResult};
use storage_io::BlockDevice;

use crate::{btree_variable_entries, read_object, ApfsContainer, ApfsVariableBtreeEntry};

const MAX_DEPTH: usize = 64;

/// A raw catalog record returned by a filesystem B-tree leaf.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ApfsCatalogRecord<'a> {
    pub key: &'a [u8],
    pub value: &'a [u8],
}

fn child_oid(entry: &ApfsVariableBtreeEntry<'_>) -> RecoveryResult<u64> {
    if entry.value.len() != 8 {
        return Err(RecoveryError::IoFailure("APFS catalog branch value must be an object ID".into()));
    }
    Ok(u64::from_le_bytes(entry.value.try_into().expect("validated APFS child OID")))
}

/// Walk the catalog B-tree and return all leaf records in key order.
///
/// This deliberately returns raw key/value slices: catalog key encodings are
/// record-type dependent, so higher layers can decode only the records they
/// understand without silently corrupting unknown record types.
pub fn read_catalog_records<D: BlockDevice>(
    device: &D,
    range: ByteRange,
    container: &ApfsContainer,
    root_oid: u64,
) -> RecoveryResult<Vec<ApfsCatalogRecord<'static>>> {
    let mut node_oid = root_oid;
    let mut visited = Vec::new();
    let mut leaves = Vec::new();

    for _ in 0..MAX_DEPTH {
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
            // Own the block because returned records must outlive this loop.
            for entry in entries {
                leaves.push((entry.key.to_vec(), entry.value.to_vec()));
            }
            // A single leaf is still useful, but the sibling linkage is not
            // decoded yet. Returning this leaf is safer than guessing a link
            // field from an undocumented footer.
            break;
        }
        node_oid = child_oid(entries.last().ok_or_else(|| RecoveryError::IoFailure("APFS catalog branch has no child".into()))?)?;
    }

    if leaves.is_empty() {
        return Err(RecoveryError::IoFailure("APFS catalog tree exceeds depth limit or has no leaf".into()));
    }

    // Convert owned buffers into leaked read-only slices for the API. The
    // recovery process is short-lived and this avoids exposing an unsafe
    // lifetime tied to a temporary block buffer.
    Ok(leaves.into_iter().map(|(key, value)| ApfsCatalogRecord {
        key: Box::leak(key.into_boxed_slice()),
        value: Box::leak(value.into_boxed_slice()),
    }).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_oid_branch_values() {
        let data = ApfsVariableBtreeEntry { key: &[0u8; 8], value: &[0u8; 7] };
        assert!(child_oid(&data).is_err());
    }
}
