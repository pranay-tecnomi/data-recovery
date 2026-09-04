use recovery_core::{ByteRange, RecoveryError, RecoveryResult};
use storage_io::BlockDevice;

use crate::{parse_directory_entries, ExFatDirectoryEntry, ExFatVolume};

const MAX_DIRECTORY_CLUSTERS: u32 = 1_000_000;

fn read_clusters<D: BlockDevice>(
    volume: &ExFatVolume,
    device: &D,
    volume_range: ByteRange,
    clusters: &[u32],
) -> RecoveryResult<Vec<u8>> {
    let cluster_bytes = usize::try_from(volume.bytes_per_cluster)
        .map_err(|_| RecoveryError::LengthTooLarge { length: volume.bytes_per_cluster })?;
    let total = clusters
        .len()
        .checked_mul(cluster_bytes)
        .ok_or(RecoveryError::RangeOverflow)?;
    let mut out = Vec::with_capacity(total);
    for &cluster in clusters {
        let range = volume.cluster_range_in(volume_range, cluster)?;
        let len = usize::try_from(range.length)
            .map_err(|_| RecoveryError::LengthTooLarge { length: range.length })?;
        let mut buf = vec![0u8; len];
        if device.read(range, &mut buf)? != len {
            return Err(RecoveryError::IoFailure("short exFAT directory cluster read".into()));
        }
        out.extend_from_slice(&buf);
    }
    Ok(out)
}

/// Reads the root directory as a bounded FAT cluster chain and parses active file entry sets.
///
/// The directory is read only through the filesystem's validated cluster mapping. A corrupt
/// chain or malformed entry set is reported instead of returning fabricated file metadata.
pub fn read_root_entries<D: BlockDevice>(
    volume: &ExFatVolume,
    device: &D,
    volume_range: ByteRange,
) -> RecoveryResult<Vec<ExFatDirectoryEntry>> {
    read_directory(volume, device, volume_range, volume.root_directory_cluster)
}

/// Reads and parses an exFAT directory beginning at `start_cluster`.
///
/// Traversal is bounded by the filesystem cluster count and an implementation cap. Directory
/// parsing stops at the exFAT end marker, while unused/deleted secondary entries are ignored by
/// the existing entry-set parser.
pub fn read_directory<D: BlockDevice>(
    volume: &ExFatVolume,
    device: &D,
    volume_range: ByteRange,
    start_cluster: u32,
) -> RecoveryResult<Vec<ExFatDirectoryEntry>> {
    let limit = volume.cluster_count.min(MAX_DIRECTORY_CLUSTERS);
    if start_cluster < 2 || start_cluster >= volume.cluster_count.saturating_add(2) {
        return Err(RecoveryError::IoFailure("invalid exFAT directory starting cluster".into()));
    }
    let chain = volume.cluster_chain(device, volume_range, start_cluster)?;
    if chain.len() > usize::try_from(limit).unwrap_or(usize::MAX) {
        return Err(RecoveryError::IoFailure("exFAT directory exceeds traversal limit".into()));
    }
    let bytes = read_clusters(volume, device, volume_range, &chain)?;
    parse_directory_entries(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Dummy;
    impl BlockDevice for Dummy {
        fn capacity(&self) -> u64 { 65_536 }
        fn read(&self, _range: ByteRange, _buffer: &mut [u8]) -> RecoveryResult<usize> {
            unreachable!("directory tests exercise validation before device reads")
        }
    }

    #[test]
    fn rejects_invalid_directory_start() {
        let volume = ExFatVolume {
            partition_offset_sectors: 0,
            volume_length_sectors: 128,
            fat_offset_sectors: 1,
            fat_length_sectors: 1,
            cluster_heap_offset_sectors: 2,
            cluster_count: 10,
            root_directory_cluster: 2,
            bytes_per_sector: 512,
            bytes_per_cluster: 1024,
        };
        assert!(read_directory(&volume, &Dummy, ByteRange::new(0, 65_536).unwrap(), 1).is_err());
    }
}
