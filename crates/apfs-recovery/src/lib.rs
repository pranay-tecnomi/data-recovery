#![forbid(unsafe_code)]

use recovery_core::{ByteRange, RecoveryError, RecoveryResult};
use storage_io::BlockDevice;

mod btree;
mod catalog;
mod filesystem;
mod fsrecord;
mod omap;
mod omap_lookup;
mod volume_omap;
pub use btree::{btree_entries, btree_fixed_entries, btree_variable_entries, parse_btree_node, ApfsBtreeEntry, ApfsBtreeNode, ApfsBtreeVariableEntry, ApfsFixedBtreeEntry, ApfsVariableBtreeEntry};
pub use catalog::{read_catalog_records, ApfsCatalogRecord};
pub use filesystem::{decode_file_extent_key, index_catalog_records, ApfsDirectoryEntry, ApfsFileExtent, ApfsFilesystemIndex};
pub use fsrecord::{decode_drec_key, decode_hashed_drec_key, decode_dir_record_value, decode_file_extent_value, decode_inode_value, decode_jkey, extent_is_sparse, extent_length, ApfsDrecKey, ApfsDirRecordValue, ApfsFileExtentValue, ApfsInodeValue, ApfsJKey, APFS_TYPE_DIR_REC, APFS_TYPE_FILE_EXTENT, APFS_TYPE_INODE};
pub use omap::{parse_object_map, parse_object_map_key, parse_object_map_value, ApfsObjectMap, ApfsObjectMapKey, ApfsObjectMapValue};
pub use omap_lookup::lookup_object_map;
pub use volume_omap::{lookup_volume_object, resolve_volume_root};

const NXSB_MAGIC: u32 = 0x4253_584e;
const APSB_MAGIC: u32 = 0x4253_5041;
const MIN_BLOCK_SIZE: u32 = 512;
const MAX_BLOCK_SIZE: u32 = 65_536;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApfsContainer { pub block_size:u32,pub block_count:u64,pub features:u64,pub read_only_compatible_features:u64,pub incompatible_features:u64,pub omap_oid:u64 }
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApfsVolume { pub fs_index:u32,pub features:u64,pub read_only_compatible_features:u64,pub incompatible_features:u64,pub unmount_time:u64,pub reserve_blocks:u64,pub quota_blocks:u64,pub allocated_blocks:u64,pub fs_reserve_blocks:u64,pub omap_oid:u64,pub root_tree_oid:u64,pub extentref_tree_oid:u64,pub snap_meta_tree_oid:u64 }
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApfsObjectHeader { pub checksum:u64,pub oid:u64,pub xid:u64,pub object_type:u32,pub flags:u32 }
fn u32_at(block:&[u8],offset:usize)->u32{u32::from_le_bytes(block[offset..offset+4].try_into().expect("fixed APFS field"))}
fn u64_at(block:&[u8],offset:usize)->u64{u64::from_le_bytes(block[offset..offset+8].try_into().expect("fixed APFS field"))}
fn require_len(block:&[u8],required:usize)->RecoveryResult<()>{if block.len()<required{return Err(RecoveryError::LengthTooLarge{length:block.len() as u64});}Ok(())}
pub fn parse_object_header(block:&[u8])->RecoveryResult<ApfsObjectHeader>{require_len(block,32)?;Ok(ApfsObjectHeader{checksum:u64_at(block,0),oid:u64_at(block,8),xid:u64_at(block,16),object_type:u32_at(block,24),flags:u32_at(block,28)})}
pub fn parse_container_superblock(block:&[u8])->RecoveryResult<ApfsContainer>{require_len(block,184)?;if u32_at(block,32)!=NXSB_MAGIC{return Err(RecoveryError::IoFailure("not an APFS container superblock".into()));}let block_size=u32_at(block,36);if !(MIN_BLOCK_SIZE..=MAX_BLOCK_SIZE).contains(&block_size)||!block_size.is_power_of_two(){return Err(RecoveryError::IoFailure("invalid APFS block size".into()));}let block_count=u64_at(block,40);if block_count==0{return Err(RecoveryError::IoFailure("APFS container has zero blocks".into()));}Ok(ApfsContainer{block_size,block_count,features:u64_at(block,48),read_only_compatible_features:u64_at(block,56),incompatible_features:u64_at(block,64),omap_oid:u64_at(block,176)})}
pub fn parse_volume_superblock(block:&[u8])->RecoveryResult<ApfsVolume>{require_len(block,160)?;if u32_at(block,32)!=APSB_MAGIC{return Err(RecoveryError::IoFailure("not an APFS volume superblock".into()));}Ok(ApfsVolume{fs_index:u32_at(block,36),features:u64_at(block,40),read_only_compatible_features:u64_at(block,48),incompatible_features:u64_at(block,56),unmount_time:u64_at(block,64),reserve_blocks:u64_at(block,72),quota_blocks:u64_at(block,80),allocated_blocks:u64_at(block,88),fs_reserve_blocks:u64_at(block,96),omap_oid:u64_at(block,128),root_tree_oid:u64_at(block,136),extentref_tree_oid:u64_at(block,144),snap_meta_tree_oid:u64_at(block,152)})}
pub fn read_object<D:BlockDevice>(device:&D,range:ByteRange,container:&ApfsContainer,oid:u64)->RecoveryResult<Vec<u8>>{if oid>=container.block_count{return Err(RecoveryError::OutOfRange{offset:oid,length:1,capacity:container.block_count});}let relative=oid.checked_mul(u64::from(container.block_size)).ok_or(RecoveryError::RangeOverflow)?;let offset=range.offset.checked_add(relative).ok_or(RecoveryError::RangeOverflow)?;let length=u64::from(container.block_size);let end=offset.checked_add(length).ok_or(RecoveryError::RangeOverflow)?;let range_end=range.offset.checked_add(range.length).ok_or(RecoveryError::RangeOverflow)?;if end>range_end{return Err(RecoveryError::OutOfRange{offset,length,capacity:range_end});}let object_range=ByteRange::new(offset,length)?;object_range.validate_within(device.capacity())?;let mut block=vec![0u8;container.block_size as usize];if device.read(object_range,&mut block)?!=block.len(){return Err(RecoveryError::IoFailure("short APFS object read".into()));}Ok(block)}
pub fn open<D:BlockDevice>(device:&D,range:ByteRange)->RecoveryResult<ApfsContainer>{range.validate_within(device.capacity())?;if range.length<512{return Err(RecoveryError::LengthTooLarge{length:range.length});}let read_len=range.length.min(4096) as usize;let mut block=vec![0u8;read_len];let read_range=ByteRange::new(range.offset,read_len as u64)?;if device.read(read_range,&mut block)?!=read_len{return Err(RecoveryError::IoFailure("short APFS superblock read".into()));}let container=parse_container_superblock(&block)?;let bytes=u64::from(container.block_size).checked_mul(container.block_count).ok_or(RecoveryError::RangeOverflow)?;if bytes>range.length{return Err(RecoveryError::IoFailure("APFS container exceeds supplied range".into()));}Ok(container)}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn parses_object_header(){let mut block=vec![0u8;32];block[0..8].copy_from_slice(&1u64.to_le_bytes());block[8..16].copy_from_slice(&2u64.to_le_bytes());block[16..24].copy_from_slice(&3u64.to_le_bytes());block[24..28].copy_from_slice(&4u32.to_le_bytes());block[28..32].copy_from_slice(&5u32.to_le_bytes());assert_eq!(parse_object_header(&block).unwrap(),ApfsObjectHeader{checksum:1,oid:2,xid:3,object_type:4,flags:5});}
    #[test] fn parses_container_geometry_and_omap_oid(){let mut block=vec![0u8;512];block[32..36].copy_from_slice(&NXSB_MAGIC.to_le_bytes());block[36..40].copy_from_slice(&4096u32.to_le_bytes());block[40..48].copy_from_slice(&1024u64.to_le_bytes());block[48..56].copy_from_slice(&7u64.to_le_bytes());block[56..64].copy_from_slice(&8u64.to_le_bytes());block[64..72].copy_from_slice(&9u64.to_le_bytes());block[176..184].copy_from_slice(&99u64.to_le_bytes());let parsed=parse_container_superblock(&block).unwrap();assert_eq!(parsed.block_size,4096);assert_eq!(parsed.block_count,1024);assert_eq!(parsed.omap_oid,99);}
    #[test] fn parses_volume_superblock_navigation_roots(){let mut block=vec![0u8;512];block[32..36].copy_from_slice(&APSB_MAGIC.to_le_bytes());block[36..40].copy_from_slice(&3u32.to_le_bytes());block[40..48].copy_from_slice(&7u64.to_le_bytes());block[48..56].copy_from_slice(&8u64.to_le_bytes());block[56..64].copy_from_slice(&9u64.to_le_bytes());block[64..72].copy_from_slice(&10u64.to_le_bytes());block[72..80].copy_from_slice(&11u64.to_le_bytes());block[80..88].copy_from_slice(&12u64.to_le_bytes());block[88..96].copy_from_slice(&13u64.to_le_bytes());block[96..104].copy_from_slice(&14u64.to_le_bytes());block[128..136].copy_from_slice(&18u64.to_le_bytes());block[136..144].copy_from_slice(&15u64.to_le_bytes());block[144..152].copy_from_slice(&16u64.to_le_bytes());block[152..160].copy_from_slice(&17u64.to_le_bytes());let parsed=parse_volume_superblock(&block).unwrap();assert_eq!(parsed.fs_index,3);assert_eq!(parsed.features,7);assert_eq!(parsed.read_only_compatible_features,8);assert_eq!(parsed.incompatible_features,9);assert_eq!(parsed.unmount_time,10);assert_eq!(parsed.reserve_blocks,11);assert_eq!(parsed.quota_blocks,12);assert_eq!(parsed.allocated_blocks,13);assert_eq!(parsed.fs_reserve_blocks,14);assert_eq!(parsed.omap_oid,18);assert_eq!(parsed.root_tree_oid,15);assert_eq!(parsed.extentref_tree_oid,16);assert_eq!(parsed.snap_meta_tree_oid,17);}
    #[test] fn rejects_wrong_volume_magic(){let mut block=vec![0u8;512];block[32..36].copy_from_slice(&NXSB_MAGIC.to_le_bytes());assert!(parse_volume_superblock(&block).is_err());}
    #[test] fn rejects_truncated_volume_superblock(){assert!(parse_volume_superblock(&[0u8;159]).is_err());}
    #[test] fn rejects_invalid_block_size(){let mut block=vec![0u8;512];block[32..36].copy_from_slice(&NXSB_MAGIC.to_le_bytes());block[36..40].copy_from_slice(&3000u32.to_le_bytes());block[40..48].copy_from_slice(&1u64.to_le_bytes());assert!(parse_container_superblock(&block).is_err());}
}