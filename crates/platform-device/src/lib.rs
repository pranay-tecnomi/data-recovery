//! Platform adapters for raw physical-device access (ADR-011).
//!
//! Engine crates depend on `BlockDevice` only; nothing above this crate
//! references a platform API. Sources are opened read-only on every platform,
//! upholding the read-only source policy (ADR-003).
//!
//! Disk-image sources do not belong here: they are unprivileged and are served
//! by `storage_io::FileImageDevice` on both platforms.

#![forbid(unsafe_code)]

pub mod align;
pub mod filename;

#[cfg(target_os = "macos")]
pub mod macos;

pub use align::{align_read, AlignedRead};
pub use filename::{sanitize_component, SanitizedName};

#[cfg(target_os = "macos")]
pub use macos::{raw_device_path, MacRawDevice};

use recovery_core::SourceId;

/// How a source is attached, which determines whether elevation is required.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceKind {
    /// A file-backed image. Never requires elevation.
    Image,
    /// A whole physical drive (`/dev/rdiskN`, `\\.\PhysicalDriveN`).
    PhysicalDrive,
    /// A mounted volume (`/dev/rdiskNsM`, `\\.\X:`).
    Volume,
}

impl SourceKind {
    /// Raw device access requires elevation; images do not. This is what keeps
    /// the image-first MVP path unprivileged on both platforms.
    pub fn requires_elevation(self) -> bool {
        !matches!(self, SourceKind::Image)
    }
}

/// Identity and geometry of a platform device.
///
/// Identity is deliberately not a path: drive letters and `/dev` node numbers
/// are reassignable, so they cannot anchor resume or source/destination
/// overlap checks.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeviceInfo {
    pub id: SourceId,
    pub kind: SourceKind,
    /// Platform-native path used to open the device.
    pub path: String,
    /// Human-readable model or volume label, for display only. Never trusted
    /// for identity or filesystem classification.
    pub display_name: Option<String>,
    pub capacity: u64,
    /// Logical sector size. Reads are aligned to this; never assume 512.
    pub logical_sector_size: u64,
    /// Physical sector size where reported (4096 on 4Kn and 512e media).
    pub physical_sector_size: Option<u64>,
    pub removable: bool,
}

impl DeviceInfo {
    /// Whether opening this device needs elevated privileges.
    pub fn requires_elevation(&self) -> bool {
        self.kind.requires_elevation()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn images_do_not_require_elevation() {
        assert!(!SourceKind::Image.requires_elevation());
    }

    #[test]
    fn raw_devices_require_elevation() {
        assert!(SourceKind::PhysicalDrive.requires_elevation());
        assert!(SourceKind::Volume.requires_elevation());
    }
}
