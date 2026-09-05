use recovery_core::{RecoveryError, RecoveryResult};

use crate::ExFatDirectoryEntry;

const DELETED_FILE: u8 = 0x05;
const DELETED_STREAM: u8 = 0x40;
const DELETED_NAME: u8 = 0x41;

/// Parse inactive exFAT file entry sets left behind by deletion.
///
/// exFAT clears the in-use bit from the file, stream, and filename entry
/// types, but otherwise retains the directory metadata. This parser accepts
/// only the inactive forms and preserves the existing entry representation;
/// callers must treat the returned records as deleted candidates.
pub fn parse_deleted_directory_entry_set(entries: &[[u8; 32]]) -> RecoveryResult<ExFatDirectoryEntry> {
    let primary = entries.first().ok_or_else(|| RecoveryError::IoFailure("empty deleted exFAT entry set".into()))?;
    if primary[0] != DELETED_FILE {
        return Err(RecoveryError::IoFailure("exFAT entry set is not a deleted file entry".into()));
    }
    let secondary = usize::from(primary[1]);
    if entries.len() != secondary.checked_add(1).ok_or(RecoveryError::RangeOverflow)? {
        return Err(RecoveryError::IoFailure("deleted exFAT secondary entry count mismatch".into()));
    }
    if entries.len() < 2 || entries[1][0] != DELETED_STREAM {
        return Err(RecoveryError::IoFailure("deleted exFAT file entry set missing stream extension".into()));
    }

    let stream = &entries[1];
    let name_len = usize::from(stream[3]);
    let required_names = name_len.div_ceil(15);
    if secondary != 1 + required_names {
        return Err(RecoveryError::IoFailure("deleted exFAT filename secondary count mismatch".into()));
    }

    let mut units = Vec::with_capacity(name_len);
    for i in 0..required_names {
        let entry = &entries[2 + i];
        if entry[0] != DELETED_NAME {
            return Err(RecoveryError::IoFailure("deleted exFAT filename entry missing or out of order".into()));
        }
        for j in 0..15 {
            if units.len() == name_len { break; }
            let p = 2 + j * 2;
            units.push(u16::from_le_bytes([entry[p], entry[p + 1]]));
        }
    }

    let name = String::from_utf16(&units)
        .map_err(|_| RecoveryError::IoFailure("invalid UTF-16 deleted exFAT filename".into()))?;
    if name.is_empty() {
        return Err(RecoveryError::IoFailure("empty deleted exFAT filename".into()));
    }

    let attributes = u16::from_le_bytes([primary[4], primary[5]]);
    let first_cluster = u32::from_le_bytes(stream[20..24].try_into().expect("fixed slice"));
    let data_length = u64::from_le_bytes(stream[24..32].try_into().expect("fixed slice"));
    if data_length > 0 && first_cluster < 2 {
        return Err(RecoveryError::IoFailure("deleted non-empty exFAT file has invalid first cluster".into()));
    }

    Ok(ExFatDirectoryEntry {
        name,
        attributes,
        first_cluster,
        data_length,
        no_fat_chain: stream[1] & 0x02 != 0,
    })
}

pub fn parse_deleted_directory_entries(bytes: &[u8]) -> RecoveryResult<Vec<ExFatDirectoryEntry>> {
    if !bytes.len().is_multiple_of(32) {
        return Err(RecoveryError::IoFailure("exFAT deleted directory buffer is not entry-aligned".into()));
    }
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let mut raw = [0u8; 32];
        raw.copy_from_slice(&bytes[i..i + 32]);
        if raw[0] == 0x00 { break; }
        if raw[0] == DELETED_FILE {
            let count = usize::from(raw[1]);
            let end = i.checked_add((count + 1).checked_mul(32).ok_or(RecoveryError::RangeOverflow)?).ok_or(RecoveryError::RangeOverflow)?;
            if end > bytes.len() {
                return Err(RecoveryError::IoFailure("truncated deleted exFAT file entry set".into()));
            }
            let mut set = Vec::with_capacity(count + 1);
            for chunk in bytes[i..end].as_chunks::<32>().0 {
                let mut entry = [0u8; 32];
                entry.copy_from_slice(chunk);
                set.push(entry);
            }
            out.push(parse_deleted_directory_entry_set(&set)?);
            i = end;
        } else {
            i += 32;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deleted_set(name: &str) -> Vec<[u8; 32]> {
        let units: Vec<u16> = name.encode_utf16().collect();
        let mut primary = [0u8; 32];
        primary[0] = DELETED_FILE;
        primary[1] = 2;
        primary[4..6].copy_from_slice(&0x20u16.to_le_bytes());
        let mut stream = [0u8; 32];
        stream[0] = DELETED_STREAM;
        stream[1] = 2;
        stream[3] = units.len() as u8;
        stream[20..24].copy_from_slice(&11u32.to_le_bytes());
        stream[24..32].copy_from_slice(&4096u64.to_le_bytes());
        let mut name_entry = [0u8; 32];
        name_entry[0] = DELETED_NAME;
        for (i, unit) in units.iter().enumerate() {
            let p = 2 + i * 2;
            name_entry[p..p + 2].copy_from_slice(&unit.to_le_bytes());
        }
        vec![primary, stream, name_entry]
    }

    #[test]
    fn parses_deleted_file_metadata() {
        let entry = parse_deleted_directory_entry_set(&deleted_set("old.txt")).unwrap();
        assert_eq!(entry.name, "old.txt");
        assert_eq!(entry.first_cluster, 11);
        assert_eq!(entry.data_length, 4096);
    }

    #[test]
    fn rejects_active_entry_type() {
        let mut set = deleted_set("old.txt");
        set[0][0] = 0x85;
        assert!(parse_deleted_directory_entry_set(&set).is_err());
    }

    #[test]
    fn rejects_truncated_deleted_set() {
        let mut bytes = Vec::new();
        for entry in &deleted_set("old.txt") { bytes.extend_from_slice(entry); }
        assert!(parse_deleted_directory_entries(&bytes[..64]).is_err());
    }
}
