//! exFAT boot region parsing and geometry validation.
//!
//! All fields are untrusted: geometry is validated against the volume before
//! any derived offset is used, and every conversion is checked.

use recovery_core::{ByteRange, RecoveryError, RecoveryResult};
use storage_io::BlockDevice;

/// Largest shift accepted for bytes-per-sector (2^12 = 4096).
const MAX_SECTOR_SHIFT: u8 = 12;
const MIN_SECTOR_SHIFT: u8 = 9;
/// A cluster may not exceed 32MB, per the exFAT specification.
const MAX_CLUSTER_BYTES: u64 = 32 * 1024 * 1024;
/// exFAT reserves the two lowest cluster indices.
pub const FIRST_CLUSTER: u32 = 2;

pub(crate) fn io_error(message: impl Into<String>) -> RecoveryError {
    RecoveryError::IoFailure(message.into())
}

/// Validated exFAT volume geometry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExfatVolume {
    pub bytes_per_sector: u64,
    pub sectors_per_cluster: u64,
    pub fat_offset_sectors: u64,
    pub fat_length_sectors: u64,
    pub cluster_heap_offset_sectors: u64,
    pub cluster_count: u32,
    pub root_directory_cluster: u32,
    pub volume_length_sectors: u64,
    pub fat_count: u8,
    /// True when the volume is flagged dirty, which is evidence that metadata
    /// may be inconsistent.
    pub volume_dirty: bool,
}

/// Parses and validates the exFAT boot sector at the start of `range`.
pub fn parse_volume<D: BlockDevice>(device: &D, range: ByteRange) -> RecoveryResult<ExfatVolume> {
    range.validate_within(device.capacity())?;
    if range.length < 512 {
        return Err(io_error("exFAT range smaller than a boot sector"));
    }
    let mut boot = [0u8; 512];
    let read = device.read(ByteRange::new(range.offset, 512)?, &mut boot)?;
    if read != boot.len() {
        return Err(io_error("short exFAT boot-sector read"));
    }

    if &boot[3..11] != b"EXFAT   " {
        return Err(io_error("missing exFAT OEM identifier"));
    }
    if boot[510] != 0x55 || boot[511] != 0xAA {
        return Err(io_error("invalid exFAT boot signature"));
    }
    // Bytes 11..64 are defined as zero; a non-zero value indicates this is not
    // a genuine exFAT boot sector.
    if boot[11..64].iter().any(|&b| b != 0) {
        return Err(io_error("exFAT reserved boot bytes are not zero"));
    }

    let u32_at = |start: usize| -> RecoveryResult<u32> {
        boot.get(start..start + 4)
            .and_then(|s| s.try_into().ok())
            .map(u32::from_le_bytes)
            .ok_or_else(|| io_error("truncated boot field"))
    };
    let u64_at = |start: usize| -> RecoveryResult<u64> {
        boot.get(start..start + 8)
            .and_then(|s| s.try_into().ok())
            .map(u64::from_le_bytes)
            .ok_or_else(|| io_error("truncated boot field"))
    };

    let volume_length = u64_at(72)?;
    let fat_offset = u64::from(u32_at(80)?);
    let fat_length = u64::from(u32_at(84)?);
    let cluster_heap_offset = u64::from(u32_at(88)?);
    let cluster_count = u32_at(92)?;
    let root_directory_cluster = u32_at(96)?;
    let sector_shift = boot[108];
    let cluster_shift = boot[109];
    let fat_count = boot[110];
    let volume_flags = u16::from_le_bytes([boot[106], boot[107]]);

    if !(MIN_SECTOR_SHIFT..=MAX_SECTOR_SHIFT).contains(&sector_shift) {
        return Err(io_error("exFAT sector shift out of range"));
    }
    // Shifts are bounded above so the shift itself cannot overflow.
    if u32::from(sector_shift) + u32::from(cluster_shift) > 25 {
        return Err(io_error("exFAT cluster shift out of range"));
    }
    // exFAT permits one or two FATs; TexFAT (two) is not supported for
    // recovery, but a valid count is still required.
    if fat_count == 0 || fat_count > 2 {
        return Err(io_error("invalid exFAT FAT count"));
    }

    let bytes_per_sector = 1u64 << sector_shift;
    let sectors_per_cluster = 1u64 << cluster_shift;
    let cluster_bytes = bytes_per_sector
        .checked_mul(sectors_per_cluster)
        .ok_or(RecoveryError::RangeOverflow)?;
    if cluster_bytes > MAX_CLUSTER_BYTES {
        return Err(io_error("exFAT cluster size exceeds 32MB"));
    }

    if volume_length == 0 || fat_offset == 0 || fat_length == 0 || cluster_heap_offset == 0 {
        return Err(io_error("exFAT geometry contains a zero required field"));
    }
    if cluster_count == 0 {
        return Err(io_error("exFAT volume declares no clusters"));
    }
    // The FAT must lie between the reserved area and the cluster heap.
    let fat_end = fat_offset
        .checked_add(
            fat_length
                .checked_mul(u64::from(fat_count))
                .ok_or(RecoveryError::RangeOverflow)?,
        )
        .ok_or(RecoveryError::RangeOverflow)?;
    if fat_end > cluster_heap_offset {
        return Err(io_error("exFAT FAT region overlaps the cluster heap"));
    }

    // The heap must fit inside the declared volume.
    let heap_sectors = u64::from(cluster_count)
        .checked_mul(sectors_per_cluster)
        .ok_or(RecoveryError::RangeOverflow)?;
    let heap_end = cluster_heap_offset
        .checked_add(heap_sectors)
        .ok_or(RecoveryError::RangeOverflow)?;
    if heap_end > volume_length {
        return Err(io_error("exFAT cluster heap extends beyond the volume"));
    }

    if root_directory_cluster < FIRST_CLUSTER
        || u64::from(root_directory_cluster) >= u64::from(cluster_count) + u64::from(FIRST_CLUSTER)
    {
        return Err(io_error("exFAT root directory cluster out of range"));
    }

    // The volume must fit in the range the caller supplied.
    let declared_bytes = volume_length
        .checked_mul(bytes_per_sector)
        .ok_or(RecoveryError::RangeOverflow)?;
    if declared_bytes > range.length {
        return Err(io_error("exFAT volume exceeds the supplied range"));
    }

    Ok(ExfatVolume {
        bytes_per_sector,
        sectors_per_cluster,
        fat_offset_sectors: fat_offset,
        fat_length_sectors: fat_length,
        cluster_heap_offset_sectors: cluster_heap_offset,
        cluster_count,
        root_directory_cluster,
        volume_length_sectors: volume_length,
        fat_count,
        // Bit 1 is VolumeDirty.
        volume_dirty: volume_flags & 0x0002 != 0,
    })
}

impl ExfatVolume {
    pub fn cluster_size(&self) -> RecoveryResult<u64> {
        self.bytes_per_sector
            .checked_mul(self.sectors_per_cluster)
            .ok_or(RecoveryError::RangeOverflow)
    }

    /// Highest valid cluster index, exclusive.
    pub fn cluster_limit(&self) -> u64 {
        u64::from(self.cluster_count) + u64::from(FIRST_CLUSTER)
    }

    pub fn is_valid_cluster(&self, cluster: u32) -> bool {
        cluster >= FIRST_CLUSTER && u64::from(cluster) < self.cluster_limit()
    }

    /// Byte offset of `cluster` within the source, given the volume's start.
    pub fn cluster_offset(&self, volume_start: u64, cluster: u32) -> RecoveryResult<u64> {
        if !self.is_valid_cluster(cluster) {
            return Err(io_error(format!("cluster {cluster} outside the exFAT heap")));
        }
        let index = u64::from(cluster - FIRST_CLUSTER);
        let sector = self
            .cluster_heap_offset_sectors
            .checked_add(
                index
                    .checked_mul(self.sectors_per_cluster)
                    .ok_or(RecoveryError::RangeOverflow)?,
            )
            .ok_or(RecoveryError::RangeOverflow)?;
        volume_start
            .checked_add(
                sector
                    .checked_mul(self.bytes_per_sector)
                    .ok_or(RecoveryError::RangeOverflow)?,
            )
            .ok_or(RecoveryError::RangeOverflow)
    }

    /// Byte offset of `cluster`'s entry in the first FAT.
    pub(crate) fn fat_entry_offset(&self, volume_start: u64, cluster: u32) -> RecoveryResult<u64> {
        if !self.is_valid_cluster(cluster) {
            return Err(io_error(format!("cluster {cluster} outside the exFAT heap")));
        }
        let base = self
            .fat_offset_sectors
            .checked_mul(self.bytes_per_sector)
            .ok_or(RecoveryError::RangeOverflow)?;
        let entry = u64::from(cluster)
            .checked_mul(4)
            .ok_or(RecoveryError::RangeOverflow)?;
        // The entry must lie inside the declared FAT, not merely be arithmetically valid.
        let fat_bytes = self
            .fat_length_sectors
            .checked_mul(self.bytes_per_sector)
            .ok_or(RecoveryError::RangeOverflow)?;
        if entry.checked_add(4).ok_or(RecoveryError::RangeOverflow)? > fat_bytes {
            return Err(io_error("FAT entry beyond the declared FAT length"));
        }
        volume_start
            .checked_add(base)
            .and_then(|v| v.checked_add(entry))
            .ok_or(RecoveryError::RangeOverflow)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testimage::{image, CLUSTER_COUNT, HEAP_SECTOR, TOTAL_SECTORS};

    fn parsed(m: &crate::testimage::Mem) -> RecoveryResult<ExfatVolume> {
        parse_volume(m, m.range())
    }

    #[test]
    fn parses_valid_geometry() {
        let v = parsed(&image()).unwrap();
        assert_eq!(v.bytes_per_sector, 512);
        assert_eq!(v.sectors_per_cluster, 1);
        assert_eq!(v.cluster_count, CLUSTER_COUNT);
        assert_eq!(v.root_directory_cluster, 2);
        assert!(!v.volume_dirty);
    }

    #[test]
    fn computes_cluster_offsets() {
        let v = parsed(&image()).unwrap();
        // Cluster 2 is the first cluster of the heap.
        assert_eq!(v.cluster_offset(0, 2).unwrap(), (HEAP_SECTOR * 512) as u64);
        assert_eq!(v.cluster_offset(0, 3).unwrap(), (HEAP_SECTOR * 512 + 512) as u64);
    }

    #[test]
    fn rejects_clusters_outside_the_heap() {
        let v = parsed(&image()).unwrap();
        assert!(v.cluster_offset(0, 0).is_err());
        assert!(v.cluster_offset(0, 1).is_err());
        assert!(v.cluster_offset(0, CLUSTER_COUNT + 2).is_err());
        assert!(v.cluster_offset(0, u32::MAX).is_err());
        assert!(!v.is_valid_cluster(u32::MAX));
    }

    #[test]
    fn rejects_missing_oem_identifier() {
        let mut m = image();
        m.0[3] = b'X';
        assert!(parsed(&m).is_err());
    }

    #[test]
    fn rejects_bad_boot_signature() {
        let mut m = image();
        m.0[510] = 0;
        assert!(parsed(&m).is_err());
    }

    #[test]
    fn rejects_nonzero_reserved_bytes() {
        let mut m = image();
        // A FAT32 BPB would leave these non-zero; exFAT requires them clear.
        m.0[11] = 1;
        assert!(parsed(&m).is_err());
    }

    #[test]
    fn rejects_out_of_range_sector_shift() {
        for shift in [8u8, 13, 255] {
            let mut m = image();
            m.0[108] = shift;
            assert!(parsed(&m).is_err(), "shift {shift} must be rejected");
        }
    }

    #[test]
    fn rejects_oversized_cluster() {
        let mut m = image();
        // 2^9 * 2^17 = 64MB, beyond the 32MB maximum.
        m.0[109] = 17;
        assert!(parsed(&m).is_err());
    }

    #[test]
    fn rejects_invalid_fat_count() {
        for count in [0u8, 3, 255] {
            let mut m = image();
            m.0[110] = count;
            assert!(parsed(&m).is_err(), "fat count {count} must be rejected");
        }
    }

    #[test]
    fn rejects_fat_overlapping_the_cluster_heap() {
        let mut m = image();
        // A FAT long enough to run into the heap.
        m.0[84..88].copy_from_slice(&1000u32.to_le_bytes());
        assert!(parsed(&m).is_err());
    }

    #[test]
    fn rejects_heap_extending_beyond_the_volume() {
        let mut m = image();
        m.0[92..96].copy_from_slice(&100_000u32.to_le_bytes());
        assert!(parsed(&m).is_err());
    }

    #[test]
    fn rejects_root_cluster_outside_the_heap() {
        for root in [0u32, 1, CLUSTER_COUNT + 2] {
            let mut m = image();
            m.0[96..100].copy_from_slice(&root.to_le_bytes());
            assert!(parsed(&m).is_err(), "root {root} must be rejected");
        }
    }

    #[test]
    fn rejects_zero_required_fields() {
        for offset in [72usize, 80, 84, 88] {
            let mut m = image();
            m.0[offset..offset + 4].copy_from_slice(&0u32.to_le_bytes());
            if offset == 72 {
                m.0[72..80].copy_from_slice(&0u64.to_le_bytes());
            }
            assert!(parsed(&m).is_err(), "zero at {offset} must be rejected");
        }
    }

    #[test]
    fn rejects_volume_larger_than_the_supplied_range() {
        let mut m = image();
        m.0[72..80].copy_from_slice(&(TOTAL_SECTORS as u64 * 4).to_le_bytes());
        assert!(parsed(&m).is_err());
    }

    #[test]
    fn rejects_range_smaller_than_a_boot_sector() {
        let m = image();
        assert!(parse_volume(&m, ByteRange::new(0, 128).unwrap()).is_err());
    }

    #[test]
    fn reports_dirty_volume_flag() {
        let mut m = image();
        m.0[106..108].copy_from_slice(&0x0002u16.to_le_bytes());
        assert!(parsed(&m).unwrap().volume_dirty);
    }

    #[test]
    fn rejects_fat_entry_beyond_the_declared_fat() {
        let mut v = parsed(&image()).unwrap();
        // A volume claiming many clusters but a one-sector FAT: entries for
        // the highest clusters fall outside the FAT and must not be read.
        v.cluster_count = 100_000;
        v.fat_length_sectors = 1;
        assert!(v.fat_entry_offset(0, 99_000).is_err());
        // A cluster whose entry does lie inside the FAT still resolves.
        assert!(v.fat_entry_offset(0, 10).is_ok());
    }

    #[test]
    fn fat_entry_offsets_are_sequential() {
        let v = parsed(&image()).unwrap();
        let a = v.fat_entry_offset(0, 2).unwrap();
        let b = v.fat_entry_offset(0, 3).unwrap();
        assert_eq!(b - a, 4);
    }
}
