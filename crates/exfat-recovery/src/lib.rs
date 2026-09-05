#![forbid(unsafe_code)]

mod active;
mod allocation;
mod deleted;
mod directory;
pub use active::active_file_extents;
pub use allocation::{read_allocation_bitmap, AllocationBitmap};
pub use deleted::{parse_deleted_directory_entries, parse_deleted_directory_entry_set};
pub use directory::{read_directory, read_root_entries, read_tree, ExFatTreeEntry};

use std::collections::BTreeSet;

use recovery_core::{ByteRange, RecoveryError, RecoveryResult};
use storage_io::BlockDevice;

const EXFAT_EOC_MIN: u32 = 0xFFFF_FFF8;
const EXFAT_BAD_CLUSTER: u32 = 0xFFFF_FFF7;
const ENTRY_FILE: u8 = 0x85;
const ENTRY_STREAM: u8 = 0xC0;
const ENTRY_NAME: u8 = 0xC1;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExFatVolume { pub partition_offset_sectors:u64, pub volume_length_sectors:u64, pub fat_offset_sectors:u32, pub fat_length_sectors:u32, pub cluster_heap_offset_sectors:u32, pub cluster_count:u32, pub root_directory_cluster:u32, pub bytes_per_sector:u64, pub bytes_per_cluster:u64 }

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExFatDirectoryEntry { pub name:String, pub attributes:u16, pub first_cluster:u32, pub data_length:u64, pub no_fat_chain:bool }

pub fn parse_directory_entry_set(entries:&[[u8;32]])->RecoveryResult<ExFatDirectoryEntry>{
    let primary=entries.first().ok_or_else(||RecoveryError::IoFailure("empty exFAT entry set".into()))?;
    if primary[0]!=ENTRY_FILE{return Err(RecoveryError::IoFailure("exFAT entry set does not start with file entry".into()));}
    let secondary=usize::from(primary[1]);
    if entries.len()!=secondary.checked_add(1).ok_or(RecoveryError::RangeOverflow)?{return Err(RecoveryError::IoFailure("exFAT secondary entry count mismatch".into()));}
    if entries.len()<2||entries[1][0]!=ENTRY_STREAM{return Err(RecoveryError::IoFailure("exFAT file entry set missing stream extension".into()));}
    let stream=&entries[1]; let name_len=usize::from(stream[3]);
    let required_names=name_len.div_ceil(15);
    if secondary!=1+required_names{return Err(RecoveryError::IoFailure("exFAT filename secondary count mismatch".into()));}
    let mut units=Vec::with_capacity(name_len);
    for i in 0..required_names { let entry=&entries[2+i]; if entry[0]!=ENTRY_NAME{return Err(RecoveryError::IoFailure("exFAT filename entry missing or out of order".into()));} for j in 0..15 { if units.len()==name_len {break;} let p=2+j*2; units.push(u16::from_le_bytes([entry[p],entry[p+1]])); } }
    let name=String::from_utf16(&units).map_err(|_|RecoveryError::IoFailure("invalid UTF-16 exFAT filename".into()))?;
    if name.is_empty(){return Err(RecoveryError::IoFailure("empty exFAT filename".into()));}
    let attributes=u16::from_le_bytes([primary[4],primary[5]]);
    let first_cluster=u32::from_le_bytes(stream[20..24].try_into().expect("fixed slice"));
    let data_length=u64::from_le_bytes(stream[24..32].try_into().expect("fixed slice"));
    if data_length>0&&first_cluster<2{return Err(RecoveryError::IoFailure("non-empty exFAT file has invalid first cluster".into()));}
    Ok(ExFatDirectoryEntry{name,attributes,first_cluster,data_length,no_fat_chain:stream[1]&0x02!=0})
}

pub fn parse_directory_entries(bytes:&[u8])->RecoveryResult<Vec<ExFatDirectoryEntry>>{
    if !bytes.len().is_multiple_of(32){return Err(RecoveryError::IoFailure("exFAT directory buffer is not entry-aligned".into()));}
    let mut out=Vec::new(); let mut i=0;
    while i<bytes.len(){let mut raw=[0u8;32];raw.copy_from_slice(&bytes[i..i+32]); if raw[0]==0x00{break;} if raw[0]==ENTRY_FILE {let count=usize::from(raw[1]); let end=i.checked_add((count+1).checked_mul(32).ok_or(RecoveryError::RangeOverflow)?).ok_or(RecoveryError::RangeOverflow)?; if end>bytes.len(){return Err(RecoveryError::IoFailure("truncated exFAT file entry set".into()));} let mut set=Vec::with_capacity(count+1);for chunk in bytes[i..end].as_chunks::<32>().0{let mut e=[0u8;32];e.copy_from_slice(chunk);set.push(e);} out.push(parse_directory_entry_set(&set)?);i=end;}else{i+=32;}}
    Ok(out)
}

impl ExFatVolume {
    pub fn cluster_range(&self, cluster:u32)->RecoveryResult<ByteRange>{
        if cluster<2 || cluster>=self.cluster_count.saturating_add(2){return Err(RecoveryError::OutOfRange{offset:u64::from(cluster),length:1,capacity:u64::from(self.cluster_count)+2});}
        let index=u64::from(cluster-2); let spc=self.bytes_per_cluster/self.bytes_per_sector;
        let sector=u64::from(self.cluster_heap_offset_sectors).checked_add(index.checked_mul(spc).ok_or(RecoveryError::RangeOverflow)?).ok_or(RecoveryError::RangeOverflow)?;
        ByteRange::new(sector.checked_mul(self.bytes_per_sector).ok_or(RecoveryError::RangeOverflow)?,self.bytes_per_cluster)
    }
    pub fn cluster_range_in(&self, volume_range:ByteRange, cluster:u32)->RecoveryResult<ByteRange>{let relative=self.cluster_range(cluster)?;let offset=volume_range.offset.checked_add(relative.offset).ok_or(RecoveryError::RangeOverflow)?;let range=ByteRange::new(offset,relative.length)?;range.validate_within(volume_range.end()?)?;Ok(range)}
    fn fat_entry_range(&self, volume_range:ByteRange, cluster:u32)->RecoveryResult<ByteRange>{if cluster<2||cluster>=self.cluster_count.saturating_add(2){return Err(RecoveryError::OutOfRange{offset:u64::from(cluster),length:1,capacity:u64::from(self.cluster_count)+2});}let fat_bytes=u64::from(self.fat_length_sectors).checked_mul(self.bytes_per_sector).ok_or(RecoveryError::RangeOverflow)?;let entry=u64::from(cluster).checked_mul(4).ok_or(RecoveryError::RangeOverflow)?;if entry.checked_add(4).ok_or(RecoveryError::RangeOverflow)?>fat_bytes{return Err(RecoveryError::IoFailure("exFAT FAT entry outside declared FAT".into()));}let relative=u64::from(self.fat_offset_sectors).checked_mul(self.bytes_per_sector).ok_or(RecoveryError::RangeOverflow)?.checked_add(entry).ok_or(RecoveryError::RangeOverflow)?;let offset=volume_range.offset.checked_add(relative).ok_or(RecoveryError::RangeOverflow)?;let range=ByteRange::new(offset,4)?;range.validate_within(volume_range.end()?)?;Ok(range)}
    pub fn next_cluster<D:BlockDevice>(&self,device:&D,volume_range:ByteRange,cluster:u32)->RecoveryResult<Option<u32>>{let range=self.fat_entry_range(volume_range,cluster)?;let mut raw=[0u8;4];if device.read(range,&mut raw)?!=4{return Err(RecoveryError::IoFailure("short exFAT FAT read".into()));}let next=u32::from_le_bytes(raw);if next>=EXFAT_EOC_MIN{return Ok(None);}if next==EXFAT_BAD_CLUSTER||next<2||next>=self.cluster_count.saturating_add(2){return Err(RecoveryError::IoFailure("invalid exFAT cluster link".into()));}Ok(Some(next))}
    pub fn cluster_chain<D:BlockDevice>(&self,device:&D,volume_range:ByteRange,start:u32)->RecoveryResult<Vec<u32>>{if start<2||start>=self.cluster_count.saturating_add(2){return Err(RecoveryError::IoFailure("invalid exFAT starting cluster".into()));}let max=usize::try_from(self.cluster_count.min(1_000_000)).map_err(|_|RecoveryError::LengthTooLarge{length:u64::from(self.cluster_count)})?;let mut seen=BTreeSet::new();let mut chain=Vec::new();let mut current=start;while chain.len()<=max{if !seen.insert(current){return Err(RecoveryError::IoFailure("exFAT cluster chain loop".into()));}chain.push(current);match self.next_cluster(device,volume_range,current)?{Some(next)=>current=next,None=>return Ok(chain)}}Err(RecoveryError::IoFailure("exFAT cluster chain exceeds traversal limit".into()))}
}
fn u32_at(b:&[u8;512],s:usize)->u32{u32::from_le_bytes(b[s..s+4].try_into().expect("fixed slice"))}
fn u64_at(b:&[u8;512],s:usize)->u64{u64::from_le_bytes(b[s..s+8].try_into().expect("fixed slice"))}
pub fn parse_boot_sector(boot:&[u8;512])->RecoveryResult<ExFatVolume>{if &boot[3..11]!=b"EXFAT   "||boot[510]!=0x55||boot[511]!=0xAA{return Err(RecoveryError::IoFailure("not an exFAT boot sector".into()));}if boot[11..64].iter().any(|&x|x!=0){return Err(RecoveryError::IoFailure("exFAT reserved boot bytes are non-zero".into()));}let ss=boot[108];let cs=boot[109];if !(9..=12).contains(&ss)||cs>25||u16::from(ss)+u16::from(cs)>30{return Err(RecoveryError::IoFailure("invalid exFAT shift geometry".into()));}let bps=1u64<<ss;let bpc=1u64<<(u16::from(ss)+u16::from(cs));let po=u64_at(boot,64);let vl=u64_at(boot,72);let fo=u32_at(boot,80);let fl=u32_at(boot,84);let cho=u32_at(boot,88);let cc=u32_at(boot,92);let root=u32_at(boot,96);if vl==0||fo==0||fl==0||cho==0||cc==0||root<2||root>=cc.saturating_add(2){return Err(RecoveryError::IoFailure("invalid exFAT geometry".into()));}let spc=1u64<<cs;let heap_end=u64::from(cho).checked_add(u64::from(cc).checked_mul(spc).ok_or(RecoveryError::RangeOverflow)?).ok_or(RecoveryError::RangeOverflow)?;if heap_end>vl||u64::from(fo)+u64::from(fl)>u64::from(cho){return Err(RecoveryError::IoFailure("inconsistent exFAT layout".into()));}Ok(ExFatVolume{partition_offset_sectors:po,volume_length_sectors:vl,fat_offset_sectors:fo,fat_length_sectors:fl,cluster_heap_offset_sectors:cho,cluster_count:cc,root_directory_cluster:root,bytes_per_sector:bps,bytes_per_cluster:bpc})}
pub fn open<D:BlockDevice>(device:&D,range:ByteRange)->RecoveryResult<ExFatVolume>{range.validate_within(device.capacity())?;if range.length<512{return Err(RecoveryError::LengthTooLarge{length:range.length});}let mut boot=[0u8;512];if device.read(ByteRange::new(range.offset,512)?,&mut boot)?!=512{return Err(RecoveryError::IoFailure("short exFAT boot-sector read".into()));}let volume=parse_boot_sector(&boot)?;let declared=volume.volume_length_sectors.checked_mul(volume.bytes_per_sector).ok_or(RecoveryError::RangeOverflow)?;if declared>range.length{return Err(RecoveryError::IoFailure("exFAT volume exceeds supplied range".into()));}Ok(volume)}
#[cfg(test)]mod tests{use super::*;fn file_set(name:&str)->Vec<[u8;32]>{let units:Vec<u16>=name.encode_utf16().collect();let mut p=[0u8;32];p[0]=ENTRY_FILE;p[1]=2;p[4..6].copy_from_slice(&0x20u16.to_le_bytes());let mut s=[0u8;32];s[0]=ENTRY_STREAM;s[1]=2;s[3]=units.len() as u8;s[20..24].copy_from_slice(&7u32.to_le_bytes());s[24..32].copy_from_slice(&123u64.to_le_bytes());let mut n=[0u8;32];n[0]=ENTRY_NAME;for (i,u) in units.iter().enumerate(){let x=2+i*2;n[x..x+2].copy_from_slice(&u.to_le_bytes());}vec![p,s,n]}#[test]fn parses_file_entry_set(){let e=parse_directory_entry_set(&file_set("hello.txt")).unwrap();assert_eq!(e.name,"hello.txt");assert_eq!(e.first_cluster,7);assert_eq!(e.data_length,123);assert!(e.no_fat_chain);}#[test]fn rejects_truncated_set(){let mut b=Vec::new();for e in &file_set("a"){b.extend_from_slice(e);}assert!(parse_directory_entries(&b[..64]).is_err());}#[test]fn rejects_bad_utf16(){let mut set=file_set("a");set[2][2]=0;set[2][3]=0xD8;assert!(parse_directory_entry_set(&set).is_err());}}
