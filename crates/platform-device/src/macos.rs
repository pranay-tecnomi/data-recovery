//! macOS raw-device source adapter.
//!
//! Uses `/dev/rdisk*` or `/dev/disk*` opened read-only. `FileExt::read_at`
//! avoids shared seek state and requires no unsafe code. Capacity and sector
//! geometry are supplied by the caller because character devices do not expose
//! reliable file lengths through ordinary metadata.

#![cfg(target_os = "macos")]

use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};

use recovery_core::{ByteRange, RecoveryError, RecoveryResult, SourceId};
use storage_io::BlockDevice;

use crate::{DeviceInfo, SourceKind};

/// A read-only macOS disk or volume device.
#[derive(Debug)]
pub struct MacRawDevice {
    file: File,
    info: DeviceInfo,
}

impl MacRawDevice {
    /// Opens a macOS device read-only.
    ///
    /// `capacity` and sector sizes come from platform enumeration rather than
    /// filesystem metadata. The path is retained only for display/opening;
    /// `source_id` is the stable identity used by recovery state.
    // Enumeration supplies every field individually; grouping them into a
    // descriptor struct is worth doing but changes a public signature, so the
    // arity lint is allowed until that refactor is scheduled.
    #[allow(clippy::too_many_arguments)]
    pub fn open(
        path: impl AsRef<Path>,
        source_id: SourceId,
        kind: SourceKind,
        capacity: u64,
        logical_sector_size: u64,
        physical_sector_size: Option<u64>,
        display_name: Option<String>,
        removable: bool,
    ) -> RecoveryResult<Self> {
        if capacity == 0 {
            return Err(RecoveryError::IoFailure("macOS device capacity is zero".into()));
        }
        if logical_sector_size == 0 || !logical_sector_size.is_power_of_two() {
            return Err(RecoveryError::Unsupported(
                "macOS logical sector size must be a non-zero power of two".into(),
            ));
        }
        if let Some(physical) = physical_sector_size {
            if physical == 0 || !physical.is_power_of_two() {
                return Err(RecoveryError::Unsupported(
                    "macOS physical sector size must be a non-zero power of two".into(),
                ));
            }
            if physical < logical_sector_size || physical % logical_sector_size != 0 {
                return Err(RecoveryError::Unsupported(
                    "macOS physical sector size must be a multiple of logical sector size".into(),
                ));
            }
        }

        let path = path.as_ref();
        let file = OpenOptions::new()
            .read(true)
            .write(false)
            .open(path)
            .map_err(|e| RecoveryError::IoFailure(format!("open {}: {e}", path.display())))?;
        let info = DeviceInfo {
            id: source_id,
            kind,
            path: path.to_string_lossy().into_owned(),
            display_name,
            capacity,
            logical_sector_size,
            physical_sector_size,
            removable,
        };
        Ok(Self { file, info })
    }

    pub fn info(&self) -> &DeviceInfo {
        &self.info
    }

    fn read_at(&self, range: ByteRange, output: &mut [u8]) -> io::Result<usize> {
        let mut total = 0usize;
        while total < output.len() {
            let offset = range
                .offset
                .checked_add(total as u64)
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "read offset overflow"))?;
            let n = self.file.read_at(&mut output[total..], offset)?;
            if n == 0 {
                break;
            }
            total += n;
        }
        Ok(total)
    }
}

impl BlockDevice for MacRawDevice {
    fn capacity(&self) -> u64 {
        self.info.capacity
    }

    fn read(&self, range: ByteRange, output: &mut [u8]) -> RecoveryResult<usize> {
        range.validate_within(self.capacity())?;
        let requested = usize::try_from(range.length)
            .map_err(|_| RecoveryError::LengthTooLarge { length: range.length })?;
        if output.len() < requested {
            return Err(RecoveryError::IoFailure(format!(
                "output buffer is too small: need {requested}, got {}",
                output.len()
            )));
        }
        self.read_at(range, &mut output[..requested])
            .map_err(|e| RecoveryError::IoFailure(format!("read {}: {e}", self.info.path)))
    }
}

fn valid_disk_suffix(rest: &str) -> bool {
    if rest.is_empty() {
        return false;
    }
    let mut parts = rest.split('s');
    let disk = parts.next().unwrap_or_default();
    if disk.is_empty() || !disk.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    match parts.next() {
        None => true,
        Some(partition) => {
            !partition.is_empty()
                && partition.chars().all(|c| c.is_ascii_digit())
                && parts.next().is_none()
        }
    }
}

/// Returns the canonical raw-device spelling used for whole-disk access.
/// `/dev/rdiskN` is preferred on macOS because it avoids the buffered disk
/// layer; partition paths retain their suffix (`rdiskNsM`).
pub fn raw_device_path(path: impl AsRef<Path>) -> RecoveryResult<PathBuf> {
    let path = path.as_ref();
    let value = path.to_string_lossy();
    if let Some(rest) = value.strip_prefix("/dev/disk") {
        if !valid_disk_suffix(rest) {
            return Err(RecoveryError::Unsupported("invalid macOS disk device path".into()));
        }
        return Ok(PathBuf::from(format!("/dev/rdisk{rest}")));
    }
    if let Some(rest) = value.strip_prefix("/dev/rdisk") {
        if !valid_disk_suffix(rest) {
            return Err(RecoveryError::Unsupported("invalid macOS raw disk device path".into()));
        }
        return Ok(path.to_path_buf());
    }
    Err(RecoveryError::Unsupported(
        "macOS raw device path must be under /dev/disk or /dev/rdisk".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_disk_to_raw_disk() {
        assert_eq!(raw_device_path("/dev/disk4").unwrap(), PathBuf::from("/dev/rdisk4"));
        assert_eq!(raw_device_path("/dev/disk4s2").unwrap(), PathBuf::from("/dev/rdisk4s2"));
    }

    #[test]
    fn preserves_raw_disk() {
        assert_eq!(raw_device_path("/dev/rdisk4").unwrap(), PathBuf::from("/dev/rdisk4"));
        assert_eq!(raw_device_path("/dev/rdisk4s2").unwrap(), PathBuf::from("/dev/rdisk4s2"));
    }

    #[test]
    fn rejects_non_device_paths() {
        assert!(raw_device_path("/tmp/image.dmg").is_err());
        assert!(raw_device_path("/dev/diskX").is_err());
        assert!(raw_device_path("/dev/disk4foo").is_err());
        assert!(raw_device_path("/dev/disk4s").is_err());
        assert!(raw_device_path("/dev/rdisk").is_err());
        assert!(raw_device_path("/dev/rdisk4foo").is_err());
    }
}
