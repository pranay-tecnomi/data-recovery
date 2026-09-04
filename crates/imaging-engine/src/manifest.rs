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
        self.record_completed_batch(&[range])
    }

    pub fn record_completed_batch(&mut self, ranges: &[ByteRange]) -> RecoveryResult<()> {
        for range in ranges {
            range.validate_within(self.source_capacity)?;
        }
        self.completed_ranges.extend_from_slice(ranges);
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

    pub fn completed_bytes(&self) -> u64 {
        self.covered_ranges().iter().map(|range| range.length).sum()
    }

    pub fn missing_ranges(&self) -> Vec<ByteRange> {
        if self.source_capacity == 0 {
            return Vec::new();
        }
        let mut cursor = 0;
        let mut missing = Vec::new();
        for range in self.covered_ranges() {
            if range.offset > cursor {
                if let Ok(gap) = ByteRange::new(cursor, range.offset - cursor) {
                    missing.push(gap);
                }
            }
            cursor = range.end().unwrap_or(cursor).max(cursor);
        }
        if cursor < self.source_capacity {
            if let Ok(gap) = ByteRange::new(cursor, self.source_capacity - cursor) {
                missing.push(gap);
            }
        }
        missing
    }

    fn covered_ranges(&self) -> Vec<ByteRange> {
        let mut ranges = self.completed_ranges.clone();
        ranges.sort_by_key(|range| range.offset);
        let mut merged: Vec<ByteRange> = Vec::new();
        for range in ranges {
            let end = match range.end() {
                Some(end) => end.min(self.source_capacity),
                None => continue,
            };
            if end <= range.offset {
                continue;
            }
            let range = match ByteRange::new(range.offset, end - range.offset) {
                Ok(range) => range,
                Err(_) => continue,
            };
            if let Some(last) = merged.last_mut() {
                let last_end = last.end().unwrap();
                if range.offset <= last_end {
                    let merged_end = last_end.max(range.end().unwrap());
                    *last = ByteRange::new(last.offset, merged_end - last.offset).unwrap();
                    continue;
                }
            }
            merged.push(range);
        }
        merged
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
        assert_eq!(manifest.completed_bytes(), 8);
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
    fn overlapping_completed_ranges_are_counted_once() {
        let mut manifest = ImageManifest::new(8);
        manifest.record_completed(ByteRange::new(0, 6).unwrap()).unwrap();
        manifest.record_completed(ByteRange::new(4, 4).unwrap()).unwrap();
        assert!(manifest.missing_ranges().is_empty());
        assert_eq!(manifest.completed_bytes(), 8);
    }

    #[test]
    fn batch_rejects_without_partial_mutation() {
        let mut manifest = ImageManifest::new(8);
        let ranges = [ByteRange::new(0, 4).unwrap(), ByteRange::new(7, 2).unwrap()];
        assert!(manifest.record_completed_batch(&ranges).is_err());
        assert!(manifest.completed_ranges.is_empty());
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
