use recovery_core::{ByteRange, RecoveryError, RecoveryResult};
use storage_io::BlockDevice;

use crate::{active_file_extents, ExFatDirectoryEntry, ExFatVolume};

/// Reads a deleted exFAT file from the clusters retained by its directory
/// entry set. Deleted files are not considered allocated by the filesystem,
/// so callers should treat the returned bytes as a best-effort recovery.
///
/// `max_bytes` prevents corrupt directory metadata from causing an unbounded
/// allocation. The same extent resolver is used for contiguous and FAT-chain
/// storage, but the caller is responsible for deciding whether a recovered
/// candidate is sufficiently trustworthy.
pub fn read_deleted_file<D: BlockDevice>(
    volume: &ExFatVolume,
    device: &D,
    volume_range: ByteRange,
    entry: &ExFatDirectoryEntry,
    max_bytes: u64,
) -> RecoveryResult<Vec<u8>> {
    if entry.data_length > max_bytes {
        return Err(RecoveryError::LengthTooLarge { length: entry.data_length });
    }
    let length = usize::try_from(entry.data_length)
        .map_err(|_| RecoveryError::LengthTooLarge { length: entry.data_length })?;
    let extents = active_file_extents(volume, device, volume_range, entry)?;
    let mut output = Vec::with_capacity(length);
    for extent in extents {
        let extent_len = usize::try_from(extent.length)
            .map_err(|_| RecoveryError::LengthTooLarge { length: extent.length })?;
        let start = output.len();
        let end = start.checked_add(extent_len).ok_or(RecoveryError::RangeOverflow)?;
        if end > length {
            return Err(RecoveryError::IoFailure("deleted exFAT file extents exceed logical file length".into()));
        }
        output.resize(end, 0);
        let read = device.read(extent, &mut output[start..end])?;
        if read != extent_len {
            return Err(RecoveryError::IoFailure("short deleted exFAT file-content read".into()));
        }
    }
    if output.len() != length {
        return Err(RecoveryError::IoFailure("deleted exFAT file extents do not cover logical file length".into()));
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Device { bytes: Vec<u8> }
    impl BlockDevice for Device {
        fn capacity(&self) -> u64 { self.bytes.len() as u64 }
        fn read(&self, range: ByteRange, buffer: &mut [u8]) -> RecoveryResult<usize> {
            let start = usize::try_from(range.offset).map_err(|_| RecoveryError::RangeOverflow)?;
            let end = start.checked_add(buffer.len()).ok_or(RecoveryError::RangeOverflow)?;
            if end > self.bytes.len() || range.length != buffer.len() {
                return Err(RecoveryError::IoFailure("mock read out of bounds".into()));
            }
            buffer.copy_from_slice(&self.bytes[start..end]);
            Ok(buffer.len())
        }
    }

    fn volume() -> ExFatVolume {
        ExFatVolume {
            partition_offset_sectors: 0,
            volume_length_sectors: 100,
            fat_offset_sectors: 1,
            fat_length_sectors: 1,
            cluster_heap_offset_sectors: 2,
            cluster_count: 10,
            root_directory_cluster: 2,
            bytes_per_sector: 512,
            bytes_per_cluster: 1024,
        }
    }

    #[test]
    fn reads_deleted_contiguous_file() {
        let mut bytes = vec![0u8; 51_200];
        bytes[3_072..4_096].fill(0xC3);
        bytes[4_096..4_568].fill(0x3C);
        let device = Device { bytes };
        let entry = ExFatDirectoryEntry {
            name: "old.bin".into(), attributes: 0, first_cluster: 4,
            data_length: 1_500, no_fat_chain: true,
        };
        let data = read_deleted_file(
            &volume(), &device, ByteRange::new(0, 51_200).unwrap(), &entry, 10_000,
        ).unwrap();
        assert_eq!(data.len(), 1_500);
        assert!(data[..1_024].iter().all(|&b| b == 0xC3));
        assert!(data[1_024..].iter().all(|&b| b == 0x3C));
    }

    #[test]
    fn rejects_deleted_file_above_limit() {
        let device = Device { bytes: vec![0; 51_200] };
        let entry = ExFatDirectoryEntry {
            name: "large.bin".into(), attributes: 0, first_cluster: 4,
            data_length: 1_500, no_fat_chain: true,
        };
        assert!(matches!(
            read_deleted_file(&volume(), &device, ByteRange::new(0, 51_200).unwrap(), &entry, 1_499),
            Err(RecoveryError::LengthTooLarge { .. })
        ));
    }
}
