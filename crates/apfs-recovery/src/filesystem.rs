use std::collections::{BTreeMap, HashMap, HashSet};

use recovery_core::{RecoveryError, RecoveryResult};

use crate::{decode_dir_record_value, decode_drec_key, decode_file_extent_value, decode_hashed_drec_key, decode_inode_value, decode_jkey, extent_is_sparse, extent_length, ApfsCatalogRecord, ApfsDrecKey, ApfsFileExtentValue, ApfsInodeValue, APFS_TYPE_DIR_REC, APFS_TYPE_FILE_EXTENT, APFS_TYPE_INODE};

const DREC_HASHED_HEADER_LEN: usize = 12;
const EXTENT_KEY_LEN: usize = 16;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApfsDirectoryEntry {
    pub parent_id: u64,
    pub file_id: u64,
    pub name: String,
    pub flags: u16,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApfsFileExtent {
    pub logical_offset: u64,
    pub length: u64,
    pub physical_block: u64,
    pub crypto_id: u64,
    pub sparse: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApfsFilesystemIndex {
    pub directories: Vec<ApfsDirectoryEntry>,
    pub inodes: BTreeMap<u64, ApfsInodeValue>,
    pub extents: BTreeMap<u64, Vec<ApfsFileExtent>>,
}

fn decode_drec(data: &[u8]) -> RecoveryResult<ApfsDrecKey> {
    if data.len() >= DREC_HASHED_HEADER_LEN {
        if let Ok(key) = decode_hashed_drec_key(data) { return Ok(key); }
    }
    decode_drec_key(data)
}

/// Decode a file-extent key: object ID/type followed by logical file offset.
pub fn decode_file_extent_key(data: &[u8]) -> RecoveryResult<(u64, u64)> {
    if data.len() < EXTENT_KEY_LEN { return Err(RecoveryError::LengthTooLarge { length: data.len() as u64 }); }
    let key = decode_jkey(data)?;
    if key.record_type != APFS_TYPE_FILE_EXTENT { return Err(RecoveryError::IoFailure("APFS key is not a file extent".into())); }
    Ok((key.object_id, u64::from_le_bytes(data[8..16].try_into().expect("validated APFS extent key"))))
}

/// Build a recovery-oriented index from the current catalog tree.
pub fn index_catalog_records(records: &[ApfsCatalogRecord]) -> RecoveryResult<ApfsFilesystemIndex> {
    let mut directories = Vec::new();
    let mut inodes = BTreeMap::new();
    let mut extents: BTreeMap<u64, Vec<ApfsFileExtent>> = BTreeMap::new();
    for record in records {
        let jkey = decode_jkey(&record.key)?;
        match jkey.record_type {
            APFS_TYPE_DIR_REC => {
                let key = decode_drec(&record.key)?;
                let value = decode_dir_record_value(&record.value)?;
                directories.push(ApfsDirectoryEntry { parent_id: key.parent_id, file_id: value.file_id, name: key.name, flags: value.flags });
            }
            APFS_TYPE_INODE => { inodes.insert(jkey.object_id, decode_inode_value(&record.value)?); }
            APFS_TYPE_FILE_EXTENT => {
                let (file_id, logical_offset) = decode_file_extent_key(&record.key)?;
                let value: ApfsFileExtentValue = decode_file_extent_value(&record.value)?;
                extents.entry(file_id).or_default().push(ApfsFileExtent { logical_offset, length: extent_length(value.length_and_flags), physical_block: value.physical_block, crypto_id: value.crypto_id, sparse: extent_is_sparse(&value) });
            }
            _ => {}
        }
    }
    for file_extents in extents.values_mut() { file_extents.sort_by_key(|extent| extent.logical_offset); }
    Ok(ApfsFilesystemIndex { directories, inodes, extents })
}

impl ApfsFilesystemIndex {
    /// Resolve a directory entry to a full path, rejecting parent cycles.
    pub fn path_for_entry(&self, entry: &ApfsDirectoryEntry) -> RecoveryResult<String> {
        let mut components = vec![entry.name.clone()];
        let mut current = entry.parent_id;
        let mut seen = HashSet::new();
        let mut by_child: HashMap<u64, &ApfsDirectoryEntry> = HashMap::new();
        for item in &self.directories { by_child.entry(item.file_id).or_insert(item); }
        while current != 0 {
            if !seen.insert(current) { return Err(RecoveryError::IoFailure("APFS directory hierarchy contains a cycle".into())); }
            let parent = match by_child.get(&current) { Some(parent) => *parent, None => break };
            components.push(parent.name.clone());
            current = parent.parent_id;
        }
        components.reverse();
        Ok(format!("/{}", components.join("/")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn jkey(ty: u64, oid: u64) -> Vec<u8> { ((ty << 60) | oid).to_le_bytes().to_vec() }

    #[test]
    fn decodes_extent_key() { let mut key = jkey(8, 55); key.extend_from_slice(&4096u64.to_le_bytes()); assert_eq!(decode_file_extent_key(&key).unwrap(), (55, 4096)); }

    #[test]
    fn rejects_extent_key_with_wrong_type() { let mut key = jkey(3, 55); key.extend_from_slice(&0u64.to_le_bytes()); assert!(decode_file_extent_key(&key).is_err()); }

    #[test]
    fn joins_catalog_records_and_sorts_extents() {
        let mut dir_key = jkey(9, 2); dir_key.extend_from_slice(&5u16.to_le_bytes()); dir_key.extend_from_slice(b"file\0");
        let mut dir_value = vec![0u8; 18]; dir_value[0..8].copy_from_slice(&42u64.to_le_bytes());
        let inode_key = jkey(3, 42); let inode_value = vec![0u8; 92];
        let mut extent_key_a = jkey(8, 42); extent_key_a.extend_from_slice(&8192u64.to_le_bytes());
        let mut extent_value_a = vec![0u8; 24]; extent_value_a[0..8].copy_from_slice(&4096u64.to_le_bytes()); extent_value_a[8..16].copy_from_slice(&100u64.to_le_bytes());
        let mut extent_key_b = jkey(8, 42); extent_key_b.extend_from_slice(&0u64.to_le_bytes());
        let mut extent_value_b = vec![0u8; 24]; extent_value_b[0..8].copy_from_slice(&4096u64.to_le_bytes()); extent_value_b[8..16].copy_from_slice(&99u64.to_le_bytes());
        let records = vec![ApfsCatalogRecord { key: dir_key, value: dir_value }, ApfsCatalogRecord { key: inode_key, value: inode_value }, ApfsCatalogRecord { key: extent_key_a, value: extent_value_a }, ApfsCatalogRecord { key: extent_key_b, value: extent_value_b }];
        let index = index_catalog_records(&records).unwrap();
        assert_eq!(index.directories[0].file_id, 42); assert!(index.inodes.contains_key(&42)); assert_eq!(index.extents[&42][0].logical_offset, 0); assert_eq!(index.extents[&42][1].physical_block, 100); assert_eq!(index.path_for_entry(&index.directories[0]).unwrap(), "/file");
    }

    #[test]
    fn rejects_directory_cycle() {
        let a = ApfsDirectoryEntry { parent_id: 3, file_id: 2, name: "a".into(), flags: 0 };
        let b = ApfsDirectoryEntry { parent_id: 2, file_id: 3, name: "b".into(), flags: 0 };
        let index = ApfsFilesystemIndex { directories: vec![a.clone(), b], inodes: BTreeMap::new(), extents: BTreeMap::new() };
        assert!(index.path_for_entry(&a).is_err());
    }
}
