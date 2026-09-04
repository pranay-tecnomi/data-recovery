#![forbid(unsafe_code)]

use std::collections::BTreeSet;

use recovery_core::{ByteRange, RecoveryError, RecoveryResult};
use storage_io::BlockDevice;

const DIR_ENTRY_SIZE: usize = 32;
const END_OF_DIRECTORY: u8 = 0x00;
const DELETED: u8 = 0xE5;
const LFN_ATTR: u8 = 0x0F;
const LFN_LAST_MASK: u8 = 0x40;
const LFN_ORDER_MASK: u8 = 0x1F;
const LFN_MAX_ORDER: u8 = 20;
const LFN_CHARS_PER_ENTRY: usize = 13;
/// Offsets of the three UTF-16 name fragments inside a long-name entry.
const LFN_CHAR_RANGES: [(usize, usize); 3] = [(1, 11), (14, 26), (28, 32)];
const FAT32_EOC_MIN: u32 = 0x0FFF_FFF8;
const FAT32_BAD_CLUSTER: u32 = 0x0FFF_FFF7;
const MAX_METADATA_CLUSTER_SIZE: u64 = 16 * 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Fat32Volume {
    pub bytes_per_sector: u64,
    pub sectors_per_cluster: u64,
    pub reserved_sectors: u64,
    pub fat_count: u64,
    pub sectors_per_fat: u64,
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
    /// Assembled long file name, when a valid checksum-matched LFN set precedes
    /// the short entry. `None` means callers must fall back to `short_name`.
    pub long_name: Option<String>,
}

impl DirectoryEntry {
    /// Preferred display name: the long name when one was recovered.
    pub fn name(&self) -> &str {
        self.long_name.as_deref().unwrap_or(&self.short_name)
    }
}

fn io_error(message: &str) -> RecoveryError { RecoveryError::IoFailure(message.into()) }

fn read_exact<D: BlockDevice>(device: &D, range: ByteRange, output: &mut [u8]) -> RecoveryResult<()> {
    if u64::try_from(output.len()).map_err(|_| RecoveryError::LengthTooLarge { length: u64::MAX })? != range.length {
        return Err(io_error("read buffer length mismatch"));
    }
    let n = device.read(range, output)?;
    if n != output.len() { return Err(io_error("short FAT32 read")); }
    Ok(())
}

fn volume_end(range: ByteRange) -> RecoveryResult<u64> { range.end() }

pub fn parse_volume<D: BlockDevice>(device: &D, range: ByteRange) -> RecoveryResult<Fat32Volume> {
    range.validate_within(device.capacity())?;
    if range.length < 512 { return Err(io_error("FAT32 range smaller than boot sector")); }
    let mut boot = [0u8; 512];
    read_exact(device, ByteRange::new(range.offset, 512)?, &mut boot)?;
    if boot[510] != 0x55 || boot[511] != 0xAA { return Err(io_error("invalid FAT32 boot signature")); }

    let bps = u64::from(u16::from_le_bytes([boot[11], boot[12]]));
    let spc = u64::from(boot[13]);
    let reserved = u64::from(u16::from_le_bytes([boot[14], boot[15]]));
    let fats = u64::from(boot[16]);
    let root_entries = u16::from_le_bytes([boot[17], boot[18]]);
    let total16 = u64::from(u16::from_le_bytes([boot[19], boot[20]]));
    let fat16 = u64::from(u16::from_le_bytes([boot[22], boot[23]]));
    let total32 = u64::from(u32::from_le_bytes(boot[32..36].try_into().map_err(|_| io_error("invalid boot sector"))?));
    let fat_size = u64::from(u32::from_le_bytes(boot[36..40].try_into().map_err(|_| io_error("invalid boot sector"))?));
    let root_cluster = u32::from_le_bytes(boot[44..48].try_into().map_err(|_| io_error("invalid boot sector"))?);
    let total = if total16 != 0 { total16 } else { total32 };

    if !bps.is_power_of_two() || !(512..=4096).contains(&bps) || !spc.is_power_of_two() || spc == 0 ||
       reserved == 0 || fats == 0 || root_entries != 0 || fat16 != 0 || fat_size == 0 || total == 0 || root_cluster < 2 {
        return Err(io_error("invalid FAT32 geometry"));
    }

    let first_data_sector = reserved.checked_add(fats.checked_mul(fat_size).ok_or_else(|| io_error("FAT size overflow"))?)
        .ok_or_else(|| io_error("data offset overflow"))?;
    if first_data_sector >= total { return Err(io_error("FAT32 data region outside volume")); }
    let clusters = (total - first_data_sector) / spc;
    if clusters == 0 || u64::from(root_cluster) >= clusters.saturating_add(2) {
        return Err(io_error("invalid FAT32 cluster geometry"));
    }

    let declared_bytes = total.checked_mul(bps).ok_or(RecoveryError::RangeOverflow)?;
    if declared_bytes > range.length { return Err(io_error("FAT32 volume exceeds supplied range")); }

    Ok(Fat32Volume {
        bytes_per_sector: bps, sectors_per_cluster: spc, reserved_sectors: reserved,
        fat_count: fats, sectors_per_fat: fat_size, first_data_sector, root_cluster, cluster_count: clusters
    })
}

impl Fat32Volume {
    pub fn cluster_size(&self) -> RecoveryResult<u64> {
        self.bytes_per_sector.checked_mul(self.sectors_per_cluster).ok_or_else(|| io_error("cluster size overflow"))
    }

    pub fn cluster_offset(&self, volume_start: u64, cluster: u32) -> RecoveryResult<u64> {
        if cluster < 2 || u64::from(cluster) >= self.cluster_count.saturating_add(2) {
            return Err(io_error("cluster outside FAT32 data region"));
        }
        let sector_delta = u64::from(cluster - 2).checked_mul(self.sectors_per_cluster)
            .ok_or_else(|| io_error("cluster sector overflow"))?;
        let sector = self.first_data_sector.checked_add(sector_delta).ok_or_else(|| io_error("cluster sector overflow"))?;
        volume_start.checked_add(sector.checked_mul(self.bytes_per_sector).ok_or_else(|| io_error("cluster byte overflow"))?)
            .ok_or_else(|| io_error("cluster offset overflow"))
    }

    fn fat_offset(&self, volume_start: u64, cluster: u32) -> RecoveryResult<u64> {
        if cluster < 2 || u64::from(cluster) >= self.cluster_count.saturating_add(2) {
            return Err(io_error("cluster outside FAT32 data region"));
        }
        let fat_base = self.reserved_sectors.checked_mul(self.bytes_per_sector).ok_or(RecoveryError::RangeOverflow)?;
        let entry = u64::from(cluster).checked_mul(4).ok_or(RecoveryError::RangeOverflow)?;
        volume_start.checked_add(fat_base).and_then(|v| v.checked_add(entry)).ok_or(RecoveryError::RangeOverflow)
    }

    pub fn next_cluster<D: BlockDevice>(&self, device: &D, volume_range: ByteRange, cluster: u32) -> RecoveryResult<Option<u32>> {
        let offset = self.fat_offset(volume_range.offset, cluster)?;
        let end = offset.checked_add(4).ok_or(RecoveryError::RangeOverflow)?;
        if end > volume_end(volume_range)? { return Err(io_error("FAT entry outside volume")); }
        let mut raw = [0u8; 4];
        read_exact(device, ByteRange::new(offset, 4)?, &mut raw)?;
        let value = u32::from_le_bytes(raw) & 0x0FFF_FFFF;
        if value >= FAT32_EOC_MIN { return Ok(None); }
        if value == FAT32_BAD_CLUSTER || value < 2 || u64::from(value) >= self.cluster_count.saturating_add(2) {
            return Err(io_error("invalid FAT32 cluster link"));
        }
        Ok(Some(value))
    }

    pub fn cluster_chain<D: BlockDevice>(&self, device: &D, volume_range: ByteRange, start: u32) -> RecoveryResult<Vec<u32>> {
        if start < 2 { return Err(io_error("invalid starting cluster")); }
        let max = usize::try_from(self.cluster_count.min(1_000_000)).map_err(|_| io_error("cluster count too large"))?;
        let mut chain = Vec::new();
        let mut seen = BTreeSet::new();
        let mut current = start;
        while chain.len() <= max {
            if !seen.insert(current) { return Err(io_error("FAT32 cluster chain loop")); }
            chain.push(current);
            match self.next_cluster(device, volume_range, current)? {
                Some(next) => current = next,
                None => return Ok(chain),
            }
        }
        Err(io_error("FAT32 cluster chain exceeds traversal limit"))
    }
}

fn short_name(entry: &[u8]) -> String {
    let base = String::from_utf8_lossy(&entry[0..8]).trim_end().to_string();
    let ext = String::from_utf8_lossy(&entry[8..11]).trim_end().to_string();
    if ext.is_empty() { base } else { format!("{base}.{ext}") }
}

/// Checksum of the raw 11-byte 8.3 name, per the FAT long-name specification.
fn short_name_checksum(entry: &[u8]) -> u8 {
    let mut sum = 0u8;
    for &byte in &entry[0..11] {
        // Rotate right one bit, then add; wrapping is specified behaviour.
        sum = sum.rotate_right(1).wrapping_add(byte);
    }
    sum
}

/// Accumulates long-name fragments, which precede their 8.3 entry in reverse
/// order. Any ordering, checksum or encoding violation discards the set so a
/// corrupted run can never graft a name onto an unrelated entry.
#[derive(Default)]
struct LfnAssembler {
    /// Fragments stored as (order, utf16 units), highest order first on disk.
    parts: Vec<(u8, Vec<u16>)>,
    checksum: Option<u8>,
    expected_next: Option<u8>,
    poisoned: bool,
}

impl LfnAssembler {
    fn reset(&mut self) {
        self.parts.clear();
        self.checksum = None;
        self.expected_next = None;
        self.poisoned = false;
    }

    fn push(&mut self, raw: &[u8]) {
        let order_byte = raw[0];
        let is_last = order_byte & LFN_LAST_MASK != 0;
        let order = order_byte & LFN_ORDER_MASK;
        let checksum = raw[13];

        // Order 0 is invalid, and the cap bounds accumulation to a 255-char name.
        if order == 0 || order > LFN_MAX_ORDER {
            self.poisoned = true;
            return;
        }

        if is_last {
            // A new set starts here; drop whatever preceded it.
            self.parts.clear();
            self.poisoned = false;
            self.checksum = Some(checksum);
        } else {
            match (self.expected_next, self.checksum) {
                // Fragments must descend contiguously and share one checksum.
                (Some(expected), Some(prev)) if expected == order && prev == checksum => {}
                _ => {
                    self.poisoned = true;
                    return;
                }
            }
        }

        let mut units = Vec::with_capacity(LFN_CHARS_PER_ENTRY);
        for (start, end) in LFN_CHAR_RANGES {
            for pair in raw[start..end].as_chunks::<2>().0 {
                units.push(u16::from_le_bytes(*pair));
            }
        }
        self.parts.push((order, units));
        self.expected_next = Some(order - 1);
    }

    /// Returns the assembled name only if the set is complete, contiguous and
    /// matches the short entry's checksum.
    fn take(&mut self, short_entry: &[u8]) -> Option<String> {
        let result = self.assemble(short_entry);
        self.reset();
        result
    }

    fn assemble(&self, short_entry: &[u8]) -> Option<String> {
        if self.poisoned || self.parts.is_empty() {
            return None;
        }
        // The run must terminate at order 1, otherwise fragments are missing.
        if self.expected_next != Some(0) {
            return None;
        }
        if self.checksum? != short_name_checksum(short_entry) {
            return None;
        }

        // On-disk order is descending, so read fragments back to front.
        let mut units: Vec<u16> = Vec::new();
        for (_, part) in self.parts.iter().rev() {
            units.extend_from_slice(part);
        }
        // Names are NUL-terminated and padded with 0xFFFF.
        let end = units
            .iter()
            .position(|&u| u == 0x0000 || u == 0xFFFF)
            .unwrap_or(units.len());
        let units = &units[..end];
        if units.is_empty() {
            return None;
        }
        // Reject unpaired surrogates rather than substituting replacement chars.
        let name = String::from_utf16(units).ok()?;
        if name.contains('\u{0}') { return None; }
        Some(name)
    }
}

fn parse_directory_bytes(data: &[u8], include_deleted: bool, entries: &mut Vec<DirectoryEntry>) {
    let mut lfn = LfnAssembler::default();
    for raw in data.as_chunks::<DIR_ENTRY_SIZE>().0 {
        if raw[0] == END_OF_DIRECTORY { break; }
        let deleted = raw[0] == DELETED;
        let attr = raw[11];

        if attr == LFN_ATTR {
            // A deleted long-name fragment cannot be tied to its short entry
            // with confidence, so it never contributes an assembled name.
            if deleted { lfn.reset(); } else { lfn.push(raw); }
            continue;
        }

        // Volume labels terminate any pending set without consuming it.
        if attr & 0x08 != 0 {
            lfn.reset();
            continue;
        }

        let long_name = lfn.take(raw);
        if deleted && !include_deleted { continue; }
        let high = u32::from(u16::from_le_bytes([raw[20], raw[21]]));
        let low = u32::from(u16::from_le_bytes([raw[26], raw[27]]));
        entries.push(DirectoryEntry {
            short_name: short_name(raw), attributes: attr, first_cluster: (high << 16) | low,
            size: u32::from_le_bytes([raw[28], raw[29], raw[30], raw[31]]), deleted,
            long_name,
        });
    }
}

pub fn read_root_entries<D: BlockDevice>(device: &D, volume_range: ByteRange, include_deleted: bool) -> RecoveryResult<Vec<DirectoryEntry>> {
    let volume = parse_volume(device, volume_range)?;
    let cluster_size = volume.cluster_size()?;
    if cluster_size > MAX_METADATA_CLUSTER_SIZE { return Err(io_error("FAT32 cluster exceeds metadata read limit")); }
    let chain = volume.cluster_chain(device, volume_range, volume.root_cluster)?;
    let mut entries = Vec::new();
    for cluster in chain {
        let offset = volume.cluster_offset(volume_range.offset, cluster)?;
        let range = ByteRange::new(offset, cluster_size)?;
        if range.end()? > volume_end(volume_range)? { return Err(io_error("directory cluster outside volume")); }
        let mut data = vec![0u8; usize::try_from(cluster_size).map_err(|_| io_error("cluster too large"))?];
        read_exact(device, range, &mut data)?;
        let before = entries.len();
        parse_directory_bytes(&data, include_deleted, &mut entries);
        if data.as_chunks::<DIR_ENTRY_SIZE>().0.iter().any(|e| e[0] == END_OF_DIRECTORY) { break; }
        if entries.len() < before { return Err(io_error("directory entry accounting failure")); }
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Mem(Vec<u8>);
    impl BlockDevice for Mem {
        fn capacity(&self) -> u64 { self.0.len() as u64 }
        fn read(&self, r: ByteRange, o: &mut [u8]) -> RecoveryResult<usize> {
            r.validate_within(self.capacity())?;
            let n = usize::try_from(r.length).unwrap();
            let start = usize::try_from(r.offset).unwrap();
            o[..n].copy_from_slice(&self.0[start..start+n]);
            Ok(n)
        }
    }

    fn image() -> Mem {
        let sectors = 66_000usize;
        let mut b = vec![0u8; 512 * sectors];
        b[11..13].copy_from_slice(&512u16.to_le_bytes());
        b[13]=1; b[14..16].copy_from_slice(&32u16.to_le_bytes()); b[16]=2;
        b[32..36].copy_from_slice(&(sectors as u32).to_le_bytes());
        b[36..40].copy_from_slice(&600u32.to_le_bytes()); b[44..48].copy_from_slice(&2u32.to_le_bytes());
        b[510]=0x55; b[511]=0xAA;
        let fat = 32 * 512;
        // cluster 2 -> 3, cluster 3 -> EOC
        b[fat+8..fat+12].copy_from_slice(&3u32.to_le_bytes());
        b[fat+12..fat+16].copy_from_slice(&0x0FFF_FFFFu32.to_le_bytes());
        let root=(32+2*600)*512;
        b[root..root+8].copy_from_slice(b"HELLO   "); b[root+8..root+11].copy_from_slice(b"TXT");
        b[root+11]=0x20; b[root+26..root+28].copy_from_slice(&5u16.to_le_bytes()); b[root+28..root+32].copy_from_slice(&12u32.to_le_bytes());
        // Keep the directory logically open across the cluster boundary. A zero
        // first byte is an end-of-directory marker, so use deleted slots for
        // the unused entries in the first cluster.
        for offset in (root + DIR_ENTRY_SIZE..root + 512).step_by(DIR_ENTRY_SIZE) {
            b[offset] = DELETED;
        }
        let root2=root+512;
        b[root2..root2+8].copy_from_slice(b"WORLD   "); b[root2+8..root2+11].copy_from_slice(b"BIN");
        b[root2+11]=0x20; b[root2+26..root2+28].copy_from_slice(&6u16.to_le_bytes()); b[root2+28..root2+32].copy_from_slice(&8u32.to_le_bytes());
        Mem(b)
    }

    #[test] fn parses_geometry() {
        let m=image(); let v=parse_volume(&m,ByteRange::new(0,m.capacity()).unwrap()).unwrap();
        assert_eq!(v.root_cluster,2); assert_eq!(v.cluster_size().unwrap(),512);
    }

    #[test] fn follows_root_cluster_chain() {
        let m=image(); let e=read_root_entries(&m,ByteRange::new(0,m.capacity()).unwrap(),false).unwrap();
        assert_eq!(e.len(),2); assert_eq!(e[0].short_name,"HELLO.TXT"); assert_eq!(e[1].short_name,"WORLD.BIN");
    }

    #[test] fn detects_chain_loop() {
        let mut m=image(); let fat=32*512; m.0[fat+12..fat+16].copy_from_slice(&2u32.to_le_bytes());
        let v=parse_volume(&m,ByteRange::new(0,m.capacity()).unwrap()).unwrap();
        assert!(v.cluster_chain(&m,ByteRange::new(0,m.capacity()).unwrap(),2).is_err());
    }

    /// Builds a long-name entry fragment for `order`, marking the final
    /// on-disk fragment with the LAST bit.
    fn lfn_entry(order: u8, last: bool, checksum: u8, chars: &[u16]) -> [u8; 32] {
        let mut e = [0u8; 32];
        e[0] = if last { order | LFN_LAST_MASK } else { order };
        e[11] = LFN_ATTR;
        e[13] = checksum;
        let mut units = chars.to_vec();
        while units.len() < LFN_CHARS_PER_ENTRY { units.push(0xFFFF); }
        if chars.len() < LFN_CHARS_PER_ENTRY { units[chars.len()] = 0x0000; }
        let mut i = 0;
        for (start, end) in LFN_CHAR_RANGES {
            for slot in e[start..end].as_chunks_mut::<2>().0 {
                *slot = units[i].to_le_bytes();
                i += 1;
            }
        }
        e
    }

    fn short_entry(name: &[u8; 11]) -> [u8; 32] {
        let mut e = [0u8; 32];
        e[0..11].copy_from_slice(name);
        e[11] = 0x20;
        e
    }

    fn utf16(s: &str) -> Vec<u16> { s.encode_utf16().collect() }

    fn parse(data: &[u8]) -> Vec<DirectoryEntry> {
        let mut out = Vec::new();
        parse_directory_bytes(data, true, &mut out);
        out
    }

    #[test]
    fn assembles_single_fragment_long_name() {
        let name = b"HELLO~1 TXT";
        let sum = short_name_checksum(&short_entry(name));
        let mut d = Vec::new();
        // Exactly 13 chars: the maximum a single fragment can hold.
        let full = "Hello Wrld.tx";
        assert_eq!(utf16(full).len(), LFN_CHARS_PER_ENTRY);
        d.extend_from_slice(&lfn_entry(1, true, sum, &utf16(full)));
        d.extend_from_slice(&short_entry(name));
        let e = parse(&d);
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].long_name.as_deref(), Some(full));
    }

    #[test]
    fn assembles_multi_fragment_long_name() {
        let name = b"LONGNA~1TXT";
        let sum = short_name_checksum(&short_entry(name));
        let full = "A Very Long File Name.txt";
        let u = utf16(full);
        let mut d = Vec::new();
        // On disk the last fragment comes first, in descending order.
        d.extend_from_slice(&lfn_entry(2, true, sum, &u[13..]));
        d.extend_from_slice(&lfn_entry(1, false, sum, &u[..13]));
        d.extend_from_slice(&short_entry(name));
        let e = parse(&d);
        assert_eq!(e[0].long_name.as_deref(), Some(full));
        assert_eq!(e[0].name(), full);
    }

    #[test]
    fn rejects_checksum_mismatch() {
        let name = b"HELLO~1 TXT";
        let sum = short_name_checksum(&short_entry(name));
        let mut d = Vec::new();
        d.extend_from_slice(&lfn_entry(1, true, sum ^ 0xFF, &utf16("Hello.txt")));
        d.extend_from_slice(&short_entry(name));
        let e = parse(&d);
        assert_eq!(e[0].long_name, None);
        assert_eq!(e[0].name(), "HELLO~1.TXT");
    }

    #[test]
    fn rejects_missing_fragment() {
        let name = b"LONGNA~1TXT";
        let sum = short_name_checksum(&short_entry(name));
        let u = utf16("A Very Long File Name.txt");
        let mut d = Vec::new();
        // Order 2 present, order 1 absent: the run never reaches order 1.
        d.extend_from_slice(&lfn_entry(2, true, sum, &u[13..]));
        d.extend_from_slice(&short_entry(name));
        assert_eq!(parse(&d)[0].long_name, None);
    }

    #[test]
    fn rejects_non_contiguous_order() {
        let name = b"LONGNA~1TXT";
        let sum = short_name_checksum(&short_entry(name));
        let u = utf16("A Very Long File Name.txt");
        let mut d = Vec::new();
        d.extend_from_slice(&lfn_entry(3, true, sum, &u[13..]));
        // Jumps 3 -> 1, skipping 2.
        d.extend_from_slice(&lfn_entry(1, false, sum, &u[..13]));
        d.extend_from_slice(&short_entry(name));
        assert_eq!(parse(&d)[0].long_name, None);
    }

    #[test]
    fn rejects_zero_and_oversized_order() {
        let name = b"HELLO~1 TXT";
        let sum = short_name_checksum(&short_entry(name));
        for bad in [0u8, LFN_MAX_ORDER + 1] {
            let mut d = Vec::new();
            d.extend_from_slice(&lfn_entry(bad, true, sum, &utf16("Hello.txt")));
            d.extend_from_slice(&short_entry(name));
            assert_eq!(parse(&d)[0].long_name, None, "order {bad} must be rejected");
        }
    }

    #[test]
    fn orphan_lfn_does_not_attach_to_later_entry() {
        let first = b"AAAAAAAATXT";
        let second = b"BBBBBBBBTXT";
        let sum = short_name_checksum(&short_entry(first));
        let mut d = Vec::new();
        // Fragment set for `first`, but `first`'s short entry never appears.
        d.extend_from_slice(&lfn_entry(1, true, sum, &utf16("Orphan.txt")));
        d.extend_from_slice(&short_entry(second));
        let e = parse(&d);
        assert_eq!(e[0].long_name, None);
        assert_eq!(e[0].short_name, "BBBBBBBB.TXT");
    }

    #[test]
    fn deleted_lfn_fragments_are_discarded() {
        let name = b"HELLO~1 TXT";
        let sum = short_name_checksum(&short_entry(name));
        let mut frag = lfn_entry(1, true, sum, &utf16("Hello.txt"));
        frag[0] = DELETED;
        let mut short = short_entry(name);
        short[0] = DELETED;
        let mut d = Vec::new();
        d.extend_from_slice(&frag);
        d.extend_from_slice(&short);
        let e = parse(&d);
        assert!(e[0].deleted);
        // 0xE5 overwrites the checksum input, so no association is provable.
        assert_eq!(e[0].long_name, None);
    }

    #[test]
    fn volume_label_resets_pending_fragments() {
        let name = b"HELLO~1 TXT";
        let sum = short_name_checksum(&short_entry(name));
        let mut label = short_entry(b"VOLUMENAME ");
        label[11] = 0x08;
        let mut d = Vec::new();
        d.extend_from_slice(&lfn_entry(1, true, sum, &utf16("Hello.txt")));
        d.extend_from_slice(&label);
        d.extend_from_slice(&short_entry(name));
        let e = parse(&d);
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].long_name, None);
    }

    #[test]
    fn rejects_unpaired_surrogate() {
        let name = b"HELLO~1 TXT";
        let sum = short_name_checksum(&short_entry(name));
        let mut d = Vec::new();
        // Lone high surrogate is not valid UTF-16.
        d.extend_from_slice(&lfn_entry(1, true, sum, &[0xD800, 0x0041]));
        d.extend_from_slice(&short_entry(name));
        assert_eq!(parse(&d)[0].long_name, None);
    }

    #[test]
    fn assembles_non_ascii_name() {
        let name = b"CAFE~1  TXT";
        let sum = short_name_checksum(&short_entry(name));
        let full = "café-\u{6f22}\u{5b57}.txt";
        let mut d = Vec::new();
        d.extend_from_slice(&lfn_entry(1, true, sum, &utf16(full)));
        d.extend_from_slice(&short_entry(name));
        assert_eq!(parse(&d)[0].long_name.as_deref(), Some(full));
    }
}
