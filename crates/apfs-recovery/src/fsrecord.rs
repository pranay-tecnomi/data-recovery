use recovery_core::{RecoveryError, RecoveryResult};

pub const APFS_TYPE_INODE: u8 = 3;
pub const APFS_TYPE_FILE_EXTENT: u8 = 8;
pub const APFS_TYPE_DIR_REC: u8 = 9;
pub const OBJ_ID_MASK: u64 = 0x0fff_ffff_ffff_ffff;
pub const OBJ_TYPE_SHIFT: u32 = 60;
pub const J_FILE_EXTENT_LEN_MASK: u64 = 0x00ff_ffff_ffff_ffff;
pub const J_DREC_LEN_MASK: u32 = 0x0000_03ff;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ApfsJKey {
    pub object_id: u64,
    pub record_type: u8,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApfsDrecKey {
    pub parent_id: u64,
    pub name: String,
    pub name_len_and_hash: Option<u32>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ApfsInodeValue {
    pub parent_id: u64,
    pub private_id: u64,
    pub create_time: u64,
    pub modify_time: u64,
    pub change_time: u64,
    pub access_time: u64,
    pub internal_flags: u64,
    pub nchildren_or_nlink: u32,
    pub bsd_flags: u32,
    pub owner: u32,
    pub group: u32,
    pub mode: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ApfsDirRecordValue {
    pub file_id: u64,
    pub date_added: u64,
    pub flags: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ApfsFileExtentValue {
    pub length_and_flags: u64,
    pub physical_block: u64,
    pub crypto_id: u64,
}

fn need(data: &[u8], length: usize) -> RecoveryResult<()> {
    if data.len() < length {
        return Err(RecoveryError::LengthTooLarge { length: data.len() as u64 });
    }
    Ok(())
}

fn u16_at(data: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(data[offset..offset + 2].try_into().expect("fixed APFS field"))
}

fn u32_at(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(data[offset..offset + 4].try_into().expect("fixed APFS field"))
}

fn u64_at(data: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(data[offset..offset + 8].try_into().expect("fixed APFS field"))
}

pub fn decode_jkey(data: &[u8]) -> RecoveryResult<ApfsJKey> {
    need(data, 8)?;
    let packed = u64_at(data, 0);
    let record_type = (packed >> OBJ_TYPE_SHIFT) as u8;
    Ok(ApfsJKey { object_id: packed & OBJ_ID_MASK, record_type })
}

fn decode_name(data: &[u8]) -> RecoveryResult<String> {
    let nul = data.iter().position(|&byte| byte == 0).ok_or_else(|| RecoveryError::IoFailure("APFS directory name is not NUL terminated".into()))?;
    String::from_utf8(data[..nul].to_vec()).map_err(|_| RecoveryError::IoFailure("APFS directory name is not valid UTF-8".into()))
}

/// Decode an unhashed APFS directory-record key.
pub fn decode_drec_key(data: &[u8]) -> RecoveryResult<ApfsDrecKey> {
    need(data, 10)?;
    let header = decode_jkey(data)?;
    if header.record_type != APFS_TYPE_DIR_REC {
        return Err(RecoveryError::IoFailure("APFS key is not a directory record".into()));
    }
    let name_len = usize::from(u16_at(data, 8));
    if name_len == 0 || 10usize.checked_add(name_len).ok_or(RecoveryError::RangeOverflow)? > data.len() {
        return Err(RecoveryError::OutOfRange { offset: 10, length: name_len as u64, capacity: data.len() as u64 });
    }
    Ok(ApfsDrecKey { parent_id: header.object_id, name: decode_name(&data[10..10 + name_len])?, name_len_and_hash: None })
}

/// Decode an APFS directory-record key carrying the packed name length/hash.
pub fn decode_hashed_drec_key(data: &[u8]) -> RecoveryResult<ApfsDrecKey> {
    need(data, 13)?;
    let header = decode_jkey(data)?;
    if header.record_type != APFS_TYPE_DIR_REC {
        return Err(RecoveryError::IoFailure("APFS key is not a directory record".into()));
    }
    let packed = u32_at(data, 8);
    let name_len = usize::try_from(packed & J_DREC_LEN_MASK).map_err(|_| RecoveryError::RangeOverflow)?;
    if name_len == 0 || 12usize.checked_add(name_len).ok_or(RecoveryError::RangeOverflow)? > data.len() {
        return Err(RecoveryError::OutOfRange { offset: 12, length: name_len as u64, capacity: data.len() as u64 });
    }
    Ok(ApfsDrecKey { parent_id: header.object_id, name: decode_name(&data[12..12 + name_len])?, name_len_and_hash: Some(packed) })
}

pub fn decode_inode_value(data: &[u8]) -> RecoveryResult<ApfsInodeValue> {
    need(data, 92)?;
    Ok(ApfsInodeValue {
        parent_id: u64_at(data, 0),
        private_id: u64_at(data, 8),
        create_time: u64_at(data, 16),
        modify_time: u64_at(data, 24),
        change_time: u64_at(data, 32),
        access_time: u64_at(data, 40),
        internal_flags: u64_at(data, 48),
        nchildren_or_nlink: u32_at(data, 56),
        bsd_flags: u32_at(data, 68),
        owner: u32_at(data, 72),
        group: u32_at(data, 76),
        mode: u16_at(data, 80),
    })
}

pub fn decode_dir_record_value(data: &[u8]) -> RecoveryResult<ApfsDirRecordValue> {
    need(data, 18)?;
    Ok(ApfsDirRecordValue { file_id: u64_at(data, 0), date_added: u64_at(data, 8), flags: u16_at(data, 16) })
}

pub fn decode_file_extent_value(data: &[u8]) -> RecoveryResult<ApfsFileExtentValue> {
    need(data, 24)?;
    Ok(ApfsFileExtentValue { length_and_flags: u64_at(data, 0), physical_block: u64_at(data, 8), crypto_id: u64_at(data, 16) })
}

pub fn extent_length(length_and_flags: u64) -> u64 {
    length_and_flags & J_FILE_EXTENT_LEN_MASK
}

pub fn extent_is_sparse(value: &ApfsFileExtentValue) -> bool {
    value.physical_block == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jkey(ty: u64, oid: u64) -> [u8; 8] { ((ty << OBJ_TYPE_SHIFT) | oid).to_le_bytes() }

    #[test]
    fn decodes_jkey_object_and_type() {
        let packed = (8u64 << OBJ_TYPE_SHIFT) | 0x1234;
        let key = decode_jkey(&packed.to_le_bytes()).unwrap();
        assert_eq!(key.object_id, 0x1234);
        assert_eq!(key.record_type, APFS_TYPE_FILE_EXTENT);
    }

    #[test]
    fn decodes_plain_directory_key() {
        let mut data = jkey(9, 42).to_vec();
        data.extend_from_slice(&5u16.to_le_bytes());
        data.extend_from_slice(b"test\0");
        let key = decode_drec_key(&data).unwrap();
        assert_eq!(key.parent_id, 42);
        assert_eq!(key.name, "test");
        assert_eq!(key.name_len_and_hash, None);
    }

    #[test]
    fn decodes_hashed_directory_key() {
        let packed = 5u32 | (0x12345u32 << 10);
        let mut data = jkey(9, 42).to_vec();
        data.extend_from_slice(&packed.to_le_bytes());
        data.extend_from_slice(b"test\0");
        let key = decode_hashed_drec_key(&data).unwrap();
        assert_eq!(key.parent_id, 42);
        assert_eq!(key.name, "test");
        assert_eq!(key.name_len_and_hash, Some(packed));
    }

    #[test]
    fn rejects_directory_key_with_wrong_type_or_truncated_name() {
        let mut wrong = jkey(8, 42).to_vec();
        wrong.extend_from_slice(&5u16.to_le_bytes());
        wrong.extend_from_slice(b"test\0");
        assert!(decode_drec_key(&wrong).is_err());
        let mut short = jkey(9, 42).to_vec();
        short.extend_from_slice(&5u16.to_le_bytes());
        short.extend_from_slice(b"te");
        assert!(decode_drec_key(&short).is_err());
    }

    #[test]
    fn decodes_inode_fixed_value() {
        let mut data = vec![0u8; 92];
        for (offset, value) in [(0, 1u64), (8, 2), (16, 3), (24, 4), (32, 5), (40, 6), (48, 7)] {
            data[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
        }
        data[56..60].copy_from_slice(&8u32.to_le_bytes());
        data[68..72].copy_from_slice(&9u32.to_le_bytes());
        data[72..76].copy_from_slice(&10u32.to_le_bytes());
        data[76..80].copy_from_slice(&11u32.to_le_bytes());
        data[80..82].copy_from_slice(&0o100644u16.to_le_bytes());
        let inode = decode_inode_value(&data).unwrap();
        assert_eq!(inode.parent_id, 1);
        assert_eq!(inode.private_id, 2);
        assert_eq!(inode.nchildren_or_nlink, 8);
        assert_eq!(inode.bsd_flags, 9);
        assert_eq!(inode.owner, 10);
        assert_eq!(inode.group, 11);
        assert_eq!(inode.mode, 0o100644);
    }

    #[test]
    fn decodes_dir_record_value() {
        let mut data = vec![0u8; 18];
        data[0..8].copy_from_slice(&42u64.to_le_bytes());
        data[8..16].copy_from_slice(&43u64.to_le_bytes());
        data[16..18].copy_from_slice(&7u16.to_le_bytes());
        assert_eq!(decode_dir_record_value(&data).unwrap(), ApfsDirRecordValue { file_id: 42, date_added: 43, flags: 7 });
    }

    #[test]
    fn decodes_file_extent_and_sparse_state() {
        let mut data = vec![0u8; 24];
        data[0..8].copy_from_slice(&(0x0100_0000_0000_1000u64).to_le_bytes());
        data[8..16].copy_from_slice(&0u64.to_le_bytes());
        data[16..24].copy_from_slice(&99u64.to_le_bytes());
        let extent = decode_file_extent_value(&data).unwrap();
        assert_eq!(extent_length(extent.length_and_flags), 0x1000);
        assert!(extent_is_sparse(&extent));
        assert_eq!(extent.crypto_id, 99);
    }

    #[test]
    fn rejects_truncated_values() {
        assert!(decode_inode_value(&[0u8; 91]).is_err());
        assert!(decode_dir_record_value(&[0u8; 17]).is_err());
        assert!(decode_file_extent_value(&[0u8; 23]).is_err());
    }
}
