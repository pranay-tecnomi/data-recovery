use crate::{BadRange, ImagingReport};
use recovery_core::{ByteRange, RecoveryError, RecoveryResult};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ImageManifest {
    pub source_capacity: u64,
    pub completed_ranges: Vec<ByteRange>,
    pub bad_ranges: Vec<BadRange>,
}

impl ImageManifest {
    pub fn new(source_capacity: u64) -> Self {
        Self { source_capacity, ..Self::default() }
    }

    pub fn record_completed(&mut self, range: ByteRange) -> RecoveryResult<()> {
        range.validate_within(self.source_capacity)?;
        self.completed_ranges.push(range);
        Ok(())
    }

    pub fn record_bad(&mut self, bad_range: BadRange) -> RecoveryResult<()> {
        bad_range.range.validate_within(self.source_capacity)?;
        self.bad_ranges.push(bad_range);
        Ok(())
    }

    pub fn is_complete(&self) -> bool {
        if self.source_capacity == 0 {
            return true;
        }
        let mut ranges = self.completed_ranges.clone();
        ranges.sort_by_key(|range| range.offset);
        let mut cursor = 0;
        for range in ranges {
            if range.offset > cursor {
                return false;
            }
            let end = match range.end() {
                Some(end) => end,
                None => return false,
            };
            cursor = cursor.max(end);
            if cursor >= self.source_capacity {
                return true;
            }
        }
        false
    }

    pub fn from_report(source_capacity: u64, report: &ImagingReport) -> Self {
        Self { source_capacity, completed_ranges: Vec::new(), bad_ranges: report.bad_ranges.clone() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_contiguous_completion() {
        let mut manifest = ImageManifest::new(8);
        manifest.record_completed(ByteRange::new(4, 4).unwrap()).unwrap();
        assert!(!manifest.is_complete());
        manifest.record_completed(ByteRange::new(0, 4).unwrap()).unwrap();
        assert!(manifest.is_complete());
    }

    #[test]
    fn rejects_ranges_outside_source() {
        let mut manifest = ImageManifest::new(8);
        assert!(matches!(
            manifest.record_completed(ByteRange::new(7, 2).unwrap()),
            Err(RecoveryError::OutOfRange { .. })
        ));
    }
}
