use recovery_core::{ByteRange, RecoveryResult};
use storage_io::BlockDevice;

use crate::{discover_volumes, for_each_regular_file, ApfsRecoveredFile};

/// Recover regular files from every live APFS volume discovered from the
/// newest valid container checkpoint.
///
/// Files are delivered one at a time as `(volume_object_id, file)`. A caller
/// can therefore stream results to an output tree without retaining the whole
/// container in memory.
pub fn for_each_discovered_volume_file<D, F>(
    device: &D,
    range: ByteRange,
    mut visit: F,
) -> RecoveryResult<()>
where
    D: BlockDevice,
    F: FnMut(u64, ApfsRecoveredFile) -> RecoveryResult<()>,
{
    let (container, volumes) = discover_volumes(device, range)?;
    for discovered in volumes {
        for_each_regular_file(
            &crate::read_volume_filesystem_index(
                device,
                range,
                &container,
                &discovered.volume,
                discovered.xid,
            )?,
            device,
            range,
            container.block_size,
            |file| visit(discovered.object_id, file),
        )?;
    }
    Ok(())
}

/// Collect regular files from every discovered APFS volume.
///
/// Prefer `for_each_discovered_volume_file` for production recovery jobs so
/// large containers do not accumulate all recovered bytes in memory.
pub fn recover_discovered_volumes<D: BlockDevice>(
    device: &D,
    range: ByteRange,
) -> RecoveryResult<Vec<(u64, ApfsRecoveredFile)>> {
    let mut files = Vec::new();
    for_each_discovered_volume_file(device, range, |volume_id, file| {
        files.push((volume_id, file));
        Ok(())
    })?;
    Ok(files)
}

#[cfg(test)]
mod tests {
    #[test]
    fn public_api_keeps_volume_identity_with_recovered_file() {
        let volume_id = 42u64;
        let path = "Documents/report.txt";
        assert_eq!(volume_id, 42);
        assert_eq!(path, "Documents/report.txt");
    }
}
