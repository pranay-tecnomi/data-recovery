//! Full-pipeline integration tests.
//!
//! These run the real flow a scan performs: discover partitions, probe the
//! filesystem, enumerate files, score candidates, and write them out. They
//! exist to catch assumptions packets make about each other that unit tests
//! cannot see.

use candidate_pipeline::run as run_pipeline;
use fat32_recovery::{deleted_candidate, file_extents, parse_volume, read_root_entries};
use filesystem_probe::{probe, FilesystemKind};
use integration_tests::fixture::{Fat32Image, MemoryDevice, SECTOR};
use partition_discovery::{discover_mbr, DiskGeometry};
use recovery_core::{
    ByteRange, CancellationToken, CandidateId, Completeness, Confidence, Extent, FileCandidate,
    Origin, Validation,
};
use recovery_output::{
    build_manifest, recover_all, validate_destination, CollisionPolicy, ItemOutcome,
};
use std::{fs, path::PathBuf, sync::atomic::{AtomicU64, Ordering}};

static ID: AtomicU64 = AtomicU64::new(0);

fn workspace(name: &str) -> PathBuf {
    let id = ID.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "data-recovery-e2e-{}-{}-{}",
        std::process::id(),
        id,
        name
    ));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).unwrap();
    path
}

/// A JPEG small enough to sit in one cluster, with a valid marker chain.
fn jpeg(body: usize) -> Vec<u8> {
    let mut v = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10];
    v.extend_from_slice(b"JFIF\0");
    v.resize(18, 0);
    v.extend(std::iter::repeat_n(0x41u8, body));
    v.extend_from_slice(&[0xFF, 0xD9]);
    v
}

/// An image holding one active JPEG and one deleted text file.
fn populated_image() -> (MemoryDevice, Vec<u8>) {
    let mut image = Fat32Image::build();
    let photo = jpeg(200);

    // Active file: PHOTO.JPG in cluster 3.
    image.set_fat(3, 0x0FFF_FFFF);
    image.put_cluster_data(3, &photo);
    image.put_root_entry(
        0,
        &Fat32Image::dir_entry(b"PHOTO   JPG", 3, photo.len() as u32, false),
    );

    // Deleted file: NOTES.TXT in cluster 5, whose FAT entry was released.
    let notes = b"deleted notes content";
    image.put_cluster_data(5, notes);
    image.put_root_entry(
        1,
        &Fat32Image::dir_entry(b"NOTES   TXT", 5, notes.len() as u32, true),
    );

    (image.into_device(), photo)
}

#[test]
fn discovers_the_partition_and_identifies_the_filesystem() {
    let (device, _) = populated_image();
    let geometry = DiskGeometry::new(SECTOR as u64).unwrap();

    let discovery = discover_mbr(&device, geometry).unwrap();
    assert_eq!(discovery.partitions.len(), 1, "expected one MBR partition");

    let partition = &discovery.partitions[0];
    // The probe must classify from structure, not from the MBR type byte.
    let evidence = probe(&device, partition.range).unwrap();
    assert_eq!(evidence.kind, FilesystemKind::Fat32);
    assert!(evidence.confidence > 0);
}

#[test]
fn recovers_an_active_file_end_to_end() {
    let root = workspace("active");
    let (device, photo) = populated_image();
    let geometry = DiskGeometry::new(SECTOR as u64).unwrap();
    let partition = discover_mbr(&device, geometry).unwrap().partitions[0].range;

    // Enumerate, then resolve extents for the active file.
    let volume = parse_volume(&device, partition).unwrap();
    let entries = read_root_entries(&device, partition, false).unwrap();
    let entry = entries
        .iter()
        .find(|e| e.short_name == "PHOTO.JPG")
        .expect("active file not enumerated");
    let extents = file_extents(&device, &volume, partition, entry).unwrap();

    let candidate = FileCandidate {
        id: CandidateId::new("photo"),
        name: entry.short_name.clone(),
        path: Vec::new(),
        origin: Origin::ActiveFilesystem,
        extents: extents.extents,
        declared_size: extents.declared_size,
        completeness: Completeness::Complete,
        validation: Validation::NotAttempted,
        evidence: Vec::new(),
    };

    // Score, then write out.
    let scored = run_pipeline(&device, vec![candidate], &CancellationToken::default()).unwrap();
    assert_eq!(scored[0].validation, Validation::Valid, "JPEG should validate");
    assert_eq!(
        scored[0].confidence(),
        Confidence::High,
        "an active, validated, complete file should reach High"
    );

    let destination = validate_destination(&root, None).unwrap();
    let results = recover_all(
        &device,
        &destination,
        &scored,
        CollisionPolicy::Rename,
        &CancellationToken::default(),
    )
    .unwrap();

    match &results[0].as_ref().unwrap().outcome {
        ItemOutcome::Written { path, bytes } => {
            assert_eq!(*bytes, photo.len() as u64);
            // The recovered bytes must match the source exactly.
            assert_eq!(fs::read(path).unwrap(), photo);
        }
        other => panic!("expected a complete write, got {other:?}"),
    }
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn deleted_file_is_recovered_with_honest_confidence() {
    let (device, _) = populated_image();
    let geometry = DiskGeometry::new(SECTOR as u64).unwrap();
    let partition = discover_mbr(&device, geometry).unwrap().partitions[0].range;

    let volume = parse_volume(&device, partition).unwrap();
    let entries = read_root_entries(&device, partition, true).unwrap();
    let deleted = entries.iter().find(|e| e.deleted).expect("deleted entry not found");

    let recovered = deleted_candidate(&device, &volume, partition, deleted).unwrap();
    // The tombstone destroyed the first character; it must not be invented.
    assert!(recovered.short_name.starts_with('?'));
    // Deletion released the chain, so contiguity is inferred, never certain.
    assert_ne!(recovered.confidence, fat32_recovery::Confidence::High);
    assert!(!recovered.evidence.is_empty());
}

#[test]
fn scanning_never_modifies_the_source() {
    let (device, _) = populated_image();
    let before = device.snapshot();
    let geometry = DiskGeometry::new(SECTOR as u64).unwrap();

    // Run the whole read path.
    let partition = discover_mbr(&device, geometry).unwrap().partitions[0].range;
    let _ = probe(&device, partition).unwrap();
    let volume = parse_volume(&device, partition).unwrap();
    let entries = read_root_entries(&device, partition, true).unwrap();
    for entry in &entries {
        if !entry.deleted && entry.attributes & 0x10 == 0 {
            let _ = file_extents(&device, &volume, partition, entry);
        }
    }
    let _ = file_carving::carve(
        &device,
        partition,
        file_carving::REGISTRY,
        &file_carving::CarveLimits::default(),
        &CancellationToken::default(),
    )
    .unwrap();

    // P0 acceptance: no source write is possible through the API.
    assert_eq!(device.snapshot(), before, "the source was modified during a scan");
}

#[test]
fn carving_finds_content_the_filesystem_also_reports() {
    let (device, photo) = populated_image();
    let geometry = DiskGeometry::new(SECTOR as u64).unwrap();
    let partition = discover_mbr(&device, geometry).unwrap().partitions[0].range;

    let carved = file_carving::carve(
        &device,
        partition,
        file_carving::REGISTRY,
        &file_carving::CarveLimits::default(),
        &CancellationToken::default(),
    )
    .unwrap();

    let found = carved
        .iter()
        .find(|c| c.extents[0].source_range.length == photo.len() as u64)
        .expect("carver missed the JPEG the filesystem records");
    assert_eq!(found.origin, Origin::Carved);
}

#[test]
fn a_carved_duplicate_defers_to_the_filesystem_record() {
    let root = workspace("dedupe");
    let (device, photo) = populated_image();
    let geometry = DiskGeometry::new(SECTOR as u64).unwrap();
    let partition = discover_mbr(&device, geometry).unwrap().partitions[0].range;

    let volume = parse_volume(&device, partition).unwrap();
    let entries = read_root_entries(&device, partition, false).unwrap();
    let entry = entries.iter().find(|e| e.short_name == "PHOTO.JPG").unwrap();
    let extents = file_extents(&device, &volume, partition, entry).unwrap();
    let offset = extents.extents[0].source_range.offset;

    let from_filesystem = FileCandidate {
        id: CandidateId::new("fs"),
        name: "PHOTO.JPG".into(),
        path: Vec::new(),
        origin: Origin::ActiveFilesystem,
        extents: extents.extents,
        declared_size: photo.len() as u64,
        completeness: Completeness::Complete,
        validation: Validation::NotAttempted,
        evidence: Vec::new(),
    };
    let from_carver = FileCandidate {
        id: CandidateId::new("carved"),
        name: "carved.jpg".into(),
        path: vec!["carved".into()],
        origin: Origin::Carved,
        extents: vec![
            Extent::new(ByteRange::new(offset, photo.len() as u64).unwrap(), 0).unwrap(),
        ],
        declared_size: photo.len() as u64,
        completeness: Completeness::Complete,
        validation: Validation::NotAttempted,
        evidence: Vec::new(),
    };

    let scored = run_pipeline(
        &device,
        vec![from_carver, from_filesystem],
        &CancellationToken::default(),
    )
    .unwrap();

    // The same bytes found twice collapse to the better-evidenced account.
    assert_eq!(scored.len(), 1);
    assert_eq!(scored[0].origin, Origin::ActiveFilesystem);
    assert_eq!(scored[0].name, "PHOTO.JPG");
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn recovery_is_blocked_when_the_destination_is_the_source() {
    let root = workspace("conflict");
    let source_image = root.join("disk.img");
    fs::write(&source_image, b"image bytes").unwrap();

    // Writing recovered files onto the source would destroy the evidence.
    assert!(validate_destination(&source_image, Some(&source_image)).is_err());
    assert!(validate_destination(&root, Some(&source_image)).is_err());
    // A separate directory is fine.
    assert!(validate_destination(&root.join("out"), Some(&source_image)).is_ok());
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn the_manifest_records_what_was_recovered() {
    let root = workspace("manifest");
    let (device, photo) = populated_image();
    let geometry = DiskGeometry::new(SECTOR as u64).unwrap();
    let partition = discover_mbr(&device, geometry).unwrap().partitions[0].range;

    let volume = parse_volume(&device, partition).unwrap();
    let entries = read_root_entries(&device, partition, false).unwrap();
    let entry = entries.iter().find(|e| e.short_name == "PHOTO.JPG").unwrap();
    let extents = file_extents(&device, &volume, partition, entry).unwrap();

    let candidate = FileCandidate {
        id: CandidateId::new("photo"),
        name: "PHOTO.JPG".into(),
        path: Vec::new(),
        origin: Origin::ActiveFilesystem,
        extents: extents.extents,
        declared_size: photo.len() as u64,
        completeness: Completeness::Complete,
        validation: Validation::NotAttempted,
        evidence: Vec::new(),
    };
    let scored = run_pipeline(&device, vec![candidate], &CancellationToken::default()).unwrap();
    let destination = validate_destination(&root, None).unwrap();
    let results = recover_all(
        &device,
        &destination,
        &scored,
        CollisionPolicy::Rename,
        &CancellationToken::default(),
    )
    .unwrap();

    let item = results[0].as_ref().unwrap();
    let manifest = build_manifest(&[(&scored[0], item)]);
    assert!(manifest.contains(r#""status":"written""#));
    assert!(manifest.contains(r#""origin":"ActiveFilesystem""#));
    assert!(manifest.contains(r#""confidence":"high""#));
    assert!(manifest.contains(r#""validation":"valid""#));
    // One record per line, so a truncated manifest still parses.
    assert_eq!(manifest.lines().count(), 1);
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_truncated_image_does_not_panic() {
    let (device, _) = populated_image();
    let full = device.snapshot();
    let geometry = DiskGeometry::new(SECTOR as u64).unwrap();

    // Cut the image at many points; every stage must fail gracefully.
    for fraction in [1usize, 2, 4, 8, 16, 100] {
        let truncated = MemoryDevice::new(full[..full.len() / fraction].to_vec());
        let Ok(discovery) = discover_mbr(&truncated, geometry) else {
            continue;
        };
        for partition in &discovery.partitions {
            let _ = probe(&truncated, partition.range);
            if let Ok(volume) = parse_volume(&truncated, partition.range)
                && let Ok(entries) = read_root_entries(&truncated, partition.range, true)
            {
                for entry in &entries {
                    if entry.attributes & 0x10 == 0 {
                        let _ = file_extents(&truncated, &volume, partition.range, entry);
                    }
                }
            }
        }
    }
}

#[test]
fn corrupted_metadata_does_not_panic() {
    let (device, _) = populated_image();
    let full = device.snapshot();
    let geometry = DiskGeometry::new(SECTOR as u64).unwrap();

    let mut reached_entries = 0usize;
    // Flip bytes across the metadata region: MBR, boot sector, FAT, root dir.
    for offset in (0..(2048 + 32 + 1100) * SECTOR).step_by(4093) {
        let mut bytes = full.clone();
        bytes[offset] ^= 0xFF;
        let corrupted = MemoryDevice::new(bytes);

        let Ok(discovery) = discover_mbr(&corrupted, geometry) else {
            continue;
        };
        for partition in &discovery.partitions {
            let _ = probe(&corrupted, partition.range);
            let Ok(volume) = parse_volume(&corrupted, partition.range) else {
                continue;
            };
            let Ok(entries) = read_root_entries(&corrupted, partition.range, true) else {
                continue;
            };
            for entry in &entries {
                if entry.attributes & 0x10 == 0 {
                    reached_entries += 1;
                    let _ = file_extents(&corrupted, &volume, partition.range, entry);
                    if entry.deleted {
                        let _ = deleted_candidate(&corrupted, &volume, partition.range, entry);
                    }
                }
            }
        }
    }
    // The sweep must actually reach the deeper parsers, not bail at the MBR.
    assert!(reached_entries > 100, "corruption sweep only reached {reached_entries} entries");
}

#[test]
fn cancellation_stops_the_pipeline_and_the_carver() {
    let (device, _) = populated_image();
    let geometry = DiskGeometry::new(SECTOR as u64).unwrap();
    let partition = discover_mbr(&device, geometry).unwrap().partitions[0].range;

    let token = CancellationToken::default();
    token.cancel();

    assert!(file_carving::carve(
        &device,
        partition,
        file_carving::REGISTRY,
        &file_carving::CarveLimits::default(),
        &token,
    )
    .is_err());

    let candidate = FileCandidate {
        id: CandidateId::new("c"),
        name: "x.jpg".into(),
        path: Vec::new(),
        origin: Origin::ActiveFilesystem,
        extents: vec![Extent::new(ByteRange::new(0, 16).unwrap(), 0).unwrap()],
        declared_size: 16,
        completeness: Completeness::Complete,
        validation: Validation::NotAttempted,
        evidence: Vec::new(),
    };
    assert!(run_pipeline(&device, vec![candidate], &token).is_err());
}
