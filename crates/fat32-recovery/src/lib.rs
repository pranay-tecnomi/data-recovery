#![forbid(unsafe_code)]

use recovery_core::{ByteRange, RecoveryError, RecoveryResult};
use storage_io::BlockDevice;

const DIR_ENTRY_SIZE: usize = 32;
const END_OF_DIRECTORY: u8 = 0x00;
const DELETED: u8 = 0xE5;
const LFN_ATTR: u8 = 0x0F;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Fat32Volume {
    pub bytes_per_sector: u64,
    pub sectors_per_cluster: u64,
    pub first_data_sector: u64,
    pub root_cluster: u32,
    pub cluster_count: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DirectoryEntry {
    pub short_name: String,
    pub attributes: u8,
    pub first_cluster: u32,
    pub size: u32,
    pub deleted: bool,
}

fn io_error(message: &str) -> RecoveryError { RecoveryError::IoFailure(message.into()) }

fn read_exact<D: BlockDevice>(device: &D, range: ByteRange, output: &mut [u8]) -> RecoveryResult<()> {
    if output.len() as u64 != range.length { return Err(io_error("read buffer length mismatch")); }
    let n = device.read(range, output)?;
    if n != output.len() { return Err(io_error("short FAT32 read")); }
    Ok(())
}

pub fn parse_volume<D: BlockDevice>(device: &D, range: ByteRange) -> RecoveryResult<Fat32Volume> {
    range.validate_within(device.capacity())?;
    if range.length < 512 { return Err(io_error("FAT32 range smaller than boot sector")); }
    let mut boot = [0u8; 512];
    read_exact(device, ByteRange::new(range.offset, 512)?, &mut boot)?;
    if boot[510] != 0x55 || boot[511] != 0xAA { return Err(io_error("invalid FAT32 boot signature")); }
    let bps = u16::from_le_bytes([boot[11], boot[12]]) as u64;
    let spc = boot[13] as u64;
    let reserved = u16::from_le_bytes([boot[14], boot[15]]) as u64;
    let fats = boot[16] as u64;
    let total = u32::from_le_bytes(boot[32..36].try_into().map_err(|_| io_error("invalid boot sector"))?) as u64;
    let fat_size = u32::from_le_bytes(boot[36..40].try_into().map_err(|_| io_error("invalid boot sector"))?) as u64;
    let root_cluster = u32::from_le_bytes(boot[44..48].try_into().map_err(|_| io_error("invalid boot sector"))?);
    if !bps.is_power_of_two() || !(512..=4096).contains(&bps) || !spc.is_power_of_two() || spc == 0 || reserved == 0 || fats == 0 || fat_size == 0 || total == 0 || root_cluster < 2 { return Err(io_error("invalid FAT32 geometry")); }
    let first_data_sector = reserved.checked_add(fats.checked_mul(fat_size).ok_or_else(|| io_error("FAT size overflow"))?).ok_or_else(|| io_error("data offset overflow"))?;
    if first_data_sector >= total { return Err(io_error("FAT32 data region outside volume")); }
    let clusters = (total - first_data_sector) / spc;
    if clusters < 65_525 || root_cluster as u64 >= clusters.saturating_add(2) { return Err(io_error("invalid FAT32 cluster geometry")); }
    Ok(Fat32Volume { bytes_per_sector: bps, sectors_per_cluster: spc, first_data_sector, root_cluster, cluster_count: clusters })
}

impl Fat32Volume {
    pub fn cluster_size(&self) -> RecoveryResult<u64> { self.bytes_per_sector.checked_mul(self.sectors_per_cluster).ok_or_else(|| io_error("cluster size overflow")) }
    pub fn cluster_offset(&self, volume_start: u64, cluster: u32) -> RecoveryResult<u64> {
        if cluster < 2 || cluster as u64 >= self.cluster_count.saturating_add(2) { return Err(io_error("cluster outside FAT32 data region")); }
        let sector_delta = (cluster as u64).checked_sub(2).ok_or_else(|| io_error("invalid cluster"))?.checked_mul(self.sectors_per_cluster).ok_or_else(|| io_error("cluster sector overflow"))?;
        let sector = self.first_data_sector.checked_add(sector_delta).ok_or_else(|| io_error("cluster sector overflow"))?;
        volume_start.checked_add(sector.checked_mul(self.bytes_per_sector).ok_or_else(|| io_error("cluster byte overflow"))?).ok_or_else(|| io_error("cluster offset overflow"))
    }
}

fn short_name(entry: &[u8]) -> String {
    let base = String::from_utf8_lossy(&entry[0..8]).trim_end().to_string();
    let ext = String::from_utf8_lossy(&entry[8..11]).trim_end().to_string();
    if ext.is_empty() { base } else { format!("{base}.{ext}") }
}

pub fn read_root_entries<D: BlockDevice>(device: &D, volume_range: ByteRange, include_deleted: bool) -> RecoveryResult<Vec<DirectoryEntry>> {
    let volume = parse_volume(device, volume_range)?;
    let size = volume.cluster_size()?;
    if size > 16 * 1024 * 1024 { return Err(io_error("FAT32 cluster exceeds metadata read limit")); }
    let offset = volume.cluster_offset(volume_range.offset, volume.root_cluster)?;
    let range = ByteRange::new(offset, size)?;
    range.validate_within(volume_range.offset.checked_add(volume_range.length).ok_or_else(|| io_error("volume range overflow"))?)?;
    let mut data = vec![0u8; size as usize];
    read_exact(device, range, &mut data)?;
    let mut entries = Vec::new();
    for raw in data.chunks_exact(DIR_ENTRY_SIZE) {
        if raw[0] == END_OF_DIRECTORY { break; }
        let deleted = raw[0] == DELETED;
        if raw[11] == LFN_ATTR || (deleted && !include_deleted) { continue; }
        let high = u16::from_le_bytes([raw[20], raw[21]]) as u32;
        let low = u16::from_le_bytes([raw[26], raw[27]]) as u32;
        let first_cluster = (high << 16) | low;
        let size = u32::from_le_bytes([raw[28], raw[29], raw[30], raw[31]]);
        entries.push(DirectoryEntry { short_name: short_name(raw), attributes: raw[11], first_cluster, size, deleted });
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Mem(Vec<u8>);
    impl BlockDevice for Mem { fn capacity(&self)->u64{self.0.len() as u64} fn read(&self,r:ByteRange,o:&mut[u8])->RecoveryResult<usize>{r.validate_within(self.capacity())?;let n=r.length as usize;o[..n].copy_from_slice(&self.0[r.offset as usize..r.offset as usize+n]);Ok(n)} }
    fn image() -> Mem { let mut b=vec![0u8; 512*200_000]; b[11..13].copy_from_slice(&512u16.to_le_bytes()); b[13]=1;b[14..16].copy_from_slice(&32u16.to_le_bytes());b[16]=2;b[32..36].copy_from_slice(&200_000u32.to_le_bytes());b[36..40].copy_from_slice(&1000u32.to_le_bytes());b[44..48].copy_from_slice(&2u32.to_le_bytes());b[510]=0x55;b[511]=0xAA; let root=(32+2*1000)*512; b[root..root+8].copy_from_slice(b"HELLO   ");b[root+8..root+11].copy_from_slice(b"TXT");b[root+11]=0x20;b[root+26..root+28].copy_from_slice(&5u16.to_le_bytes());b[root+28..root+32].copy_from_slice(&12u32.to_le_bytes()); Mem(b) }
    #[test] fn parses_geometry() { let m=image(); let v=parse_volume(&m,ByteRange::new(0,m.capacity()).unwrap()).unwrap();assert_eq!(v.root_cluster,2);assert_eq!(v.cluster_size().unwrap(),512); }
    #[test] fn reads_root_short_entries() { let m=image();let e=read_root_entries(&m,ByteRange::new(0,m.capacity()).unwrap(),false).unwrap();assert_eq!(e.len(),1);assert_eq!(e[0].short_name,"HELLO.TXT");assert_eq!(e[0].first_cluster,5); }
}
