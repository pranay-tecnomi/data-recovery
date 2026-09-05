use recovery_core::{RecoveryError, RecoveryResult};

const OMAP_HEADER_LEN: usize = 96;
const OMAP_TREE_TYPE_MASK: u32 = 0x0000_ffff;
const OBJECT_TYPE_BTREE: u32 = 0x0000_0002;
const OMAP_VAL_DELETED: u32 = 0x0000_0001;
const OMAP_VAL_SAVED: u32 = 0x0000_0002;
const OMAP_VAL_ENCRYPTED: u32 = 0x0000_0004;
const OMAP_VAL_NOHEADER: u32 = 0x0000_0008;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApfsObjectMap {
    pub flags: u32,
    pub snapshot_count: u32,
    pub tree_type: u32,
    pub snapshot_tree_type: u32,
    pub tree_oid: u64,
    pub snapshot_tree_oid: u64,
    pub most_recent_snapshot: u64,
    pub pending_revert_min: u64,
    pub pending_revert_max: u64,
    pub min_oid: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ApfsObjectMapKey {
    pub oid: u64,
    pub xid: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ApfsObjectMapValue {
    pub flags: u32,
    pub size: u32,
    pub physical_address: u64,
}

impl ApfsObjectMapValue {
    pub fn is_deleted(self) -> bool { self.flags & OMAP_VAL_DELETED != 0 }
    pub fn is_saved(self) -> bool { self.flags & OMAP_VAL_SAVED != 0 }
    pub fn is_encrypted(self) -> bool { self.flags & OMAP_VAL_ENCRYPTED != 0 }
    pub fn has_no_header(self) -> bool { self.flags & OMAP_VAL_NOHEADER != 0 }
}

fn u32_at(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(data[offset..offset + 4].try_into().expect("fixed APFS field"))
}

fn u64_at(data: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(data[offset..offset + 8].try_into().expect("fixed APFS field"))
}

pub fn parse_object_map(data: &[u8]) -> RecoveryResult<ApfsObjectMap> {
    if data.len() < OMAP_HEADER_LEN {
        return Err(RecoveryError::LengthTooLarge { length: data.len() as u64 });
    }
    let tree_type = u32_at(data, 40);
    let snapshot_tree_type = u32_at(data, 44);
    if tree_type & OMAP_TREE_TYPE_MASK != OBJECT_TYPE_BTREE {
        return Err(RecoveryError::IoFailure("APFS object map has invalid tree type".into()));
    }
    if snapshot_tree_type & OMAP_TREE_TYPE_MASK != OBJECT_TYPE_BTREE {
        return Err(RecoveryError::IoFailure("APFS object map has invalid snapshot tree type".into()));
    }
    let tree_oid = u64_at(data, 48);
    if tree_oid == 0 {
        return Err(RecoveryError::IoFailure("APFS object map has no tree root".into()));
    }
    Ok(ApfsObjectMap {
        flags: u32_at(data, 32),
        snapshot_count: u32_at(data, 36),
        tree_type,
        snapshot_tree_type,
        tree_oid,
        snapshot_tree_oid: u64_at(data, 56),
        most_recent_snapshot: u64_at(data, 64),
        pending_revert_min: u64_at(data, 72),
        pending_revert_max: u64_at(data, 80),
        min_oid: u64_at(data, 88),
    })
}

pub fn parse_object_map_key(data: &[u8]) -> RecoveryResult<ApfsObjectMapKey> {
    if data.len() < 16 {
        return Err(RecoveryError::LengthTooLarge { length: data.len() as u64 });
    }
    Ok(ApfsObjectMapKey { oid: u64_at(data, 0), xid: u64_at(data, 8) })
}

pub fn parse_object_map_value(data: &[u8]) -> RecoveryResult<ApfsObjectMapValue> {
    if data.len() < 16 {
        return Err(RecoveryError::LengthTooLarge { length: data.len() as u64 });
    }
    Ok(ApfsObjectMapValue {
        flags: u32_at(data, 0),
        size: u32_at(data, 4),
        physical_address: u64_at(data, 8),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_object_map_metadata() {
        let mut data = vec![0u8; 96];
        data[32..36].copy_from_slice(&7u32.to_le_bytes());
        data[36..40].copy_from_slice(&3u32.to_le_bytes());
        data[40..44].copy_from_slice(&0x4000_0002u32.to_le_bytes());
        data[44..48].copy_from_slice(&0x8000_0002u32.to_le_bytes());
        data[48..56].copy_from_slice(&42u64.to_le_bytes());
        data[56..64].copy_from_slice(&43u64.to_le_bytes());
        data[64..72].copy_from_slice(&44u64.to_le_bytes());
        data[72..80].copy_from_slice(&45u64.to_le_bytes());
        data[80..88].copy_from_slice(&46u64.to_le_bytes());
        data[88..96].copy_from_slice(&47u64.to_le_bytes());
        let omap = parse_object_map(&data).unwrap();
        assert_eq!(omap.tree_oid, 42);
        assert_eq!(omap.snapshot_tree_oid, 43);
        assert_eq!(omap.min_oid, 47);
    }

    #[test]
    fn parses_key_and_value() {
        let mut key = [0u8; 16];
        key[0..8].copy_from_slice(&9u64.to_le_bytes());
        key[8..16].copy_from_slice(&10u64.to_le_bytes());
        assert_eq!(parse_object_map_key(&key).unwrap(), ApfsObjectMapKey { oid: 9, xid: 10 });

        let mut value = [0u8; 16];
        value[0..4].copy_from_slice(&OMAP_VAL_DELETED.to_le_bytes());
        value[4..8].copy_from_slice(&4096u32.to_le_bytes());
        value[8..16].copy_from_slice(&99u64.to_le_bytes());
        let parsed = parse_object_map_value(&value).unwrap();
        assert!(parsed.is_deleted());
        assert_eq!(parsed.size, 4096);
        assert_eq!(parsed.physical_address, 99);
    }

    #[test]
    fn rejects_invalid_tree_type_and_zero_root() {
        let mut data = vec![0u8; 96];
        data[40..44].copy_from_slice(&1u32.to_le_bytes());
        data[44..48].copy_from_slice(&2u32.to_le_bytes());
        assert!(parse_object_map(&data).is_err());
        data[40..44].copy_from_slice(&0x4000_0002u32.to_le_bytes());
        data[44..48].copy_from_slice(&0x4000_0002u32.to_le_bytes());
        assert!(parse_object_map(&data).is_err());
    }
}
