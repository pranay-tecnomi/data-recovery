use recovery_core::{ByteRange, RecoveryError, RecoveryResult};
use storage_io::BlockDevice;

use crate::{btree_fixed_entries, read_object, ApfsContainer, ApfsObjectMap, ApfsObjectMapKey, ApfsObjectMapValue};

const OMAP_KEY_SIZE: usize = 16;
const OMAP_LEAF_VALUE_SIZE: usize = 16;
const OMAP_BRANCH_VALUE_SIZE: usize = 8;
const MAX_DEPTH: usize = 64;

fn u64_at(data: &[u8]) -> u64 {
    u64::from_le_bytes(data.try_into().expect("fixed APFS integer"))
}

fn compare_key(bytes: &[u8], target: ApfsObjectMapKey) -> RecoveryResult<std::cmp::Ordering> {
    if bytes.len() != OMAP_KEY_SIZE {
        return Err(RecoveryError::IoFailure("APFS object-map key has unexpected size".into()));
    }
    let key = ApfsObjectMapKey {
        oid: u64::from_le_bytes(bytes[0..8].try_into().expect("fixed APFS key")),
        xid: u64::from_le_bytes(bytes[8..16].try_into().expect("fixed APFS key")),
    };
    Ok((key.oid, key.xid).cmp(&(target.oid, target.xid)))
}

pub fn lookup_object_map<D: BlockDevice>(
    device: &D,
    range: ByteRange,
    container: &ApfsContainer,
    omap: &ApfsObjectMap,
    target: ApfsObjectMapKey,
) -> RecoveryResult<ApfsObjectMapValue> {
    if target.oid < omap.min_oid {
        return Err(RecoveryError::IoFailure("APFS object-map lookup is below minimum OID".into()));
    }
    if omap.tree_oid >= container.block_count {
        return Err(RecoveryError::OutOfRange { offset: omap.tree_oid, length: 1, capacity: container.block_count });
    }
    let mut node_oid = omap.tree_oid;
    let mut visited = Vec::new();
    for _depth in 0..MAX_DEPTH {
        if visited.contains(&node_oid) {
            return Err(RecoveryError::IoFailure("APFS object-map tree contains a cycle".into()));
        }
        visited.push(node_oid);
        let block = read_object(device, range, container, node_oid)?;
        let node = crate::parse_btree_node(&block)?;
        if !node.has_fixed_kv_size() {
            return Err(RecoveryError::IoFailure("APFS object-map node is not fixed-size".into()));
        }
        let value_size = if node.is_leaf() { OMAP_LEAF_VALUE_SIZE } else { OMAP_BRANCH_VALUE_SIZE };
        let entries = btree_fixed_entries(&block, OMAP_KEY_SIZE, value_size)?;
        if entries.is_empty() {
            return Err(RecoveryError::IoFailure("APFS object-map node has no entries".into()));
        }
        if node.is_leaf() {
            let mut best: Option<ApfsObjectMapValue> = None;
            for entry in entries {
                match compare_key(entry.key, target)? {
                    std::cmp::Ordering::Equal => {
                        return Ok(ApfsObjectMapValue {
                            flags: u32::from_le_bytes(entry.value[0..4].try_into().expect("fixed APFS value")),
                            size: u32::from_le_bytes(entry.value[4..8].try_into().expect("fixed APFS value")),
                            physical_address: u64_at(&entry.value[8..16]),
                        });
                    }
                    std::cmp::Ordering::Less => {}
                    std::cmp::Ordering::Greater => break,
                }
                if compare_key(entry.key, target)? == std::cmp::Ordering::Less {
                    let value = ApfsObjectMapValue {
                        flags: u32::from_le_bytes(entry.value[0..4].try_into().expect("fixed APFS value")),
                        size: u32::from_le_bytes(entry.value[4..8].try_into().expect("fixed APFS value")),
                        physical_address: u64_at(&entry.value[8..16]),
                    };
                    best = Some(value);
                }
            }
            return best.ok_or_else(|| RecoveryError::IoFailure("APFS object-map key is not present".into()));
        }
        let mut child = None;
        for entry in entries {
            if compare_key(entry.key, target)? != std::cmp::Ordering::Greater {
                child = Some(u64_at(entry.value));
            } else {
                break;
            }
        }
        node_oid = child.ok_or_else(|| RecoveryError::IoFailure("APFS object-map lookup has no child for target".into()))?;
        if node_oid >= container.block_count {
            return Err(RecoveryError::OutOfRange { offset: node_oid, length: 1, capacity: container.block_count });
        }
    }
    Err(RecoveryError::IoFailure("APFS object-map tree exceeds depth limit".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compares_object_map_keys_in_oid_then_xid_order() {
        assert_eq!(compare_key(&[1u8; 16], ApfsObjectMapKey { oid: 2, xid: 1 }).unwrap(), std::cmp::Ordering::Less);
        let mut key = [0u8; 16];
        key[0..8].copy_from_slice(&7u64.to_le_bytes());
        key[8..16].copy_from_slice(&9u64.to_le_bytes());
        assert_eq!(compare_key(&key, ApfsObjectMapKey { oid: 7, xid: 9 }).unwrap(), std::cmp::Ordering::Equal);
    }
}
