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
        self.missing_ranges().is_empty()
    }

    pub fn missing_ranges(&self) -> Vec<ByteRange> {
        if self.source_capacity == 0 {
            return Vec::new();
        }
        let mut ranges = self.completed_ranges.clone();
        ranges.sort_by_key(|range| range.offset);
        let mut cursor = 0;
        let mut missing = Vec::new();
        for range in ranges {
            let end = match range.end() {
                Some(end) => end.min(self.source_capacity),
                None => continue,
            };
            if end <= cursor {
                continue;
            }
            if range.offset > cursor {
                if let Ok(gap) = ByteRange::new(cursor, range.offset - cursor) {
                    missing.push(gap);
                }
            }
            cursor = cursor.max(end);
            if cursor >= self.source_capacity {
                break;
            }
        }
        if cursor < self.source_capacity {
            if let Ok(gap) = ByteRange::new(cursor, self.source_capacity - cursor) {
                missing.push(gap);
            }
        }
        missing
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
    fn reports_gaps_for_resume() {
        let mut manifest = ImageManifest::new(12);
        manifest.record_completed(ByteRange::new(2, 2).unwrap()).unwrap();
        manifest.record_completed(ByteRange::new(6, 4).unwrap()).unwrap();
        assert_eq!(
            manifest.missing_ranges(),
            vec![
                ByteRange::new(0, 2).unwrap(),
                ByteRange::new(4, 2).unwrap(),
                ByteRange::new(10, 2).unwrap(),
            ]
        );
    }

    #[test]
    fn overlapping_completed_ranges_do_not_create_false_gaps() {
        let mut manifest = ImageManifest::new(8);
        manifest.record_completed(ByteRange::new(0, 6).unwrap()).unwrap();
        manifest.record_completed(ByteRange::new(4, 4).unwrap()).unwrap();
        assert!(manifest.missing_ranges().is_empty());
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
