use recovery_core::{ByteRange, RecoveryError, RecoveryResult};
use storage_io::BlockDevice;

use crate::{ApfsCatalogRecord, ApfsContainer, ApfsVolume, read_catalog_records, read_object};

const APFS_TYPE_SNAP_METADATA: u8 = 1;
const APFS_TYPE_SNAP_NAME: u8 = 11;
const OBJ_ID_MASK: u64 = 0x0fff_ffff_ffff_ffff;
const OBJ_TYPE_SHIFT: u32 = 60;
const SNAP_METADATA_VALUE_LEN: usize = 50;
const SNAP_NAME_KEY_HEADER_LEN: usize = 10;
const MAX_SNAPSHOT_NAME_LEN: usize = 4096;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApfsSnapshot {
    pub xid: u64,
    pub extentref_tree_oid: u64,
    pub sblock_oid: u64,
    pub create_time: u64,
    pub change_time: u64,
    pub inum: u64,
    pub extentref_tree_type: u32,
    pub flags: u32,
    pub name: Option<String>,
}

fn jkey(data: &[u8]) -> RecoveryResult<(u64, u8)> {
    if data.len() < 8 {
        return Err(RecoveryError::LengthTooLarge {
            length: data.len() as u64,
        });
    }
    let packed = u64::from_le_bytes(data[..8].try_into().expect("validated APFS snapshot key"));
    Ok((packed & OBJ_ID_MASK, (packed >> OBJ_TYPE_SHIFT) as u8))
}

fn utf8_name(bytes: &[u8], field: &str) -> RecoveryResult<String> {
    let nul = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    if nul == 0 || nul > MAX_SNAPSHOT_NAME_LEN {
        return Err(RecoveryError::IoFailure(format!(
            "APFS {field} has an invalid name length"
        )));
    }
    String::from_utf8(bytes[..nul].to_vec())
        .map_err(|_| RecoveryError::IoFailure(format!("APFS {field} name is not valid UTF-8")))
}

fn u32_at(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        data[offset..offset + 4]
            .try_into()
            .expect("fixed APFS snapshot field"),
    )
}
fn u64_at(data: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(
        data[offset..offset + 8]
            .try_into()
            .expect("fixed APFS snapshot field"),
    )
}
fn u16_at(data: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(
        data[offset..offset + 2]
            .try_into()
            .expect("fixed APFS snapshot field"),
    )
}

fn decode_metadata(record: &ApfsCatalogRecord) -> RecoveryResult<ApfsSnapshot> {
    let (xid, record_type) = jkey(&record.key)?;
    if record_type != APFS_TYPE_SNAP_METADATA {
        return Err(RecoveryError::IoFailure(
            "APFS record is not snapshot metadata".into(),
        ));
    }
    if record.value.len() < SNAP_METADATA_VALUE_LEN {
        return Err(RecoveryError::LengthTooLarge {
            length: record.value.len() as u64,
        });
    }
    let name_len = usize::from(u16_at(&record.value, 48));
    if name_len == 0
        || name_len > MAX_SNAPSHOT_NAME_LEN
        || 50usize
            .checked_add(name_len)
            .ok_or(RecoveryError::RangeOverflow)?
            > record.value.len()
    {
        return Err(RecoveryError::OutOfRange {
            offset: 50,
            length: name_len as u64,
            capacity: record.value.len() as u64,
        });
    }
    let name = utf8_name(&record.value[50..50 + name_len], "snapshot metadata")?;
    Ok(ApfsSnapshot {
        xid,
        extentref_tree_oid: u64_at(&record.value, 0),
        sblock_oid: u64_at(&record.value, 8),
        create_time: u64_at(&record.value, 16),
        change_time: u64_at(&record.value, 24),
        inum: u64_at(&record.value, 32),
        extentref_tree_type: u32_at(&record.value, 40),
        flags: u32_at(&record.value, 44),
        name: Some(name),
    })
}

fn decode_name(record: &ApfsCatalogRecord) -> RecoveryResult<(String, u64)> {
    let (_, record_type) = jkey(&record.key)?;
    if record_type != APFS_TYPE_SNAP_NAME {
        return Err(RecoveryError::IoFailure(
            "APFS record is not a snapshot name".into(),
        ));
    }
    if record.key.len() < SNAP_NAME_KEY_HEADER_LEN || record.value.len() < 8 {
        return Err(RecoveryError::LengthTooLarge {
            length: record.key.len().min(record.value.len()) as u64,
        });
    }
    let name_len = usize::from(u16_at(&record.key, 8));
    if name_len == 0
        || name_len > MAX_SNAPSHOT_NAME_LEN
        || 10usize
            .checked_add(name_len)
            .ok_or(RecoveryError::RangeOverflow)?
            > record.key.len()
    {
        return Err(RecoveryError::OutOfRange {
            offset: 10,
            length: name_len as u64,
            capacity: record.key.len() as u64,
        });
    }
    Ok((
        utf8_name(&record.key[10..10 + name_len], "snapshot name")?,
        u64_at(&record.value, 0),
    ))
}

pub fn index_snapshot_records(records: &[ApfsCatalogRecord]) -> RecoveryResult<Vec<ApfsSnapshot>> {
    let mut snapshots = Vec::new();
    let mut names = std::collections::BTreeMap::<u64, String>::new();
    for record in records {
        let (_, record_type) = jkey(&record.key)?;
        match record_type {
            APFS_TYPE_SNAP_METADATA => snapshots.push(decode_metadata(record)?),
            APFS_TYPE_SNAP_NAME => {
                let (name, xid) = decode_name(record)?;
                names.insert(xid, name);
            }
            _ => {}
        }
    }
    for snapshot in &mut snapshots {
        if let Some(name) = names.get(&snapshot.xid) {
            snapshot.name = Some(name.clone());
        }
    }
    snapshots.sort_by_key(|snapshot| snapshot.xid);
    Ok(snapshots)
}

pub fn list_snapshots<D: BlockDevice>(
    device: &D,
    range: ByteRange,
    container: &ApfsContainer,
    volume: &ApfsVolume,
) -> RecoveryResult<Vec<ApfsSnapshot>> {
    if volume.snap_meta_tree_oid == 0 {
        return Ok(Vec::new());
    }
    let records = read_catalog_records(device, range, container, volume.snap_meta_tree_oid)?;
    index_snapshot_records(&records)
}

pub fn mount_snapshot<D: BlockDevice>(
    device: &D,
    range: ByteRange,
    container: &ApfsContainer,
    _live_volume: &ApfsVolume,
    snapshot: &ApfsSnapshot,
) -> RecoveryResult<ApfsVolume> {
    if snapshot.sblock_oid >= container.block_count {
        return Err(RecoveryError::OutOfRange {
            offset: snapshot.sblock_oid,
            length: 1,
            capacity: container.block_count,
        });
    }
    let block = read_object(device, range, container, snapshot.sblock_oid)?;
    // The snapshot superblock carries the object-map OID for the snapshot's
    // filesystem state. Do not replace it with the live volume's OMAP: doing so
    // silently mixes snapshot metadata with current filesystem objects.
    crate::parse_volume_superblock(&block)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn jkey(record_type: u8, object_id: u64) -> Vec<u8> {
        // The object id occupies the low 60 bits only; letting it overflow into
        // the type nibble silently retypes the record.
        ((u64::from(record_type) << OBJ_TYPE_SHIFT) | (object_id & OBJ_ID_MASK))
            .to_le_bytes()
            .to_vec()
    }

    #[test]
    fn indexes_snapshot_metadata_and_name() {
        let mut value = vec![0u8; 56];
        value[0..8].copy_from_slice(&17u64.to_le_bytes());
        value[8..16].copy_from_slice(&18u64.to_le_bytes());
        value[16..24].copy_from_slice(&19u64.to_le_bytes());
        value[24..32].copy_from_slice(&20u64.to_le_bytes());
        value[32..40].copy_from_slice(&21u64.to_le_bytes());
        value[40..44].copy_from_slice(&22u32.to_le_bytes());
        value[44..48].copy_from_slice(&23u32.to_le_bytes());
        value[48..50].copy_from_slice(&6u16.to_le_bytes());
        value[50..56].copy_from_slice(b"snap\0\0");
        let metadata = ApfsCatalogRecord {
            key: jkey(APFS_TYPE_SNAP_METADATA, 99),
            value,
        };
        let mut name_key = jkey(APFS_TYPE_SNAP_NAME, OBJ_ID_MASK);
        name_key.extend_from_slice(&6u16.to_le_bytes());
        name_key.extend_from_slice(b"named\0");
        let name = ApfsCatalogRecord {
            key: name_key,
            value: 99u64.to_le_bytes().to_vec(),
        };
        let snapshots = index_snapshot_records(&[metadata, name]).unwrap();
        assert_eq!(snapshots[0].xid, 99);
        assert_eq!(snapshots[0].sblock_oid, 18);
        assert_eq!(snapshots[0].name.as_deref(), Some("named"));
    }

    #[test]
    fn rejects_oversized_snapshot_name() {
        let mut key = jkey(APFS_TYPE_SNAP_NAME, OBJ_ID_MASK);
        key.extend_from_slice(&5000u16.to_le_bytes());
        let record = ApfsCatalogRecord {
            key,
            value: 1u64.to_le_bytes().to_vec(),
        };
        assert!(index_snapshot_records(&[record]).is_err());
    }
}
