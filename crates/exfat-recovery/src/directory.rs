//! exFAT directory entry sets.
//!
//! A file is described by a set of 32-byte entries: a File entry, a Stream
//! Extension entry, and one or more File Name entries. A set is reconstructed
//! only when its structure and checksum agree; an invalid set is retained as
//! diagnostic evidence rather than silently treated as valid.

use std::collections::BTreeSet;

use recovery_core::{ByteRange, RecoveryError, RecoveryResult};
use storage_io::BlockDevice;

use crate::boot::{ExfatVolume, io_error};

pub const ENTRY_SIZE: usize = 32;

const TYPE_END_OF_DIRECTORY: u8 = 0x00;
const TYPE_BITMAP: u8 = 0x81;
const TYPE_UPCASE: u8 = 0x82;
const TYPE_VOLUME_LABEL: u8 = 0x83;
const TYPE_FILE: u8 = 0x85;
const TYPE_STREAM: u8 = 0xC0;
const TYPE_FILE_NAME: u8 = 0xC1;
/// Bit 7 of the entry type marks an entry as in use; deletion clears it.
const IN_USE_MASK: u8 = 0x80;

pub const ATTR_DIRECTORY: u16 = 0x0010;

/// Characters carried by one File Name entry.
const NAME_CHARS_PER_ENTRY: usize = 15;
/// exFAT caps a name at 255 characters, so at most 17 name entries.
const MAX_NAME_ENTRIES: u8 = 17;
/// A directory is never traversed for more clusters than this.
const MAX_DIRECTORY_CLUSTERS: usize = 1 << 16;

/// Why an entry set was rejected. Retained as evidence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EntrySetError {
    /// Secondary count disagrees with the entries actually present.
    SecondaryCountMismatch { declared: u8, found: u8 },
    /// The set checksum does not match its entries.
    ChecksumMismatch { declared: u16, computed: u16 },
    /// A File entry was not followed by a Stream Extension entry.
    MissingStream,
    /// The name entries did not supply the declared number of characters.
    NameLengthMismatch { declared: u8, found: usize },
    /// The name contained an unpaired surrogate or a forbidden character.
    InvalidName,
    /// Secondary count exceeds what exFAT permits.
    SecondaryCountOutOfRange { declared: u8 },
}

/// A parsed exFAT directory entry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DirectoryEntry {
    pub name: String,
    pub attributes: u16,
    pub first_cluster: u32,
    pub data_length: u64,
    /// Bytes actually allocated, which may exceed `data_length`.
    pub valid_data_length: u64,
    /// True when the stream is contiguous and the FAT need not be followed.
    pub no_fat_chain: bool,
    /// True when the entry set was marked not-in-use, i.e. deleted.
    pub deleted: bool,
}

impl DirectoryEntry {
    pub fn is_directory(&self) -> bool {
        self.attributes & ATTR_DIRECTORY != 0
    }
}

/// Reads a FAT entry, returning the next cluster or `None` at end of chain.
pub(crate) fn next_cluster<D: BlockDevice>(
    device: &D,
    volume: &ExfatVolume,
    volume_range: ByteRange,
    cluster: u32,
) -> RecoveryResult<Option<u32>> {
    let offset = volume.fat_entry_offset(volume_range.offset, cluster)?;
    let end = offset.checked_add(4).ok_or(RecoveryError::RangeOverflow)?;
    if end > volume_range.end()? {
        return Err(io_error("FAT entry outside the volume"));
    }
    let mut raw = [0u8; 4];
    let read = device.read(ByteRange::new(offset, 4)?, &mut raw)?;
    if read != 4 {
        return Err(io_error("short FAT read"));
    }
    let value = u32::from_le_bytes(raw);
    // 0xFFFFFFFF ends a chain; 0xFFFFFFF7 marks a bad cluster.
    if value == 0xFFFF_FFFF {
        return Ok(None);
    }
    if value == 0xFFFF_FFF7 {
        return Err(io_error("FAT chain enters a bad cluster"));
    }
    if !volume.is_valid_cluster(value) {
        return Err(io_error(format!(
            "FAT link {value} outside the cluster heap"
        )));
    }
    Ok(Some(value))
}

/// Resolves a cluster chain with loop detection and a traversal bound.
pub fn cluster_chain<D: BlockDevice>(
    device: &D,
    volume: &ExfatVolume,
    volume_range: ByteRange,
    start: u32,
) -> RecoveryResult<Vec<u32>> {
    if !volume.is_valid_cluster(start) {
        return Err(io_error("chain start outside the cluster heap"));
    }
    let bound = usize::try_from(volume.cluster_count).unwrap_or(usize::MAX);
    let mut chain = Vec::new();
    let mut seen = BTreeSet::new();
    let mut current = start;
    while chain.len() <= bound {
        if !seen.insert(current) {
            return Err(io_error("exFAT cluster chain loops"));
        }
        chain.push(current);
        match next_cluster(device, volume, volume_range, current)? {
            Some(next) => current = next,
            None => return Ok(chain),
        }
    }
    Err(io_error("exFAT cluster chain exceeds the traversal bound"))
}

/// The exFAT entry-set checksum, computed over every entry in the set with the
/// checksum field itself excluded.
fn entry_set_checksum(entries: &[[u8; ENTRY_SIZE]]) -> u16 {
    let mut sum: u16 = 0;
    for (entry_index, entry) in entries.iter().enumerate() {
        for (byte_index, &byte) in entry.iter().enumerate() {
            // Bytes 2 and 3 of the first entry hold the checksum itself.
            if entry_index == 0 && (byte_index == 2 || byte_index == 3) {
                continue;
            }
            sum = sum.rotate_right(1).wrapping_add(u16::from(byte));
        }
    }
    sum
}

/// Assembles the UTF-16 name carried by the File Name entries of a set.
fn assemble_name(entries: &[[u8; ENTRY_SIZE]], declared_len: u8) -> Result<String, EntrySetError> {
    let mut units: Vec<u16> = Vec::new();
    for entry in entries
        .iter()
        .filter(|e| e[0] & !IN_USE_MASK == TYPE_FILE_NAME & !IN_USE_MASK)
    {
        for pair in entry[2..32].as_chunks::<2>().0 {
            units.push(u16::from_le_bytes(*pair));
        }
    }
    let declared = usize::from(declared_len);
    // A name cannot exceed what the maximum number of name entries can carry.
    if declared > usize::from(MAX_NAME_ENTRIES) * NAME_CHARS_PER_ENTRY {
        return Err(EntrySetError::NameLengthMismatch {
            declared: declared_len,
            found: units.len(),
        });
    }
    // The name entries must actually carry the declared character count.
    if declared > units.len() {
        return Err(EntrySetError::NameLengthMismatch {
            declared: declared_len,
            found: units.len(),
        });
    }
    units.truncate(declared);
    // Reject unpaired surrogates rather than substituting replacement characters.
    let name = String::from_utf16(&units).map_err(|_| EntrySetError::InvalidName)?;
    // Path separators in a name would let a recovered file escape its directory.
    if name.is_empty() || name.contains('/') || name.contains('\\') || name.contains('\0') {
        return Err(EntrySetError::InvalidName);
    }
    Ok(name)
}

/// Parses one entry set beginning at `entries[0]`, which must be a File entry.
///
/// Returns the parsed entry and the number of 32-byte slots consumed, or the
/// reason the set was rejected alongside the slot count to skip.
fn parse_entry_set(entries: &[[u8; ENTRY_SIZE]]) -> (Result<DirectoryEntry, EntrySetError>, usize) {
    let file = &entries[0];
    let deleted = file[0] & IN_USE_MASK == 0;
    let secondary_count = file[1];
    let declared_checksum = u16::from_le_bytes([file[2], file[3]]);
    let attributes = u16::from_le_bytes([file[4], file[5]]);

    // A set is one File entry plus its secondaries: a Stream entry and at most
    // 17 name entries. Bounding this caps how far a corrupt count can skip.
    if secondary_count == 0 || secondary_count > MAX_NAME_ENTRIES + 1 {
        return (
            Err(EntrySetError::SecondaryCountOutOfRange {
                declared: secondary_count,
            }),
            1,
        );
    }
    let total = usize::from(secondary_count) + 1;
    if entries.len() < total {
        return (
            Err(EntrySetError::SecondaryCountMismatch {
                declared: secondary_count,
                found: (entries.len() - 1) as u8,
            }),
            entries.len(),
        );
    }
    let set = &entries[..total];

    // The Stream Extension entry must directly follow the File entry.
    let stream = &set[1];
    if stream[0] & !IN_USE_MASK != TYPE_STREAM & !IN_USE_MASK {
        return (Err(EntrySetError::MissingStream), total);
    }

    let computed = entry_set_checksum(set);
    if computed != declared_checksum {
        // Retained as evidence; the set is not trusted.
        return (
            Err(EntrySetError::ChecksumMismatch {
                declared: declared_checksum,
                computed,
            }),
            total,
        );
    }

    let name_length = stream[3];
    let general_flags = stream[1];
    let first_cluster = u32::from_le_bytes([stream[20], stream[21], stream[22], stream[23]]);
    let valid_data_length = u64::from_le_bytes(stream[8..16].try_into().unwrap_or([0; 8]));
    let data_length = u64::from_le_bytes(stream[24..32].try_into().unwrap_or([0; 8]));

    let name = match assemble_name(&set[2..], name_length) {
        Ok(name) => name,
        Err(error) => return (Err(error), total),
    };

    (
        Ok(DirectoryEntry {
            name,
            attributes,
            first_cluster,
            data_length,
            valid_data_length,
            // Bit 1 of the general secondary flags means "no FAT chain":
            // the stream is contiguous.
            no_fat_chain: general_flags & 0x02 != 0,
            deleted,
        }),
        total,
    )
}

/// Reads a directory's entries, following its cluster chain.
///
/// Entries whose sets fail validation are reported through `rejected` rather
/// than being silently dropped or treated as valid.
pub fn read_directory<D: BlockDevice>(
    device: &D,
    volume: &ExfatVolume,
    volume_range: ByteRange,
    start_cluster: u32,
    include_deleted: bool,
    rejected: &mut Vec<EntrySetError>,
) -> RecoveryResult<Vec<DirectoryEntry>> {
    let cluster_size = volume.cluster_size()?;
    let chain = cluster_chain(device, volume, volume_range, start_cluster)?;
    if chain.len() > MAX_DIRECTORY_CLUSTERS {
        return Err(io_error("directory exceeds the supported cluster count"));
    }

    // Read the chain as one stream so an entry set spanning a cluster boundary
    // still parses.
    let mut data = Vec::new();
    for cluster in chain {
        let offset = volume.cluster_offset(volume_range.offset, cluster)?;
        let range = ByteRange::new(offset, cluster_size)?;
        if range.end()? > volume_range.end()? {
            return Err(io_error("directory cluster outside the volume"));
        }
        let start = data.len();
        data.resize(
            start + usize::try_from(cluster_size).map_err(|_| io_error("cluster too large"))?,
            0,
        );
        let read = device.read(range, &mut data[start..])?;
        if read != data.len() - start {
            return Err(io_error("short directory read"));
        }
    }

    let entries: Vec<[u8; ENTRY_SIZE]> = data.as_chunks::<ENTRY_SIZE>().0.to_vec();
    let mut out = Vec::new();
    let mut index = 0usize;
    while index < entries.len() {
        let entry_type = entries[index][0];
        if entry_type == TYPE_END_OF_DIRECTORY {
            break;
        }
        // Bitmap, upcase table and volume label are not files.
        let base = entry_type & !IN_USE_MASK;
        if base == TYPE_BITMAP & !IN_USE_MASK
            || base == TYPE_UPCASE & !IN_USE_MASK
            || base == TYPE_VOLUME_LABEL & !IN_USE_MASK
        {
            index += 1;
            continue;
        }
        if base != TYPE_FILE & !IN_USE_MASK {
            index += 1;
            continue;
        }

        let (result, consumed) = parse_entry_set(&entries[index..]);
        match result {
            Ok(entry) => {
                if !entry.deleted || include_deleted {
                    out.push(entry);
                }
            }
            Err(error) => rejected.push(error),
        }
        // Always advance, so a malformed set cannot stall traversal.
        index += consumed.max(1);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testimage::{Mem, ROOT_CLUSTER, image};

    /// Builds a valid File/Stream/Name entry set with a correct checksum.
    fn entry_set(
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

        // Fill in the checksum now that the set is complete.
        let sum = entry_set_checksum(&set);
        set[0][2..4].copy_from_slice(&sum.to_le_bytes());
        set
    }

    fn write_set(m: &mut Mem, slot: usize, set: &[[u8; ENTRY_SIZE]]) {
        for (i, e) in set.iter().enumerate() {
            m.put_entry(ROOT_CLUSTER, slot + i, e);
        }
    }

    fn read_root(m: &Mem, include_deleted: bool) -> (Vec<DirectoryEntry>, Vec<EntrySetError>) {
        let v = crate::parse_volume(m, m.range()).unwrap();
        let mut rejected = Vec::new();
        let entries = read_directory(
            m,
            &v,
            m.range(),
            ROOT_CLUSTER,
            include_deleted,
            &mut rejected,
        )
        .unwrap();
        (entries, rejected)
    }

    #[test]
    fn parses_a_valid_entry_set() {
        let mut m = image();
        write_set(&mut m, 0, &entry_set("Report.pdf", 5, 1234, 0, false));
        let (entries, rejected) = read_root(&m, false);
        assert!(rejected.is_empty());
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "Report.pdf");
        assert_eq!(entries[0].first_cluster, 5);
        assert_eq!(entries[0].data_length, 1234);
        assert!(!entries[0].deleted);
    }

    #[test]
    fn parses_a_long_name_spanning_several_entries() {
        let mut m = image();
        // 40 characters needs three name entries.
        let name = "A rather long exFAT file name here.txt";
        write_set(&mut m, 0, &entry_set(name, 5, 10, 0, false));
        let (entries, _) = read_root(&m, false);
        assert_eq!(entries[0].name, name);
    }

    #[test]
    fn parses_non_ascii_names() {
        let mut m = image();
        write_set(&mut m, 0, &entry_set("café-漢字.txt", 5, 10, 0, false));
        let (entries, _) = read_root(&m, false);
        assert_eq!(entries[0].name, "café-漢字.txt");
    }

    #[test]
    fn identifies_directories() {
        let mut m = image();
        write_set(&mut m, 0, &entry_set("Photos", 6, 0, ATTR_DIRECTORY, false));
        let (entries, _) = read_root(&m, false);
        assert!(entries[0].is_directory());
    }

    #[test]
    fn rejects_a_checksum_mismatch() {
        let mut m = image();
        let mut set = entry_set("Report.pdf", 5, 10, 0, false);
        // Corrupt a byte the checksum covers.
        set[1][20] ^= 0xFF;
        write_set(&mut m, 0, &set);
        let (entries, rejected) = read_root(&m, false);
        // An invalid set is evidence, not a valid file.
        assert!(entries.is_empty());
        assert!(matches!(
            rejected[0],
            EntrySetError::ChecksumMismatch { .. }
        ));
    }

    #[test]
    fn rejects_a_set_without_a_stream_entry() {
        let mut m = image();
        let mut set = entry_set("Report.pdf", 5, 10, 0, false);
        set[1][0] = TYPE_FILE_NAME;
        let sum = entry_set_checksum(&set);
        set[0][2..4].copy_from_slice(&sum.to_le_bytes());
        write_set(&mut m, 0, &set);
        let (entries, rejected) = read_root(&m, false);
        assert!(entries.is_empty());
        assert_eq!(rejected[0], EntrySetError::MissingStream);
    }

    #[test]
    fn rejects_out_of_range_secondary_counts() {
        for count in [0u8, 30, 255] {
            let mut m = image();
            let mut set = entry_set("A.txt", 5, 10, 0, false);
            set[0][1] = count;
            write_set(&mut m, 0, &set);
            let (entries, rejected) = read_root(&m, false);
            assert!(entries.is_empty(), "count {count} must not parse");
            assert!(!rejected.is_empty());
        }
    }

    #[test]
    fn rejects_a_name_shorter_than_declared() {
        let mut m = image();
        let mut set = entry_set("A.txt", 5, 10, 0, false);
        // Claim 200 characters while supplying one name entry.
        set[1][3] = 200;
        let sum = entry_set_checksum(&set);
        set[0][2..4].copy_from_slice(&sum.to_le_bytes());
        write_set(&mut m, 0, &set);
        let (entries, rejected) = read_root(&m, false);
        assert!(entries.is_empty());
        assert!(matches!(
            rejected[0],
            EntrySetError::NameLengthMismatch { .. }
        ));
    }

    #[test]
    fn rejects_a_name_containing_a_path_separator() {
        let mut m = image();
        // A separator would let a recovered file escape its directory.
        write_set(&mut m, 0, &entry_set("../escape.txt", 5, 10, 0, false));
        let (entries, rejected) = read_root(&m, false);
        assert!(entries.is_empty());
        assert_eq!(rejected[0], EntrySetError::InvalidName);
    }

    #[test]
    fn deleted_sets_are_excluded_unless_requested() {
        let mut m = image();
        write_set(&mut m, 0, &entry_set("Deleted.txt", 5, 10, 0, true));
        assert!(read_root(&m, false).0.is_empty());
        let (entries, _) = read_root(&m, true);
        assert_eq!(entries.len(), 1);
        assert!(entries[0].deleted);
        assert_eq!(entries[0].name, "Deleted.txt");
    }

    #[test]
    fn a_malformed_set_does_not_stall_traversal() {
        let mut m = image();
        let mut bad = entry_set("Bad.txt", 5, 10, 0, false);
        bad[1][20] ^= 0xFF;
        write_set(&mut m, 0, &bad);
        write_set(&mut m, bad.len(), &entry_set("Good.txt", 6, 20, 0, false));
        let (entries, rejected) = read_root(&m, false);
        // The valid set after the malformed one is still found.
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "Good.txt");
        assert_eq!(rejected.len(), 1);
    }

    #[test]
    fn stops_at_end_of_directory() {
        let mut m = image();
        write_set(&mut m, 0, &entry_set("First.txt", 5, 10, 0, false));
        // Slot 4 is left zero, marking the end; anything after is not read.
        write_set(&mut m, 8, &entry_set("After.txt", 6, 10, 0, false));
        let (entries, _) = read_root(&m, false);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "First.txt");
    }

    #[test]
    fn contiguous_streams_are_flagged() {
        let mut m = image();
        let mut set = entry_set("Contig.bin", 5, 4096, 0, false);
        set[1][1] |= 0x02;
        let sum = entry_set_checksum(&set);
        set[0][2..4].copy_from_slice(&sum.to_le_bytes());
        write_set(&mut m, 0, &set);
        let (entries, _) = read_root(&m, false);
        assert!(entries[0].no_fat_chain);
    }

    #[test]
    fn resolves_a_cluster_chain() {
        let mut m = image();
        m.link(5, 6);
        m.link(6, 0xFFFF_FFFF);
        let v = crate::parse_volume(&m, m.range()).unwrap();
        assert_eq!(cluster_chain(&m, &v, m.range(), 5).unwrap(), vec![5, 6]);
    }

    #[test]
    fn detects_a_looping_chain() {
        let mut m = image();
        m.link(5, 6);
        m.link(6, 5);
        let v = crate::parse_volume(&m, m.range()).unwrap();
        assert!(cluster_chain(&m, &v, m.range(), 5).is_err());
    }

    #[test]
    fn rejects_a_chain_leaving_the_heap() {
        let mut m = image();
        m.link(5, 9999);
        let v = crate::parse_volume(&m, m.range()).unwrap();
        assert!(cluster_chain(&m, &v, m.range(), 5).is_err());
    }

    #[test]
    fn rejects_a_chain_entering_a_bad_cluster() {
        let mut m = image();
        m.link(5, 0xFFFF_FFF7);
        let v = crate::parse_volume(&m, m.range()).unwrap();
        assert!(cluster_chain(&m, &v, m.range(), 5).is_err());
    }
}
