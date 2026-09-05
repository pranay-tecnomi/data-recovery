//! Desktop shell for the data recovery engine.
//!
//! The UI is unprivileged and owns no parsing logic: it calls into the engine
//! crates and renders what they report, including the evidence behind every
//! confidence classification.

#![forbid(unsafe_code)]
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod engine;

use engine::{RecoveryResultView, ScanResult};

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
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![scan_image, recover_files])
        .run(tauri::generate_context!())
        .expect("failed to start the desktop shell");
}
