use recovery_core::{ByteRange, RecoveryError, RecoveryResult};
use storage_io::BlockDevice;

use crate::{ExFatDirectoryEntry, ExFatVolume};

/// Resolves the on-volume byte extents of an active exFAT file.
///
/// The returned ranges are partition-relative absolute device ranges and are
/// trimmed to the logical data length. Contiguous streams use the exFAT
/// no-FAT-chain flag; fragmented streams are resolved through the FAT.
pub fn active_file_extents<D: BlockDevice>(
    volume: &ExFatVolume,
    device: &D,
    volume_range: ByteRange,
    entry: &ExFatDirectoryEntry,
) -> RecoveryResult<Vec<ByteRange>> {
    if entry.data_length == 0 {
        return Ok(Vec::new());
    }
    if entry.first_cluster < 2 {
        return Err(RecoveryError::IoFailure("non-empty exFAT file has invalid first cluster".into()));
    }

    let clusters_needed = entry
        .data_length
        .checked_add(volume.bytes_per_cluster - 1)
        .ok_or(RecoveryError::RangeOverflow)?
        / volume.bytes_per_cluster;
    let clusters_needed = u32::try_from(clusters_needed)
        .map_err(|_| RecoveryError::LengthTooLarge { length: entry.data_length })?;

    let clusters = if entry.no_fat_chain {
        let last = entry.first_cluster
            .checked_add(clusters_needed - 1)
            .ok_or(RecoveryError::RangeOverflow)?;
        if last >= volume.cluster_count.saturating_add(2) {
            return Err(RecoveryError::IoFailure("contiguous exFAT file exceeds cluster heap".into()));
        }
        (entry.first_cluster..=last).collect()
    } else {
        let chain = volume.cluster_chain(device, volume_range, entry.first_cluster)?;
        if chain.len() < usize::try_from(clusters_needed).map_err(|_| RecoveryError::LengthTooLarge { length: u64::from(clusters_needed) })? {
            return Err(RecoveryError::IoFailure("exFAT file cluster chain is shorter than data length".into()));
        }
        chain.into_iter().take(clusters_needed as usize).collect()
    };

    let mut remaining = entry.data_length;
    let mut out = Vec::with_capacity(clusters.len());
    for cluster in clusters {
        let full = volume.cluster_range_in(volume_range, cluster)?;
        let length = remaining.min(full.length);
        out.push(ByteRange::new(full.offset, length)?);
        remaining -= length;
    }
    if remaining != 0 {
        return Err(RecoveryError::IoFailure("exFAT extent resolution produced insufficient data".into()));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contiguous_file_uses_trimmed_ranges() {
        let volume = ExFatVolume {
            partition_offset_sectors: 0, volume_length_sectors: 100,
            fat_offset_sectors: 1, fat_length_sectors: 1,
            cluster_heap_offset_sectors: 2, cluster_count: 10,
            root_directory_cluster: 2, bytes_per_sector: 512, bytes_per_cluster: 1024,
        };
        let entry = ExFatDirectoryEntry {
            name: "x".into(), attributes: 0, first_cluster: 3,
            data_length: 1500, no_fat_chain: true,
        };
        let ranges = active_file_extents(&volume, &Dummy, ByteRange::new(0, 51_200).unwrap(), &entry).unwrap();
        assert_eq!(ranges.len(), 2);
        assert_eq!(ranges[0].length, 1024);
        assert_eq!(ranges[1].length, 476);
    }

    struct Dummy;
    impl BlockDevice for Dummy {
        fn capacity(&self) -> u64 { 51_200 }
        fn read(&self, _range: ByteRange, _buffer: &mut [u8]) -> RecoveryResult<usize> { unreachable!("contiguous file does not read FAT") }
    }
}
