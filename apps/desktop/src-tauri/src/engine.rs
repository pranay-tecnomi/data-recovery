//! Wires the recovery crates into the operations the UI performs.
//!
//! This module owns no parsing logic of its own; it orchestrates the engine
//! crates and converts their types into serialisable views for the front end.

use std::path::{Path, PathBuf};

use candidate_pipeline::run as run_pipeline;
use file_carving::{carve, CarveLimits, REGISTRY};
use filesystem_probe::{probe, FilesystemKind};
use partition_discovery::{discover_gpt, discover_mbr, DiskGeometry};
use recovery_core::{
    ByteRange, CancellationToken, CandidateId, Completeness, Confidence, FileCandidate,
    Origin, RecoveryError, Validation,
};
use recovery_output::{
    build_manifest, recover_all, validate_destination, CollisionPolicy, ItemOutcome,
};
use serde::Serialize;
use storage_io::{BlockDevice, FileImageDevice};

/// Assumed sector size for image sources, which carry no geometry of their own.
const DEFAULT_SECTOR_SIZE: u64 = 512;

/// A partition as presented to the UI.
#[derive(Debug, Serialize)]
pub struct PartitionView {
    pub index: usize,
    pub offset: u64,
    pub length: u64,
    pub filesystem: String,
    /// Structural evidence supporting the classification.
    pub evidence: Vec<String>,
}

/// A recoverable file as presented to the UI.
#[derive(Debug, Serialize)]
pub struct CandidateView {
    pub id: String,
    pub name: String,
    pub path: String,
    pub origin: String,
    pub size: u64,
    pub recovered_size: u64,
    pub confidence: String,
    pub validation: String,
    pub completeness: String,
    /// Reasons behind the classification, so the UI can explain rather than
    /// assert a score.
    pub evidence: Vec<EvidenceView>,
}

#[derive(Debug, Serialize)]
pub struct EvidenceView {
    pub detail: String,
    pub supporting: bool,
}

/// Summary of a completed scan.
#[derive(Debug, Serialize)]
pub struct ScanResult {
    pub source: String,
    pub capacity: u64,
    pub partitions: Vec<PartitionView>,
    pub candidates: Vec<CandidateView>,
    /// Non-fatal findings worth showing the user.
    pub diagnostics: Vec<String>,
}

/// Outcome of a recovery job.
#[derive(Debug, Serialize)]
pub struct RecoveryResultView {
    pub written: usize,
    pub partial: usize,
    pub skipped: usize,
    pub failed: usize,
    pub destination: String,
    pub manifest_path: String,
}

fn confidence_label(confidence: Confidence) -> &'static str {
    match confidence {
        Confidence::High => "high",
        Confidence::Medium => "medium",
        Confidence::Low => "low",
        Confidence::Unknown => "unknown",
    }
}

fn validation_label(validation: Validation) -> &'static str {
    match validation {
        Validation::Valid => "valid",
        Validation::PartiallyValid => "partial",
        Validation::Invalid => "invalid",
        Validation::Indeterminate => "indeterminate",
        Validation::NotAttempted => "unchecked",
    }
}

fn completeness_label(completeness: Completeness) -> &'static str {
    match completeness {
        Completeness::Complete => "complete",
        Completeness::Partial => "partial",
        Completeness::MetadataOnly => "metadata only",
    }
}

fn origin_label(origin: Origin) -> &'static str {
    match origin {
        Origin::ActiveFilesystem => "file record",
        Origin::DeletedFilesystem => "deleted record",
        Origin::Carved => "carved",
    }
}

fn view(candidate: &FileCandidate) -> CandidateView {
    CandidateView {
        id: candidate.id.as_str().to_string(),
        name: candidate.name.clone(),
        path: candidate.display_path(),
        origin: origin_label(candidate.origin).to_string(),
        size: candidate.declared_size,
        recovered_size: candidate.recovered_size().unwrap_or(0),
        confidence: confidence_label(candidate.confidence()).to_string(),
        validation: validation_label(candidate.validation).to_string(),
        completeness: completeness_label(candidate.completeness).to_string(),
        evidence: candidate
            .evidence
            .iter()
            .map(|e| EvidenceView { detail: e.detail.clone(), supporting: e.supporting })
            .collect(),
    }
}

/// Enumerates partitions, preferring GPT and falling back to MBR.
fn discover_partitions<D: BlockDevice>(
    device: &D,
    diagnostics: &mut Vec<String>,
) -> Vec<ByteRange> {
    let Ok(geometry) = DiskGeometry::new(DEFAULT_SECTOR_SIZE) else {
        return Vec::new();
    };

    if let Ok(result) = discover_gpt(device, geometry)
        && !result.partitions.is_empty()
    {
        return result.partitions.iter().map(|p| p.range).collect();
    }
    match discover_mbr(device, geometry) {
        Ok(result) if !result.partitions.is_empty() => {
            result.partitions.iter().map(|p| p.range).collect()
        }
        _ => {
            // A volume image with no partition table is scanned whole.
            diagnostics
                .push("No partition table found; treating the source as a single volume.".into());
            ByteRange::new(0, device.capacity()).into_iter().collect()
        }
    }
}

/// Collects candidates from a FAT32 volume.
fn scan_fat32<D: BlockDevice>(
    device: &D,
    range: ByteRange,
    diagnostics: &mut Vec<String>,
) -> Vec<FileCandidate> {
    let Ok(volume) = fat32_recovery::parse_volume(device, range) else {
        return Vec::new();
    };
    let Ok(entries) = fat32_recovery::read_root_entries(device, range, true) else {
        diagnostics.push("FAT32 root directory could not be read.".into());
        return Vec::new();
    };

    let mut out = Vec::new();
    for entry in &entries {
        // Directories are traversed, not streamed.
        if entry.attributes & 0x10 != 0 {
            continue;
        }
        if entry.deleted {
            if let Ok(candidate) = fat32_recovery::deleted_candidate(device, &volume, range, entry) {
                out.push(FileCandidate {
                    id: CandidateId::new(format!("fat32-del-{}", out.len())),
                    name: candidate.long_name.unwrap_or(candidate.short_name),
                    path: Vec::new(),
                    origin: Origin::DeletedFilesystem,
                    extents: candidate.extents,
                    declared_size: candidate.declared_size,
                    completeness: match candidate.state {
                        fat32_recovery::ExtentState::Recoverable => Completeness::Complete,
                        fat32_recovery::ExtentState::PartiallyRecoverable => Completeness::Partial,
                        fat32_recovery::ExtentState::MetadataOnly => Completeness::MetadataOnly,
                    },
                    validation: Validation::NotAttempted,
                    evidence: candidate
                        .evidence
                        .into_iter()
                        .map(recovery_core::Evidence::supporting)
                        .collect(),
                });
            }
        } else if let Ok(extents) = fat32_recovery::file_extents(device, &volume, range, entry) {
            out.push(FileCandidate {
                id: CandidateId::new(format!("fat32-{}", out.len())),
                name: entry.name().to_string(),
                path: Vec::new(),
                origin: Origin::ActiveFilesystem,
                extents: extents.extents,
                declared_size: extents.declared_size,
                completeness: match extents.state {
                    fat32_recovery::ExtentState::Recoverable => Completeness::Complete,
                    fat32_recovery::ExtentState::PartiallyRecoverable => Completeness::Partial,
                    fat32_recovery::ExtentState::MetadataOnly => Completeness::MetadataOnly,
                },
                validation: Validation::NotAttempted,
                evidence: Vec::new(),
            });
        }
    }
    out
}

/// Collects candidates from an exFAT volume.
fn scan_exfat<D: BlockDevice>(
    device: &D,
    range: ByteRange,
    diagnostics: &mut Vec<String>,
) -> Vec<FileCandidate> {
    let Ok(volume) = exfat_recovery::parse_volume(device, range) else {
        return Vec::new();
    };
    let mut rejected = Vec::new();
    let Ok(entries) = exfat_recovery::read_directory(
        device,
        &volume,
        range,
        volume.root_directory_cluster,
        true,
        &mut rejected,
    ) else {
        diagnostics.push("exFAT root directory could not be read.".into());
        return Vec::new();
    };
    if !rejected.is_empty() {
        diagnostics.push(format!(
            "{} exFAT entry set(s) failed validation and were not trusted.",
            rejected.len()
        ));
    }

    let mut out = Vec::new();
    for entry in &entries {
        if entry.is_directory() {
            continue;
        }
        if entry.deleted {
            if let Ok(candidate) =
                exfat_recovery::deleted_candidate(&volume, range, None, entry)
            {
                out.push(FileCandidate {
                    id: CandidateId::new(format!("exfat-del-{}", out.len())),
                    name: candidate.name,
                    path: Vec::new(),
                    origin: Origin::DeletedFilesystem,
                    extents: candidate.extents,
                    declared_size: candidate.declared_size,
                    completeness: match candidate.state {
                        exfat_recovery::ExtentState::Recoverable => Completeness::Complete,
                        exfat_recovery::ExtentState::PartiallyRecoverable => Completeness::Partial,
                        exfat_recovery::ExtentState::MetadataOnly => Completeness::MetadataOnly,
                    },
                    validation: Validation::NotAttempted,
                    evidence: candidate
                        .evidence
                        .into_iter()
                        .map(recovery_core::Evidence::supporting)
                        .collect(),
                });
            }
        } else if let Ok(extents) = exfat_recovery::stream_extents(device, &volume, range, entry) {
            out.push(FileCandidate {
                id: CandidateId::new(format!("exfat-{}", out.len())),
                name: entry.name.clone(),
                path: Vec::new(),
                origin: Origin::ActiveFilesystem,
                extents: extents.extents,
                declared_size: extents.declared_size,
                completeness: match extents.state {
                    exfat_recovery::ExtentState::Recoverable => Completeness::Complete,
                    exfat_recovery::ExtentState::PartiallyRecoverable => Completeness::Partial,
                    exfat_recovery::ExtentState::MetadataOnly => Completeness::MetadataOnly,
                },
                validation: Validation::NotAttempted,
                evidence: Vec::new(),
            });
        }
    }
    out
}

/// Scans an image file: partitions, filesystems, carving, then scoring.
pub fn scan_image(path: &Path, include_carving: bool) -> Result<ScanResult, RecoveryError> {
    let device = FileImageDevice::open(path)?;
    let cancel = CancellationToken::default();
    let mut diagnostics = Vec::new();
    let mut partitions = Vec::new();
    let mut candidates: Vec<FileCandidate> = Vec::new();

    for (index, range) in discover_partitions(&device, &mut diagnostics)
        .into_iter()
        .enumerate()
    {
        let evidence = probe(&device, range).ok();
        let kind = evidence.as_ref().map(|e| e.kind).unwrap_or(FilesystemKind::Unknown);

        partitions.push(PartitionView {
            index,
            offset: range.offset,
            length: range.length,
            filesystem: match kind {
                FilesystemKind::Fat32 => "FAT32",
                FilesystemKind::ExFat => "exFAT",
                FilesystemKind::Unknown => "unrecognised",
            }
            .to_string(),
            evidence: evidence
                .map(|e| e.notes.iter().map(|n| n.to_string()).collect())
                .unwrap_or_default(),
        });

        match kind {
            FilesystemKind::Fat32 => {
                candidates.extend(scan_fat32(&device, range, &mut diagnostics))
            }
            FilesystemKind::ExFat => {
                candidates.extend(scan_exfat(&device, range, &mut diagnostics))
            }
            FilesystemKind::Unknown => diagnostics.push(format!(
                "Partition {index} holds no filesystem this build can read; carving may still find files."
            )),
        }
    }

    if include_carving
        && let Ok(whole) = ByteRange::new(0, device.capacity())
        && let Ok(carved) = carve(&device, whole, REGISTRY, &CarveLimits::default(), &cancel)
    {
        candidates.extend(carved);
    }

    // Scoring, deduplication and validation all happen here.
    let candidates = run_pipeline(&device, candidates, &cancel)?;

    Ok(ScanResult {
        source: path.display().to_string(),
        capacity: device.capacity(),
        partitions,
        candidates: candidates.iter().map(view).collect(),
        diagnostics,
    })
}

/// Recovers the selected candidates to a destination directory.
///
/// The destination is validated against the source before anything is written.
pub fn recover(
    source: &Path,
    destination: &Path,
    selected: &[String],
    include_carving: bool,
) -> Result<RecoveryResultView, RecoveryError> {
    let device = FileImageDevice::open(source)?;
    let cancel = CancellationToken::default();

    // The mandatory safety gate: never write onto the source.
    let safe = validate_destination(destination, Some(source))
        .map_err(RecoveryError::from)?;
    std::fs::create_dir_all(safe.path())
        .map_err(|e| RecoveryError::IoFailure(e.to_string()))?;

    // Rebuild the candidate set so ids match what the UI selected from.
    let mut diagnostics = Vec::new();
    let mut candidates: Vec<FileCandidate> = Vec::new();
    for range in discover_partitions(&device, &mut diagnostics) {
        match probe(&device, range).map(|e| e.kind) {
            Ok(FilesystemKind::Fat32) => {
                candidates.extend(scan_fat32(&device, range, &mut diagnostics))
            }
            Ok(FilesystemKind::ExFat) => {
                candidates.extend(scan_exfat(&device, range, &mut diagnostics))
            }
            _ => {}
        }
    }
    if include_carving
        && let Ok(whole) = ByteRange::new(0, device.capacity())
        && let Ok(carved) = carve(&device, whole, REGISTRY, &CarveLimits::default(), &cancel)
    {
        candidates.extend(carved);
    }
    let candidates = run_pipeline(&device, candidates, &cancel)?;

    let chosen: Vec<FileCandidate> = candidates
        .into_iter()
        .filter(|c| selected.iter().any(|id| id == c.id.as_str()))
        .collect();

    let results = recover_all(&device, &safe, &chosen, CollisionPolicy::Rename, &cancel)?;

    let (mut written, mut partial, mut skipped, mut failed) = (0, 0, 0, 0);
    let mut manifest_entries = Vec::new();
    for (candidate, result) in chosen.iter().zip(results.iter()) {
        match result {
            Ok(item) => {
                match item.outcome {
                    ItemOutcome::Written { .. } => written += 1,
                    ItemOutcome::WrittenPartial { .. } => partial += 1,
                    ItemOutcome::Skipped { .. } => skipped += 1,
                    ItemOutcome::NoContent => failed += 1,
                }
                manifest_entries.push((candidate, item));
            }
            Err(_) => failed += 1,
        }
    }

    let manifest_path: PathBuf = safe.path().join("recovery-manifest.jsonl");
    std::fs::write(&manifest_path, build_manifest(&manifest_entries))
        .map_err(|e| RecoveryError::IoFailure(e.to_string()))?;

    Ok(RecoveryResultView {
        written,
        partial,
        skipped,
        failed,
        destination: safe.path().display().to_string(),
        manifest_path: manifest_path.display().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_cover_every_variant() {
        // Every enum value must render, or the UI shows nothing.
        for c in [Confidence::High, Confidence::Medium, Confidence::Low, Confidence::Unknown] {
            assert!(!confidence_label(c).is_empty());
        }
        for v in [
            Validation::Valid,
            Validation::PartiallyValid,
            Validation::Invalid,
            Validation::Indeterminate,
            Validation::NotAttempted,
        ] {
            assert!(!validation_label(v).is_empty());
        }
        for c in [Completeness::Complete, Completeness::Partial, Completeness::MetadataOnly] {
            assert!(!completeness_label(c).is_empty());
        }
        for o in [Origin::ActiveFilesystem, Origin::DeletedFilesystem, Origin::Carved] {
            assert!(!origin_label(o).is_empty());
        }
    }

    #[test]
    fn recovery_refuses_to_write_onto_the_source() {
        let dir = std::env::temp_dir().join(format!("dr-ui-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let image = dir.join("disk.img");
        std::fs::write(&image, vec![0u8; 4096]).unwrap();

        // The gate must reject the source itself as a destination.
        assert!(recover(&image, &image, &[], false).is_err());
        // And a directory containing it.
        assert!(recover(&image, &dir, &[], false).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn scanning_a_non_image_fails_cleanly() {
        let dir = std::env::temp_dir().join(format!("dr-ui-junk-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let junk = dir.join("notes.txt");
        std::fs::write(&junk, b"this is not a disk image").unwrap();

        // Unreadable sources must not panic; a scan simply finds nothing.
        let result = scan_image(&junk, true);
        if let Ok(scan) = result {
            assert!(scan.candidates.is_empty());
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

#[cfg(test)]
mod smoke {
    use super::*;

    /// Drives the exact path the UI's scan command uses.
    #[test]
    fn scans_a_generated_image_and_recovers_from_it() {
        let image = std::path::PathBuf::from("/tmp/dr-demo/sample.img");
        if !image.exists() {
            // The sample is generated by the make-image binary; skip without it.
            return;
        }
        let scan = scan_image(&image, true).expect("scan failed");
        assert!(!scan.partitions.is_empty(), "no partitions discovered");
        assert_eq!(scan.partitions[0].filesystem, "FAT32");
        assert!(!scan.candidates.is_empty(), "no candidates found");

        // The active JPEG should validate and rank highest.
        let photo = scan
            .candidates
            .iter()
            .find(|c| c.name == "PHOTO.JPG")
            .expect("active photo missing");
        assert_eq!(photo.validation, "valid");
        assert_eq!(photo.confidence, "high");

        // The mislabelled file's content contradicts its name.
        let fake = scan.candidates.iter().find(|c| c.name == "FAKE.JPG");
        if let Some(fake) = fake {
            assert_eq!(fake.validation, "invalid");
            assert_eq!(fake.confidence, "unknown");
        }

        // The deleted entry keeps its lost first character marked.
        assert!(
            scan.candidates.iter().any(|c| c.name.starts_with('?')),
            "deleted candidate should mark its lost character"
        );

        // Recover the photo through the same command path the UI uses.
        let out = std::env::temp_dir().join(format!("dr-smoke-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&out);
        let result = recover(&image, &out, &[photo.id.clone()], true).expect("recovery failed");
        assert_eq!(result.written, 1, "expected one recovered file");
        assert!(std::path::Path::new(&result.manifest_path).exists());
        let _ = std::fs::remove_dir_all(&out);
    }
}
