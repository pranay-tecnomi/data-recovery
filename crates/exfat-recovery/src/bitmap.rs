//! exFAT allocation bitmap.
//!
//! The bitmap records which clusters are in use. For deleted-file recovery it
//! is evidence, not a guarantee: a free cluster may still have been overwritten
//! by data the filesystem has since discarded.

use recovery_core::{ByteRange, RecoveryResult};
use storage_io::BlockDevice;

use crate::boot::{ExfatVolume, FIRST_CLUSTER, io_error};

/// Refuses to buffer a bitmap larger than this, so a corrupted length cannot
/// drive an unbounded allocation.
const MAX_BITMAP_BYTES: u64 = 64 * 1024 * 1024;

/// An in-memory copy of the volume's allocation bitmap.
#[derive(Clone, Debug)]
pub struct AllocationBitmap {
    bits: Vec<u8>,
    cluster_count: u32,
}

impl AllocationBitmap {
    /// Loads the bitmap from its cluster chain.
    ///
    /// `length_bytes` comes from the bitmap's directory entry and is validated
    /// against the volume's cluster count before allocating.
    pub fn load<D: BlockDevice>(
        device: &D,
        volume: &ExfatVolume,
        volume_range: ByteRange,
        first_cluster: u32,
        length_bytes: u64,
    ) -> RecoveryResult<Self> {
        // One bit per cluster: the declared length must match the geometry.
        let required = u64::from(volume.cluster_count).div_ceil(8);
        if length_bytes < required {
            return Err(io_error(format!(
                "allocation bitmap declares {length_bytes} bytes but {required} are required"
            )));
        }
        if length_bytes > MAX_BITMAP_BYTES {
            return Err(io_error("allocation bitmap exceeds the supported size"));
        }

        let cluster_size = volume.cluster_size()?;
        let capacity = usize::try_from(length_bytes)
            .map_err(|_| io_error("allocation bitmap too large for this platform"))?;
        let mut bits = Vec::with_capacity(capacity);

        let mut cluster = first_cluster;
        let mut visited = std::collections::BTreeSet::new();
        while (bits.len() as u64) < length_bytes {
            if !volume.is_valid_cluster(cluster) {
                return Err(io_error("allocation bitmap chain leaves the cluster heap"));
            }
            // A cyclic bitmap chain must not spin forever.
            if !visited.insert(cluster) {
                return Err(io_error("allocation bitmap chain loops"));
            }
            let offset = volume.cluster_offset(volume_range.offset, cluster)?;
            let remaining = length_bytes - bits.len() as u64;
            let take = remaining.min(cluster_size);
            let range = ByteRange::new(offset, take)?;
            if range.end()? > volume_range.end()? {
                return Err(io_error("allocation bitmap extends outside the volume"));
            }
            let start = bits.len();
            bits.resize(
                start + usize::try_from(take).map_err(|_| io_error("bitmap chunk too large"))?,
                0,
            );
            let read = device.read(range, &mut bits[start..])?;
            if read != bits.len() - start {
                return Err(io_error("short allocation bitmap read"));
            }
            if (bits.len() as u64) >= length_bytes {
                break;
            }
            match crate::directory::next_cluster(device, volume, volume_range, cluster)? {
                Some(next) => cluster = next,
                None => return Err(io_error("allocation bitmap chain ended early")),
            }
        }

        Ok(Self {
            bits,
            cluster_count: volume.cluster_count,
        })
    }

    /// Whether `cluster` is marked in use.
    ///
    /// Returns `None` when the cluster lies outside the heap, so callers must
    /// distinguish "free" from "not a real cluster".
    pub fn is_allocated(&self, cluster: u32) -> Option<bool> {
        if cluster < FIRST_CLUSTER {
            return None;
        }
        let index = u64::from(cluster - FIRST_CLUSTER);
        if index >= u64::from(self.cluster_count) {
            return None;
        }
        let byte = usize::try_from(index / 8).ok()?;
        let bit = (index % 8) as u8;
        // exFAT stores cluster N in bit N%8, least significant bit first.
        Some(self.bits.get(byte)? & (1 << bit) != 0)
    }

    /// Number of clusters the bitmap reports as in use.
    pub fn allocated_count(&self) -> u64 {
        (0..self.cluster_count)
            .filter(|&c| self.is_allocated(c + FIRST_CLUSTER).unwrap_or(false))
            .count() as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testimage::{BITMAP_CLUSTER, CLUSTER_COUNT, image};

    fn load(m: &crate::testimage::Mem) -> RecoveryResult<AllocationBitmap> {
        let v = crate::parse_volume(m, m.range()).unwrap();
        let required = u64::from(CLUSTER_COUNT).div_ceil(8);
        AllocationBitmap::load(m, &v, m.range(), BITMAP_CLUSTER, required)
    }

    #[test]
    fn reports_allocated_and_free_clusters() {
        let mut m = image();
        m.allocate(7);
        let b = load(&m).unwrap();
        assert_eq!(b.is_allocated(7), Some(true));
        assert_eq!(b.is_allocated(8), Some(false));
        // Root and bitmap clusters are allocated by the fixture.
        assert_eq!(b.is_allocated(2), Some(true));
        assert_eq!(b.is_allocated(3), Some(true));
    }

    #[test]
    fn clusters_outside_the_heap_are_not_classified() {
        let b = load(&image()).unwrap();
        assert_eq!(b.is_allocated(0), None);
        assert_eq!(b.is_allocated(1), None);
        assert_eq!(b.is_allocated(CLUSTER_COUNT + 2), None);
        assert_eq!(b.is_allocated(u32::MAX), None);
    }

    #[test]
    fn counts_allocated_clusters() {
        let mut m = image();
        m.allocate(9);
        assert_eq!(load(&m).unwrap().allocated_count(), 3);
    }

    #[test]
    fn rejects_bitmap_shorter_than_the_cluster_count() {
        let m = image();
        let v = crate::parse_volume(&m, m.range()).unwrap();
        // One byte covers 8 clusters, far fewer than the volume declares.
        assert!(AllocationBitmap::load(&m, &v, m.range(), BITMAP_CLUSTER, 1).is_err());
    }

    #[test]
    fn rejects_oversized_bitmap_length() {
        let m = image();
        let v = crate::parse_volume(&m, m.range()).unwrap();
        // A corrupted length must not drive an unbounded allocation.
        assert!(AllocationBitmap::load(&m, &v, m.range(), BITMAP_CLUSTER, u64::MAX).is_err());
    }

    #[test]
    fn rejects_chain_leaving_the_heap() {
        let m = image();
        let v = crate::parse_volume(&m, m.range()).unwrap();
        assert!(AllocationBitmap::load(&m, &v, m.range(), 0, 8).is_err());
        assert!(AllocationBitmap::load(&m, &v, m.range(), CLUSTER_COUNT + 5, 8).is_err());
    }

    #[test]
    fn rejects_looping_bitmap_chain() {
        let mut m = image();
        // A bitmap spanning more than one cluster whose chain cycles.
        m.link(BITMAP_CLUSTER, 4);
        m.link(4, BITMAP_CLUSTER);
        let v = crate::parse_volume(&m, m.range()).unwrap();
        assert!(AllocationBitmap::load(&m, &v, m.range(), BITMAP_CLUSTER, 2000).is_err());
    }
}
