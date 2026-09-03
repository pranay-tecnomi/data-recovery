#![forbid(unsafe_code)]

mod gpt;
mod mbr;

use recovery_core::{ByteRange, RecoveryError, RecoveryResult};
use storage_io::BlockDevice;

pub use gpt::{discover_gpt, parse_gpt_header, GptHeader};
pub use mbr::{discover_mbr, MbrEntry};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DiskGeometry {
    pub logical_sector_size: u64,
}

impl DiskGeometry {
    pub fn new(logical_sector_size: u64) -> RecoveryResult<Self> {
        if logical_sector_size < 512 {
            return Err(RecoveryError::IoFailure("logical sector size below MBR minimum".into()));
        }
        Ok(Self { logical_sector_size })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PartitionCandidate {
    pub index: u8,
    pub range: ByteRange,
    pub type_code: u8,
    pub bootable: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Diagnostic {
    InvalidMbrSignature,
    RangeOutOfBounds { index: u8 },
    RangeOverflow { index: u8 },
    UnsupportedExtendedPartition { index: u8 },
    Overlap { left: u8, right: u8 },
    GptRangeInvalid { index: u32 },
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DiscoveryResult {
    pub partitions: Vec<PartitionCandidate>,
    pub diagnostics: Vec<Diagnostic>,
}

pub fn detect_overlaps(result: &mut DiscoveryResult) {
    let mut ordered: Vec<_> = result.partitions.iter().collect();
    ordered.sort_by_key(|p| p.range.offset);
    for pair in ordered.windows(2) {
        let left = pair[0];
        let right = pair[1];
        if let Ok(end) = left.range.end()
            && end > right.range.offset
        {
            result.diagnostics.push(Diagnostic::Overlap { left: left.index, right: right.index });
        }
    }
}

pub fn read_exact_at<D: BlockDevice>(device: &D, offset: u64, output: &mut [u8]) -> RecoveryResult<()> {
    let length = u64::try_from(output.len()).map_err(|_| RecoveryError::LengthTooLarge { length: u64::MAX })?;
    let range = ByteRange::new(offset, length)?;
    let read = device.read(range, output)?;
    if read != output.len() {
        return Err(RecoveryError::IoFailure("short read while exact read required".into()));
    }
    Ok(())
}
