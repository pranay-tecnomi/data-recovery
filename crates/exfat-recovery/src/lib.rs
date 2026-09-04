#![forbid(unsafe_code)]

use recovery_core::{ByteRange, RecoveryError, RecoveryResult};
use storage_io::BlockDevice;

/// Validated geometry from the exFAT main boot sector.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExFatVolume {
    pub partition_offset_sectors: u64,
    pub volume_length_sectors: u64,
    pub fat_offset_sectors: u32,
    pub fat_length_sectors: u32,
    pub cluster_heap_offset_sectors: u32,
    pub cluster_count: u32,
    pub root_directory_cluster: u32,
    pub bytes_per_sector: u64,
    pub bytes_per_cluster: u64,
}

impl ExFatVolume {
    pub fn cluster_range(&self, cluster: u32) -> RecoveryResult<ByteRange> {
        if cluster < 2 || cluster >= self.cluster_count.saturating_add(2) {
            return Err(RecoveryError::InvalidRange);
        }
        let index = u64::from(cluster - 2);
        let sector = u64::from(self.cluster_heap_offset_sectors)
            .checked_add(index.checked_mul(self.bytes_per_cluster / self.bytes_per_sector).ok_or(RecoveryError::InvalidRange)?)
            .ok_or(RecoveryError::InvalidRange)?;
        ByteRange::new(
            sector.checked_mul(self.bytes_per_sector).ok_or(RecoveryError::InvalidRange)?,
            self.bytes_per_cluster,
        )
    }
}

fn u32_at(b: &[u8; 512], start: usize) -> u32 {
    u32::from_le_bytes(b[start..start + 4].try_into().expect("fixed slice"))
}
fn u64_at(b: &[u8; 512], start: usize) -> u64 {
    u64::from_le_bytes(b[start..start + 8].try_into().expect("fixed slice"))
}

pub fn parse_boot_sector(boot: &[u8; 512]) -> RecoveryResult<ExFatVolume> {
    if &boot[3..11] != b"EXFAT   " || boot[510] != 0x55 || boot[511] != 0xAA {
        return Err(RecoveryError::IoFailure("not an exFAT boot sector".into()));
    }
    if boot[11..64].iter().any(|&x| x != 0) {
        return Err(RecoveryError::IoFailure("exFAT reserved boot bytes are non-zero".into()));
    }
    let sector_shift = boot[108];
    let cluster_shift = boot[109];
    if !(9..=12).contains(&sector_shift) || cluster_shift > 25 || u16::from(sector_shift) + u16::from(cluster_shift) > 30 {
        return Err(RecoveryError::IoFailure("invalid exFAT shift geometry".into()));
    }
    let bytes_per_sector = 1u64 << sector_shift;
    let bytes_per_cluster = 1u64 << (u16::from(sector_shift) + u16::from(cluster_shift));
    let partition_offset_sectors = u64_at(boot, 64);
    let volume_length_sectors = u64_at(boot, 72);
    let fat_offset_sectors = u32_at(boot, 80);
    let fat_length_sectors = u32_at(boot, 84);
    let cluster_heap_offset_sectors = u32_at(boot, 88);
    let cluster_count = u32_at(boot, 92);
    let root_directory_cluster = u32_at(boot, 96);
    if volume_length_sectors == 0 || fat_offset_sectors == 0 || fat_length_sectors == 0 || cluster_heap_offset_sectors == 0 || cluster_count == 0 || root_directory_cluster < 2 || root_directory_cluster >= cluster_count.saturating_add(2) {
        return Err(RecoveryError::IoFailure("invalid exFAT geometry".into()));
    }
    let sectors_per_cluster = 1u64 << cluster_shift;
    let heap_end = u64::from(cluster_heap_offset_sectors)
        .checked_add(u64::from(cluster_count).checked_mul(sectors_per_cluster).ok_or(RecoveryError::InvalidRange)?)
        .ok_or(RecoveryError::InvalidRange)?;
    if heap_end > volume_length_sectors || u64::from(fat_offset_sectors) + u64::from(fat_length_sectors) > u64::from(cluster_heap_offset_sectors) {
        return Err(RecoveryError::IoFailure("inconsistent exFAT layout".into()));
    }
    Ok(ExFatVolume { partition_offset_sectors, volume_length_sectors, fat_offset_sectors, fat_length_sectors, cluster_heap_offset_sectors, cluster_count, root_directory_cluster, bytes_per_sector, bytes_per_cluster })
}

pub fn open<D: BlockDevice>(device: &D, range: ByteRange) -> RecoveryResult<ExFatVolume> {
    range.validate_within(device.capacity())?;
    if range.length < 512 { return Err(RecoveryError::InvalidRange); }
    let mut boot = [0u8; 512];
    if device.read(ByteRange::new(range.offset, 512)?, &mut boot)? != 512 { return Err(RecoveryError::IoFailure("short exFAT boot-sector read".into())); }
    parse_boot_sector(&boot)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn boot() -> [u8; 512] { let mut b=[0u8;512]; b[3..11].copy_from_slice(b"EXFAT   "); b[64..72].copy_from_slice(&0u64.to_le_bytes()); b[72..80].copy_from_slice(&10_000u64.to_le_bytes()); b[80..84].copy_from_slice(&24u32.to_le_bytes()); b[84..88].copy_from_slice(&100u32.to_le_bytes()); b[88..92].copy_from_slice(&128u32.to_le_bytes()); b[92..96].copy_from_slice(&1000u32.to_le_bytes()); b[96..100].copy_from_slice(&2u32.to_le_bytes()); b[108]=9; b[109]=3; b[510]=0x55; b[511]=0xAA; b }
    #[test] fn parses_and_maps_clusters() { let v=parse_boot_sector(&boot()).unwrap(); assert_eq!(v.bytes_per_cluster,4096); assert_eq!(v.cluster_range(2).unwrap(),ByteRange::new(65536,4096).unwrap()); assert_eq!(v.cluster_range(1002).unwrap_err(),RecoveryError::InvalidRange); }
    #[test] fn rejects_heap_beyond_volume() { let mut b=boot(); b[72..80].copy_from_slice(&129u64.to_le_bytes()); assert!(parse_boot_sector(&b).is_err()); }
    #[test] fn rejects_nonzero_reserved() { let mut b=boot(); b[12]=1; assert!(parse_boot_sector(&b).is_err()); }
}
