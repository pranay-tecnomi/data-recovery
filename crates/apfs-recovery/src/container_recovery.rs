use recovery_core::{ByteRange, RecoveryResult};
use storage_io::BlockDevice;

use crate::{
    ApfsRecoveredFile, ApfsRecoveredFileHeader, discover_volumes, for_each_regular_file,
    for_each_regular_file_chunk,
};

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

/// Stream regular-file data from every live APFS volume.
///
/// The callback receives the volume object ID, file metadata, logical offset,
/// and one bounded data chunk at a time. No recovered file's content is
/// accumulated by this API.
pub fn for_each_discovered_volume_file_chunk<D, F>(
    device: &D,
    range: ByteRange,
    chunk_size: usize,
    mut visit: F,
) -> RecoveryResult<()>
where
    D: BlockDevice,
    F: FnMut(u64, &ApfsRecoveredFileHeader, u64, &[u8]) -> RecoveryResult<()>,
{
    if chunk_size == 0 {
        return Err(recovery_core::RecoveryError::IoFailure(
            "invalid APFS container file chunk size".into(),
        ));
    }
    let (container, volumes) = discover_volumes(device, range)?;
    for discovered in volumes {
        let index = crate::read_volume_filesystem_index(
            device,
            range,
            &container,
            &discovered.volume,
            discovered.xid,
        )?;
        for_each_regular_file_chunk(
            &index,
            device,
            range,
            container.block_size,
            chunk_size,
            |header, offset, chunk| visit(discovered.object_id, header, offset, chunk),
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
    use super::*;
    use recovery_core::ByteRange;
    use std::sync::{Arc, Mutex};

    struct EmptyDevice(Arc<Mutex<Vec<u8>>>);
    impl BlockDevice for EmptyDevice {
        fn capacity(&self) -> u64 {
            self.0.lock().expect("test device lock").len() as u64
        }
        fn read(&self, range: ByteRange, buffer: &mut [u8]) -> RecoveryResult<usize> {
            let data = self.0.lock().expect("test device lock");
            range.validate_within(data.len() as u64)?;
            let start = usize::try_from(range.offset).expect("test offset");
            buffer.copy_from_slice(&data[start..start + buffer.len()]);
            Ok(buffer.len())
        }
    }

    fn device() -> EmptyDevice {
        EmptyDevice(Arc::new(Mutex::new(vec![0u8; 4096])))
    }

    #[test]
    fn streaming_recovery_rejects_a_zero_chunk_size() {
        // A zero chunk size would loop forever rather than making progress.
        let device = device();
        let range = ByteRange::new(0, 4096).unwrap();
        let result = for_each_discovered_volume_file_chunk(&device, range, 0, |_, _, _, _| Ok(()));
        assert!(result.is_err(), "a zero chunk size must be refused");
    }

    #[test]
    fn recovery_refuses_a_container_that_does_not_parse() {
        // All-zero bytes carry no NXSB magic; recovery must report failure
        // rather than inventing volumes.
        let device = device();
        let range = ByteRange::new(0, 4096).unwrap();
        assert!(recover_discovered_volumes(&device, range).is_err());
        assert!(for_each_discovered_volume_file(&device, range, |_, _| Ok(())).is_err());
    }
}
