#![forbid(unsafe_code)]

mod manifest;

pub use manifest::ImageManifest;

use recovery_core::{ByteRange, CancellationToken, RecoveryError, RecoveryResult};
use storage_io::BlockDevice;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ImagingPolicy {
    pub chunk_size: u64,
    pub max_retries: usize,
    pub min_chunk_size: u64,
}

impl ImagingPolicy {
    pub fn validate(self) -> RecoveryResult<()> {
        if self.chunk_size == 0 || self.min_chunk_size == 0 || self.min_chunk_size > self.chunk_size {
            return Err(RecoveryError::IoFailure("invalid imaging policy".into()));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BadRange {
    pub range: ByteRange,
    pub attempts: usize,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ImagingReport {
    pub bytes_copied: u64,
    pub bad_ranges: Vec<BadRange>,
}

pub trait ImageSink {
    fn write(&mut self, offset: u64, data: &[u8]) -> RecoveryResult<()>;
}

pub trait ImagingProgress {
    fn copied(&mut self, bytes_copied: u64, total_bytes: u64);
    fn bad_range(&mut self, bad_range: BadRange);
}

pub struct ImagingEngine {
    policy: ImagingPolicy,
}

impl ImagingEngine {
    pub fn new(policy: ImagingPolicy) -> RecoveryResult<Self> {
        policy.validate()?;
        Ok(Self { policy })
    }

    pub fn image<D: BlockDevice, S: ImageSink>(
        &self,
        source: &D,
        sink: &mut S,
        cancellation: &CancellationToken,
    ) -> RecoveryResult<ImagingReport> {
        self.image_with_progress(source, sink, cancellation, None)
    }

    pub fn image_with_progress<D: BlockDevice, S: ImageSink>(
        &self,
        source: &D,
        sink: &mut S,
        cancellation: &CancellationToken,
        mut progress: Option<&mut dyn ImagingProgress>,
    ) -> RecoveryResult<ImagingReport> {
        let mut report = ImagingReport::default();
        let mut offset = 0;
        let capacity = source.capacity();

        while offset < capacity {
            if cancellation.is_cancelled() {
                return Err(RecoveryError::Cancelled);
            }
            let length = self.policy.chunk_size.min(capacity - offset);
            self.copy_range(
                source,
                sink,
                ByteRange::new(offset, length)?,
                cancellation,
                &mut report,
                capacity,
                &mut progress,
            )?;
            offset += length;
        }
        Ok(report)
    }

    fn copy_range<D: BlockDevice, S: ImageSink>(
        &self,
        source: &D,
        sink: &mut S,
        range: ByteRange,
        cancellation: &CancellationToken,
        report: &mut ImagingReport,
        total_bytes: u64,
        progress: &mut Option<&mut dyn ImagingProgress>,
    ) -> RecoveryResult<()> {
        if cancellation.is_cancelled() {
            return Err(RecoveryError::Cancelled);
        }
        let length = usize::try_from(range.length)
            .map_err(|_| RecoveryError::LengthTooLarge { length: range.length })?;
        let mut buffer = vec![0; length];
        let mut attempts = 0;
        loop {
            attempts += 1;
            match source.read(range, &mut buffer) {
                Ok(read) if read == length => {
                    sink.write(range.offset, &buffer)?;
                    report.bytes_copied += read as u64;
                    if let Some(progress) = progress.as_deref_mut() {
                        progress.copied(report.bytes_copied, total_bytes);
                    }
                    return Ok(());
                }
                Ok(_) | Err(_) if attempts <= self.policy.max_retries => continue,
                Ok(_) | Err(_) if range.length > self.policy.min_chunk_size => {
                    let left_length = (range.length / 2).max(self.policy.min_chunk_size);
                    let right_length = range.length - left_length;
                    self.copy_range(source, sink, ByteRange::new(range.offset, left_length)?, cancellation, report, total_bytes, progress)?;
                    if right_length > 0 {
                        self.copy_range(source, sink, ByteRange::new(range.offset + left_length, right_length)?, cancellation, report, total_bytes, progress)?;
                    }
                    return Ok(());
                }
                Ok(_) | Err(_) => {
                    let bad_range = BadRange { range, attempts };
                    report.bad_ranges.push(bad_range);
                    if let Some(progress) = progress.as_deref_mut() {
                        progress.bad_range(bad_range);
                    }
                    return Ok(());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use storage_io::{Fault, FaultInjectingDevice};

    struct MemoryDevice(Vec<u8>);
    impl BlockDevice for MemoryDevice {
        fn capacity(&self) -> u64 { self.0.len() as u64 }
        fn read(&self, range: ByteRange, output: &mut [u8]) -> RecoveryResult<usize> {
            range.validate_within(self.capacity())?;
            let n = usize::try_from(range.length).unwrap();
            if output.len() < n { return Err(RecoveryError::OutputBufferTooSmall { required: n, provided: output.len() }); }
            output[..n].copy_from_slice(&self.0[range.offset as usize..range.end().unwrap() as usize]);
            Ok(n)
        }
    }

    #[derive(Default)]
    struct MemorySink(Vec<u8>);
    impl ImageSink for MemorySink {
        fn write(&mut self, offset: u64, data: &[u8]) -> RecoveryResult<()> {
            let start = offset as usize;
            let end = start + data.len();
            if self.0.len() < end { self.0.resize(end, 0); }
            self.0[start..end].copy_from_slice(data);
            Ok(())
        }
    }

    fn engine() -> ImagingEngine {
        ImagingEngine::new(ImagingPolicy { chunk_size: 4, max_retries: 1, min_chunk_size: 1 }).unwrap()
    }

    #[test]
    fn copies_source_in_bounded_chunks() {
        let source = MemoryDevice(b"abcdefgh".to_vec());
        let mut sink = MemorySink::default();
        let report = engine().image(&source, &mut sink, &CancellationToken::new()).unwrap();
        assert_eq!(sink.0, b"abcdefgh");
        assert_eq!(report.bytes_copied, 8);
        assert!(report.bad_ranges.is_empty());
    }

    #[test]
    fn records_permanent_failures_after_reduction() {
        let source = FaultInjectingDevice::new(MemoryDevice(b"abcd".to_vec()), Fault::Permanent);
        let mut sink = MemorySink::default();
        let report = engine().image(&source, &mut sink, &CancellationToken::new()).unwrap();
        assert_eq!(report.bytes_copied, 0);
        assert_eq!(report.bad_ranges.len(), 4);
        assert!(report.bad_ranges.iter().all(|bad| bad.range.length == 1));
    }

    #[test]
    fn cancellation_stops_before_reading() {
        let source = MemoryDevice(b"abcd".to_vec());
        let mut sink = MemorySink::default();
        let token = CancellationToken::new();
        token.cancel();
        assert_eq!(engine().image(&source, &mut sink, &token).unwrap_err(), RecoveryError::Cancelled);
    }

    #[derive(Default)]
    struct Progress { copied: Vec<(u64, u64)> }
    impl ImagingProgress for Progress {
        fn copied(&mut self, bytes_copied: u64, total_bytes: u64) { self.copied.push((bytes_copied, total_bytes)); }
        fn bad_range(&mut self, _: BadRange) {}
    }

    #[test]
    fn reports_monotonic_copy_progress() {
        let source = MemoryDevice(b"abcdefgh".to_vec());
        let mut sink = MemorySink::default();
        let mut progress = Progress::default();
        engine().image_with_progress(&source, &mut sink, &CancellationToken::new(), Some(&mut progress)).unwrap();
        assert_eq!(progress.copied, vec![(4, 8), (8, 8)]);
    }
}
