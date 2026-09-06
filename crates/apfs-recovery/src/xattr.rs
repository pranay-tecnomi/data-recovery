use std::collections::BTreeMap;

use recovery_core::{RecoveryError, RecoveryResult};

use crate::{decode_jkey, ApfsCatalogRecord, APFS_TYPE_XATTR};

pub const XATTR_DATA_EMBEDDED: u16 = 0x0000;
pub const XATTR_DATA_STREAM: u16 = 0x0001;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApfsXattr {
    pub inode_id: u64,
    pub name: String,
    pub flags: u16,
    pub data: Vec<u8>,
}

fn u16_at(data: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(data[offset..offset + 2].try_into().expect("validated APFS field"))
}

/// Decode an APFS XATTR key: j_key + u16 name length + NUL-terminated name.
pub fn decode_xattr_key(data: &[u8]) -> RecoveryResult<(u64, String)> {
    if data.len() < 11 {
        return Err(RecoveryError::LengthTooLarge { length: data.len() as u64 });
    }
    let key = decode_jkey(data)?;
    if key.record_type != APFS_TYPE_XATTR {
        return Err(RecoveryError::IoFailure("APFS key is not an xattr".into()));
    }
    let name_len = usize::from(u16_at(data, 8));
    let end = 10usize.checked_add(name_len).ok_or(RecoveryError::RangeOverflow)?;
    if name_len == 0 || end > data.len() {
        return Err(RecoveryError::OutOfRange { offset: 10, length: name_len as u64, capacity: data.len() as u64 });
    }
    let name = &data[10..end];
    let name_end = name.iter().position(|&b| b == 0).unwrap_or(name.len());
    if name_end == 0 {
        return Err(RecoveryError::IoFailure("APFS xattr has an empty name".into()));
    }
    let name = String::from_utf8(name[..name_end].to_vec())
        .map_err(|_| RecoveryError::IoFailure("APFS xattr name is not valid UTF-8".into()))?;
    Ok((key.object_id, name))
}

/// Decode an APFS XATTR value: flags, data length, then inline data.
/// Stream-backed xattrs are deliberately rejected here because their payload
/// requires a DSTREAM/FILE_EXTENT lookup rather than treating metadata as data.
pub fn decode_xattr_value(data: &[u8]) -> RecoveryResult<(u16, Vec<u8>)> {
    if data.len() < 4 {
        return Err(RecoveryError::LengthTooLarge { length: data.len() as u64 });
    }
    let flags = u16_at(data, 0);
    let data_len = usize::from(u16_at(data, 2));
    let end = 4usize.checked_add(data_len).ok_or(RecoveryError::RangeOverflow)?;
    if end > data.len() {
        return Err(RecoveryError::OutOfRange { offset: 4, length: data_len as u64, capacity: data.len() as u64 });
    }
    if flags & XATTR_DATA_STREAM != 0 {
        return Err(RecoveryError::IoFailure("APFS stream-backed xattr requires its data stream".into()));
    }
    if flags & !XATTR_DATA_STREAM != XATTR_DATA_EMBEDDED {
        return Err(RecoveryError::IoFailure("unsupported APFS xattr flags".into()));
    }
    Ok((flags, data[4..end].to_vec()))
}

/// Index embedded APFS xattrs by inode object ID. Malformed records fail the
/// whole index instead of silently attaching bytes to the wrong inode.
pub fn index_xattrs(records: &[ApfsCatalogRecord]) -> RecoveryResult<BTreeMap<u64, Vec<ApfsXattr>>> {
    let mut result: BTreeMap<u64, Vec<ApfsXattr>> = BTreeMap::new();
    for record in records {
        let key = decode_jkey(&record.key)?;
        if key.record_type != APFS_TYPE_XATTR {
            continue;
        }
        let (inode_id, name) = decode_xattr_key(&record.key)?;
        let (flags, data) = decode_xattr_value(&record.value)?;
        result.entry(inode_id).or_default().push(ApfsXattr { inode_id, name, flags, data });
    }
    for values in result.values_mut() {
        values.sort_by(|a, b| a.name.cmp(&b.name));
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jkey(ty: u64, oid: u64) -> [u8; 8] { ((ty << 60) | oid).to_le_bytes() }

    #[test]
    fn decodes_embedded_xattr() {
        let mut key = jkey(4, 42).to_vec();
        key.extend_from_slice(&6u16.to_le_bytes());
        key.extend_from_slice(b"author\0");
        let value = [XATTR_DATA_EMBEDDED as u8, 0, 5, 0, b'a', b'l', b'i', b'c', b'e'];
        let record = ApfsCatalogRecord { key, value: value.to_vec() };
        let indexed = index_xattrs(&[record]).unwrap();
        assert_eq!(indexed[&42][0].name, "author");
        assert_eq!(indexed[&42][0].data, b"alice");
    }

    #[test]
    fn rejects_stream_xattr_for_inline_decoder() {
        let value = [XATTR_DATA_STREAM as u8, 0, 0, 0];
        assert!(decode_xattr_value(&value).is_err());
    }

    #[test]
    fn rejects_truncated_xattr_key() {
        let mut key = jkey(4, 42).to_vec();
        key.extend_from_slice(&10u16.to_le_bytes());
        key.extend_from_slice(b"short\0");
        assert!(decode_xattr_key(&key).is_err());
    }
}
