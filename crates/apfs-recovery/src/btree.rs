use recovery_core::{RecoveryError, RecoveryResult};

const NODE_HEADER_LEN: usize = 56;
const KV_OFFSET_LEN: usize = 4;
const FLAG_LEAF: u16 = 0x0001;
const FLAG_ROOT: u16 = 0x0002;
const FLAG_FIXED_KV_SIZE: u16 = 0x0004;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ApfsBtreeNode {
    pub flags: u16,
    pub level: u16,
    pub key_count: u32,
    pub table_space_offset: u16,
    pub table_space_length: u16,
    pub free_space_offset: u16,
    pub free_space_length: u16,
    pub key_free_list_offset: u16,
    pub key_free_list_length: u16,
    pub value_free_list_offset: u16,
    pub value_free_list_length: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ApfsBtreeEntry {
    pub key_offset: u16,
    pub value_offset: u16,
}

impl ApfsBtreeNode {
    pub fn is_leaf(self) -> bool { self.flags & FLAG_LEAF != 0 }
    pub fn is_root(self) -> bool { self.flags & FLAG_ROOT != 0 }
    pub fn has_fixed_kv_size(self) -> bool { self.flags & FLAG_FIXED_KV_SIZE != 0 }
}

fn u16_at(data: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(data[offset..offset + 2].try_into().expect("fixed APFS field"))
}

fn u32_at(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(data[offset..offset + 4].try_into().expect("fixed APFS field"))
}

fn checked_end(offset: usize, length: usize, capacity: usize) -> RecoveryResult<usize> {
    let end = offset.checked_add(length).ok_or(RecoveryError::RangeOverflow)?;
    if end > capacity {
        return Err(RecoveryError::OutOfRange { offset: offset as u64, length: length as u64, capacity: capacity as u64 });
    }
    Ok(end)
}

pub fn parse_btree_node(data: &[u8]) -> RecoveryResult<ApfsBtreeNode> {
    if data.len() < NODE_HEADER_LEN {
        return Err(RecoveryError::LengthTooLarge { length: data.len() as u64 });
    }
    let node = ApfsBtreeNode {
        flags: u16_at(data, 32), level: u16_at(data, 34), key_count: u32_at(data, 36),
        table_space_offset: u16_at(data, 40), table_space_length: u16_at(data, 42),
        free_space_offset: u16_at(data, 44), free_space_length: u16_at(data, 46),
        key_free_list_offset: u16_at(data, 48), key_free_list_length: u16_at(data, 50),
        value_free_list_offset: u16_at(data, 52), value_free_list_length: u16_at(data, 54),
    };
    let table_start = usize::from(node.table_space_offset);
    let table_len = usize::from(node.table_space_length);
    checked_end(table_start, table_len, data.len())?;
    let entries_len = usize::try_from(node.key_count).ok()
        .and_then(|count| count.checked_mul(KV_OFFSET_LEN)).ok_or(RecoveryError::RangeOverflow)?;
    checked_end(NODE_HEADER_LEN, entries_len, data.len())?;
    if node.key_count != 0 && table_start < NODE_HEADER_LEN {
        return Err(RecoveryError::IoFailure("APFS B-tree table space overlaps node header".into()));
    }
    Ok(node)
}

pub fn btree_entries(data: &[u8]) -> RecoveryResult<Vec<ApfsBtreeEntry>> {
    let node = parse_btree_node(data)?;
    let count = usize::try_from(node.key_count).map_err(|_| RecoveryError::RangeOverflow)?;
    let table_start = usize::from(node.table_space_offset);
    let table_end = checked_end(table_start, usize::from(node.table_space_length), data.len())?;
    let mut entries = Vec::with_capacity(count);
    for index in 0..count {
        let pos = NODE_HEADER_LEN.checked_add(index.checked_mul(KV_OFFSET_LEN).ok_or(RecoveryError::RangeOverflow)?).ok_or(RecoveryError::RangeOverflow)?;
        let key_offset = u16_at(data, pos);
        let value_offset = u16_at(data, pos + 2);
        if usize::from(key_offset) < table_start || usize::from(key_offset) >= table_end {
            return Err(RecoveryError::IoFailure("APFS B-tree key offset is outside table space".into()));
        }
        if usize::from(value_offset) < table_start || usize::from(value_offset) > table_end {
            return Err(RecoveryError::IoFailure("APFS B-tree value offset is outside table space".into()));
        }
        if value_offset < key_offset {
            return Err(RecoveryError::IoFailure("APFS B-tree value precedes key".into()));
        }
        entries.push(ApfsBtreeEntry { key_offset, value_offset });
    }
    Ok(entries)
}

pub fn btree_key<'a>(data: &'a [u8], entry: ApfsBtreeEntry, _next: Option<ApfsBtreeEntry>) -> RecoveryResult<&'a [u8]> {
    let start = usize::from(entry.key_offset);
    let end = usize::from(entry.value_offset);
    if end < start || end > data.len() {
        return Err(RecoveryError::IoFailure("APFS B-tree key range is invalid".into()));
    }
    Ok(&data[start..end])
}

#[cfg(test)]
mod tests {
    use super::*;
    fn node() -> Vec<u8> {
        let mut data = vec![0u8; 512];
        data[32..34].copy_from_slice(&(FLAG_LEAF | FLAG_ROOT).to_le_bytes());
        data[34..36].copy_from_slice(&1u16.to_le_bytes());
        data[36..40].copy_from_slice(&2u32.to_le_bytes());
        data[40..42].copy_from_slice(&64u16.to_le_bytes());
        data[42..44].copy_from_slice(&128u16.to_le_bytes());
        data
    }
    #[test]
    fn parses_node_header_and_flags() {
        let parsed = parse_btree_node(&node()).unwrap();
        assert!(parsed.is_leaf()); assert!(parsed.is_root()); assert_eq!(parsed.level, 1); assert_eq!(parsed.key_count, 2);
    }
    #[test]
    fn parses_and_bounds_entries() {
        let mut data = node();
        data[56..58].copy_from_slice(&64u16.to_le_bytes()); data[58..60].copy_from_slice(&72u16.to_le_bytes());
        data[60..62].copy_from_slice(&80u16.to_le_bytes()); data[62..64].copy_from_slice(&88u16.to_le_bytes());
        let entries = btree_entries(&data).unwrap();
        assert_eq!(entries, vec![ApfsBtreeEntry { key_offset: 64, value_offset: 72 }, ApfsBtreeEntry { key_offset: 80, value_offset: 88 }]);
        assert_eq!(btree_key(&data, entries[0], Some(entries[1])).unwrap(), &data[64..72]);
    }
    #[test]
    fn rejects_entry_outside_table() {
        let mut data = node(); data[56..58].copy_from_slice(&300u16.to_le_bytes()); data[58..60].copy_from_slice(&72u16.to_le_bytes());
        assert!(btree_entries(&data).is_err());
    }
    #[test]
    fn rejects_value_before_key() {
        let mut data = node(); data[56..58].copy_from_slice(&100u16.to_le_bytes()); data[58..60].copy_from_slice(&90u16.to_le_bytes());
        assert!(btree_entries(&data).is_err());
    }
    #[test]
    fn rejects_truncated_node() { assert!(parse_btree_node(&[0u8; 55]).is_err()); }
}
