use std::collections::{BTreeMap, HashMap, HashSet};

use recovery_core::{ByteRange, RecoveryError, RecoveryResult};
use storage_io::BlockDevice;

use crate::{
    APFS_TYPE_DIR_REC, APFS_TYPE_FILE_EXTENT, APFS_TYPE_INODE, ApfsCatalogRecord, ApfsDrecKey,
    ApfsFileExtentValue, ApfsVolume, ApfsXattr, decode_dir_record_value, decode_drec_key,
    decode_file_extent_value, decode_hashed_drec_key, decode_inode_value, decode_jkey,
    extent_is_sparse, extent_length, index_xattrs, read_volume_catalog_records,
};

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
    pub inodes: BTreeMap<u64, crate::ApfsInodeValue>,
    /// FILE_EXTENT records are keyed by the inode's `private_id` (the dstream OID), not the inode OID.
    pub extents: BTreeMap<u64, Vec<ApfsFileExtent>>,
    /// Extended attributes indexed by their owning inode object ID.
    pub xattrs: BTreeMap<u64, Vec<ApfsXattr>>,
}

fn decode_drec(data: &[u8]) -> RecoveryResult<ApfsDrecKey> {
    if data.len() >= DREC_HASHED_HEADER_LEN
        && let Ok(key) = decode_hashed_drec_key(data)
    {
        return Ok(key);
    }
    decode_drec_key(data)
}

pub fn decode_file_extent_key(data: &[u8]) -> RecoveryResult<(u64, u64)> {
    if data.len() < EXTENT_KEY_LEN {
        return Err(RecoveryError::LengthTooLarge {
            length: data.len() as u64,
        });
    }
    let key = decode_jkey(data)?;
    if key.record_type != APFS_TYPE_FILE_EXTENT {
        return Err(RecoveryError::IoFailure(
            "APFS key is not a file extent".into(),
        ));
    }
    Ok((
        key.object_id,
        u64::from_le_bytes(data[8..16].try_into().expect("validated APFS extent key")),
    ))
}

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
                directories.push(ApfsDirectoryEntry {
                    parent_id: key.parent_id,
                    file_id: value.file_id,
                    name: key.name,
                    flags: value.flags,
                });
            }
            APFS_TYPE_INODE => {
                inodes.insert(jkey.object_id, decode_inode_value(&record.value)?);
            }
            APFS_TYPE_FILE_EXTENT => {
                let (dstream_id, logical_offset) = decode_file_extent_key(&record.key)?;
                let value: ApfsFileExtentValue = decode_file_extent_value(&record.value)?;
                extents.entry(dstream_id).or_default().push(ApfsFileExtent {
                    logical_offset,
                    length: extent_length(value.length_and_flags),
                    physical_block: value.physical_block,
                    crypto_id: value.crypto_id,
                    sparse: extent_is_sparse(&value),
                });
            }
            _ => {}
        }
    }
    let xattrs = index_xattrs(records)?;
    for file_extents in extents.values_mut() {
        file_extents.sort_by_key(|extent| extent.logical_offset);
    }
    Ok(ApfsFilesystemIndex {
        directories,
        inodes,
        extents,
        xattrs,
    })
}

pub fn read_volume_filesystem_index<D: BlockDevice>(
    device: &D,
    range: ByteRange,
    container: &crate::ApfsContainer,
    volume: &ApfsVolume,
    xid: u64,
) -> RecoveryResult<ApfsFilesystemIndex> {
    let records = read_volume_catalog_records(device, range, container, volume, xid)?;
    index_catalog_records(&records)
}

pub fn read_file_extents<D: BlockDevice>(
    device: &D,
    container_range: ByteRange,
    block_size: u32,
    extents: &[ApfsFileExtent],
    file_size: u64,
) -> RecoveryResult<Vec<u8>> {
    if block_size == 0 || !block_size.is_power_of_two() {
        return Err(RecoveryError::IoFailure("invalid APFS block size".into()));
    }
    // Validate the requested output size against the supplied image range before
    // allocating. Corrupt inode metadata must not be able to force an unbounded
    // allocation during recovery.
    if file_size > container_range.length {
        return Err(RecoveryError::OutOfRange {
            offset: 0,
            length: file_size,
            capacity: container_range.length,
        });
    }
    let output_len = usize::try_from(file_size)
        .map_err(|_| RecoveryError::LengthTooLarge { length: file_size })?;
    let mut output = vec![0u8; output_len];
    let mut previous_end = 0u64;
    let container_end = container_range
        .offset
        .checked_add(container_range.length)
        .ok_or(RecoveryError::RangeOverflow)?;
    for extent in extents {
        if extent.length == 0 {
            continue;
        }
        let end = extent
            .logical_offset
            .checked_add(extent.length)
            .ok_or(RecoveryError::RangeOverflow)?;
        if extent.logical_offset < previous_end {
            return Err(RecoveryError::IoFailure("APFS file extents overlap".into()));
        }
        previous_end = end;
        if end > file_size {
            return Err(RecoveryError::OutOfRange {
                offset: extent.logical_offset,
                length: extent.length,
                capacity: file_size,
            });
        }
        let start =
            usize::try_from(extent.logical_offset).map_err(|_| RecoveryError::LengthTooLarge {
                length: extent.logical_offset,
            })?;
        let len = usize::try_from(extent.length).map_err(|_| RecoveryError::LengthTooLarge {
            length: extent.length,
        })?;
        if extent.crypto_id != 0 {
            return Err(RecoveryError::IoFailure(
                "encrypted APFS extent requires key material".into(),
            ));
        }
        if extent.sparse {
            continue;
        }
        let relative = extent
            .physical_block
            .checked_mul(u64::from(block_size))
            .ok_or(RecoveryError::RangeOverflow)?;
        let physical = container_range
            .offset
            .checked_add(relative)
            .ok_or(RecoveryError::RangeOverflow)?;
        let physical_end = physical
            .checked_add(extent.length)
            .ok_or(RecoveryError::RangeOverflow)?;
        if physical_end > container_end {
            return Err(RecoveryError::OutOfRange {
                offset: physical,
                length: extent.length,
                capacity: container_end,
            });
        }
        let read_range = ByteRange::new(physical, extent.length)?;
        read_range.validate_within(device.capacity())?;
        if device.read(read_range, &mut output[start..start + len])? != len {
            return Err(RecoveryError::IoFailure("short APFS extent read".into()));
        }
    }
    Ok(output)
}

/// Stream a file's extent data without allocating a buffer proportional to the
/// complete file size. Sparse extents are emitted as zero-filled chunks.
///
/// The callback receives `(logical_offset, chunk)` in ascending logical order.
/// Physical reads are bounded by `chunk_size`, making this suitable for large
/// recovery outputs where `read_file_extents` would otherwise allocate the
/// entire file in memory.
pub fn for_each_file_extent_chunk<D, F>(
    device: &D,
    container_range: ByteRange,
    block_size: u32,
    extents: &[ApfsFileExtent],
    file_size: u64,
    chunk_size: usize,
    mut visit: F,
) -> RecoveryResult<()>
where
    D: BlockDevice,
    F: FnMut(u64, &[u8]) -> RecoveryResult<()>,
{
    if block_size == 0 || !block_size.is_power_of_two() {
        return Err(RecoveryError::IoFailure("invalid APFS block size".into()));
    }
    if chunk_size == 0 {
        return Err(RecoveryError::IoFailure(
            "invalid APFS extent chunk size".into(),
        ));
    }
    if file_size > container_range.length {
        return Err(RecoveryError::OutOfRange {
            offset: 0,
            length: file_size,
            capacity: container_range.length,
        });
    }
    let container_end = container_range
        .offset
        .checked_add(container_range.length)
        .ok_or(RecoveryError::RangeOverflow)?;
    let mut previous_end = 0u64;
    for extent in extents {
        if extent.length == 0 {
            continue;
        }
        let end = extent
            .logical_offset
            .checked_add(extent.length)
            .ok_or(RecoveryError::RangeOverflow)?;
        if extent.logical_offset < previous_end {
            return Err(RecoveryError::IoFailure("APFS file extents overlap".into()));
        }
        previous_end = end;
        if end > file_size {
            return Err(RecoveryError::OutOfRange {
                offset: extent.logical_offset,
                length: extent.length,
                capacity: file_size,
            });
        }
        if extent.crypto_id != 0 {
            return Err(RecoveryError::IoFailure(
                "encrypted APFS extent requires key material".into(),
            ));
        }
        let mut logical = extent.logical_offset;
        let mut remaining = extent.length;
        while remaining != 0 {
            let amount = remaining.min(chunk_size as u64);
            let amount_usize = usize::try_from(amount)
                .map_err(|_| RecoveryError::LengthTooLarge { length: amount })?;
            if extent.sparse {
                let zeros = vec![0u8; amount_usize];
                visit(logical, &zeros)?;
            } else {
                let block_relative = extent
                    .physical_block
                    .checked_mul(u64::from(block_size))
                    .ok_or(RecoveryError::RangeOverflow)?;
                let within_extent = logical
                    .checked_sub(extent.logical_offset)
                    .ok_or(RecoveryError::RangeOverflow)?;
                let physical = container_range
                    .offset
                    .checked_add(block_relative)
                    .and_then(|offset| offset.checked_add(within_extent))
                    .ok_or(RecoveryError::RangeOverflow)?;
                let physical_end = physical
                    .checked_add(amount)
                    .ok_or(RecoveryError::RangeOverflow)?;
                if physical_end > container_end {
                    return Err(RecoveryError::OutOfRange {
                        offset: physical,
                        length: amount,
                        capacity: container_end,
                    });
                }
                let read_range = ByteRange::new(physical, amount)?;
                read_range.validate_within(device.capacity())?;
                let mut buffer = vec![0u8; amount_usize];
                if device.read(read_range, &mut buffer)? != amount_usize {
                    return Err(RecoveryError::IoFailure("short APFS extent read".into()));
                }
                visit(logical, &buffer)?;
            }
            logical = logical
                .checked_add(amount)
                .ok_or(RecoveryError::RangeOverflow)?;
            remaining -= amount;
        }
    }
    Ok(())
}

impl ApfsFilesystemIndex {
    pub fn path_for_entry(&self, entry: &ApfsDirectoryEntry) -> RecoveryResult<String> {
        let mut components = vec![entry.name.clone()];
        let mut current = entry.parent_id;
        let mut seen = HashSet::new();
        let mut by_child: HashMap<u64, &ApfsDirectoryEntry> = HashMap::new();
        for item in &self.directories {
            by_child.entry(item.file_id).or_insert(item);
        }
        while current != 0 {
            if !seen.insert(current) {
                return Err(RecoveryError::IoFailure(
                    "APFS directory hierarchy contains a cycle".into(),
                ));
            }
            let parent = match by_child.get(&current) {
                Some(parent) => *parent,
                None => break,
            };
            components.push(parent.name.clone());
            current = parent.parent_id;
        }
        components.reverse();
        Ok(format!("/{}", components.join("/")))
    }

    /// Reconstruct a directory entry's data stream. The inode's `private_id` selects FILE_EXTENT records and its DSTREAM xfield supplies the logical file size.
    pub fn read_entry_data<D: BlockDevice>(
        &self,
        device: &D,
        container_range: ByteRange,
        block_size: u32,
        entry: &ApfsDirectoryEntry,
    ) -> RecoveryResult<Vec<u8>> {
        let inode = self.inodes.get(&entry.file_id).ok_or_else(|| {
            RecoveryError::IoFailure("APFS directory entry has no inode record".into())
        })?;
        let file_size = inode
            .data_stream_size
            .ok_or_else(|| RecoveryError::IoFailure("APFS inode has no DSTREAM size".into()))?;
        let extents = self
            .extents
            .get(&inode.private_id)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        read_file_extents(device, container_range, block_size, extents, file_size)
    }

    /// Return the extended attributes owned by a directory entry's inode.
    pub fn xattrs_for_entry(&self, entry: &ApfsDirectoryEntry) -> &[ApfsXattr] {
        self.xattrs
            .get(&entry.file_id)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xattr::XATTR_DATA_EMBEDDED;
    use std::sync::{Arc, Mutex};
    struct MemoryDevice {
        data: Arc<Mutex<Vec<u8>>>,
    }
    impl BlockDevice for MemoryDevice {
        fn capacity(&self) -> u64 {
            self.data.lock().unwrap().len() as u64
        }
        fn read(&self, range: ByteRange, output: &mut [u8]) -> RecoveryResult<usize> {
            range.validate_within(self.capacity())?;
            let len = usize::try_from(range.length).map_err(|_| RecoveryError::LengthTooLarge {
                length: range.length,
            })?;
            if output.len() < len {
                return Err(RecoveryError::OutputBufferTooSmall {
                    required: len,
                    provided: output.len(),
                });
            }
            output[..len].copy_from_slice(
                &self.data.lock().unwrap()[range.offset as usize..range.offset as usize + len],
            );
            Ok(len)
        }
    }
    fn jkey(ty: u64, oid: u64) -> Vec<u8> {
        ((ty << 60) | oid).to_le_bytes().to_vec()
    }
    #[test]
    fn decodes_extent_key() {
        let mut key = jkey(8, 55);
        key.extend_from_slice(&4096u64.to_le_bytes());
        assert_eq!(decode_file_extent_key(&key).unwrap(), (55, 4096));
    }
    #[test]
    fn rejects_extent_key_with_wrong_type() {
        let mut key = jkey(3, 55);
        key.extend_from_slice(&0u64.to_le_bytes());
        assert!(decode_file_extent_key(&key).is_err());
    }
    #[test]
    fn joins_catalog_records_and_sorts_extents() {
        let mut dir_key = jkey(9, 2);
        dir_key.extend_from_slice(&5u16.to_le_bytes());
        dir_key.extend_from_slice(b"file\0");
        let dir_value = {
            let mut v = vec![0u8; 18];
            v[0..8].copy_from_slice(&42u64.to_le_bytes());
            v
        };
        let inode_key = jkey(3, 42);
        let mut inode_value = vec![0u8; 92];
        inode_value[8..16].copy_from_slice(&77u64.to_le_bytes());
        let mut extent_key_a = jkey(8, 77);
        extent_key_a.extend_from_slice(&8192u64.to_le_bytes());
        let mut extent_value_a = vec![0u8; 24];
        extent_value_a[0..8].copy_from_slice(&4096u64.to_le_bytes());
        extent_value_a[8..16].copy_from_slice(&100u64.to_le_bytes());
        let mut extent_key_b = jkey(8, 77);
        extent_key_b.extend_from_slice(&0u64.to_le_bytes());
        let mut extent_value_b = vec![0u8; 24];
        extent_value_b[0..8].copy_from_slice(&4096u64.to_le_bytes());
        extent_value_b[8..16].copy_from_slice(&99u64.to_le_bytes());
        let records = vec![
            ApfsCatalogRecord {
                key: dir_key,
                value: dir_value,
            },
            ApfsCatalogRecord {
                key: inode_key,
                value: inode_value,
            },
            ApfsCatalogRecord {
                key: extent_key_a,
                value: extent_value_a,
            },
            ApfsCatalogRecord {
                key: extent_key_b,
                value: extent_value_b,
            },
        ];
        let index = index_catalog_records(&records).unwrap();
        assert_eq!(index.directories[0].file_id, 42);
        assert_eq!(index.inodes[&42].private_id, 77);
        assert_eq!(index.extents[&77][0].logical_offset, 0);
        assert_eq!(index.extents[&77][1].physical_block, 100);
        assert!(index.xattrs.is_empty());
        assert_eq!(
            index.path_for_entry(&index.directories[0]).unwrap(),
            "/file"
        );
    }
    #[test]
    fn indexes_xattrs_with_their_inode() {
        let mut key = jkey(4, 42);
        key.extend_from_slice(&7u16.to_le_bytes());
        key.extend_from_slice(b"author\0");
        let value = [
            XATTR_DATA_EMBEDDED as u8,
            0,
            5,
            0,
            b'a',
            b'l',
            b'i',
            b'c',
            b'e',
        ];
        let index = index_catalog_records(&[ApfsCatalogRecord {
            key,
            value: value.to_vec(),
        }])
        .unwrap();
        let entry = ApfsDirectoryEntry {
            parent_id: 0,
            file_id: 42,
            name: "file".into(),
            flags: 0,
        };
        assert_eq!(index.xattrs_for_entry(&entry)[0].name, "author");
        assert_eq!(index.xattrs_for_entry(&entry)[0].data, b"alice");
    }
    #[test]
    fn rejects_directory_cycle() {
        let a = ApfsDirectoryEntry {
            parent_id: 3,
            file_id: 2,
            name: "a".into(),
            flags: 0,
        };
        let b = ApfsDirectoryEntry {
            parent_id: 2,
            file_id: 3,
            name: "b".into(),
            flags: 0,
        };
        let index = ApfsFilesystemIndex {
            directories: vec![a.clone(), b],
            inodes: BTreeMap::new(),
            extents: BTreeMap::new(),
            xattrs: BTreeMap::new(),
        };
        assert!(index.path_for_entry(&a).is_err());
    }
    #[test]
    fn rejects_untrusted_file_size_before_allocation() {
        let image = MemoryDevice {
            data: Arc::new(Mutex::new(vec![0u8; 16])),
        };
        let result = read_file_extents(&image, ByteRange::new(0, 16).unwrap(), 4, &[], u64::MAX);
        assert!(result.is_err());
    }
    #[test]
    fn reconstructs_sparse_and_physical_extents() {
        let image = MemoryDevice {
            data: Arc::new(Mutex::new(vec![
                0, 0, 0, 0, 0x41, 0x42, 0x43, 0x44, 0, 0, 0, 0, 0x51, 0x52, 0x53, 0x54,
            ])),
        };
        let extents = vec![
            ApfsFileExtent {
                logical_offset: 0,
                length: 4,
                physical_block: 1,
                crypto_id: 0,
                sparse: false,
            },
            ApfsFileExtent {
                logical_offset: 4,
                length: 4,
                physical_block: 0,
                crypto_id: 0,
                sparse: true,
            },
            ApfsFileExtent {
                logical_offset: 8,
                length: 4,
                physical_block: 3,
                crypto_id: 0,
                sparse: false,
            },
        ];
        let output =
            read_file_extents(&image, ByteRange::new(0, 16).unwrap(), 4, &extents, 12).unwrap();
        assert_eq!(output, b"ABCD\0\0\0\0QRST");
    }
    #[test]
    fn streams_extent_chunks_without_full_file_allocation() {
        let image = MemoryDevice {
            data: Arc::new(Mutex::new(vec![
                0, 0, 0, 0, 0x41, 0x42, 0x43, 0x44, 0, 0, 0, 0, 0x51, 0x52, 0x53, 0x54,
            ])),
        };
        let extents = vec![
            ApfsFileExtent {
                logical_offset: 0,
                length: 4,
                physical_block: 1,
                crypto_id: 0,
                sparse: false,
            },
            ApfsFileExtent {
                logical_offset: 4,
                length: 4,
                physical_block: 0,
                crypto_id: 0,
                sparse: true,
            },
            ApfsFileExtent {
                logical_offset: 8,
                length: 4,
                physical_block: 3,
                crypto_id: 0,
                sparse: false,
            },
        ];
        let mut chunks = Vec::new();
        for_each_file_extent_chunk(
            &image,
            ByteRange::new(0, 16).unwrap(),
            4,
            &extents,
            12,
            2,
            |offset, data| {
                chunks.push((offset, data.to_vec()));
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(
            chunks,
            vec![
                (0, b"AB".to_vec()),
                (2, b"CD".to_vec()),
                (4, vec![0, 0]),
                (6, vec![0, 0]),
                (8, b"QR".to_vec()),
                (10, b"ST".to_vec())
            ]
        );
    }
    #[test]
    fn rejects_invalid_chunk_size() {
        let image = MemoryDevice {
            data: Arc::new(Mutex::new(vec![0u8; 16])),
        };
        assert!(
            for_each_file_extent_chunk(
                &image,
                ByteRange::new(0, 16).unwrap(),
                4,
                &[],
                0,
                0,
                |_, _| Ok(())
            )
            .is_err()
        );
    }
    #[test]
    fn rejects_overlapping_or_encrypted_extents() {
        let image = MemoryDevice {
            data: Arc::new(Mutex::new(vec![0u8; 16])),
        };
        let overlap = vec![
            ApfsFileExtent {
                logical_offset: 0,
                length: 8,
                physical_block: 0,
                crypto_id: 0,
                sparse: true,
            },
            ApfsFileExtent {
                logical_offset: 4,
                length: 4,
                physical_block: 0,
                crypto_id: 0,
                sparse: true,
            },
        ];
        assert!(read_file_extents(&image, ByteRange::new(0, 16).unwrap(), 4, &overlap, 8).is_err());
        let encrypted = vec![ApfsFileExtent {
            logical_offset: 0,
            length: 4,
            physical_block: 0,
            crypto_id: 1,
            sparse: false,
        }];
        assert!(
            read_file_extents(&image, ByteRange::new(0, 16).unwrap(), 4, &encrypted, 4).is_err()
        );
    }
}
