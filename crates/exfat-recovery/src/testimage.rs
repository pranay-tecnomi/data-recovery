//! Shared in-memory exFAT fixture for tests.

use recovery_core::{ByteRange, RecoveryResult};
use storage_io::BlockDevice;

pub const SECTOR: usize = 512;
pub const SECTORS_PER_CLUSTER: usize = 1;
pub const CLUSTER: usize = SECTOR * SECTORS_PER_CLUSTER;
pub const FAT_SECTOR: usize = 8;
pub const FAT_LENGTH: usize = 8;
pub const HEAP_SECTOR: usize = 32;
pub const CLUSTER_COUNT: u32 = 64;
pub const ROOT_CLUSTER: u32 = 2;
pub const BITMAP_CLUSTER: u32 = 3;
pub const TOTAL_SECTORS: usize = HEAP_SECTOR + CLUSTER_COUNT as usize * SECTORS_PER_CLUSTER;

pub struct Mem(pub Vec<u8>);

impl BlockDevice for Mem {
    fn capacity(&self) -> u64 {
        self.0.len() as u64
    }
    fn read(&self, r: ByteRange, o: &mut [u8]) -> RecoveryResult<usize> {
        r.validate_within(self.capacity())?;
        let n = usize::try_from(r.length).unwrap();
        let start = usize::try_from(r.offset).unwrap();
        o[..n].copy_from_slice(&self.0[start..start + n]);
        Ok(n)
    }
}

impl Mem {
    pub fn range(&self) -> ByteRange {
        ByteRange::new(0, self.capacity()).unwrap()
    }

    /// Byte offset of a cluster in the heap.
    pub fn cluster_at(&self, cluster: u32) -> usize {
        HEAP_SECTOR * SECTOR + (cluster as usize - 2) * CLUSTER
    }

    /// Writes a 32-byte directory entry into `cluster` at `slot`.
    pub fn put_entry(&mut self, cluster: u32, slot: usize, entry: &[u8; 32]) {
        let off = self.cluster_at(cluster) + slot * 32;
        self.0[off..off + 32].copy_from_slice(entry);
    }

    /// Sets a FAT link.
    pub fn link(&mut self, cluster: u32, next: u32) {
        let off = FAT_SECTOR * SECTOR + cluster as usize * 4;
        self.0[off..off + 4].copy_from_slice(&next.to_le_bytes());
    }

    /// Marks a cluster allocated in the bitmap.
    pub fn allocate(&mut self, cluster: u32) {
        let index = cluster as usize - 2;
        let off = self.cluster_at(BITMAP_CLUSTER) + index / 8;
        self.0[off] |= 1 << (index % 8);
    }
}

/// A structurally valid exFAT image with a root directory and bitmap.
pub fn image() -> Mem {
    let mut b = vec![0u8; TOTAL_SECTORS * SECTOR];
    b[3..11].copy_from_slice(b"EXFAT   ");
    b[72..80].copy_from_slice(&(TOTAL_SECTORS as u64).to_le_bytes());
    b[80..84].copy_from_slice(&(FAT_SECTOR as u32).to_le_bytes());
    b[84..88].copy_from_slice(&(FAT_LENGTH as u32).to_le_bytes());
    b[88..92].copy_from_slice(&(HEAP_SECTOR as u32).to_le_bytes());
    b[92..96].copy_from_slice(&CLUSTER_COUNT.to_le_bytes());
    b[96..100].copy_from_slice(&ROOT_CLUSTER.to_le_bytes());
    b[108] = 9; // 512-byte sectors
    b[109] = 0; // one sector per cluster
    b[110] = 1; // one FAT
    b[510] = 0x55;
    b[511] = 0xAA;

    let mut m = Mem(b);
    // Root and bitmap chains terminate immediately.
    m.link(ROOT_CLUSTER, 0xFFFF_FFFF);
    m.link(BITMAP_CLUSTER, 0xFFFF_FFFF);
    m.allocate(ROOT_CLUSTER);
    m.allocate(BITMAP_CLUSTER);
    m
}
