//! Builds a byte-accurate MBR + FAT32 disk image in memory.

use recovery_core::{ByteRange, RecoveryResult};
use storage_io::BlockDevice;

pub const SECTOR: usize = 512;

/// An in-memory block device that refuses to expose any write path.
///
/// `BlockDevice` has no write method, so this type structurally cannot mutate
/// a source through the recovery API.
pub struct MemoryDevice(Vec<u8>);

impl MemoryDevice {
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    pub fn range(&self) -> ByteRange {
        ByteRange::new(0, self.capacity()).unwrap()
    }

    /// Snapshot for verifying the source was never modified.
    pub fn snapshot(&self) -> Vec<u8> {
        self.0.clone()
    }
}

impl BlockDevice for MemoryDevice {
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

/// Layout constants for the generated FAT32 volume.
pub struct Fat32Image {
    pub bytes: Vec<u8>,
    /// Byte offset of the FAT32 partition within the disk.
    pub partition_offset: u64,
    pub partition_length: u64,
    reserved_sectors: usize,
    sectors_per_fat: usize,
    sectors_per_cluster: usize,
}

impl Fat32Image {
    /// Builds a disk with one MBR-declared FAT32 partition.
    ///
    /// The volume is sized just past the 65525-cluster minimum so it is a
    /// genuine FAT32, not a FAT16 that merely claims to be one.
    pub fn build() -> Self {
        let partition_start_sector = 2048usize;
        let reserved_sectors = 32usize;
        let sectors_per_fat = 520usize;
        let sectors_per_cluster = 1usize;
        let fat_count = 2usize;
        let data_sectors = 66_000usize;
        let volume_sectors = reserved_sectors + fat_count * sectors_per_fat + data_sectors;
        let total_sectors = partition_start_sector + volume_sectors;

        let mut bytes = vec![0u8; total_sectors * SECTOR];

        // --- MBR ---
        let entry = 446usize;
        bytes[entry] = 0x00; // not bootable
        bytes[entry + 4] = 0x0C; // FAT32 LBA
        bytes[entry + 8..entry + 12]
            .copy_from_slice(&(partition_start_sector as u32).to_le_bytes());
        bytes[entry + 12..entry + 16].copy_from_slice(&(volume_sectors as u32).to_le_bytes());
        bytes[510] = 0x55;
        bytes[511] = 0xAA;

        // --- FAT32 boot sector ---
        let vbr = partition_start_sector * SECTOR;
        bytes[vbr + 11..vbr + 13].copy_from_slice(&(SECTOR as u16).to_le_bytes());
        bytes[vbr + 13] = sectors_per_cluster as u8;
        bytes[vbr + 14..vbr + 16].copy_from_slice(&(reserved_sectors as u16).to_le_bytes());
        bytes[vbr + 16] = fat_count as u8;
        bytes[vbr + 32..vbr + 36].copy_from_slice(&(volume_sectors as u32).to_le_bytes());
        bytes[vbr + 36..vbr + 40].copy_from_slice(&(sectors_per_fat as u32).to_le_bytes());
        bytes[vbr + 44..vbr + 48].copy_from_slice(&2u32.to_le_bytes()); // root cluster
        bytes[vbr + 510] = 0x55;
        bytes[vbr + 511] = 0xAA;

        let mut image = Self {
            bytes,
            partition_offset: (partition_start_sector * SECTOR) as u64,
            partition_length: (volume_sectors * SECTOR) as u64,
            reserved_sectors,
            sectors_per_fat,
            sectors_per_cluster,
        };
        // Root directory chain terminates immediately.
        image.set_fat(2, 0x0FFF_FFFF);
        image
    }

    fn volume_base(&self) -> usize {
        self.partition_offset as usize
    }

    /// Byte offset of the first FAT.
    fn fat_base(&self) -> usize {
        self.volume_base() + self.reserved_sectors * SECTOR
    }

    /// Byte offset of a cluster's data.
    pub fn cluster_offset(&self, cluster: u32) -> usize {
        let first_data_sector = self.reserved_sectors + 2 * self.sectors_per_fat;
        self.volume_base()
            + (first_data_sector + (cluster as usize - 2) * self.sectors_per_cluster) * SECTOR
    }

    /// Writes a FAT entry in both copies, so the FATs agree.
    pub fn set_fat(&mut self, cluster: u32, value: u32) {
        for copy in 0..2usize {
            let offset =
                self.fat_base() + copy * self.sectors_per_fat * SECTOR + cluster as usize * 4;
            self.bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
    }

    /// Writes a 32-byte directory entry into the root directory.
    pub fn put_root_entry(&mut self, slot: usize, entry: &[u8; 32]) {
        let offset = self.cluster_offset(2) + slot * 32;
        self.bytes[offset..offset + 32].copy_from_slice(entry);
    }

    /// Writes file content into a cluster.
    pub fn put_cluster_data(&mut self, cluster: u32, data: &[u8]) {
        let offset = self.cluster_offset(cluster);
        self.bytes[offset..offset + data.len()].copy_from_slice(data);
    }

    /// Builds an 8.3 directory entry.
    pub fn dir_entry(name: &[u8; 11], cluster: u32, size: u32, deleted: bool) -> [u8; 32] {
        let mut e = [0u8; 32];
        e[0..11].copy_from_slice(name);
        if deleted {
            e[0] = 0xE5;
        }
        e[11] = 0x20; // archive
        e[20..22].copy_from_slice(&((cluster >> 16) as u16).to_le_bytes());
        e[26..28].copy_from_slice(&(cluster as u16).to_le_bytes());
        e[28..32].copy_from_slice(&size.to_le_bytes());
        e
    }

    pub fn into_device(self) -> MemoryDevice {
        MemoryDevice::new(self.bytes)
    }
}
