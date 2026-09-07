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

const ENTRY_SIZE: usize = 32;
const TYPE_FILE: u8 = 0x85;
const TYPE_STREAM: u8 = 0xC0;
const TYPE_FILE_NAME: u8 = 0xC1;
const IN_USE_MASK: u8 = 0x80;
const NAME_CHARS_PER_ENTRY: usize = 15;

/// exFAT's directory entry-set checksum, over every byte except the checksum
/// field itself in the primary entry.
fn entry_set_checksum(entries: &[[u8; ENTRY_SIZE]]) -> u16 {
    let mut sum: u16 = 0;
    for (entry_index, entry) in entries.iter().enumerate() {
        for (byte_index, byte) in entry.iter().enumerate() {
            if entry_index == 0 && (byte_index == 2 || byte_index == 3) {
                continue;
            }
            sum = sum.rotate_right(1);
            sum = sum.wrapping_add(u16::from(*byte));
        }
    }
    sum
}

/// Builds a valid File/Stream/Name entry set with a correct checksum.
///
/// A deleted set is byte-identical except that the in-use bit is cleared in
/// each entry type, which is exactly what deletion does on disk.
pub fn entry_set(
    name: &str,
    cluster: u32,
    size: u64,
    attributes: u16,
    deleted: bool,
) -> Vec<[u8; ENTRY_SIZE]> {
    let units: Vec<u16> = name.encode_utf16().collect();
    let name_entries = units.len().div_ceil(NAME_CHARS_PER_ENTRY).max(1);
    let secondary = 1 + name_entries;

    let mut file = [0u8; ENTRY_SIZE];
    file[0] = if deleted {
        TYPE_FILE & !IN_USE_MASK
    } else {
        TYPE_FILE
    };
    file[1] = secondary as u8;
    file[4..6].copy_from_slice(&attributes.to_le_bytes());

    let mut stream = [0u8; ENTRY_SIZE];
    stream[0] = if deleted {
        TYPE_STREAM & !IN_USE_MASK
    } else {
        TYPE_STREAM
    };
    stream[3] = units.len() as u8;
    stream[8..16].copy_from_slice(&size.to_le_bytes());
    stream[20..24].copy_from_slice(&cluster.to_le_bytes());
    stream[24..32].copy_from_slice(&size.to_le_bytes());

    let mut set = vec![file, stream];
    for chunk in units.chunks(NAME_CHARS_PER_ENTRY) {
        let mut e = [0u8; ENTRY_SIZE];
        e[0] = if deleted {
            TYPE_FILE_NAME & !IN_USE_MASK
        } else {
            TYPE_FILE_NAME
        };
        for (i, unit) in chunk.iter().enumerate() {
            e[2 + i * 2..4 + i * 2].copy_from_slice(&unit.to_le_bytes());
        }
        set.push(e);
    }

    let sum = entry_set_checksum(&set);
    set[0][2..4].copy_from_slice(&sum.to_le_bytes());
    set
}

impl Mem {
    /// Writes an entry set into consecutive root-directory slots.
    pub fn write_set(&mut self, slot: usize, set: &[[u8; ENTRY_SIZE]]) {
        for (i, e) in set.iter().enumerate() {
            self.put_entry(ROOT_CLUSTER, slot + i, e);
        }
    }

    /// Writes file contents into a cluster chain, linking and allocating as it
    /// goes, and returns the clusters used.
    pub fn write_file(&mut self, first_cluster: u32, contents: &[u8]) -> Vec<u32> {
        let mut used = Vec::new();
        for (cluster, chunk) in (first_cluster..).zip(contents.chunks(CLUSTER)) {
            let at = self.cluster_at(cluster);
            self.0[at..at + chunk.len()].copy_from_slice(chunk);
            self.allocate(cluster);
            used.push(cluster);
        }
        for pair in used.windows(2) {
            self.link(pair[0], pair[1]);
        }
        if let Some(last) = used.last() {
            self.link(*last, 0xFFFF_FFFF);
        }
        used
    }

    /// The image's bytes, for writing it to a file.
    pub fn bytes(&self) -> &[u8] {
        &self.0
    }
}
