use recovery_core::{ByteRange, RecoveryError, RecoveryResult};
use storage_io::BlockDevice;

use crate::{
    detect_overlaps, read_exact_at, Diagnostic, DiscoveryResult, DiskGeometry, PartitionCandidate,
};

const MBR_SIZE: usize = 512;
const SIGNATURE_OFFSET: usize = 510;
const PARTITION_TABLE_OFFSET: usize = 446;
const PARTITION_ENTRY_SIZE: usize = 16;
const PARTITION_COUNT: usize = 4;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MbrEntry {
    pub index: u8,
    pub bootable: bool,
    pub type_code: u8,
    pub start_lba: u32,
    pub sector_count: u32,
}

impl MbrEntry {
    fn parse(index: u8, bytes: &[u8]) -> Self {
        Self {
            index,
            bootable: bytes[0] == 0x80,
            type_code: bytes[4],
            start_lba: u32::from_le_bytes(bytes[8..12].try_into().expect("fixed slice")),
            sector_count: u32::from_le_bytes(bytes[12..16].try_into().expect("fixed slice")),
        }
    }

    fn is_extended(&self) -> bool {
        matches!(self.type_code, 0x05 | 0x0F | 0x85)
    }
}

pub fn discover_mbr<D: BlockDevice>(
    device: &D,
    geometry: DiskGeometry,
) -> RecoveryResult<DiscoveryResult> {
    let mut sector = [0u8; MBR_SIZE];
    read_exact_at(device, 0, &mut sector)?;

    let mut result = DiscoveryResult::default();
    if sector[SIGNATURE_OFFSET..MBR_SIZE] != [0x55, 0xAA] {
        result.diagnostics.push(Diagnostic::InvalidMbrSignature);
        return Ok(result);
    }

    for i in 0..PARTITION_COUNT {
        let offset = PARTITION_TABLE_OFFSET + i * PARTITION_ENTRY_SIZE;
        let entry = MbrEntry::parse(i as u8, &sector[offset..offset + PARTITION_ENTRY_SIZE]);
        if entry.type_code == 0 || entry.sector_count == 0 {
            continue;
        }
        if entry.is_extended() {
            result
                .diagnostics
                .push(Diagnostic::UnsupportedExtendedPartition { index: entry.index });
            continue;
        }

        let start = match u64::from(entry.start_lba).checked_mul(geometry.logical_sector_size) {
            Some(v) => v,
            None => {
                result.diagnostics.push(Diagnostic::RangeOverflow { index: entry.index });
                continue;
            }
        };
        let length = match u64::from(entry.sector_count).checked_mul(geometry.logical_sector_size) {
            Some(v) => v,
            None => {
                result.diagnostics.push(Diagnostic::RangeOverflow { index: entry.index });
                continue;
            }
        };
        let range = match ByteRange::new(start, length) {
            Ok(v) => v,
            Err(RecoveryError::RangeOverflow) => {
                result.diagnostics.push(Diagnostic::RangeOverflow { index: entry.index });
                continue;
            }
            Err(e) => return Err(e),
        };
        if range.validate_within(device.capacity()).is_err() {
            result.diagnostics.push(Diagnostic::RangeOutOfBounds { index: entry.index });
            continue;
        }
        result.partitions.push(PartitionCandidate {
            index: entry.index,
            range,
            type_code: entry.type_code,
            bootable: entry.bootable,
        });
    }

    detect_overlaps(&mut result);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use recovery_core::RecoveryResult;

    struct MemoryDevice(Vec<u8>);
    impl BlockDevice for MemoryDevice {
        fn capacity(&self) -> u64 { self.0.len() as u64 }
        fn read(&self, range: ByteRange, output: &mut [u8]) -> RecoveryResult<usize> {
            range.validate_within(self.capacity())?;
            let n = usize::try_from(range.length).unwrap();
            let start = usize::try_from(range.offset).unwrap();
            output[..n].copy_from_slice(&self.0[start..start + n]);
            Ok(n)
        }
    }

    fn image_with_entry(start: u32, count: u32) -> Vec<u8> {
        let mut image = vec![0u8; 512 * 32];
        image[510] = 0x55;
        image[511] = 0xAA;
        let o = 446;
        image[o + 4] = 0x0C;
        image[o + 8..o + 12].copy_from_slice(&start.to_le_bytes());
        image[o + 12..o + 16].copy_from_slice(&count.to_le_bytes());
        image
    }

    #[test]
    fn discovers_valid_partition() {
        let d = MemoryDevice(image_with_entry(1, 4));
        let r = discover_mbr(&d, DiskGeometry::new(512).unwrap()).unwrap();
        assert_eq!(r.partitions.len(), 1);
        assert_eq!(r.partitions[0].range.offset, 512);
        assert_eq!(r.partitions[0].range.length, 2048);
    }

    #[test]
    fn reports_invalid_signature() {
        let d = MemoryDevice(vec![0u8; 512]);
        let r = discover_mbr(&d, DiskGeometry::new(512).unwrap()).unwrap();
        assert!(matches!(r.diagnostics.as_slice(), [Diagnostic::InvalidMbrSignature]));
    }

    #[test]
    fn rejects_partition_outside_source() {
        let d = MemoryDevice(image_with_entry(30, 8));
        let r = discover_mbr(&d, DiskGeometry::new(512).unwrap()).unwrap();
        assert!(r.partitions.is_empty());
        assert!(matches!(r.diagnostics[0], Diagnostic::RangeOutOfBounds { .. }));
    }
}
