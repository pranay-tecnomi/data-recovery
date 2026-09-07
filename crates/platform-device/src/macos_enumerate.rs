//! Enumerates macOS disks and volumes via `diskutil`.
//!
//! `diskutil` is used rather than IOKit because it needs no elevation and no
//! unsafe FFI, which keeps device listing available to the unprivileged UI.
//! Only listing is unprivileged: opening `/dev/rdisk*` for reading still
//! requires the user to grant access.
//!
//! Everything `diskutil` reports is treated as untrusted input. It is a parsed
//! plist from a subprocess, so a device with an implausible capacity or sector
//! size is skipped rather than propagated into the engine.

#![cfg(target_os = "macos")]

use std::process::Command;

use recovery_core::{RecoveryError, RecoveryResult, SourceId};

use crate::{DeviceInfo, SourceKind};

/// Upper bound on a plausible sector size. Anything larger is a parse error or
/// a hostile value, not a real device.
const MAX_SECTOR_SIZE: u64 = 65_536;

/// Runs `diskutil` and returns its stdout.
fn diskutil(args: &[&str]) -> RecoveryResult<String> {
    let output = Command::new("/usr/sbin/diskutil")
        .args(args)
        .output()
        .map_err(|e| RecoveryError::IoFailure(format!("diskutil: {e}")))?;
    if !output.status.success() {
        return Err(RecoveryError::IoFailure(format!(
            "diskutil {} failed",
            args.join(" ")
        )));
    }
    String::from_utf8(output.stdout)
        .map_err(|_| RecoveryError::IoFailure("diskutil returned non-UTF-8 output".into()))
}

/// Extracts the string values of a plist `<array>` following `key`.
fn plist_string_array(plist: &str, key: &str) -> Vec<String> {
    let needle = format!("<key>{key}</key>");
    let Some(start) = plist.find(&needle) else {
        return Vec::new();
    };
    let rest = &plist[start + needle.len()..];
    let Some(open) = rest.find("<array>") else {
        return Vec::new();
    };
    let Some(close) = rest.find("</array>") else {
        return Vec::new();
    };
    if close < open {
        return Vec::new();
    }
    let mut out = Vec::new();
    for chunk in rest[open..close].split("<string>").skip(1) {
        if let Some(end) = chunk.find("</string>") {
            out.push(chunk[..end].to_string());
        }
    }
    out
}

/// Reads an integer-valued plist key.
fn plist_integer(plist: &str, key: &str) -> Option<u64> {
    let needle = format!("<key>{key}</key>");
    let start = plist.find(&needle)? + needle.len();
    let rest = &plist[start..];
    let open = rest.find("<integer>")? + "<integer>".len();
    let end = rest.find("</integer>")?;
    if end < open {
        return None;
    }
    rest[open..end].trim().parse().ok()
}

/// Reads a string-valued plist key.
fn plist_string(plist: &str, key: &str) -> Option<String> {
    let needle = format!("<key>{key}</key>");
    let start = plist.find(&needle)? + needle.len();
    let rest = &plist[start..];
    let open = rest.find("<string>")? + "<string>".len();
    let end = rest.find("</string>")?;
    if end < open {
        return None;
    }
    Some(rest[open..end].to_string())
}

/// Reads a boolean-valued plist key, which plists encode as empty elements.
fn plist_bool(plist: &str, key: &str) -> Option<bool> {
    let needle = format!("<key>{key}</key>");
    let start = plist.find(&needle)? + needle.len();
    let rest = plist[start..].trim_start();
    if rest.starts_with("<true/>") {
        Some(true)
    } else if rest.starts_with("<false/>") {
        Some(false)
    } else {
        None
    }
}

/// Describes one device `diskutil` reported.
fn describe(identifier: &str) -> RecoveryResult<DeviceInfo> {
    let plist = diskutil(&["info", "-plist", identifier])?;

    let capacity = plist_integer(&plist, "Size")
        .or_else(|| plist_integer(&plist, "IOKitSize"))
        .ok_or_else(|| {
            RecoveryError::IoFailure(format!("{identifier} reports no usable capacity"))
        })?;
    if capacity == 0 {
        return Err(RecoveryError::IoFailure(format!(
            "{identifier} reports a zero capacity"
        )));
    }

    // Never assume 512: Apple internal SSDs commonly report 4096.
    let logical_sector_size = plist_integer(&plist, "DeviceBlockSize").unwrap_or(512);
    if logical_sector_size == 0
        || !logical_sector_size.is_power_of_two()
        || logical_sector_size > MAX_SECTOR_SIZE
    {
        return Err(RecoveryError::Unsupported(format!(
            "{identifier} reports an implausible sector size {logical_sector_size}"
        )));
    }

    // A whole disk has no partition suffix; `diskutil` also states it directly.
    let whole = plist_bool(&plist, "WholeDisk").unwrap_or(!identifier.contains('s'));
    let kind = if whole {
        SourceKind::PhysicalDrive
    } else {
        SourceKind::Volume
    };

    Ok(DeviceInfo {
        id: SourceId::new(identifier),
        kind,
        path: format!("/dev/r{identifier}"),
        display_name: plist_string(&plist, "VolumeName")
            .filter(|name| !name.is_empty())
            .or_else(|| plist_string(&plist, "MediaName"))
            .or_else(|| plist_string(&plist, "IORegistryEntryName")),
        capacity,
        logical_sector_size,
        physical_sector_size: plist_integer(&plist, "DeviceBlockSize"),
        removable: plist_bool(&plist, "RemovableMediaOrExternalDevice")
            .or_else(|| plist_bool(&plist, "Removable"))
            .unwrap_or(false),
    })
}

/// Lists the disks and volumes available as recovery sources.
///
/// A device that cannot be described is skipped rather than failing the whole
/// enumeration: one unreadable disk must not hide every other one from the
/// user. Ordering follows `diskutil`, which is stable across runs.
pub fn enumerate_devices() -> RecoveryResult<Vec<DeviceInfo>> {
    let plist = diskutil(&["list", "-plist"])?;
    let mut out = Vec::new();
    for identifier in plist_string_array(&plist, "AllDisks") {
        // Guard against a hostile identifier reaching a command line or a path.
        if identifier.is_empty()
            || !identifier.chars().all(|c| c.is_ascii_alphanumeric())
            || !identifier.starts_with("disk")
        {
            continue;
        }
        if let Ok(info) = describe(&identifier) {
            out.push(info);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<plist version="1.0"><dict>
        <key>AllDisks</key><array>
            <string>disk0</string><string>disk0s1</string>
        </array>
        <key>Size</key><integer>500277792768</integer>
        <key>DeviceBlockSize</key><integer>4096</integer>
        <key>MediaName</key><string>APPLE SSD AP0512Z</string>
        <key>Removable</key><false/>
        <key>Internal</key><true/>
    </dict></plist>"#;

    #[test]
    fn reads_arrays_integers_strings_and_booleans() {
        assert_eq!(
            plist_string_array(SAMPLE, "AllDisks"),
            vec!["disk0".to_string(), "disk0s1".to_string()]
        );
        assert_eq!(plist_integer(SAMPLE, "Size"), Some(500_277_792_768));
        assert_eq!(plist_integer(SAMPLE, "DeviceBlockSize"), Some(4096));
        assert_eq!(
            plist_string(SAMPLE, "MediaName").as_deref(),
            Some("APPLE SSD AP0512Z")
        );
        assert_eq!(plist_bool(SAMPLE, "Removable"), Some(false));
        assert_eq!(plist_bool(SAMPLE, "Internal"), Some(true));
    }

    #[test]
    fn missing_keys_yield_nothing_rather_than_panicking() {
        assert!(plist_string_array(SAMPLE, "Absent").is_empty());
        assert_eq!(plist_integer(SAMPLE, "Absent"), None);
        assert_eq!(plist_string(SAMPLE, "Absent"), None);
        assert_eq!(plist_bool(SAMPLE, "Absent"), None);
    }

    #[test]
    fn malformed_plists_do_not_panic() {
        for broken in [
            "",
            "<plist>",
            "<key>Size</key>",
            "<key>Size</key><integer>",
            "<key>Size</key><integer>not-a-number</integer>",
            "<key>AllDisks</key><array>",
            "<key>AllDisks</key></array><array>",
        ] {
            let _ = plist_string_array(broken, "AllDisks");
            let _ = plist_integer(broken, "Size");
            let _ = plist_string(broken, "MediaName");
            let _ = plist_bool(broken, "Removable");
        }
    }

    #[test]
    fn enumeration_reports_real_devices_on_this_machine() {
        // Listing needs no elevation, so this runs anywhere macOS does.
        let devices = enumerate_devices().expect("diskutil is available on macOS");
        assert!(!devices.is_empty(), "a Mac always has at least one disk");
        for device in &devices {
            assert!(device.capacity > 0);
            assert!(device.logical_sector_size.is_power_of_two());
            assert!(device.path.starts_with("/dev/rdisk"));
        }
        assert!(
            devices.iter().any(|d| d.kind == SourceKind::PhysicalDrive),
            "at least one whole disk must be reported"
        );
    }
}
