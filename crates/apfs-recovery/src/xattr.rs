use std::collections::BTreeMap;

use recovery_core::{RecoveryError, RecoveryResult};

use crate::{decode_jkey, ApfsCatalogRecord, APFS_TYPE_XATTR};

pub const XATTR_DATA_STREAM: u16 = 0x0001;
pub const XATTR_DATA_EMBEDDED: u16 = 0x0002;
pub const XATTR_FILE_SYSTEM_OWNED: u16 = 0x0004;
pub const XATTR_PRIVATE_DSTREAM: u16 = 0x0010;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApfsXattr {
    pub inode_id: u64,
    pub name: String,
    pub flags: u16,
    pub data: Vec<u8>,
    pub stream_id: Option<u64>,
    pub stream_size: Option<u64>,
}

fn u16_at(data: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(data[offset..offset + 2].try_into().expect("validated APFS field"))
}

fn u64_at(data: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(data[offset..offset + 8].try_into().expect("validated APFS field"))
}

pub fn decode_xattr_key(data: &[u8]) -> RecoveryResult<(u64, String)> {
    if data.len() < 10 {
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

pub fn decode_xattr_value(data: &[u8]) -> RecoveryResult<(u16, Vec<u8>, Option<u64>, Option<u64>)> {
    if data.len() < 4 {
        return Err(RecoveryError::LengthTooLarge { length: data.len() as u64 });
    }
    let flags = u16_at(data, 0);
    let data_len = usize::from(u16_at(data, 2));
    let end = 4usize.checked_add(data_len).ok_or(RecoveryError::RangeOverflow)?;
    if end > data.len() {
        return Err(RecoveryError::OutOfRange { offset: 4, length: data_len as u64, capacity: data.len() as u64 });
    }
    let has_stream = flags & XATTR_DATA_STREAM != 0;
    let has_embedded = flags & XATTR_DATA_EMBEDDED != 0;
    if has_stream == has_embedded {
        return Err(RecoveryError::IoFailure("APFS xattr must select exactly one data location".into()));
    }
    let payload = &data[4..end];
    if has_embedded {
        return Ok((flags, payload.to_vec(), None, None));
    }
    if payload.len() < 16 {
        return Err(RecoveryError::LengthTooLarge { length: payload.len() as u64 });
    }
    let stream_id = u64_at(payload, 0);
    let stream_size = u64_at(payload, 8);
    Ok((flags, Vec::new(), Some(stream_id), Some(stream_size)))
}

pub fn index_xattrs(records: &[ApfsCatalogRecord]) -> RecoveryResult<BTreeMap<u64, Vec<ApfsXattr>>> {
    let mut result: BTreeMap<u64, Vec<ApfsXattr>> = BTreeMap::new();
    for record in records {
        let key = decode_jkey(&record.key)?;
        if key.record_type != APFS_TYPE_XATTR {
            continue;
        }
        let (inode_id, name) = decode_xattr_key(&record.key)?;
        let (flags, data, stream_id, stream_size) = decode_xattr_value(&record.value)?;
        result.entry(inode_id).or_default().push(ApfsXattr {
            inode_id, name, flags, data, stream_id, stream_size,
        });
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
        key.extend_from_slice(&7u16.to_le_bytes());
        key.extend_from_slice(b"author\0");
        let value = [XATTR_DATA_EMBEDDED as u8, 0, 5, 0, b'a', b'l', b'i', b'c', b'e'];
        let record = ApfsCatalogRecord { key, value: value.to_vec() };
        let indexed = index_xattrs(&[record]).unwrap();
        assert_eq!(indexed[&42][0].name, "author");
        assert_eq!(indexed[&42][0].data, b"alice");
        assert_eq!(indexed[&42][0].flags, XATTR_DATA_EMBEDDED);
    }

    #[test]
    fn decodes_stream_xattr_metadata() {
        let mut key = jkey(4, 42).to_vec();
        key.extend_from_slice(&5u16.to_le_bytes());
        key.extend_from_slice(b"fork\0");
        let mut value = vec![0u8; 52];
        value[0..2].copy_from_slice(&XATTR_DATA_STREAM.to_le_bytes());
        value[2..4].copy_from_slice(&48u16.to_le_bytes());
        value[4..12].copy_from_slice(&77u64.to_le_bytes());
        value[12..20].copy_from_slice(&1234u64.to_le_bytes());
        let record = ApfsCatalogRecord { key, value };
        let indexed = index_xattrs(&[record]).unwrap();
        let xattr = &indexed[&42][0];
        assert_eq!(xattr.stream_id, Some(77));
        assert_eq!(xattr.stream_size, Some(1234));
        assert!(xattr.data.is_empty());
    }

    #[test]
    fn rejects_xattr_with_both_or_neither_storage_flags() {
        assert!(decode_xattr_value(&[0x03, 0, 0, 0]).is_err());
        assert!(decode_xattr_value(&[0, 0, 0, 0]).is_err());
    }

    #[test]
    fn rejects_truncated_xattr_key() {
        let mut key = jkey(4, 42).to_vec();
        key.extend_from_slice(&10u16.to_le_bytes());
        key.extend_from_slice(b"short\0");
        assert!(decode_xattr_key(&key).is_err());
    }
}
