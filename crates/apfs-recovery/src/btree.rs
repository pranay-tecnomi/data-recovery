use recovery_core::{RecoveryError, RecoveryResult};

const NODE_HEADER_LEN: usize = 56;
const BTREE_FOOTER_LEN: usize = 40;
const KVOFF_LEN: usize = 4;
const KVLOC_LEN: usize = 8;
const FLAG_ROOT: u16 = 0x0001;
const FLAG_LEAF: u16 = 0x0002;
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ApfsBtreeVariableEntry {
    pub key_offset: u16,
    pub key_length: u16,
    pub value_offset: u16,
    pub value_length: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ApfsFixedBtreeEntry<'a> {
    pub key: &'a [u8],
    pub value: &'a [u8],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ApfsVariableBtreeEntry<'a> {
    pub key: &'a [u8],
    pub value: &'a [u8],
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

fn table_bounds(data: &[u8], node: ApfsBtreeNode) -> RecoveryResult<(usize, usize)> {
    let start = NODE_HEADER_LEN.checked_add(usize::from(node.table_space_offset)).ok_or(RecoveryError::RangeOverflow)?;
    let end = checked_end(start, usize::from(node.table_space_length), data.len())?;
    Ok((start, end))
}

fn kv_bounds(data: &[u8], node: ApfsBtreeNode) -> RecoveryResult<(usize, usize)> {
    let (_, table_end) = table_bounds(data, node)?;
    let end = if node.is_root() {
        data.len().checked_sub(BTREE_FOOTER_LEN).ok_or(RecoveryError::RangeOverflow)?
    } else {
        data.len()
    };
    if table_end > end {
        return Err(RecoveryError::IoFailure("APFS B-tree key/value area is invalid".into()));
    }
    Ok((table_end, end))
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
    let (_, table_len) = table_bounds(data, node)?;
    let entries_len = usize::try_from(node.key_count).ok()
        .and_then(|count| count.checked_mul(if node.has_fixed_kv_size() { KVOFF_LEN } else { KVLOC_LEN }))
        .ok_or(RecoveryError::RangeOverflow)?;
    if entries_len > table_len.saturating_sub(NODE_HEADER_LEN + usize::from(node.table_space_offset)) {
        return Err(RecoveryError::IoFailure("APFS B-tree table space cannot contain all entries".into()));
    }
    Ok(node)
}

pub fn btree_entries(data: &[u8]) -> RecoveryResult<Vec<ApfsBtreeEntry>> {
    let node = parse_btree_node(data)?;
    if !node.has_fixed_kv_size() {
        return Err(RecoveryError::IoFailure("APFS B-tree node uses variable-size entries".into()));
    }
    let count = usize::try_from(node.key_count).map_err(|_| RecoveryError::RangeOverflow)?;
    let (table_start, table_end) = table_bounds(data, node)?;
    let mut entries = Vec::with_capacity(count);
    for index in 0..count {
        let pos = table_start.checked_add(index.checked_mul(KVOFF_LEN).ok_or(RecoveryError::RangeOverflow)?).ok_or(RecoveryError::RangeOverflow)?;
        if pos.checked_add(KVOFF_LEN).ok_or(RecoveryError::RangeOverflow)? > table_end { return Err(RecoveryError::OutOfRange { offset: pos as u64, length: KVOFF_LEN as u64, capacity: table_end as u64 }); }
        entries.push(ApfsBtreeEntry { key_offset: u16_at(data, pos), value_offset: u16_at(data, pos + 2) });
    }
    Ok(entries)
}

pub fn btree_variable_entries(data: &[u8]) -> RecoveryResult<Vec<ApfsBtreeVariableEntry>> {
    let node = parse_btree_node(data)?;
    if node.has_fixed_kv_size() {
        return Err(RecoveryError::IoFailure("APFS B-tree node uses fixed-size entries".into()));
    }
    let count = usize::try_from(node.key_count).map_err(|_| RecoveryError::RangeOverflow)?;
    let (table_start, table_end) = table_bounds(data, node)?;
    let (kv_start, value_end) = kv_bounds(data, node)?;
    let mut entries = Vec::with_capacity(count);
    for index in 0..count {
        let pos = table_start.checked_add(index.checked_mul(KVLOC_LEN).ok_or(RecoveryError::RangeOverflow)?).ok_or(RecoveryError::RangeOverflow)?;
        if pos.checked_add(KVLOC_LEN).ok_or(RecoveryError::RangeOverflow)? > table_end { return Err(RecoveryError::OutOfRange { offset: pos as u64, length: KVLOC_LEN as u64, capacity: table_end as u64 }); }
        let key_offset = u16_at(data, pos);
        let key_length = u16_at(data, pos + 2);
        let value_offset = u16_at(data, pos + 4);
        let value_length = u16_at(data, pos + 6);
        let key_start = kv_start.checked_add(usize::from(key_offset)).ok_or(RecoveryError::RangeOverflow)?;
        let key_end = checked_end(key_start, usize::from(key_length), value_end)?;
        let value_from_end = usize::from(value_offset);
        let value_end_for_entry = value_end.checked_sub(value_from_end).ok_or(RecoveryError::IoFailure("APFS B-tree value offset underflows node".into()))?;
        let value_start = value_end_for_entry.checked_sub(usize::from(value_length)).ok_or(RecoveryError::IoFailure("APFS B-tree value length exceeds node".into()))?;
        if value_start < key_end || value_end_for_entry > value_end {
            return Err(RecoveryError::IoFailure("APFS B-tree key/value ranges overlap or exceed node".into()));
        }
        entries.push(ApfsVariableBtreeEntry { key: &data[key_start..key_end], value: &data[value_start..value_end_for_entry] });
    }
    Ok(entries)
}

pub fn btree_fixed_entries<'a>(data: &'a [u8], key_size: usize, value_size: usize) -> RecoveryResult<Vec<ApfsFixedBtreeEntry<'a>>> {
    let node = parse_btree_node(data)?;
    if !node.has_fixed_kv_size() { return Err(RecoveryError::IoFailure("APFS B-tree node is not fixed-size".into())); }
    if key_size == 0 || value_size == 0 { return Err(RecoveryError::IoFailure("APFS B-tree fixed key/value sizes must be nonzero".into())); }
    let entries = btree_entries(data)?;
    let (kv_start, value_end) = kv_bounds(data, node)?;
    let mut result = Vec::with_capacity(entries.len());
    for entry in entries {
        let key_start = kv_start.checked_add(usize::from(entry.key_offset)).ok_or(RecoveryError::RangeOverflow)?;
        let key_end = checked_end(key_start, key_size, value_end)?;
        let value_end_for_entry = value_end.checked_sub(usize::from(entry.value_offset)).ok_or(RecoveryError::IoFailure("APFS B-tree value offset underflows node".into()))?;
        let value_start = value_end_for_entry.checked_sub(value_size).ok_or(RecoveryError::IoFailure("APFS B-tree value size exceeds node".into()))?;
        if value_start < key_end || value_end_for_entry > value_end { return Err(RecoveryError::IoFailure("APFS B-tree key/value ranges overlap or exceed node".into())); }
        result.push(ApfsFixedBtreeEntry { key: &data[key_start..key_end], value: &data[value_start..value_end_for_entry] });
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn node() -> Vec<u8> {
        let mut data = vec![0u8; 512];
        data[32..34].copy_from_slice(&(FLAG_LEAF | FLAG_ROOT | FLAG_FIXED_KV_SIZE).to_le_bytes());
        data[34..36].copy_from_slice(&0u16.to_le_bytes()); data[36..40].copy_from_slice(&2u32.to_le_bytes());
        data[40..42].copy_from_slice(&0u16.to_le_bytes()); data[42..44].copy_from_slice(&8u16.to_le_bytes());
        data[56..58].copy_from_slice(&0u16.to_le_bytes()); data[58..60].copy_from_slice(&0u16.to_le_bytes());
        data[60..62].copy_from_slice(&8u16.to_le_bytes()); data[62..64].copy_from_slice(&16u16.to_le_bytes());
        data[64..72].copy_from_slice(b"KEY00001"); data[72..80].copy_from_slice(b"KEY00002");
        data[432..448].copy_from_slice(b"VALUE00000000001"); data[448..464].copy_from_slice(b"VALUE00000000002");
        data
    }
    fn variable_node() -> Vec<u8> {
        let mut data = vec![0u8; 512];
        data[32..34].copy_from_slice(&(FLAG_LEAF | FLAG_ROOT).to_le_bytes()); data[36..40].copy_from_slice(&2u32.to_le_bytes());
        data[40..42].copy_from_slice(&0u16.to_le_bytes()); data[42..44].copy_from_slice(&16u16.to_le_bytes());
        data[56..58].copy_from_slice(&0u16.to_le_bytes()); data[58..60].copy_from_slice(&5u16.to_le_bytes()); data[60..62].copy_from_slice(&4u16.to_le_bytes()); data[62..64].copy_from_slice(&3u16.to_le_bytes());
        data[64..69].copy_from_slice(b"KEY-A"); data[69..73].copy_from_slice(b"KEYB");
        data[469..472].copy_from_slice(b"VA1"); data[464..468].copy_from_slice(b"VAL2");
        data
    }
    #[test] fn parses_node_header_and_flags(){let p=parse_btree_node(&node()).unwrap();assert!(p.is_leaf());assert!(p.is_root());assert!(p.has_fixed_kv_size());assert_eq!(p.key_count,2);}
    #[test] fn parses_fixed_entries_using_relative_offsets(){let e=btree_fixed_entries(&node(),8,16).unwrap();assert_eq!(e[0].key,b"KEY00001");assert_eq!(e[0].value,b"VALUE00000000001");assert_eq!(e[1].key,b"KEY00002");assert_eq!(e[1].value,b"VALUE00000000002");}
    #[test] fn parses_variable_entries_with_lengths(){let e=btree_variable_entries(&variable_node()).unwrap();assert_eq!(e[0].key,b"KEY-A");assert_eq!(e[0].value,b"VA1");assert_eq!(e[1].key,b"KEYB");assert_eq!(e[1].value,b"VAL2");}
    #[test] fn rejects_table_too_small(){let mut d=node();d[42..44].copy_from_slice(&4u16.to_le_bytes());assert!(parse_btree_node(&d).is_err());}
    #[test] fn rejects_fixed_nodes_for_variable_parser(){assert!(btree_variable_entries(&node()).is_err());}
    #[test] fn rejects_variable_nodes_for_fixed_parser(){let mut d=variable_node();assert!(btree_fixed_entries(&d,8,16).is_err());d[32..34].copy_from_slice(&(FLAG_LEAF|FLAG_ROOT).to_le_bytes());assert!(btree_entries(&d).is_err());}
    #[test] fn rejects_truncated_node(){assert!(parse_btree_node(&[0u8;55]).is_err());}
    #[test] fn rejects_variable_value_underflow(){let mut d=variable_node();d[60..62].copy_from_slice(&500u16.to_le_bytes());assert!(btree_variable_entries(&d).is_err());}
}