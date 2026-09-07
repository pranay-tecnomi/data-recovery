//! The FAT32 workflow driven through the engine exactly as the app performs
//! it, complementing `end_to_end.rs`, which exercises the crates directly.

use std::path::PathBuf;

use integration_tests::Fat32Image;

fn workspace(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fat32-workflow-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("workspace is creatable");
    dir
}

struct Fixture {
    bytes: Vec<u8>,
    active: Vec<u8>,
    deleted: Vec<u8>,
}

fn fixture() -> Fixture {
    let mut image = Fat32Image::build();

    // Sized so the last cluster is only partly used, which is where a failure
    // to truncate to the declared size would show up.
    let active: Vec<u8> = (0..3000).map(|i| (i % 251) as u8).collect();
    let deleted: Vec<u8> = (0..700).map(|i| (i % 97) as u8).collect();

    // Clusters are one 512-byte sector, so a 3000-byte file needs a six-link
    // chain. Writing only the first cluster would silently truncate it.
    const CLUSTER: usize = 512;
    let first = 3u32;
    let clusters: Vec<u32> = (0..active.len().div_ceil(CLUSTER) as u32)
        .map(|i| first + i)
        .collect();
    for (i, cluster) in clusters.iter().enumerate() {
        let start = i * CLUSTER;
        let end = (start + CLUSTER).min(active.len());
        image.put_cluster_data(*cluster, &active[start..end]);
        let next = clusters.get(i + 1).copied().unwrap_or(0x0FFF_FFFF);
        image.set_fat(*cluster, next);
    }
    image.put_root_entry(
        0,
        &Fat32Image::dir_entry(b"REPORT  BIN", first, active.len() as u32, false),
    );

    // A deleted entry keeps its metadata; its FAT chain has been released.
    let deleted_first = first + clusters.len() as u32;
    for (i, chunk) in deleted.chunks(CLUSTER).enumerate() {
        image.put_cluster_data(deleted_first + i as u32, chunk);
    }
    image.put_root_entry(
        1,
        &Fat32Image::dir_entry(b"ERASED  BIN", deleted_first, deleted.len() as u32, true),
    );

    Fixture {
        bytes: image.into_device().snapshot(),
        active,
        deleted,
    }
}

fn image_file(dir: &std::path::Path, f: &Fixture) -> PathBuf {
    let path = dir.join("source.img");
    std::fs::write(&path, &f.bytes).expect("image is writable");
    path
}

#[test]
fn detects_fat32_and_finds_active_and_deleted_files() {
    let dir = workspace("detect");
    let f = fixture();
    let source = image_file(&dir, &f);

    let scan = recovery_engine::scan_image(&source, false).expect("scan succeeds");
    assert!(
        scan.partitions.iter().any(|p| p.filesystem == "FAT32"),
        "expected a FAT32 partition, got {:?}",
        scan.partitions
            .iter()
            .map(|p| &p.filesystem)
            .collect::<Vec<_>>()
    );

    let names: Vec<String> = scan
        .candidates
        .iter()
        .map(|c| c.name.to_uppercase())
        .collect();
    assert!(
        names.iter().any(|n| n.starts_with("REPORT")),
        "active file missing: {names:?}"
    );
    // FAT32 overwrites the first character of a deleted name on disk, so the
    // recovered name carries a placeholder for it. What matters is that the
    // entry was found from filesystem metadata rather than by carving.
    assert!(
        names.len() >= 2,
        "the deleted entry must still be reported: {names:?}"
    );
}

#[test]
fn recovers_fat32_files_byte_for_byte() {
    let dir = workspace("recover");
    let f = fixture();
    let source = image_file(&dir, &f);
    let destination = dir.join("out");

    let scan = recovery_engine::scan_image(&source, false).unwrap();
    let selected: Vec<String> = scan.candidates.iter().map(|c| c.id.clone()).collect();
    let outcome = recovery_engine::recover(&source, &destination, &selected, false)
        .expect("recovery succeeds");
    assert!(outcome.written > 0, "nothing was written");

    // The active file's chain is intact, so it must come back exactly.
    let path = find(&destination, "REPORT").expect("the active file was not written");
    let written = std::fs::read(&path).unwrap();
    assert_eq!(
        written.len(),
        f.active.len(),
        "the active file was recovered at the wrong length"
    );
    assert!(
        written == f.active,
        "the active file was not recovered byte for byte"
    );

    // The deleted file keeps its metadata but its FAT chain was released, so
    // the engine can only trust the first cluster. Whatever it does write must
    // still be a true prefix of the original - never invented bytes.
    if let Some(path) = find(&destination, "RASED") {
        let recovered = std::fs::read(&path).unwrap();
        assert!(
            !recovered.is_empty() && f.deleted.starts_with(&recovered),
            "recovered deleted content must be a prefix of the original"
        );
    }

    assert_eq!(std::fs::read(&source).unwrap(), f.bytes, "source modified");
}

fn find(root: &std::path::Path, stem: &str) -> Option<PathBuf> {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).ok()?.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path
                .file_name()
                .map(|n| n.to_string_lossy().to_uppercase().contains(stem))
                .unwrap_or(false)
            {
                return Some(path);
            }
        }
    }
    None
}
