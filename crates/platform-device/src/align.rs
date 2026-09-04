//! Alignment arithmetic for raw-device adapters.
//!
//! Windows raw volume and physical-drive handles reject reads whose offset or
//! length is not a multiple of the logical sector size. The BlockDevice
//! contract nevertheless requires callers to pass arbitrary byte ranges, so
//! adapters read the enclosing aligned span and trim in memory.
//!
//! This module is pure arithmetic and is compiled and tested on every platform,
//! so the logic is verifiable without the target hardware.

use recovery_core::{ByteRange, RecoveryError, RecoveryResult};

/// An aligned span covering a caller's requested range.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AlignedRead {
    /// Sector-aligned range to issue against the device.
    pub aligned: ByteRange,
    /// Offset of the caller's data within the aligned buffer.
    pub inner_offset: usize,
    /// Number of bytes the caller asked for.
    pub inner_length: usize,
}

/// Computes the aligned span enclosing `range`.
///
/// `sector_size` must be a non-zero power of two. `capacity` bounds the span:
/// the tail is clamped so a read near the end of the device does not run past
/// it, which would otherwise fail on a device whose capacity is not a whole
/// number of sectors.
pub fn align_read(
    range: ByteRange,
    sector_size: u64,
    capacity: u64,
) -> RecoveryResult<AlignedRead> {
    if sector_size == 0 || !sector_size.is_power_of_two() {
        return Err(RecoveryError::Unsupported(format!(
            "sector size {sector_size} is not a non-zero power of two"
        )));
    }
    range.validate_within(capacity)?;

    let end = range.end()?;
    // Round the start down and the end up to sector boundaries. The mask form
    // is exact because sector_size is a power of two.
    let start = range.offset & !(sector_size - 1);
    let aligned_end = end
        .checked_add(sector_size - 1)
        .ok_or(RecoveryError::RangeOverflow)?
        & !(sector_size - 1);
    // Never read past the device, even when rounding up would.
    let aligned_end = aligned_end.min(capacity);

    let inner_offset = usize::try_from(range.offset - start)
        .map_err(|_| RecoveryError::RangeOverflow)?;
    let inner_length = usize::try_from(range.length)
        .map_err(|_| RecoveryError::LengthTooLarge { length: range.length })?;

    Ok(AlignedRead {
        aligned: ByteRange::new(start, aligned_end - start)?,
        inner_offset,
        inner_length,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: u64 = 512;
    const CAP: u64 = 1 << 20;

    fn aligned(offset: u64, length: u64) -> AlignedRead {
        align_read(ByteRange::new(offset, length).unwrap(), S, CAP).unwrap()
    }

    #[test]
    fn already_aligned_range_is_unchanged() {
        let a = aligned(1024, 512);
        assert_eq!(a.aligned.offset, 1024);
        assert_eq!(a.aligned.length, 512);
        assert_eq!(a.inner_offset, 0);
        assert_eq!(a.inner_length, 512);
    }

    #[test]
    fn unaligned_start_rounds_down() {
        let a = aligned(100, 8);
        assert_eq!(a.aligned.offset, 0);
        assert_eq!(a.aligned.length, 512);
        assert_eq!(a.inner_offset, 100);
        assert_eq!(a.inner_length, 8);
    }

    #[test]
    fn span_crossing_sector_boundary_covers_both() {
        // 500..524 straddles the boundary at 512.
        let a = aligned(500, 24);
        assert_eq!(a.aligned.offset, 0);
        assert_eq!(a.aligned.length, 1024);
        assert_eq!(a.inner_offset, 500);
    }

    #[test]
    fn tail_is_clamped_to_capacity() {
        // A device whose capacity is not a whole number of sectors.
        let cap = 1000;
        let a = align_read(ByteRange::new(900, 100).unwrap(), S, cap).unwrap();
        assert_eq!(a.aligned.offset, 512);
        // Rounding up would reach 1024; capacity stops it at 1000.
        assert_eq!(a.aligned.end().unwrap(), cap);
        assert_eq!(a.inner_offset, 388);
    }

    #[test]
    fn zero_length_is_preserved() {
        let a = aligned(700, 0);
        assert_eq!(a.inner_length, 0);
    }

    #[test]
    fn four_kn_geometry_is_supported() {
        let a = align_read(ByteRange::new(5000, 10).unwrap(), 4096, CAP).unwrap();
        assert_eq!(a.aligned.offset, 4096);
        assert_eq!(a.aligned.length, 4096);
        assert_eq!(a.inner_offset, 904);
    }

    #[test]
    fn rejects_non_power_of_two_sector_size() {
        assert!(matches!(
            align_read(ByteRange::new(0, 1).unwrap(), 513, CAP),
            Err(RecoveryError::Unsupported(_))
        ));
        assert!(matches!(
            align_read(ByteRange::new(0, 1).unwrap(), 0, CAP),
            Err(RecoveryError::Unsupported(_))
        ));
    }

    #[test]
    fn rejects_range_beyond_capacity() {
        assert!(matches!(
            align_read(ByteRange::new(CAP, 1).unwrap(), S, CAP),
            Err(RecoveryError::OutOfRange { .. })
        ));
    }

    #[test]
    fn aligned_span_always_contains_request() {
        // Exhaustive over a small space: the trimmed window must never fall
        // outside the aligned span that was read.
        for offset in 0..600u64 {
            for length in 0..40u64 {
                let a = aligned(offset, length);
                assert!(a.aligned.offset <= offset);
                let need = a.inner_offset as u64 + length;
                assert!(need <= a.aligned.length, "offset={offset} length={length}");
            }
        }
    }
}
