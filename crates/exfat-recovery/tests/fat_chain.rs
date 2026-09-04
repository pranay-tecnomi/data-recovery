use exfat_recovery::ExFatVolume;
use recovery_core::{ByteRange, RecoveryResult};
use storage_io::BlockDevice;

struct MemoryDevice(Vec<u8>);

impl BlockDevice for MemoryDevice {
    fn capacity(&self) -> u64 { self.0.len() as u64 }

    fn read(&self, range: ByteRange, output: &mut [u8]) -> RecoveryResult<usize> {
        let start = usize::try_from(range.offset).unwrap();
        let end = start.checked_add(output.len()).unwrap();
        if end > self.0.len() { return Ok(0); }
        output.copy_from_slice(&self.0[start..end]);
        Ok(output.len())
    }
}

fn volume() -> ExFatVolume {
    ExFatVolume {
        partition_offset_sectors: 0,
        volume_length_sectors: 64,
        fat_offset_sectors: 1,
        fat_length_sectors: 1,
        cluster_heap_offset_sectors: 8,
        cluster_count: 16,
        root_directory_cluster: 2,
        bytes_per_sector: 512,
        bytes_per_cluster: 512,
    }
}

fn device_with_fat(entries: &[(u32, u32)]) -> MemoryDevice {
    let mut bytes = vec![0u8; 64 * 512];
    for &(cluster, next) in entries {
        let offset = 512 + usize::try_from(cluster).unwrap() * 4;
        bytes[offset..offset + 4].copy_from_slice(&next.to_le_bytes());
    }
    MemoryDevice(bytes)
}

fn range() -> ByteRange { ByteRange::new(0, 64 * 512).unwrap() }

#[test]
fn traverses_normal_chain_to_eoc() {
    let device = device_with_fat(&[(2, 3), (3, 4), (4, 0xFFFF_FFFF)]);
    assert_eq!(volume().cluster_chain(&device, range(), 2).unwrap(), vec![2, 3, 4]);
}

#[test]
fn rejects_looping_chain() {
    let device = device_with_fat(&[(2, 3), (3, 2)]);
    assert!(volume().cluster_chain(&device, range(), 2).is_err());
}

#[test]
fn rejects_bad_cluster_marker() {
    let device = device_with_fat(&[(2, 0xFFFF_FFF7)]);
    assert!(volume().cluster_chain(&device, range(), 2).is_err());
}

#[test]
fn rejects_out_of_range_link() {
    let device = device_with_fat(&[(2, 99)]);
    assert!(volume().cluster_chain(&device, range(), 2).is_err());
}
