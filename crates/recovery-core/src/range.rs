use crate::error::{RecoveryError, RecoveryResult};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ByteRange {
    pub offset: u64,
    pub length: u64,
}

impl ByteRange {
    pub fn new(offset: u64, length: u64) -> RecoveryResult<Self> {
        offset
            .checked_add(length)
            .ok_or(RecoveryError::RangeOverflow)?;
        Ok(Self { offset, length })
    }

    pub fn end(self) -> RecoveryResult<u64> {
        self.offset
            .checked_add(self.length)
            .ok_or(RecoveryError::RangeOverflow)
    }

    pub fn validate_within(self, capacity: u64) -> RecoveryResult<()> {
        if self.end()? > capacity {
            return Err(RecoveryError::OutOfRange {
                offset: self.offset,
                length: self.length,
                capacity,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_overflow() {
        assert_eq!(
            ByteRange::new(u64::MAX, 1).unwrap_err(),
            RecoveryError::RangeOverflow
        );
    }

    #[test]
    fn validates_capacity() {
        let range = ByteRange::new(8, 4).unwrap();
        assert!(range.validate_within(12).is_ok());
        assert!(range.validate_within(11).is_err());
    }
}
