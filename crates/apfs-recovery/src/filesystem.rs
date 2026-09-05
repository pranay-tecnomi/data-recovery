use std::collections::{BTreeMap, HashMap, HashSet};

use recovery_core::{ByteRange, RecoveryError, RecoveryResult};
use storage_io::BlockDevice;

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

/// Reconstruct one file from physical APFS extents. Sparse extents and holes are zero-filled.
/// Encrypted extents are rejected rather than returning silently corrupted plaintext.
pub fn read_file_extents<D: BlockDevice>(device: &D, container_range: ByteRange, block_size: u32, extents: &[ApfsFileExtent], file_size: u64) -> RecoveryResult<Vec<u8>> {
    if block_size == 0 || !block_size.is_power_of_two() { return Err(RecoveryError::IoFailure("invalid APFS block size".into())); }
    let output_len = usize::try_from(file_size).map_err(|_| RecoveryError::LengthTooLarge { length: file_size })?;
    let mut output = vec![0u8; output_len];
    let mut previous_end = 0u64;
    let container_end = container_range.offset.checked_add(container_range.length).ok_or(RecoveryError::RangeOverflow)?;
    for extent in extents {
        if extent.length == 0 { continue; }
        let end = extent.logical_offset.checked_add(extent.length).ok_or(RecoveryError::RangeOverflow)?;
        if extent.logical_offset < previous_end { return Err(RecoveryError::IoFailure("APFS file extents overlap".into())); }
        previous_end = end;
        if end > file_size { return Err(RecoveryError::OutOfRange { offset: extent.logical_offset, length: extent.length, capacity: file_size }); }
        let start = usize::try_from(extent.logical_offset).map_err(|_| RecoveryError::LengthTooLarge { length: extent.logical_offset })?;
        let len = usize::try_from(extent.length).map_err(|_| RecoveryError::LengthTooLarge { length: extent.length })?;
        if extent.crypto_id != 0 { return Err(RecoveryError::IoFailure("encrypted APFS extent requires key material".into())); }
        if extent.sparse { continue; }
        let relative = extent.physical_block.checked_mul(u64::from(block_size)).ok_or(RecoveryError::RangeOverflow)?;
        let physical = container_range.offset.checked_add(relative).ok_or(RecoveryError::RangeOverflow)?;
        let physical_end = physical.checked_add(extent.length).ok_or(RecoveryError::RangeOverflow)?;
        if physical_end > container_end { return Err(RecoveryError::OutOfRange { offset: physical, length: extent.length, capacity: container_end }); }
        let read_range = ByteRange::new(physical, extent.length)?;
        read_range.validate_within(device.capacity())?;
        if device.read(read_range, &mut output[start..start + len])? != len { return Err(RecoveryError::IoFailure("short APFS extent read".into())); }
    }
    Ok(output)
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
    use std::{sync::{Arc, Mutex}};

    struct MemoryDevice { data: Arc<Mutex<Vec<u8>>> }
    impl BlockDevice for MemoryDevice {
        fn capacity(&self) -> u64 { self.data.lock().unwrap().len() as u64 }
        fn read(&self, range: ByteRange, output: &mut [u8]) -> RecoveryResult<usize> {
            range.validate_within(self.capacity())?;
            let len = usize::try_from(range.length).map_err(|_| RecoveryError::LengthTooLarge { length: range.length })?;
            if output.len() < len { return Err(RecoveryError::OutputBufferTooSmall { required: len, provided: output.len() }); }
            output[..len].copy_from_slice(&self.data.lock().unwrap()[range.offset as usize..range.offset as usize + len]);
            Ok(len)
        }
    }

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

    #[test]
    fn reconstructs_sparse_and_physical_extents() {
        let image = MemoryDevice { data: Arc::new(Mutex::new(vec![0, 0, 0, 0, 0x41, 0x42, 0x43, 0x44, 0, 0, 0, 0, 0x51, 0x52, 0x53, 0x54])) };
        let extents = vec![
            ApfsFileExtent { logical_offset: 0, length: 4, physical_block: 1, crypto_id: 0, sparse: false },
            ApfsFileExtent { logical_offset: 4, length: 4, physical_block: 0, crypto_id: 0, sparse: true },
            ApfsFileExtent { logical_offset: 8, length: 4, physical_block: 3, crypto_id: 0, sparse: false },
        ];
        let output = read_file_extents(&image, ByteRange::new(0, 16).unwrap(), 4, &extents, 12).unwrap();
        assert_eq!(output, b"ABCD\0\0\0\0QRST");
    }

    #[test]
    fn rejects_overlapping_or_encrypted_extents() {
        let image = MemoryDevice { data: Arc::new(Mutex::new(vec![0u8; 16])) };
        let overlap = vec![ApfsFileExtent { logical_offset: 0, length: 8, physical_block: 0, crypto_id: 0, sparse: true }, ApfsFileExtent { logical_offset: 4, length: 4, physical_block: 0, crypto_id: 0, sparse: true }];
        assert!(read_file_extents(&image, ByteRange::new(0, 16).unwrap(), 4, &overlap, 8).is_err());
        let encrypted = vec![ApfsFileExtent { logical_offset: 0, length: 4, physical_block: 0, crypto_id: 1, sparse: false }];
        assert!(read_file_extents(&image, ByteRange::new(0, 16).unwrap(), 4, &encrypted, 4).is_err());
    }
}
