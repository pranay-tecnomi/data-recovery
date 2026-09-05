use recovery_core::{ByteRange, RecoveryError, RecoveryResult};
use storage_io::BlockDevice;

use crate::{active_file_extents, parse_directory_entries, parse_deleted_directory_entries, ExFatDirectoryEntry, ExFatVolume};

const MAX_DIRECTORY_CLUSTERS: u32 = 1_000_000;
const MAX_TREE_DEPTH: usize = 128;
const MAX_TREE_ENTRIES: usize = 1_000_000;

fn read_ranges<D: BlockDevice>(device: &D, ranges: &[ByteRange], total_hint: u64, what: &str) -> RecoveryResult<Vec<u8>> {
    let capacity = usize::try_from(total_hint).map_err(|_| RecoveryError::LengthTooLarge { length: total_hint })?;
    let mut out = Vec::with_capacity(capacity);
    for &range in ranges {
        let len = usize::try_from(range.length).map_err(|_| RecoveryError::LengthTooLarge { length: range.length })?;
        let mut buf = vec![0u8; len];
        if device.read(range, &mut buf)? != len { return Err(RecoveryError::IoFailure(format!("short exFAT {what} read"))); }
        out.extend_from_slice(&buf);
    }
    Ok(out)
}

fn read_clusters<D: BlockDevice>(volume: &ExFatVolume, device: &D, volume_range: ByteRange, clusters: &[u32]) -> RecoveryResult<Vec<u8>> {
    let cluster_bytes = usize::try_from(volume.bytes_per_cluster).map_err(|_| RecoveryError::LengthTooLarge { length: volume.bytes_per_cluster })?;
    let total = clusters.len().checked_mul(cluster_bytes).ok_or(RecoveryError::RangeOverflow)?;
    let ranges: Vec<_> = clusters.iter().map(|&cluster| volume.cluster_range_in(volume_range, cluster)).collect::<RecoveryResult<_>>()?;
    read_ranges(device, &ranges, u64::try_from(total).map_err(|_| RecoveryError::LengthTooLarge { length: total as u64 })?, "directory cluster")
}

fn directory_chain<D: BlockDevice>(volume: &ExFatVolume, device: &D, volume_range: ByteRange, start_cluster: u32) -> RecoveryResult<Vec<u32>> {
    let limit = volume.cluster_count.min(MAX_DIRECTORY_CLUSTERS);
    if start_cluster < 2 || start_cluster >= volume.cluster_count.saturating_add(2) { return Err(RecoveryError::IoFailure("invalid exFAT directory starting cluster".into())); }
    let chain = volume.cluster_chain(device, volume_range, start_cluster)?;
    if chain.len() > usize::try_from(limit).unwrap_or(usize::MAX) { return Err(RecoveryError::IoFailure("exFAT directory exceeds traversal limit".into())); }
    Ok(chain)
}

pub fn read_root_entries<D: BlockDevice>(volume: &ExFatVolume, device: &D, volume_range: ByteRange) -> RecoveryResult<Vec<ExFatDirectoryEntry>> {
    read_directory(volume, device, volume_range, volume.root_directory_cluster)
}

pub fn read_directory<D: BlockDevice>(volume: &ExFatVolume, device: &D, volume_range: ByteRange, start_cluster: u32) -> RecoveryResult<Vec<ExFatDirectoryEntry>> {
    let chain = directory_chain(volume, device, volume_range, start_cluster)?;
    let bytes = read_clusters(volume, device, volume_range, &chain)?;
    parse_directory_entries(&bytes)
}

/// Scans an exFAT directory stream for inactive file entry sets left by deletion.
/// Unlike active parsing, this intentionally ignores active file sets and keeps
/// scanning after end-of-directory markers so callers can inspect slack/tail data.
pub fn read_deleted_directory<D: BlockDevice>(volume: &ExFatVolume, device: &D, volume_range: ByteRange, start_cluster: u32) -> RecoveryResult<Vec<ExFatDirectoryEntry>> {
    let chain = directory_chain(volume, device, volume_range, start_cluster)?;
    let bytes = read_clusters(volume, device, volume_range, &chain)?;
    parse_deleted_directory_entries(&bytes)
}

pub fn read_deleted_root_entries<D: BlockDevice>(volume: &ExFatVolume, device: &D, volume_range: ByteRange) -> RecoveryResult<Vec<ExFatDirectoryEntry>> {
    read_deleted_directory(volume, device, volume_range, volume.root_directory_cluster)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExFatTreeEntry { pub path: String, pub entry: ExFatDirectoryEntry }

fn read_subdirectory<D: BlockDevice>(volume: &ExFatVolume, device: &D, volume_range: ByteRange, entry: &ExFatDirectoryEntry) -> RecoveryResult<Vec<ExFatDirectoryEntry>> {
    if entry.data_length == 0 { return Ok(Vec::new()); }
    let ranges = active_file_extents(volume, device, volume_range, entry)?;
    if !entry.data_length.is_multiple_of(32) { return Err(RecoveryError::IoFailure("exFAT directory data length is not entry-aligned".into())); }
    let bytes = read_ranges(device, &ranges, entry.data_length, "subdirectory")?;
    parse_directory_entries(&bytes)
}

fn walk_directory<D: BlockDevice>(volume: &ExFatVolume, device: &D, volume_range: ByteRange, entries: Vec<ExFatDirectoryEntry>, prefix: &str, depth: usize, visited: &mut std::collections::BTreeSet<u32>, out: &mut Vec<ExFatTreeEntry>) -> RecoveryResult<()> {
    if depth > MAX_TREE_DEPTH { return Err(RecoveryError::IoFailure("exFAT directory tree exceeds depth limit".into())); }
    for entry in entries {
        let path = if prefix.is_empty() { entry.name.clone() } else { format!("{prefix}/{}", entry.name) };
        out.push(ExFatTreeEntry { path: path.clone(), entry: entry.clone() });
        if out.len() > MAX_TREE_ENTRIES { return Err(RecoveryError::IoFailure("exFAT directory tree exceeds entry limit".into())); }
        if entry.attributes & 0x0010 == 0 || entry.data_length == 0 { continue; }
        if !visited.insert(entry.first_cluster) { return Err(RecoveryError::IoFailure("exFAT directory tree contains a cluster cycle".into())); }
        let children = read_subdirectory(volume, device, volume_range, &entry)?;
        walk_directory(volume, device, volume_range, children, &path, depth + 1, visited, out)?;
    }
    Ok(())
}

pub fn read_tree<D: BlockDevice>(volume: &ExFatVolume, device: &D, volume_range: ByteRange) -> RecoveryResult<Vec<ExFatTreeEntry>> {
    let root = read_root_entries(volume, device, volume_range)?;
    let mut visited = std::collections::BTreeSet::new();
    visited.insert(volume.root_directory_cluster);
    let mut out = Vec::new();
    walk_directory(volume, device, volume_range, root, "", 0, &mut visited, &mut out)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Dummy;
    impl BlockDevice for Dummy { fn capacity(&self) -> u64 { 65_536 } fn read(&self, _range: ByteRange, _buffer: &mut [u8]) -> RecoveryResult<usize> { unreachable!("directory tests exercise validation before device reads") } }
    #[test]
    fn rejects_invalid_directory_start() {
        let volume = ExFatVolume { partition_offset_sectors: 0, volume_length_sectors: 128, fat_offset_sectors: 1, fat_length_sectors: 1, cluster_heap_offset_sectors: 2, cluster_count: 10, root_directory_cluster: 2, bytes_per_sector: 512, bytes_per_cluster: 1024 };
        assert!(read_directory(&volume, &Dummy, ByteRange::new(0, 65_536).unwrap(), 1).is_err());
        assert!(read_deleted_directory(&volume, &Dummy, ByteRange::new(0, 65_536).unwrap(), 1).is_err());
    }
    #[test]
    fn tree_entry_preserves_relative_path() {
        let entry = ExFatDirectoryEntry { name: "photo.jpg".into(), attributes: 0, first_cluster: 7, data_length: 12, no_fat_chain: true };
        let mut out = Vec::new(); let mut visited = std::collections::BTreeSet::new();
        let volume = ExFatVolume { partition_offset_sectors: 0, volume_length_sectors: 128, fat_offset_sectors: 1, fat_length_sectors: 1, cluster_heap_offset_sectors: 2, cluster_count: 10, root_directory_cluster: 2, bytes_per_sector: 512, bytes_per_cluster: 1024 };
        let result = walk_directory(&volume, &Dummy, ByteRange::new(0, 65_536).unwrap(), vec![entry], "DCIM", 0, &mut visited, &mut out);
        assert!(result.is_ok()); assert_eq!(out[0].path, "DCIM/photo.jpg");
    }
}
