use recovery_core::{ByteRange, RecoveryError, RecoveryResult};
use storage_io::BlockDevice;

use crate::ExFatVolume;

const ENTRY_ALLOCATION_BITMAP: u8 = 0x81;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AllocationBitmap {
    bytes: Vec<u8>,
    cluster_count: u32,
}

impl AllocationBitmap {
    pub fn parse(entry: &[u8; 32], bytes: Vec<u8>, cluster_count: u32) -> RecoveryResult<Self> {
        if entry[0] != ENTRY_ALLOCATION_BITMAP {
            return Err(RecoveryError::IoFailure("not an exFAT allocation bitmap entry".into()));
        }
        if entry[1] & !0x01 != 0 {
            return Err(RecoveryError::IoFailure("invalid exFAT allocation bitmap flags".into()));
        }
        let first_cluster = u32::from_le_bytes(entry[20..24].try_into().expect("fixed slice"));
        let data_length = u64::from_le_bytes(entry[24..32].try_into().expect("fixed slice"));
        let required = u64::from(cluster_count)
            .checked_add(7)
            .ok_or(RecoveryError::RangeOverflow)? / 8;
        if data_length < required || u64::try_from(bytes.len()).map_err(|_| RecoveryError::LengthTooLarge { length: u64::MAX })? < required {
            return Err(RecoveryError::IoFailure("exFAT allocation bitmap is shorter than cluster count".into()));
        }
        if first_cluster < 2 || first_cluster >= cluster_count.saturating_add(2) {
            return Err(RecoveryError::IoFailure("exFAT allocation bitmap has invalid first cluster".into()));
        }
        Ok(Self { bytes, cluster_count })
    }

    pub fn is_allocated(&self, cluster: u32) -> RecoveryResult<bool> {
        if cluster < 2 || cluster >= self.cluster_count.saturating_add(2) {
            return Err(RecoveryError::OutOfRange {
                offset: u64::from(cluster), length: 1,
                capacity: u64::from(self.cluster_count) + 2,
            });
        }
        let index = usize::try_from(cluster - 2)
            .map_err(|_| RecoveryError::LengthTooLarge { length: u64::from(cluster) })?;
        let byte = self.bytes.get(index / 8)
            .ok_or_else(|| RecoveryError::IoFailure("allocation bitmap is truncated".into()))?;
        Ok(byte & (1 << (index % 8)) != 0)
    }
}

fn read_clusters<D: BlockDevice>(volume: &ExFatVolume, device: &D, volume_range: ByteRange, clusters: &[u32], length: u64) -> RecoveryResult<Vec<u8>> {
    let wanted = usize::try_from(length).map_err(|_| RecoveryError::LengthTooLarge { length })?;
    let mut out = Vec::with_capacity(wanted);
    for &cluster in clusters {
        if out.len() >= wanted { break; }
        let range = volume.cluster_range_in(volume_range, cluster)?;
        let mut buf = vec![0u8; usize::try_from(range.length)
            .map_err(|_| RecoveryError::LengthTooLarge { length: range.length })?];
        if device.read(range, &mut buf)? != buf.len() {
            return Err(RecoveryError::IoFailure("short exFAT cluster read".into()));
        }
        let remaining = wanted - out.len();
        out.extend_from_slice(&buf[..buf.len().min(remaining)]);
    }
    if out.len() != wanted {
        return Err(RecoveryError::IoFailure("exFAT cluster chain is shorter than declared data length".into()));
    }
    Ok(out)
}

pub fn read_allocation_bitmap<D: BlockDevice>(volume: &ExFatVolume, device: &D, volume_range: ByteRange) -> RecoveryResult<AllocationBitmap> {
    let root_chain = volume.cluster_chain(device, volume_range, volume.root_directory_cluster)?;
    let root_len = u64::try_from(root_chain.len())
        .map_err(|_| RecoveryError::LengthTooLarge { length: u64::MAX })?
        .checked_mul(volume.bytes_per_cluster).ok_or(RecoveryError::RangeOverflow)?;
    let root = read_clusters(volume, device, volume_range, &root_chain, root_len)?;
    for chunk in root.chunks_exact(32) {
        if chunk[0] == 0 { break; }
        if chunk[0] == ENTRY_ALLOCATION_BITMAP {
            let mut entry = [0u8; 32];
            entry.copy_from_slice(chunk);
            let first_cluster = u32::from_le_bytes(entry[20..24].try_into().expect("fixed slice"));
            let data_length = u64::from_le_bytes(entry[24..32].try_into().expect("fixed slice"));
            let chain = volume.cluster_chain(device, volume_range, first_cluster)?;
            let bytes = read_clusters(volume, device, volume_range, &chain, data_length)?;
            return AllocationBitmap::parse(&entry, bytes, volume.cluster_count);
        }
    }
    Err(RecoveryError::IoFailure("exFAT root directory has no allocation bitmap entry".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_allocation_bits() {
        let mut entry = [0u8; 32];
        entry[0] = ENTRY_ALLOCATION_BITMAP;
        entry[20..24].copy_from_slice(&2u32.to_le_bytes());
        entry[24..32].copy_from_slice(&1u64.to_le_bytes());
        let bitmap = AllocationBitmap::parse(&entry, vec![0b0000_0101], 3).unwrap();
        assert!(bitmap.is_allocated(2).unwrap());
        assert!(!bitmap.is_allocated(3).unwrap());
        assert!(bitmap.is_allocated(4).unwrap());
    }

    #[test]
    fn rejects_short_bitmap() {
        let mut entry = [0u8; 32];
        entry[0] = ENTRY_ALLOCATION_BITMAP;
        entry[20..24].copy_from_slice(&2u32.to_le_bytes());
        entry[24..32].copy_from_slice(&0u64.to_le_bytes());
        assert!(AllocationBitmap::parse(&entry, vec![], 1).is_err());
    }
}
