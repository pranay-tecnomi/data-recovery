#![forbid(unsafe_code)]

mod file_image;

use recovery_core::{ByteRange, RecoveryResult};

pub use file_image::FileImageDevice;

pub trait BlockDevice: Send + Sync {
    fn capacity(&self) -> u64;
    fn read(&self, range: ByteRange, output: &mut [u8]) -> RecoveryResult<usize>;
}
