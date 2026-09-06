use recovery_core::{ByteRange, RecoveryResult};
use storage_io::BlockDevice;

use crate::{ApfsDirectoryEntry, ApfsFilesystemIndex, ApfsXattr};

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
/// `for_each_regular_file` for large recoveries where the caller can write each
/// file immediately instead.
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
}
