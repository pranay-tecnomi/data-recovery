use recovery_core::{ByteRange, RecoveryError, RecoveryResult};
use storage_io::BlockDevice;

use crate::{for_each_file_extent_chunk, ApfsDirectoryEntry, ApfsFilesystemIndex};

const S_IFMT: u16 = 0o170000;
const S_IFLNK: u16 = 0o120000;

/// A recovered symbolic link. The target is the link's byte-for-byte data
/// stream decoded as UTF-8; invalid UTF-8 is rejected rather than silently
/// producing a different target.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApfsRecoveredSymlink {
    pub path: String,
    pub target: String,
    pub xattrs: Vec<crate::ApfsRecoveredXattr>,
}

fn read_symlink_target<D: BlockDevice>(
    index: &ApfsFilesystemIndex,
    entry: &ApfsDirectoryEntry,
    device: &D,
    container_range: ByteRange,
    block_size: u32,
) -> RecoveryResult<Vec<u8>> {
    let inode = index.inodes.get(&entry.file_id)
        .ok_or_else(|| RecoveryError::IoFailure("APFS symlink has no inode record".into()))?;
    let size = inode.data_stream_size
        .ok_or_else(|| RecoveryError::IoFailure("APFS symlink has no DSTREAM size".into()))?;
    let extents = index.extents.get(&inode.private_id).map(Vec::as_slice).unwrap_or(&[]);
    let mut data = Vec::new();
    for_each_file_extent_chunk(
        device,
        container_range,
        block_size,
        extents,
        size,
        64 * 1024,
        |_offset, chunk| {
            data.extend_from_slice(chunk);
            Ok(())
        },
    )?;
    if data.len() as u64 != size {
        return Err(RecoveryError::IoFailure("APFS symlink data length mismatch".into()));
    }
    Ok(data)
}

/// Recover every symbolic link from an indexed APFS filesystem.
///
/// Symlinks use the same DSTREAM/FILE_EXTENT machinery as regular files, but
/// are exposed separately so callers never mistake a link target for ordinary
/// file content.
pub fn recover_symlinks<D: BlockDevice>(
    index: &ApfsFilesystemIndex,
    device: &D,
    container_range: ByteRange,
    block_size: u32,
) -> RecoveryResult<Vec<ApfsRecoveredSymlink>> {
    let mut links = Vec::new();
    for entry in &index.directories {
        let Some(inode) = index.inodes.get(&entry.file_id) else { continue; };
        if inode.mode & S_IFMT != S_IFLNK { continue; }
        let path = index.path_for_entry(entry)?;
        let target_bytes = read_symlink_target(index, entry, device, container_range, block_size)?;
        let target = String::from_utf8(target_bytes)
            .map_err(|_| RecoveryError::IoFailure("APFS symlink target is not valid UTF-8".into()))?;
        let xattrs = index.xattrs_for_entry(entry).iter().map(|xattr| {
            Ok(crate::ApfsRecoveredXattr {
                name: xattr.name.clone(),
                flags: xattr.flags,
                data: crate::read_xattr_data(device, container_range, block_size, xattr, &index.extents)?,
            })
        }).collect::<RecoveryResult<Vec<_>>>()?;
        links.push(ApfsRecoveredSymlink { path, target, xattrs });
    }
    Ok(links)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_symlink_mode() {
        assert_eq!(0o120777u16 & S_IFMT, S_IFLNK);
        assert_ne!(0o100644u16 & S_IFMT, S_IFLNK);
    }
}
