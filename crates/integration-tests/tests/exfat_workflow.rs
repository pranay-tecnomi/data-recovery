//! End-to-end exFAT recovery through the real engine: detect, scan, recover
//! to a destination, and verify the written bytes against the known source.

use std::path::PathBuf;

use exfat_recovery::testimage::{CLUSTER, Mem, entry_set, image};

fn workspace(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("exfat-workflow-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("workspace is creatable");
    dir
}

/// An exFAT volume holding one active file spanning several clusters and one
/// deleted file, both with known contents.
struct Fixture {
    image: Mem,
    active: Vec<u8>,
    deleted: Vec<u8>,
}

fn fixture() -> Fixture {
    // Deliberately not a whole number of clusters, so the final cluster is
    // only partly used and the recovered file must be truncated to its size.
    let active: Vec<u8> = (0..CLUSTER * 2 + 137).map(|i| (i % 251) as u8).collect();
    let deleted: Vec<u8> = (0..CLUSTER + 9).map(|i| (i % 97) as u8).collect();

    let mut m = image();
    m.write_file(10, &active);
    m.write_file(20, &deleted);
    // Entry sets must be contiguous: a zero entry marks the end of the
    // directory, so a gap would hide everything after it.
    let active_set = entry_set("report.bin", 10, active.len() as u64, 0, false);
    let deleted_set = entry_set("erased.bin", 20, deleted.len() as u64, 0, true);
    m.write_set(0, &active_set);
    m.write_set(active_set.len(), &deleted_set);

    Fixture {
        image: m,
        active,
        deleted,
    }
}

fn image_file(dir: &std::path::Path, fixture: &Fixture) -> PathBuf {
    let path = dir.join("source.img");
    std::fs::write(&path, fixture.image.bytes()).expect("image is writable");
    path
}

#[test]
fn detects_exfat_and_finds_active_and_deleted_files() {
    let dir = workspace("detect");
    let f = fixture();
    let source = image_file(&dir, &f);

    let scan = recovery_engine::scan_image(&source, false).expect("scan succeeds");
    assert!(
        scan.partitions.iter().any(|p| p.filesystem == "exFAT"),
        "expected an exFAT partition, got {:?}",
        scan.partitions
            .iter()
            .map(|p| &p.filesystem)
            .collect::<Vec<_>>()
    );

    let names: Vec<&str> = scan.candidates.iter().map(|c| c.name.as_str()).collect();
    assert!(
        names.contains(&"report.bin"),
        "active file missing: {names:?}"
    );
    assert!(
        names.contains(&"erased.bin"),
        "deleted file must be found from its directory entry, not by carving: {names:?}"
    );
}

#[test]
fn recovers_exfat_files_byte_for_byte() {
    let dir = workspace("recover");
    let f = fixture();
    let source = image_file(&dir, &f);
    let destination = dir.join("out");

    let scan = recovery_engine::scan_image(&source, false).unwrap();
    let selected: Vec<String> = scan.candidates.iter().map(|c| c.id.clone()).collect();
    let outcome = recovery_engine::recover(&source, &destination, &selected, false)
        .expect("recovery succeeds");
    assert!(outcome.written > 0, "nothing was written");

    let expected = [("report.bin", &f.active), ("erased.bin", &f.deleted)];
    for (name, contents) in expected {
        let path = find(&destination, name);
        let Some(path) = path else {
            panic!("{name} was not written to the destination");
        };
        assert_eq!(
            &std::fs::read(&path).unwrap(),
            contents,
            "{name} was not recovered byte for byte"
        );
    }

    // The source must be untouched.
    assert_eq!(std::fs::read(&source).unwrap(), f.image.bytes());
}

/// Finds a recovered file by name, allowing for collision-renaming suffixes.
fn find(root: &std::path::Path, name: &str) -> Option<PathBuf> {
    let stem = name.split('.').next().unwrap();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).ok()?.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path
                .file_name()
                .map(|n| n.to_string_lossy().starts_with(stem))
                .unwrap_or(false)
            {
                return Some(path);
            }
        }
    }
    None
}
