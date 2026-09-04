//! Extents mapping a reconstructed stream onto ranges of its source.

use crate::{ByteRange, RecoveryError, RecoveryResult};

/// One contiguous run of a reconstructed file.
///
/// `logical_offset` is the position of this run within the output stream, so a
/// fragmented file is described by several extents in logical order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Extent {
    pub source_range: ByteRange,
    pub logical_offset: u64,
}

impl Extent {
    pub fn new(source_range: ByteRange, logical_offset: u64) -> RecoveryResult<Self> {
        // The logical end must be representable, or later output arithmetic
        // could wrap.
        logical_offset
            .checked_add(source_range.length)
            .ok_or(RecoveryError::RangeOverflow)?;
        Ok(Self { source_range, logical_offset })
    }

    pub fn length(&self) -> u64 {
        self.source_range.length
    }

    pub fn logical_end(&self) -> RecoveryResult<u64> {
        self.logical_offset
            .checked_add(self.source_range.length)
            .ok_or(RecoveryError::RangeOverflow)
    }
}

/// Total byte count covered by `extents`.
pub fn total_length(extents: &[Extent]) -> RecoveryResult<u64> {
    let mut total: u64 = 0;
    for extent in extents {
        total = total
            .checked_add(extent.length())
            .ok_or(RecoveryError::RangeOverflow)?;
    }
    Ok(total)
}

/// Verifies the domain invariant that extents of a reconstructed stream are
/// non-overlapping in logical output space, and additionally that they are
/// contiguous and ordered, so writing them in sequence yields the whole file
/// with no gap silently filled by zeroes.
pub fn validate_logical_layout(extents: &[Extent]) -> RecoveryResult<()> {
    let mut expected: u64 = 0;
    for extent in extents {
        if extent.logical_offset != expected {
            return Err(RecoveryError::IoFailure(format!(
                "extent logical offset {} breaks contiguity, expected {expected}",
                extent.logical_offset
            )));
        }
        expected = extent.logical_end()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extent(offset: u64, length: u64, logical: u64) -> Extent {
        Extent::new(ByteRange::new(offset, length).unwrap(), logical).unwrap()
    }

    #[test]
    fn reports_length_and_logical_end() {
        let e = extent(1024, 512, 4096);
        assert_eq!(e.length(), 512);
        assert_eq!(e.logical_end().unwrap(), 4608);
    }

    #[test]
    fn rejects_logical_overflow() {
        let range = ByteRange::new(0, 16).unwrap();
        assert_eq!(
            Extent::new(range, u64::MAX - 8).unwrap_err(),
            RecoveryError::RangeOverflow
        );
    }

    #[test]
    fn sums_total_length() {
        let e = [extent(0, 100, 0), extent(4096, 50, 100)];
        assert_eq!(total_length(&e).unwrap(), 150);
        assert_eq!(total_length(&[]).unwrap(), 0);
    }

    #[test]
    fn accepts_contiguous_layout() {
        // Fragmented in source order, but contiguous logically.
        let e = [extent(8192, 512, 0), extent(1024, 512, 512)];
        assert!(validate_logical_layout(&e).is_ok());
        assert!(validate_logical_layout(&[]).is_ok());
    }

    #[test]
    fn rejects_logical_gap() {
        // A gap would be silently zero-filled on output.
        let e = [extent(0, 512, 0), extent(4096, 512, 1024)];
        assert!(validate_logical_layout(&e).is_err());
    }

    #[test]
    fn rejects_logical_overlap() {
        let e = [extent(0, 512, 0), extent(4096, 512, 256)];
        assert!(validate_logical_layout(&e).is_err());
    }
}
