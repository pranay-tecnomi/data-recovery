use recovery_core::{ByteRange, RecoveryResult};
use storage_io::BlockDevice;

use crate::{ApfsDirectoryEntry, ApfsFilesystemIndex};

const S_IFMT: u16 = 0o170000;
const S_IFREG: u16 = 0o100000;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApfsRecoveredFile {
    pub path: String,
    pub size: u64,
    pub data: Vec<u8>,
}

/// Recover every catalog entry whose inode identifies it as a regular file.
///
/// Directory entries without an inode or without a DSTREAM are skipped so one
/// damaged record does not abort recovery of unrelated files. Individual data
/// reconstruction errors are still returned because silently emitting corrupt
/// bytes would be worse than stopping the caller at the damaged file.
pub fn recover_regular_files<D: BlockDevice>(
    index: &ApfsFilesystemIndex,
    device: &D,
    container_range: ByteRange,
    block_size: u32,
) -> RecoveryResult<Vec<ApfsRecoveredFile>> {
    let mut files = Vec::new();
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
        debug_assert_eq!(data.len() as u64, size);
        files.push(ApfsRecoveredFile { path, size, data });
    }
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
