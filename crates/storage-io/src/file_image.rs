use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::Mutex,
};

use crate::BlockDevice;
use recovery_core::{ByteRange, RecoveryError, RecoveryResult};

#[derive(Debug)]
pub struct FileImageDevice {
    path: PathBuf,
    capacity: u64,
    file: Mutex<File>,
}

impl FileImageDevice {
    pub fn open(path: impl AsRef<Path>) -> RecoveryResult<Self> {
        let path = path.as_ref().to_path_buf();
        let file = File::open(&path).map_err(|e| RecoveryError::IoFailure(e.to_string()))?;
        let capacity = file
            .metadata()
            .map_err(|e| RecoveryError::IoFailure(e.to_string()))?
            .len();
        Ok(Self {
            path,
            capacity,
            file: Mutex::new(file),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl BlockDevice for FileImageDevice {
    fn capacity(&self) -> u64 {
        self.capacity
    }

    fn read(&self, range: ByteRange, output: &mut [u8]) -> RecoveryResult<usize> {
        range.validate_within(self.capacity)?;
        let length = usize::try_from(range.length)
            .map_err(|_| RecoveryError::LengthTooLarge { length: range.length })?;
        if output.len() < length {
            return Err(RecoveryError::OutputBufferTooSmall {
                required: length,
                provided: output.len(),
            });
        }
        if length == 0 {
            return Ok(0);
        }
        let mut file = self
            .file
            .lock()
            .map_err(|_| RecoveryError::IoFailure("file lock poisoned".into()))?;
        file.seek(SeekFrom::Start(range.offset))
            .map_err(|e| RecoveryError::IoFailure(e.to_string()))?;
        file.read_exact(&mut output[..length])
            .map_err(|e| RecoveryError::IoFailure(e.to_string()))?;
        Ok(length)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::Write,
        sync::atomic::{AtomicU64, Ordering},
    };

    static FIXTURE_ID: AtomicU64 = AtomicU64::new(0);

    fn fixture() -> std::path::PathBuf {
        let id = FIXTURE_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "data-recovery-m0-{}-{}-image.bin",
            std::process::id(),
            id
        ));
        let mut f = File::create(&path).unwrap();
        f.write_all(b"abcdefgh").unwrap();
        path
    }

    #[test]
    fn reads_bounded_range() {
        let path = fixture();
        let d = FileImageDevice::open(&path).unwrap();
        let mut out = [0; 3];
        assert_eq!(
            d.read(ByteRange::new(2, 3).unwrap(), &mut out).unwrap(),
            3
        );
        assert_eq!(&out, b"cde");
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn rejects_out_of_range() {
        let path = fixture();
        let d = FileImageDevice::open(&path).unwrap();
        let mut out = [0; 2];
        assert!(matches!(
            d.read(ByteRange::new(7, 2).unwrap(), &mut out),
            Err(RecoveryError::OutOfRange { .. })
        ));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn rejects_small_buffer() {
        let path = fixture();
        let d = FileImageDevice::open(&path).unwrap();
        let mut out = [0; 1];
        assert!(matches!(
            d.read(ByteRange::new(0, 2).unwrap(), &mut out),
            Err(RecoveryError::OutputBufferTooSmall { .. })
        ));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn accepts_zero_length() {
        let path = fixture();
        let d = FileImageDevice::open(&path).unwrap();
        let mut out = [];
        assert_eq!(
            d.read(ByteRange::new(8, 0).unwrap(), &mut out).unwrap(),
            0
        );
        std::fs::remove_file(path).unwrap();
    }
}
