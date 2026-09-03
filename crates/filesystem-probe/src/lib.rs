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
    if n != 512 { return Err(RecoveryError::IoFailure("short boot-sector read".into())); }
    Ok(boot)
}

pub fn probe<D: BlockDevice>(device: &D, range: ByteRange) -> RecoveryResult<ProbeEvidence> {
    range.validate_within(device.capacity())?;
    let boot = read_boot(device, range)?;
    if &boot[3..11] == b"EXFAT   " && boot[510] == 0x55 && boot[511] == 0xAA {
        return Ok(ProbeEvidence { kind: FilesystemKind::ExFat, confidence: 90, notes: vec!["exFAT OEM identifier", "boot signature"] });
    }
    let bytes_per_sector = u16::from_le_bytes([boot[11], boot[12]]);
    let sectors_per_cluster = boot[13];
    let fats = boot[16];
    let root_cluster = u32::from_le_bytes([boot[44], boot[45], boot[46], boot[47]]);
    let fat32_marker = &boot[82..90] == b"FAT32   ";
    if bytes_per_sector.is_power_of_two()
        && (512..=4096).contains(&bytes_per_sector)
        && sectors_per_cluster.is_power_of_two() && sectors_per_cluster > 0
        && fats > 0 && root_cluster >= 2 && fat32_marker
        && boot[510] == 0x55 && boot[511] == 0xAA {
        return Ok(ProbeEvidence { kind: FilesystemKind::Fat32, confidence: 90, notes: vec!["FAT32 type label", "plausible BPB", "boot signature"] });
    }
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
    #[test] fn detects_exfat() { let mut b=vec![0u8;512]; b[3..11].copy_from_slice(b"EXFAT   "); b[510]=0x55;b[511]=0xAA; let r=probe(&Mem(b),ByteRange::new(0,512).unwrap()).unwrap(); assert_eq!(r.kind,FilesystemKind::ExFat); }
    #[test] fn detects_fat32() { let mut b=vec![0u8;512]; b[11..13].copy_from_slice(&512u16.to_le_bytes()); b[13]=1;b[16]=2;b[44..48].copy_from_slice(&2u32.to_le_bytes()); b[82..90].copy_from_slice(b"FAT32   ");b[510]=0x55;b[511]=0xAA; let r=probe(&Mem(b),ByteRange::new(0,512).unwrap()).unwrap(); assert_eq!(r.kind,FilesystemKind::Fat32); }
}
