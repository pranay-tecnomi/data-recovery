#![forbid(unsafe_code)]

use std::collections::BTreeSet;

use recovery_core::{ByteRange, RecoveryError, RecoveryResult};
use storage_io::BlockDevice;

const EXFAT_EOC_MIN: u32 = 0xFFFF_FFF8;
const EXFAT_BAD_CLUSTER: u32 = 0xFFFF_FFF7;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExFatVolume { pub partition_offset_sectors:u64, pub volume_length_sectors:u64, pub fat_offset_sectors:u32, pub fat_length_sectors:u32, pub cluster_heap_offset_sectors:u32, pub cluster_count:u32, pub root_directory_cluster:u32, pub bytes_per_sector:u64, pub bytes_per_cluster:u64 }

impl ExFatVolume {
    pub fn cluster_range(&self, cluster:u32)->RecoveryResult<ByteRange>{
        if cluster<2 || cluster>=self.cluster_count.saturating_add(2){return Err(RecoveryError::OutOfRange{offset:u64::from(cluster),length:1,capacity:u64::from(self.cluster_count)+2});}
        let index=u64::from(cluster-2); let spc=self.bytes_per_cluster/self.bytes_per_sector;
        let sector=u64::from(self.cluster_heap_offset_sectors).checked_add(index.checked_mul(spc).ok_or(RecoveryError::RangeOverflow)?).ok_or(RecoveryError::RangeOverflow)?;
        ByteRange::new(sector.checked_mul(self.bytes_per_sector).ok_or(RecoveryError::RangeOverflow)?,self.bytes_per_cluster)
    }
    /// Maps a cluster relative to the supplied volume byte range. The boot
    /// sector's partition offset is metadata and is not double-applied.
    pub fn cluster_range_in(&self, volume_range:ByteRange, cluster:u32)->RecoveryResult<ByteRange>{
        let relative=self.cluster_range(cluster)?;
        let offset=volume_range.offset.checked_add(relative.offset).ok_or(RecoveryError::RangeOverflow)?;
        let range=ByteRange::new(offset,relative.length)?;
        range.validate_within(volume_range.end()?)?;
        Ok(range)
    }
    fn fat_entry_range(&self, volume_range:ByteRange, cluster:u32)->RecoveryResult<ByteRange>{
        if cluster<2 || cluster>=self.cluster_count.saturating_add(2){return Err(RecoveryError::OutOfRange{offset:u64::from(cluster),length:1,capacity:u64::from(self.cluster_count)+2});}
        let fat_bytes=u64::from(self.fat_length_sectors).checked_mul(self.bytes_per_sector).ok_or(RecoveryError::RangeOverflow)?;
        let entry=u64::from(cluster).checked_mul(4).ok_or(RecoveryError::RangeOverflow)?;
        if entry.checked_add(4).ok_or(RecoveryError::RangeOverflow)?>fat_bytes{return Err(RecoveryError::IoFailure("exFAT FAT entry outside declared FAT".into()));}
        let relative=u64::from(self.fat_offset_sectors).checked_mul(self.bytes_per_sector).ok_or(RecoveryError::RangeOverflow)?.checked_add(entry).ok_or(RecoveryError::RangeOverflow)?;
        let offset=volume_range.offset.checked_add(relative).ok_or(RecoveryError::RangeOverflow)?;
        let range=ByteRange::new(offset,4)?; range.validate_within(volume_range.end()?)?; Ok(range)
    }
    pub fn next_cluster<D:BlockDevice>(&self,device:&D,volume_range:ByteRange,cluster:u32)->RecoveryResult<Option<u32>>{
        let range=self.fat_entry_range(volume_range,cluster)?; let mut raw=[0u8;4];
        if device.read(range,&mut raw)?!=4{return Err(RecoveryError::IoFailure("short exFAT FAT read".into()));}
        let next=u32::from_le_bytes(raw);
        if next>=EXFAT_EOC_MIN{return Ok(None);}
        if next==EXFAT_BAD_CLUSTER||next<2||next>=self.cluster_count.saturating_add(2){return Err(RecoveryError::IoFailure("invalid exFAT cluster link".into()));}
        Ok(Some(next))
    }
    pub fn cluster_chain<D:BlockDevice>(&self,device:&D,volume_range:ByteRange,start:u32)->RecoveryResult<Vec<u32>>{
        if start<2||start>=self.cluster_count.saturating_add(2){return Err(RecoveryError::IoFailure("invalid exFAT starting cluster".into()));}
        let max=usize::try_from(self.cluster_count.min(1_000_000)).map_err(|_|RecoveryError::LengthTooLarge{length:u64::from(self.cluster_count)})?;
        let mut seen=BTreeSet::new(); let mut chain=Vec::new(); let mut current=start;
        while chain.len()<=max { if !seen.insert(current){return Err(RecoveryError::IoFailure("exFAT cluster chain loop".into()));} chain.push(current); match self.next_cluster(device,volume_range,current)?{Some(next)=>current=next,None=>return Ok(chain)} }
        Err(RecoveryError::IoFailure("exFAT cluster chain exceeds traversal limit".into()))
    }
}
fn u32_at(b:&[u8;512],s:usize)->u32{u32::from_le_bytes(b[s..s+4].try_into().expect("fixed slice"))}
fn u64_at(b:&[u8;512],s:usize)->u64{u64::from_le_bytes(b[s..s+8].try_into().expect("fixed slice"))}

pub fn parse_boot_sector(boot:&[u8;512])->RecoveryResult<ExFatVolume>{
 if &boot[3..11]!=b"EXFAT   "||boot[510]!=0x55||boot[511]!=0xAA{return Err(RecoveryError::IoFailure("not an exFAT boot sector".into()));}
 if boot[11..64].iter().any(|&x|x!=0){return Err(RecoveryError::IoFailure("exFAT reserved boot bytes are non-zero".into()));}
 let ss=boot[108];let cs=boot[109];if !(9..=12).contains(&ss)||cs>25||u16::from(ss)+u16::from(cs)>30{return Err(RecoveryError::IoFailure("invalid exFAT shift geometry".into()));}
 let bps=1u64<<ss;let bpc=1u64<<(u16::from(ss)+u16::from(cs));let po=u64_at(boot,64);let vl=u64_at(boot,72);let fo=u32_at(boot,80);let fl=u32_at(boot,84);let cho=u32_at(boot,88);let cc=u32_at(boot,92);let root=u32_at(boot,96);
 if vl==0||fo==0||fl==0||cho==0||cc==0||root<2||root>=cc.saturating_add(2){return Err(RecoveryError::IoFailure("invalid exFAT geometry".into()));}
 let spc=1u64<<cs;let heap_end=u64::from(cho).checked_add(u64::from(cc).checked_mul(spc).ok_or(RecoveryError::RangeOverflow)?).ok_or(RecoveryError::RangeOverflow)?;
 if heap_end>vl||u64::from(fo)+u64::from(fl)>u64::from(cho){return Err(RecoveryError::IoFailure("inconsistent exFAT layout".into()));}
 Ok(ExFatVolume{partition_offset_sectors:po,volume_length_sectors:vl,fat_offset_sectors:fo,fat_length_sectors:fl,cluster_heap_offset_sectors:cho,cluster_count:cc,root_directory_cluster:root,bytes_per_sector:bps,bytes_per_cluster:bpc})
}
pub fn open<D:BlockDevice>(device:&D,range:ByteRange)->RecoveryResult<ExFatVolume>{range.validate_within(device.capacity())?;if range.length<512{return Err(RecoveryError::LengthTooLarge{length:range.length});}let mut boot=[0u8;512];if device.read(ByteRange::new(range.offset,512)?,&mut boot)?!=512{return Err(RecoveryError::IoFailure("short exFAT boot-sector read".into()));}let volume=parse_boot_sector(&boot)?;let declared=volume.volume_length_sectors.checked_mul(volume.bytes_per_sector).ok_or(RecoveryError::RangeOverflow)?;if declared>range.length{return Err(RecoveryError::IoFailure("exFAT volume exceeds supplied range".into()));}Ok(volume)}
#[cfg(test)] mod tests{use super::*;fn boot()->[u8;512]{let mut b=[0u8;512];b[3..11].copy_from_slice(b"EXFAT   ");b[72..80].copy_from_slice(&10_000u64.to_le_bytes());b[80..84].copy_from_slice(&24u32.to_le_bytes());b[84..88].copy_from_slice(&100u32.to_le_bytes());b[88..92].copy_from_slice(&128u32.to_le_bytes());b[92..96].copy_from_slice(&1000u32.to_le_bytes());b[96..100].copy_from_slice(&2u32.to_le_bytes());b[108]=9;b[109]=3;b[510]=0x55;b[511]=0xAA;b}#[test]fn parses_and_maps_clusters(){let v=parse_boot_sector(&boot()).unwrap();assert_eq!(v.bytes_per_cluster,4096);assert_eq!(v.cluster_range(2).unwrap(),ByteRange::new(65536,4096).unwrap());assert!(v.cluster_range(1002).is_err());}#[test]fn rejects_heap_beyond_volume(){let mut b=boot();b[72..80].copy_from_slice(&129u64.to_le_bytes());assert!(parse_boot_sector(&b).is_err());}#[test]fn rejects_nonzero_reserved(){let mut b=boot();b[12]=1;assert!(parse_boot_sector(&b).is_err());}}
