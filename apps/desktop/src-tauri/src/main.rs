//! Desktop shell for the data recovery engine.
//!
//! The UI is unprivileged and owns no parsing logic: it calls into the engine
//! crates and renders what they report, including the evidence behind every
//! confidence classification.

#![forbid(unsafe_code)]
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use recovery_engine as engine;
use recovery_engine::{RecoveryResultView, ScanResult, SourceView};

/// Scans an image file and returns everything the results screen needs.
#[tauri::command]
async fn scan_image(path: String, include_carving: bool) -> Result<ScanResult, String> {
    // Scanning is CPU- and IO-bound, so it must not block the UI thread.
    tauri::async_runtime::spawn_blocking(move || {
        engine::scan_image(std::path::Path::new(&path), include_carving)
            .map_err(|e| format!("{e:?}"))
    })
    .await
    .map_err(|e| format!("scan task failed: {e}"))?
}

/// Lists the disks and volumes attached to this machine.
///
/// Listing needs no elevation; opening a device to read it does. Raw-device
/// access is macOS-only for now; other platforms use the image workflow.
#[cfg(target_os = "macos")]
#[tauri::command]
async fn list_devices() -> Result<Vec<SourceView>, String> {
    tauri::async_runtime::spawn_blocking(engine::list_devices)
        .await
        .map_err(|e| format!("device listing failed: {e}"))?
        .map_err(|e| format!("{e:?}"))
}

/// Scans an attached device read-only.
#[cfg(target_os = "macos")]
#[tauri::command]
async fn scan_device(identifier: String, include_carving: bool) -> Result<ScanResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        engine::scan_macos_device(&identifier, include_carving).map_err(|e| format!("{e:?}"))
    })
    .await
    .map_err(|e| format!("scan task failed: {e}"))?
}

/// Recovers the selected candidates from an attached device.
#[cfg(target_os = "macos")]
#[tauri::command]
async fn recover_from_device(
    identifier: String,
    destination: String,
    selected: Vec<String>,
    include_carving: bool,
) -> Result<RecoveryResultView, String> {
    tauri::async_runtime::spawn_blocking(move || {
        engine::recover_from_macos_device(
            &identifier,
            std::path::Path::new(&destination),
            &selected,
            include_carving,
        )
        .map_err(|e| format!("{e:?}"))
    })
    .await
    .map_err(|e| format!("recovery task failed: {e}"))?
}

/// Recovers the selected candidates to a destination directory.
#[tauri::command]
async fn recover_files(
    source: String,
    destination: String,
    selected: Vec<String>,
    include_carving: bool,
) -> Result<RecoveryResultView, String> {
    tauri::async_runtime::spawn_blocking(move || {
        engine::recover(
            std::path::Path::new(&source),
            std::path::Path::new(&destination),
            &selected,
            include_carving,
        )
        .map_err(|e| format!("{e:?}"))
    })
    .await
    .map_err(|e| format!("recovery task failed: {e}"))?
}

fn main() {
    let builder = tauri::Builder::default();

    // Raw-device commands exist only where the platform adapter does; other
    // platforms expose the image workflow alone.
    #[cfg(target_os = "macos")]
    let builder = builder.invoke_handler(tauri::generate_handler![
        scan_image,
        list_devices,
        scan_device,
        recover_from_device,
        recover_files
    ]);
    #[cfg(not(target_os = "macos"))]
    let builder = builder.invoke_handler(tauri::generate_handler![scan_image, recover_files]);

    builder
        .run(tauri::generate_context!())
        .expect("failed to start the desktop shell");
}
