use recovery_core::{ByteRange, RecoveryResult};
use storage_io::BlockDevice;

use crate::{for_each_file_extent_chunk, ApfsDirectoryEntry, ApfsFilesystemIndex, ApfsXattr};

const S_IFMT: u16 = 0o170000;
const S_IFREG: u16 = 0o100000;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApfsRecoveredXattr {
    pub name: String,
    pub flags: u16,
    pub data: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApfsRecoveredFile {
    pub path: String,
    pub size: u64,
    pub data: Vec<u8>,
    pub xattrs: Vec<ApfsRecoveredXattr>,
}

/// Metadata shared by all chunks belonging to one recovered regular file.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApfsRecoveredFileHeader {
    pub path: String,
    pub size: u64,
    pub xattrs: Vec<ApfsRecoveredXattr>,
}

fn recover_xattrs<D: BlockDevice>(
    index: &ApfsFilesystemIndex,
    entry: &ApfsDirectoryEntry,
    device: &D,
    container_range: ByteRange,
    block_size: u32,
) -> RecoveryResult<Vec<ApfsRecoveredXattr>> {
    index.xattrs_for_entry(entry).iter().map(|xattr: &ApfsXattr| {
        Ok(ApfsRecoveredXattr {
            name: xattr.name.clone(),
            flags: xattr.flags,
            data: crate::read_xattr_data(device, container_range, block_size, xattr, &index.extents)?,
        })
    }).collect()
}

fn regular_file_metadata<D: BlockDevice>(
    index: &ApfsFilesystemIndex,
    entry: &ApfsDirectoryEntry,
    device: &D,
    container_range: ByteRange,
    block_size: u32,
) -> RecoveryResult<Option<ApfsRecoveredFileHeader>> {
    let Some(inode) = index.inodes.get(&entry.file_id) else {
        return Ok(None);
    };
    if inode.mode & S_IFMT != S_IFREG {
        return Ok(None);
    }
    let Some(size) = inode.data_stream_size else {
        return Ok(None);
    };
    Ok(Some(ApfsRecoveredFileHeader {
        path: index.path_for_entry(entry)?,
        size,
        xattrs: recover_xattrs(index, entry, device, container_range, block_size)?,
    }))
}

/// Stream every regular file in bounded chunks.
///
/// The callback receives the file metadata and one `(logical_offset, chunk)`
/// at a time. Chunks include holes as zero-filled data and are emitted in
/// logical order. At most `chunk_size` file bytes are materialized at once.
pub fn for_each_regular_file_chunk<D, F>(
    index: &ApfsFilesystemIndex,
    device: &D,
    container_range: ByteRange,
    block_size: u32,
    chunk_size: usize,
    mut visit: F,
) -> RecoveryResult<()>
where
    D: BlockDevice,
    F: FnMut(&ApfsRecoveredFileHeader, u64, &[u8]) -> RecoveryResult<()>,
{
    if chunk_size == 0 {
        return Err(recovery_core::RecoveryError::IoFailure("invalid APFS file chunk size".into()));
    }

    for entry in &index.directories {
        let Some(header) = regular_file_metadata(index, entry, device, container_range, block_size)? else {
            continue;
        };
        let inode = index.inodes.get(&entry.file_id).expect("validated regular-file metadata");
        let extents = index.extents.get(&inode.private_id).map(Vec::as_slice).unwrap_or(&[]);
        let mut next_logical = 0u64;
        for_each_file_extent_chunk(
            device,
            container_range,
            block_size,
            extents,
            header.size,
            chunk_size,
            |offset, chunk| {
                if offset > next_logical {
                    let mut gap = next_logical;
                    while gap < offset {
                        let amount = (offset - gap).min(chunk_size as u64) as usize;
                        let zeros = vec![0u8; amount];
                        visit(&header, gap, &zeros)?;
                        gap += amount as u64;
                    }
                }
                visit(&header, offset, chunk)?;
                next_logical = offset.checked_add(chunk.len() as u64).ok_or(recovery_core::RecoveryError::RangeOverflow)?;
                Ok(())
            },
        )?;
        while next_logical < header.size {
            let amount = (header.size - next_logical).min(chunk_size as u64) as usize;
            let zeros = vec![0u8; amount];
            visit(&header, next_logical, &zeros)?;
            next_logical += amount as u64;
        }
    }
    Ok(())
}

/// Visit every catalog entry whose inode identifies it as a regular file.
///
/// The callback receives one fully reconstructed file at a time. This avoids
/// retaining the entire recovered volume in memory, which is important when a
/// recovery image contains large files or many files.
pub fn for_each_regular_file<D, F>(
    index: &ApfsFilesystemIndex,
    device: &D,
    container_range: ByteRange,
    block_size: u32,
    mut visit: F,
) -> RecoveryResult<()>
where
    D: BlockDevice,
    F: FnMut(ApfsRecoveredFile) -> RecoveryResult<()>,
{
    for entry in &index.directories {
        let Some(inode) = index.inodes.get(&entry.file_id) else {
            continue;
        };
        if inode.mode & S_IFMT != S_IFREG {
            continue;
        }
        let Some(size) = inode.data_stream_size else {
            continue;
        };
        let path = index.path_for_entry(entry)?;
        let data = index.read_entry_data(device, container_range, block_size, entry)?;
        let xattrs = recover_xattrs(index, entry, device, container_range, block_size)?;
        debug_assert_eq!(data.len() as u64, size);
        visit(ApfsRecoveredFile { path, size, data, xattrs })?;
    }
    Ok(())
}

/// Recover every catalog entry whose inode identifies it as a regular file.
///
/// This convenience API collects all recovered files in memory. Call
/// `for_each_regular_file_chunk` for large recoveries where the caller can
/// write each file incrementally instead.
pub fn recover_regular_files<D: BlockDevice>(
    index: &ApfsFilesystemIndex,
    device: &D,
    container_range: ByteRange,
    block_size: u32,
) -> RecoveryResult<Vec<ApfsRecoveredFile>> {
    let mut files = Vec::new();
    for_each_regular_file(index, device, container_range, block_size, |file| {
        files.push(file);
        Ok(())
    })?;
    Ok(files)
}

/// Return the filesystem entry used to recover a file at `path`.
pub fn find_entry_by_path<'a>(
    index: &'a ApfsFilesystemIndex,
    path: &str,
) -> RecoveryResult<Option<&'a ApfsDirectoryEntry>> {
    for entry in &index.directories {
        if index.path_for_entry(entry)? == path {
            return Ok(Some(entry));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_regular_file_mode() {
        assert_eq!(0o100644u16 & S_IFMT, S_IFREG);
        assert_ne!(0o040755u16 & S_IFMT, S_IFREG);
    }

    #[test]
    fn rejects_zero_chunk_size() {
        let result = for_each_regular_file_chunk::<_, fn(&ApfsRecoveredFileHeader, u64, &[u8]) -> RecoveryResult<()>>(
            &ApfsFilesystemIndex {
                directories: Vec::new(),
                inodes: std::collections::BTreeMap::new(),
                extents: std::collections::BTreeMap::new(),
                xattrs: std::collections::BTreeMap::new(),
            },
            &EmptyDevice,
            ByteRange::new(0, 0).unwrap(),
            4096,
            0,
            |_header, _offset, _chunk| Ok(()),
        );
        assert!(result.is_err());
    }

    struct EmptyDevice;
    impl BlockDevice for EmptyDevice {
        fn capacity(&self) -> u64 { 0 }
        fn read(&self, _range: ByteRange, _output: &mut [u8]) -> RecoveryResult<usize> { Ok(0) }
    }
}
