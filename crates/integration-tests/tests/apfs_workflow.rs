//! The complete product workflow over an APFS image, as the app performs it:
//! detect the filesystem, scan for candidates, recover to a destination
//! directory, and verify the written bytes against the known source.

use std::path::PathBuf;

use integration_tests::apfs_fixture::ApfsImage;

fn workspace(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("apfs-workflow-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("workspace is creatable");
    dir
}

/// Writes the fixture to a real file, since the engine works on image paths.
fn image_file(dir: &std::path::Path, image: &ApfsImage) -> PathBuf {
    let path = dir.join("source.img");
    std::fs::write(&path, &image.bytes).expect("image is writable");
    path
}

#[test]
fn detects_apfs_and_lists_recoverable_files() {
    let dir = workspace("detect");
    let image = ApfsImage::build();
    let source = image_file(&dir, &image);

    let result = recovery_engine::scan_image(&source, false).expect("scan succeeds");

    assert!(
        result.partitions.iter().any(|p| p.filesystem == "APFS"),
        "the scan must report an APFS filesystem, got {:?}",
        result
            .partitions
            .iter()
            .map(|p| &p.filesystem)
            .collect::<Vec<_>>()
    );
    // Every file in the fixture should surface, including the deleted one.
    assert_eq!(
        result.candidates.len(),
        image.files.len(),
        "expected one candidate per fixture file"
    );
}

#[test]
fn recovers_apfs_files_to_a_destination_and_the_bytes_match() {
    let dir = workspace("recover");
    let image = ApfsImage::build();
    let source = image_file(&dir, &image);
    let destination = dir.join("recovered");

    let scan = recovery_engine::scan_image(&source, false).expect("scan succeeds");
    let selected: Vec<String> = scan.candidates.iter().map(|c| c.id.clone()).collect();
    assert!(!selected.is_empty(), "nothing was found to recover");

    let outcome = recovery_engine::recover(&source, &destination, &selected, false)
        .expect("recovery succeeds");
    assert!(outcome.written > 0, "no files were written");

    // Every recovered file must match the source byte for byte.
    let mut verified = 0usize;
    for file in &image.files {
        let mut found = None;
        for entry in walk(&destination) {
            let name = entry.file_name().unwrap().to_string_lossy().to_string();
            // Deleted files are named from their inode, active ones by name.
            let matches = if file.linked {
                name.starts_with(file.name.split('.').next().unwrap())
            } else {
                name.contains(&file.inode_id.to_string())
            };
            if matches {
                found = Some(entry);
                break;
            }
        }
        let Some(path) = found else { continue };
        let written = std::fs::read(&path).expect("recovered file is readable");
        assert_eq!(
            written, file.contents,
            "{} was not recovered byte for byte",
            file.name
        );
        verified += 1;
    }
    assert!(
        verified >= 2,
        "expected to verify at least the two active files, verified {verified}"
    );

    // The source must be untouched by the recovery.
    assert_eq!(
        std::fs::read(&source).unwrap(),
        image.bytes,
        "the source image was modified during recovery"
    );
}

#[test]
fn recovery_refuses_to_write_onto_the_source() {
    let dir = workspace("guard");
    let image = ApfsImage::build();
    let source = image_file(&dir, &image);
    let scan = recovery_engine::scan_image(&source, false).unwrap();
    let selected: Vec<String> = scan.candidates.iter().map(|c| c.id.clone()).collect();
    // Recovering into the directory holding the source must be refused.
    assert!(
        recovery_engine::recover(&source, &dir, &selected, false).is_err(),
        "writing into the source's own directory must be blocked"
    );
}

fn walk(root: &std::path::Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                out.push(path);
            }
        }
    }
    out
}
