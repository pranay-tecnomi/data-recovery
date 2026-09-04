use recovery_core::{RecoveryError, RecoveryResult};

const ENTRY_ALLOCATION_BITMAP: u8 = 0x81;

/// Location and length of an exFAT allocation bitmap.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AllocationBitmap {
    pub first_cluster: u32,
    pub data_length: u64,
}

/// Finds and validates the active allocation bitmap entry in a directory
/// buffer. exFAT stores allocation metadata as system directory entries.
pub fn parse_allocation_bitmap_entry(bytes: &[u8]) -> RecoveryResult<AllocationBitmap> {
    if bytes.len() % 32 != 0 {
        return Err(RecoveryError::IoFailure(
            "exFAT directory buffer is not entry-aligned".into(),
        ));
    }

    let mut found = None;
    for entry in bytes.chunks_exact(32) {
        if entry[0] == 0x00 {
            break;
        }
        if entry[0] != ENTRY_ALLOCATION_BITMAP {
            continue;
        }
        // Bit 0 selects the active bitmap when multiple copies exist.
        if entry[1] & 0x01 != 0 {
            continue;
        }
        let first_cluster = u32::from_le_bytes(entry[20..24].try_into().expect("fixed slice"));
        let data_length = u64::from_le_bytes(entry[24..32].try_into().expect("fixed slice"));
        if first_cluster < 2 || data_length == 0 {
            return Err(RecoveryError::IoFailure(
                "invalid exFAT allocation bitmap geometry".into(),
            ));
        }
        let bitmap = AllocationBitmap { first_cluster, data_length };
        if found.replace(bitmap).is_some() {
            return Err(RecoveryError::IoFailure(
                "multiple active exFAT allocation bitmaps".into(),
            ));
        }
    }

    found.ok_or_else(|| RecoveryError::IoFailure("exFAT allocation bitmap not found".into()))
}

/// Returns whether a cluster is allocated according to an already-loaded
/// allocation bitmap. The bitmap is indexed from cluster 2.
pub fn is_cluster_allocated(
    bitmap: &[u8],
    cluster_count: u32,
    cluster: u32,
) -> RecoveryResult<bool> {
    if cluster < 2 || cluster >= cluster_count.saturating_add(2) {
        return Err(RecoveryError::IoFailure("invalid exFAT bitmap cluster".into()));
    }
    let index = usize::try_from(cluster - 2)
        .map_err(|_| RecoveryError::LengthTooLarge { length: u64::from(cluster_count) })?;
    let byte = index / 8;
    let bit = index % 8;
    if byte >= bitmap.len() {
        return Err(RecoveryError::IoFailure("exFAT allocation bitmap is truncated".into()));
    }
    Ok(bitmap[byte] & (1u8 << bit) != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bitmap_entry() {
        let mut bytes = vec![0u8; 64];
        bytes[0] = ENTRY_ALLOCATION_BITMAP;
        bytes[20..24].copy_from_slice(&7u32.to_le_bytes());
        bytes[24..32].copy_from_slice(&4u64.to_le_bytes());
        assert_eq!(
            parse_allocation_bitmap_entry(&bytes).unwrap(),
            AllocationBitmap { first_cluster: 7, data_length: 4 }
        );
    }

    #[test]
    fn reads_bitmap_bits() {
        let bitmap = [0b0000_0101u8];
        assert!(is_cluster_allocated(&bitmap, 3, 2).unwrap());
        assert!(!is_cluster_allocated(&bitmap, 3, 3).unwrap());
        assert!(is_cluster_allocated(&bitmap, 3, 4).unwrap());
    }

    #[test]
    fn rejects_truncated_bitmap() {
        assert!(is_cluster_allocated(&[], 1, 2).is_err());
    }
}
