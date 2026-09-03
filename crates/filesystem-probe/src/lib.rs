#![forbid(unsafe_code)]

use recovery_core::{ByteRange, RecoveryError, RecoveryResult};
use storage_io::BlockDevice;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FilesystemKind { Unknown, Fat32, ExFat }

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProbeEvidence {
    pub kind: FilesystemKind,
    pub confidence: u8,
    pub notes: Vec<&'static str>,
}

fn read_boot<D: BlockDevice>(device: &D, range: ByteRange) -> RecoveryResult<[u8; 512]> {
    if range.length < 512 { return Err(RecoveryError::IoFailure("probe range smaller than boot sector".into())); }
    let mut boot = [0u8; 512];
    let n = device.read(ByteRange::new(range.offset, 512)?, &mut boot)?;
    if n != boot.len() { return Err(RecoveryError::IoFailure("short boot-sector read".into())); }
    Ok(boot)
}

fn valid_signature(boot: &[u8; 512]) -> bool { boot[510] == 0x55 && boot[511] == 0xAA }
fn power_of_two_in(value: u64, min: u64, max: u64) -> bool { value.is_power_of_two() && (min..=max).contains(&value) }

fn probe_exfat(boot: &[u8; 512]) -> Option<ProbeEvidence> {
    if &boot[3..11] != b"EXFAT   " || !valid_signature(boot) { return None; }
    // exFAT reserves bytes 11..=63 as zero in the main boot region.
    if boot[11..64].iter().any(|&b| b != 0) { return None; }
    let sector_shift = boot[108];
    let cluster_shift = boot[109];
    let fat_offset = u32::from_le_bytes(boot[80..84].try_into().ok()?);
    let fat_length = u32::from_le_bytes(boot[84..88].try_into().ok()?);
    let cluster_heap_offset = u32::from_le_bytes(boot[88..92].try_into().ok()?);
    let cluster_count = u32::from_le_bytes(boot[92..96].try_into().ok()?);
    let root_cluster = u32::from_le_bytes(boot[96..100].try_into().ok()?);
    if !(9..=12).contains(&sector_shift) || cluster_shift > 25 { return None; }
    if fat_offset == 0 || fat_length == 0 || cluster_heap_offset == 0 || cluster_count == 0 { return None; }
    if root_cluster < 2 || root_cluster >= cluster_count.saturating_add(2) { return None; }
    Some(ProbeEvidence { kind: FilesystemKind::ExFat, confidence: 95, notes: vec!["exFAT OEM identifier", "reserved bytes", "plausible boot geometry", "boot signature"] })
}

fn probe_fat32(boot: &[u8; 512]) -> Option<ProbeEvidence> {
    if !valid_signature(boot) { return None; }
    let bytes_per_sector = u16::from_le_bytes([boot[11], boot[12]]) as u64;
    let sectors_per_cluster = boot[13] as u64;
    let reserved_sectors = u16::from_le_bytes([boot[14], boot[15]]) as u64;
    let fats = boot[16] as u64;
    let root_entry_count = u16::from_le_bytes([boot[17], boot[18]]);
    let total16 = u16::from_le_bytes([boot[19], boot[20]]) as u64;
    let total32 = u32::from_le_bytes([boot[32], boot[33], boot[34], boot[35]]) as u64;
    let fat16 = u16::from_le_bytes([boot[22], boot[23]]) as u64;
    let fat32 = u32::from_le_bytes([boot[36], boot[37], boot[38], boot[39]]) as u64;
    let root_cluster = u32::from_le_bytes([boot[44], boot[45], boot[46], boot[47]]);
    let total = if total16 != 0 { total16 } else { total32 };
    if !power_of_two_in(bytes_per_sector, 512, 4096) || !sectors_per_cluster.is_power_of_two() || sectors_per_cluster == 0 { return None; }
    if reserved_sectors == 0 || fats == 0 || root_entry_count != 0 || fat16 != 0 || fat32 == 0 || total == 0 || root_cluster < 2 { return None; }
    let data_start = reserved_sectors.checked_add(fats.checked_mul(fat32)?)?;
    if data_start >= total { return None; }
    let data_sectors = total - data_start;
    let clusters = data_sectors / sectors_per_cluster;
    // FAT32 requires at least 65525 clusters; do not trust the cosmetic type label alone.
    if clusters < 65_525 || root_cluster >= clusters.saturating_add(2) { return None; }
    Some(ProbeEvidence { kind: FilesystemKind::Fat32, confidence: 95, notes: vec!["FAT32 BPB geometry", "FAT32 cluster count", "root cluster", "boot signature"] })
}

pub fn probe<D: BlockDevice>(device: &D, range: ByteRange) -> RecoveryResult<ProbeEvidence> {
    range.validate_within(device.capacity())?;
    let boot = read_boot(device, range)?;
    if let Some(evidence) = probe_exfat(&boot) { return Ok(evidence); }
    if let Some(evidence) = probe_fat32(&boot) { return Ok(evidence); }
    Ok(ProbeEvidence { kind: FilesystemKind::Unknown, confidence: 0, notes: vec!["no supported filesystem signature"] })
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Mem(Vec<u8>);
    impl BlockDevice for Mem {
        fn capacity(&self) -> u64 { self.0.len() as u64 }
        fn read(&self, r: ByteRange, o: &mut [u8]) -> RecoveryResult<usize> { r.validate_within(self.capacity())?; let n=r.length as usize; o[..n].copy_from_slice(&self.0[r.offset as usize..r.offset as usize+n]); Ok(n) }
    }
    fn exfat() -> Vec<u8> { let mut b=vec![0u8;512]; b[3..11].copy_from_slice(b"EXFAT   "); b[80..84].copy_from_slice(&24u32.to_le_bytes()); b[84..88].copy_from_slice(&128u32.to_le_bytes()); b[88..92].copy_from_slice(&256u32.to_le_bytes()); b[92..96].copy_from_slice(&100u32.to_le_bytes()); b[96..100].copy_from_slice(&2u32.to_le_bytes()); b[108]=9;b[109]=3;b[510]=0x55;b[511]=0xAA;b }
    fn fat32() -> Vec<u8> { let mut b=vec![0u8;512]; b[11..13].copy_from_slice(&512u16.to_le_bytes()); b[13]=1;b[14..16].copy_from_slice(&32u16.to_le_bytes());b[16]=2;b[32..36].copy_from_slice(&200_000u32.to_le_bytes());b[36..40].copy_from_slice(&1000u32.to_le_bytes());b[44..48].copy_from_slice(&2u32.to_le_bytes());b[510]=0x55;b[511]=0xAA;b }
    #[test] fn detects_exfat() { let r=probe(&Mem(exfat()),ByteRange::new(0,512).unwrap()).unwrap(); assert_eq!(r.kind,FilesystemKind::ExFat); }
    #[test] fn rejects_exfat_nonzero_reserved_bytes() { let mut b=exfat(); b[11]=1; let r=probe(&Mem(b),ByteRange::new(0,512).unwrap()).unwrap(); assert_eq!(r.kind,FilesystemKind::Unknown); }
    #[test] fn detects_fat32_without_cosmetic_label() { let r=probe(&Mem(fat32()),ByteRange::new(0,512).unwrap()).unwrap(); assert_eq!(r.kind,FilesystemKind::Fat32); }
    #[test] fn rejects_small_fat_variant() { let mut b=fat32(); b[32..36].copy_from_slice(&1000u32.to_le_bytes()); let r=probe(&Mem(b),ByteRange::new(0,512).unwrap()).unwrap(); assert_eq!(r.kind,FilesystemKind::Unknown); }
}
