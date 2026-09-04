//! Streaming recovery output with atomic finalisation.
//!
//! Files are streamed from source extents through a bounded buffer, written to
//! a temporary name, and only then renamed into place. A partially recovered
//! file is labelled so it can never be mistaken for a complete one.

use std::{
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
};

use platform_device::sanitize_component;
use recovery_core::{
    ByteRange, CancellationToken, Completeness, FileCandidate, RecoveryError, RecoveryResult,
};
use storage_io::BlockDevice;

use crate::destination::SafeDestination;

/// Copy buffer size. Bounded so a large file cannot exhaust memory.
const COPY_BUFFER: usize = 1 << 20;

/// Marker appended to files whose content is known to be incomplete.
const PARTIAL_SUFFIX: &str = ".partial";

/// How to handle a name that already exists at the destination.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CollisionPolicy {
    /// Append a numeric suffix: `photo.jpg`, `photo (2).jpg`.
    Rename,
    /// Leave the existing file and skip the candidate.
    Skip,
}

/// What happened to one candidate.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ItemOutcome {
    /// Written in full.
    Written { path: PathBuf, bytes: u64 },
    /// Written, but the content is known to be incomplete.
    WrittenPartial { path: PathBuf, bytes: u64, declared: u64 },
    /// Skipped because a file already existed and the policy said to skip.
    Skipped { path: PathBuf },
    /// Nothing was written because the candidate had no locatable content.
    NoContent,
}

/// The result of recovering one candidate, for the manifest.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoveredItem {
    pub candidate_id: String,
    pub outcome: ItemOutcome,
}

/// Builds the on-disk name for a candidate.
///
/// Names come from untrusted metadata, so each path component is sanitised.
/// A candidate whose content is incomplete is marked, since a truncated file
/// that looks complete is worse than one that announces itself.
fn output_path(destination: &Path, candidate: &FileCandidate) -> PathBuf {
    let mut path = destination.to_path_buf();
    for component in &candidate.path {
        path.push(sanitize_component(component).name);
    }
    let mut name = sanitize_component(&candidate.name).name;
    if candidate.completeness != Completeness::Complete {
        name.push_str(PARTIAL_SUFFIX);
    }
    path.push(name);
    path
}

/// Resolves a collision deterministically, so repeated runs agree.
fn resolve_collision(path: &Path, policy: CollisionPolicy) -> Option<PathBuf> {
    if !path.exists() {
        return Some(path.to_path_buf());
    }
    match policy {
        CollisionPolicy::Skip => None,
        CollisionPolicy::Rename => {
            let parent = path.parent()?;
            let stem = path.file_stem()?.to_string_lossy().to_string();
            let extension = path
                .extension()
                .map(|e| format!(".{}", e.to_string_lossy()))
                .unwrap_or_default();
            // Bounded so a directory full of collisions cannot spin forever.
            for index in 2..10_000u32 {
                let candidate = parent.join(format!("{stem} ({index}){extension}"));
                if !candidate.exists() {
                    return Some(candidate);
                }
            }
            None
        }
    }
}

/// Streams one candidate's extents to `writer`, returning bytes written.
///
/// Cancellation is checked per chunk, so a large file stays interruptible.
/// A short read ends the copy: the bytes written so far are real, and the
/// shortfall is reported rather than padded.
fn stream_candidate<D: BlockDevice, W: Write>(
    device: &D,
    candidate: &FileCandidate,
    writer: &mut W,
    cancel: &CancellationToken,
) -> RecoveryResult<u64> {
    let mut buffer = vec![0u8; COPY_BUFFER];
    let mut total: u64 = 0;

    for extent in &candidate.extents {
        let mut offset = extent.source_range.offset;
        let mut remaining = extent.source_range.length;
        while remaining > 0 {
            cancel.check()?;
            let take = remaining.min(COPY_BUFFER as u64);
            let length = usize::try_from(take).map_err(|_| {
                RecoveryError::LengthTooLarge { length: take }
            })?;
            let range = ByteRange::new(offset, take)?;
            range.validate_within(device.capacity())?;

            let read = device.read(range, &mut buffer[..length])?;
            if read == 0 {
                // No progress is possible; stop rather than loop.
                return Ok(total);
            }
            writer
                .write_all(&buffer[..read])
                .map_err(|e| RecoveryError::IoFailure(e.to_string()))?;
            total = total
                .checked_add(read as u64)
                .ok_or(RecoveryError::RangeOverflow)?;
            offset = offset
                .checked_add(read as u64)
                .ok_or(RecoveryError::RangeOverflow)?;
            remaining -= read as u64;

            // A short read means the extent could not be fully satisfied.
            if read < length {
                return Ok(total);
            }
        }
    }
    Ok(total)
}

/// Recovers one candidate to the destination.
///
/// The file is written under a temporary name and renamed into place only
/// after a successful flush, so an interrupted run never leaves a
/// partially-written file under its final name.
pub fn recover_candidate<D: BlockDevice>(
    device: &D,
    destination: &SafeDestination,
    candidate: &FileCandidate,
    policy: CollisionPolicy,
    cancel: &CancellationToken,
) -> RecoveryResult<RecoveredItem> {
    let outcome = recover_one(device, destination, candidate, policy, cancel)?;
    Ok(RecoveredItem {
        candidate_id: candidate.id.as_str().to_string(),
        outcome,
    })
}

fn recover_one<D: BlockDevice>(
    device: &D,
    destination: &SafeDestination,
    candidate: &FileCandidate,
    policy: CollisionPolicy,
    cancel: &CancellationToken,
) -> RecoveryResult<ItemOutcome> {
    if candidate.extents.is_empty() {
        return Ok(ItemOutcome::NoContent);
    }

    let target = output_path(destination.path(), candidate);
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|e| RecoveryError::IoFailure(e.to_string()))?;
    }
    let Some(final_path) = resolve_collision(&target, policy) else {
        return Ok(ItemOutcome::Skipped { path: target });
    };

    // Write to a temporary name in the same directory, so the rename is atomic.
    let temp_path = final_path.with_extension(format!(
        "{}partial-tmp",
        final_path
            .extension()
            .map(|e| format!("{}.", e.to_string_lossy()))
            .unwrap_or_default()
    ));

    let bytes = {
        let mut file =
            File::create(&temp_path).map_err(|e| RecoveryError::IoFailure(e.to_string()))?;
        let result = stream_candidate(device, candidate, &mut file, cancel);
        match result {
            Ok(bytes) => {
                file.flush().map_err(|e| RecoveryError::IoFailure(e.to_string()))?;
                // Durability before the rename makes the finalisation meaningful.
                file.sync_all().map_err(|e| RecoveryError::IoFailure(e.to_string()))?;
                bytes
            }
            Err(error) => {
                // Never leave a temporary file behind on failure or cancellation.
                drop(file);
                let _ = fs::remove_file(&temp_path);
                return Err(error);
            }
        }
    };

    fs::rename(&temp_path, &final_path).map_err(|e| {
        let _ = fs::remove_file(&temp_path);
        RecoveryError::IoFailure(e.to_string())
    })?;

    // Content that fell short of the declared size is reported as partial even
    // when the candidate claimed to be complete.
    if candidate.completeness != Completeness::Complete || bytes < candidate.declared_size {
        Ok(ItemOutcome::WrittenPartial {
            path: final_path,
            bytes,
            declared: candidate.declared_size,
        })
    } else {
        Ok(ItemOutcome::Written { path: final_path, bytes })
    }
}

/// Recovers many candidates, collecting per-item outcomes.
///
/// One candidate's failure does not abort the job: the error is recorded and
/// the run continues, since partial recovery is the point. Cancellation does
/// stop the run, because the user asked for it.
pub fn recover_all<D: BlockDevice>(
    device: &D,
    destination: &SafeDestination,
    candidates: &[FileCandidate],
    policy: CollisionPolicy,
    cancel: &CancellationToken,
) -> RecoveryResult<Vec<Result<RecoveredItem, (String, RecoveryError)>>> {
    let mut results = Vec::new();
    for candidate in candidates {
        cancel.check()?;
        match recover_candidate(device, destination, candidate, policy, cancel) {
            Ok(item) => results.push(Ok(item)),
            Err(RecoveryError::Cancelled) => return Err(RecoveryError::Cancelled),
            Err(error) => {
                results.push(Err((candidate.id.as_str().to_string(), error)));
            }
        }
    }
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::destination::validate_destination;
    use recovery_core::{CandidateId, Extent, Origin, Validation};
    use std::sync::atomic::{AtomicU64, Ordering};

    static ID: AtomicU64 = AtomicU64::new(0);

    struct Mem(Vec<u8>);
    impl BlockDevice for Mem {
        fn capacity(&self) -> u64 {
            self.0.len() as u64
        }
        fn read(&self, r: ByteRange, o: &mut [u8]) -> RecoveryResult<usize> {
            r.validate_within(self.capacity())?;
            let n = usize::try_from(r.length).unwrap();
            let start = usize::try_from(r.offset).unwrap();
            o[..n].copy_from_slice(&self.0[start..start + n]);
            Ok(n)
        }
    }

    fn workspace(name: &str) -> PathBuf {
        let id = ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "data-recovery-out-{}-{}-{}",
            std::process::id(),
            id,
            name
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn device() -> Mem {
        // "ABCDEFGHIJ" repeated, so content is easy to assert.
        Mem((0..4096u32).map(|i| b'A' + (i % 10) as u8).collect())
    }

    fn candidate(name: &str, offset: u64, size: u64, completeness: Completeness) -> FileCandidate {
        FileCandidate {
            id: CandidateId::new(name),
            name: name.into(),
            path: Vec::new(),
            origin: Origin::ActiveFilesystem,
            extents: vec![Extent::new(ByteRange::new(offset, size).unwrap(), 0).unwrap()],
            declared_size: size,
            completeness,
            validation: Validation::Valid,
            evidence: Vec::new(),
        }
    }

    fn safe(root: &Path) -> SafeDestination {
        validate_destination(root, None).unwrap()
    }

    #[test]
    fn writes_a_complete_file() {
        let root = workspace("write");
        let c = candidate("hello.txt", 0, 10, Completeness::Complete);
        let item = recover_candidate(
            &device(),
            &safe(&root),
            &c,
            CollisionPolicy::Rename,
            &CancellationToken::default(),
        )
        .unwrap();

        match item.outcome {
            ItemOutcome::Written { path, bytes } => {
                assert_eq!(bytes, 10);
                assert_eq!(fs::read(&path).unwrap(), b"ABCDEFGHIJ");
            }
            other => panic!("expected a complete write, got {other:?}"),
        }
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn reassembles_fragmented_extents_in_logical_order() {
        let root = workspace("fragmented");
        let mut c = candidate("frag.bin", 0, 5, Completeness::Complete);
        // Second extent comes earlier on disk but later in the file.
        c.extents = vec![
            Extent::new(ByteRange::new(10, 5).unwrap(), 0).unwrap(),
            Extent::new(ByteRange::new(0, 5).unwrap(), 5).unwrap(),
        ];
        c.declared_size = 10;

        let item = recover_candidate(
            &device(),
            &safe(&root),
            &c,
            CollisionPolicy::Rename,
            &CancellationToken::default(),
        )
        .unwrap();
        match item.outcome {
            ItemOutcome::Written { path, .. } => {
                // Logical order, not disk order.
                assert_eq!(fs::read(&path).unwrap(), b"ABCDEABCDE");
            }
            other => panic!("expected a write, got {other:?}"),
        }
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn partial_candidates_are_labelled() {
        let root = workspace("partial");
        let mut c = candidate("photo.jpg", 0, 10, Completeness::Partial);
        c.declared_size = 100;

        let item = recover_candidate(
            &device(),
            &safe(&root),
            &c,
            CollisionPolicy::Rename,
            &CancellationToken::default(),
        )
        .unwrap();
        match item.outcome {
            ItemOutcome::WrittenPartial { path, bytes, declared } => {
                assert_eq!((bytes, declared), (10, 100));
                // A truncated file must announce itself.
                assert!(path.to_string_lossy().ends_with(".partial"));
            }
            other => panic!("expected a partial write, got {other:?}"),
        }
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn creates_nested_directories_from_the_candidate_path() {
        let root = workspace("nested");
        let mut c = candidate("inner.txt", 0, 5, Completeness::Complete);
        c.path = vec!["DCIM".into(), "100APPLE".into()];

        let item = recover_candidate(
            &device(),
            &safe(&root),
            &c,
            CollisionPolicy::Rename,
            &CancellationToken::default(),
        )
        .unwrap();
        match item.outcome {
            ItemOutcome::Written { path, .. } => {
                assert!(path.ends_with("DCIM/100APPLE/inner.txt"), "{path:?}");
            }
            other => panic!("expected a write, got {other:?}"),
        }
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn hostile_names_cannot_escape_the_destination() {
        let root = workspace("escape");
        let mut c = candidate("../../escaped.txt", 0, 5, Completeness::Complete);
        c.path = vec!["..".into()];

        let dest = safe(&root);
        let item = recover_candidate(
            &device(),
            &dest,
            &c,
            CollisionPolicy::Rename,
            &CancellationToken::default(),
        )
        .unwrap();
        match item.outcome {
            ItemOutcome::Written { path, .. } => {
                // Compare against the canonicalised root the writer used.
                assert!(path.starts_with(dest.path()), "{path:?} escaped {:?}", dest.path());
                // The traversal was neutralised rather than honoured.
                assert!(!path.to_string_lossy().contains("/../"));
            }
            other => panic!("expected a write, got {other:?}"),
        }
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn collisions_are_renamed_deterministically() {
        let root = workspace("collide");
        let c = candidate("dup.txt", 0, 5, Completeness::Complete);
        let d = device();
        let dest = safe(&root);
        let token = CancellationToken::default();

        for expected in ["dup.txt", "dup (2).txt", "dup (3).txt"] {
            let item =
                recover_candidate(&d, &dest, &c, CollisionPolicy::Rename, &token).unwrap();
            match item.outcome {
                ItemOutcome::Written { path, .. } => {
                    assert!(path.ends_with(expected), "{path:?} != {expected}");
                }
                other => panic!("expected a write, got {other:?}"),
            }
        }
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn skip_policy_leaves_the_existing_file_untouched() {
        let root = workspace("skip");
        let c = candidate("keep.txt", 0, 5, Completeness::Complete);
        let d = device();
        let dest = safe(&root);
        let token = CancellationToken::default();

        recover_candidate(&d, &dest, &c, CollisionPolicy::Skip, &token).unwrap();
        let existing = root.join("keep.txt");
        fs::write(&existing, b"ORIGINAL").unwrap();

        let item = recover_candidate(&d, &dest, &c, CollisionPolicy::Skip, &token).unwrap();
        assert!(matches!(item.outcome, ItemOutcome::Skipped { .. }));
        assert_eq!(fs::read(&existing).unwrap(), b"ORIGINAL");
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn candidates_without_content_write_nothing() {
        let root = workspace("empty");
        let mut c = candidate("gone.txt", 0, 0, Completeness::MetadataOnly);
        c.extents.clear();

        let item = recover_candidate(
            &device(),
            &safe(&root),
            &c,
            CollisionPolicy::Rename,
            &CancellationToken::default(),
        )
        .unwrap();
        assert_eq!(item.outcome, ItemOutcome::NoContent);
        assert!(fs::read_dir(&root).unwrap().next().is_none());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn cancellation_leaves_no_temporary_file() {
        let root = workspace("cancel");
        let c = candidate("big.bin", 0, 4096, Completeness::Complete);
        let token = CancellationToken::default();
        token.cancel();

        let result = recover_candidate(
            &device(),
            &safe(&root),
            &c,
            CollisionPolicy::Rename,
            &token,
        );
        assert!(result.is_err());
        // An interrupted write must not leave debris behind.
        assert!(
            fs::read_dir(&root).unwrap().next().is_none(),
            "temporary file survived cancellation"
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn extents_beyond_the_device_fail_without_a_partial_file() {
        let root = workspace("oob");
        let c = candidate("bad.bin", 4000, 1000, Completeness::Complete);

        let result = recover_candidate(
            &device(),
            &safe(&root),
            &c,
            CollisionPolicy::Rename,
            &CancellationToken::default(),
        );
        assert!(result.is_err());
        assert!(fs::read_dir(&root).unwrap().next().is_none());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn recover_all_continues_past_a_failing_candidate() {
        let root = workspace("batch");
        let good = candidate("good.txt", 0, 10, Completeness::Complete);
        let bad = candidate("bad.txt", 4000, 1000, Completeness::Complete);
        let also_good = candidate("also.txt", 10, 10, Completeness::Complete);

        let results = recover_all(
            &device(),
            &safe(&root),
            &[good, bad, also_good],
            CollisionPolicy::Rename,
            &CancellationToken::default(),
        )
        .unwrap();

        assert_eq!(results.len(), 3);
        assert!(results[0].is_ok());
        // One failure does not abort the job.
        assert!(results[1].is_err());
        assert!(results[2].is_ok());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn recover_all_stops_on_cancellation() {
        let root = workspace("batch-cancel");
        let token = CancellationToken::default();
        token.cancel();
        let result = recover_all(
            &device(),
            &safe(&root),
            &[candidate("a.txt", 0, 10, Completeness::Complete)],
            CollisionPolicy::Rename,
            &token,
        );
        assert!(result.is_err());
        fs::remove_dir_all(&root).unwrap();
    }
}
