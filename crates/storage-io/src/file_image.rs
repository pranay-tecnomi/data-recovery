use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::Mutex,
};

use recovery_core::{ByteRange, RecoveryError, RecoveryResult};

use crate::BlockDevice;

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
        if output.len() < range.length as usize {
            return Err(RecoveryError::IoFailure("output buffer too small".into()));
        }

        let mut file = self
            .file
            .lock()
            .map_err(|_| RecoveryError::IoFailure("file lock poisoned".into()))?;
        file.seek(SeekFrom::Start(range.offset))
            .map_err(|e| RecoveryError::IoFailure(e.to_string()))?;
        file.read_exact(&mut output[..range.length as usize])
            .map_err(|e| RecoveryError::IoFailure(e.to_string()))?;

        Ok(range.length as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn reads_a_bounded_range() {
        let path = std::env::temp_dir().join("data-recovery-m0-image.bin");
        let mut f = File::create(&path).unwrap();
        f.write_all(b"abcdefgh").unwrap();
        drop(f);

        let device = FileImageDevice::open(&path).unwrap();
        let mut output = [0u8; 3];
        let count = device.read(ByteRange::new(2, 3).unwrap(), &mut output).unwrap();
        assert_eq!(count, 3);
        assert_eq!(&output, b"cde");

        std::fs::remove_file(path).unwrap();
    }
}
