//! Session persistence and resume.
//!
//! A checkpoint records enough state to resume a scan safely. Resume is
//! refused unless the schema, source fingerprint and capacity all match, so a
//! session can never be replayed against different media. Invalid checkpoints
//! fail closed: a partially decoded state is never resumed from.

use std::{fs, path::{Path, PathBuf}};

use recovery_core::{ByteRange, RecoveryError, RecoveryResult};

/// Bumped whenever the persisted layout changes. Migrations are forward-only.
pub const SCHEMA_VERSION: u32 = 1;

/// Refuses to parse a checkpoint larger than this.
const MAX_CHECKPOINT_BYTES: u64 = 16 * 1024 * 1024;

/// Evidence binding a session to the media it was created against.
///
/// Identity is structural, never a path: a drive letter or /dev node can be
/// reassigned between runs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceFingerprint {
    pub capacity: u64,
    pub logical_sector_size: u64,
    /// Digest of a sample of the source's content, so different media with the
    /// same geometry are still distinguishable.
    pub content_digest: u64,
}

impl SourceFingerprint {
    /// Whether `other` describes the same source.
    pub fn matches(&self, other: &Self) -> bool {
        self == other
    }
}

/// Why a checkpoint could not be resumed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResumeRejection {
    /// The file was written by an incompatible schema version.
    SchemaMismatch { found: u32, expected: u32 },
    /// The source is not the one the session was created against.
    SourceMismatch,
    /// The checkpoint could not be decoded.
    Corrupt(String),
}

impl From<ResumeRejection> for RecoveryError {
    fn from(value: ResumeRejection) -> Self {
        RecoveryError::IoFailure(match value {
            ResumeRejection::SchemaMismatch { found, expected } => {
                format!("checkpoint schema {found} is not compatible with {expected}")
            }
            ResumeRejection::SourceMismatch => {
                "checkpoint was created against a different source".into()
            }
            ResumeRejection::Corrupt(detail) => format!("checkpoint is corrupt: {detail}"),
        })
    }
}

/// A resumable scan checkpoint.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Checkpoint {
    pub schema_version: u32,
    pub session_id: String,
    pub fingerprint: SourceFingerprint,
    /// Monotonic counter, so the newer of two checkpoints is identifiable.
    pub generation: u64,
    /// Ranges fully scanned. Normalised: sorted and non-overlapping.
    pub completed: Vec<ByteRange>,
    /// Ranges that could not be read and will not be retried.
    pub unreadable: Vec<ByteRange>,
}

impl Checkpoint {
    pub fn new(session_id: impl Into<String>, fingerprint: SourceFingerprint) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            session_id: session_id.into(),
            fingerprint,
            generation: 0,
            completed: Vec::new(),
            unreadable: Vec::new(),
        }
    }

    /// Merges sorted ranges, coalescing those that touch or overlap, so the
    /// accounting stays normalised and non-overlapping.
    fn normalize(ranges: &mut Vec<ByteRange>) {
        ranges.sort_by_key(|r| r.offset);
        let mut merged: Vec<ByteRange> = Vec::new();
        for range in ranges.iter() {
            let Ok(end) = range.end() else { continue };
            match merged.last_mut() {
                Some(last) if range.offset <= last.end().unwrap_or(u64::MAX) => {
                    let last_end = last.end().unwrap_or(0).max(end);
                    if let Ok(extended) = ByteRange::new(last.offset, last_end - last.offset) {
                        *last = extended;
                    }
                }
                _ => merged.push(*range),
            }
        }
        *ranges = merged;
    }

    /// Records a completed range and re-normalises the accounting.
    pub fn complete(&mut self, range: ByteRange) {
        self.completed.push(range);
        Self::normalize(&mut self.completed);
        self.generation += 1;
    }

    /// Records a range that is permanently unreadable.
    pub fn mark_unreadable(&mut self, range: ByteRange) {
        self.unreadable.push(range);
        Self::normalize(&mut self.unreadable);
        self.generation += 1;
    }

    /// Ranges still to scan, given the source capacity.
    pub fn remaining(&self, capacity: u64) -> Vec<ByteRange> {
        // Accounted ranges are those already done or known bad.
        let mut accounted: Vec<ByteRange> =
            self.completed.iter().chain(self.unreadable.iter()).copied().collect();
        Self::normalize(&mut accounted);

        let mut gaps = Vec::new();
        let mut cursor = 0u64;
        for range in accounted {
            if range.offset > cursor
                && let Ok(gap) = ByteRange::new(cursor, range.offset - cursor)
            {
                gaps.push(gap);
            }
            cursor = cursor.max(range.end().unwrap_or(cursor));
        }
        if cursor < capacity
            && let Ok(gap) = ByteRange::new(cursor, capacity - cursor)
        {
            gaps.push(gap);
        }
        gaps
    }

    /// Serialises the checkpoint. Fields are fixed-order so parsing needs no
    /// general JSON reader.
    pub fn encode(&self) -> String {
        let ranges = |list: &[ByteRange]| {
            list.iter()
                .map(|r| format!("{}:{}", r.offset, r.length))
                .collect::<Vec<_>>()
                .join(",")
        };
        format!(
            "schema={}\nsession={}\ncapacity={}\nsector={}\ndigest={}\ngeneration={}\ncompleted={}\nunreadable={}\n",
            self.schema_version,
            self.session_id,
            self.fingerprint.capacity,
            self.fingerprint.logical_sector_size,
            self.fingerprint.content_digest,
            self.generation,
            ranges(&self.completed),
            ranges(&self.unreadable),
        )
    }

    /// Parses a checkpoint, failing closed on anything unexpected.
    pub fn decode(text: &str) -> Result<Self, ResumeRejection> {
        let field = |name: &str| -> Result<String, ResumeRejection> {
            text.lines()
                .find_map(|line| line.strip_prefix(&format!("{name}=")))
                .map(|v| v.to_string())
                .ok_or_else(|| ResumeRejection::Corrupt(format!("missing field {name}")))
        };
        let number = |name: &str| -> Result<u64, ResumeRejection> {
            field(name)?
                .parse::<u64>()
                .map_err(|_| ResumeRejection::Corrupt(format!("field {name} is not a number")))
        };

        let schema = u32::try_from(number("schema")?)
            .map_err(|_| ResumeRejection::Corrupt("schema is out of range".into()))?;
        // Version is checked before anything else is trusted.
        if schema != SCHEMA_VERSION {
            return Err(ResumeRejection::SchemaMismatch {
                found: schema,
                expected: SCHEMA_VERSION,
            });
        }

        let parse_ranges = |raw: &str| -> Result<Vec<ByteRange>, ResumeRejection> {
            if raw.is_empty() {
                return Ok(Vec::new());
            }
            raw.split(',')
                .map(|part| {
                    let (offset, length) = part.split_once(':').ok_or_else(|| {
                        ResumeRejection::Corrupt("malformed range entry".into())
                    })?;
                    let offset = offset.parse::<u64>().map_err(|_| {
                        ResumeRejection::Corrupt("range offset is not a number".into())
                    })?;
                    let length = length.parse::<u64>().map_err(|_| {
                        ResumeRejection::Corrupt("range length is not a number".into())
                    })?;
                    ByteRange::new(offset, length)
                        .map_err(|_| ResumeRejection::Corrupt("range overflows".into()))
                })
                .collect()
        };

        let checkpoint = Self {
            schema_version: schema,
            session_id: field("session")?,
            fingerprint: SourceFingerprint {
                capacity: number("capacity")?,
                logical_sector_size: number("sector")?,
                content_digest: number("digest")?,
            },
            generation: number("generation")?,
            completed: parse_ranges(&field("completed")?)?,
            unreadable: parse_ranges(&field("unreadable")?)?,
        };

        // Ranges must lie within the source the checkpoint describes.
        for range in checkpoint.completed.iter().chain(checkpoint.unreadable.iter()) {
            if range.validate_within(checkpoint.fingerprint.capacity).is_err() {
                return Err(ResumeRejection::Corrupt(
                    "a recorded range lies outside the source capacity".into(),
                ));
            }
        }
        Ok(checkpoint)
    }

    /// Writes the checkpoint atomically: temporary file, then rename, so an
    /// interrupted save never replaces a good checkpoint with a partial one.
    pub fn save(&self, path: &Path) -> RecoveryResult<()> {
        let temp = path.with_extension("tmp");
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| RecoveryError::IoFailure(e.to_string()))?;
        }
        fs::write(&temp, self.encode()).map_err(|e| RecoveryError::IoFailure(e.to_string()))?;
        fs::rename(&temp, path).map_err(|e| {
            let _ = fs::remove_file(&temp);
            RecoveryError::IoFailure(e.to_string())
        })
    }

    /// Loads a checkpoint and verifies it describes `current`.
    pub fn load(path: &Path, current: &SourceFingerprint) -> Result<Self, ResumeRejection> {
        let size = fs::metadata(path)
            .map_err(|e| ResumeRejection::Corrupt(e.to_string()))?
            .len();
        // A corrupted length must not drive an unbounded read.
        if size > MAX_CHECKPOINT_BYTES {
            return Err(ResumeRejection::Corrupt("checkpoint is implausibly large".into()));
        }
        let text = fs::read_to_string(path)
            .map_err(|e| ResumeRejection::Corrupt(e.to_string()))?;
        let checkpoint = Self::decode(&text)?;
        // Resuming against different media would produce nonsense results.
        if !checkpoint.fingerprint.matches(current) {
            return Err(ResumeRejection::SourceMismatch);
        }
        Ok(checkpoint)
    }
}

/// Default checkpoint location inside a destination directory.
pub fn checkpoint_path(destination: &Path, session_id: &str) -> PathBuf {
    destination.join(format!(".recovery-session-{session_id}.checkpoint"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static ID: AtomicU64 = AtomicU64::new(0);

    fn fingerprint() -> SourceFingerprint {
        SourceFingerprint { capacity: 1 << 20, logical_sector_size: 512, content_digest: 0xABCD }
    }

    fn workspace(name: &str) -> PathBuf {
        let id = ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "data-recovery-session-{}-{}-{}",
            std::process::id(), id, name
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn range(offset: u64, length: u64) -> ByteRange {
        ByteRange::new(offset, length).unwrap()
    }

    #[test]
    fn round_trips_through_encoding() {
        let mut c = Checkpoint::new("s1", fingerprint());
        c.complete(range(0, 4096));
        c.mark_unreadable(range(8192, 512));
        let decoded = Checkpoint::decode(&c.encode()).unwrap();
        assert_eq!(decoded, c);
    }

    #[test]
    fn normalises_overlapping_completed_ranges() {
        let mut c = Checkpoint::new("s1", fingerprint());
        c.complete(range(0, 1000));
        c.complete(range(500, 1000));
        // Overlapping ranges coalesce into one.
        assert_eq!(c.completed.len(), 1);
        assert_eq!(c.completed[0].length, 1500);
    }

    #[test]
    fn coalesces_adjacent_ranges() {
        let mut c = Checkpoint::new("s1", fingerprint());
        c.complete(range(0, 512));
        c.complete(range(512, 512));
        assert_eq!(c.completed.len(), 1);
        assert_eq!(c.completed[0].length, 1024);
    }

    #[test]
    fn ranges_recorded_out_of_order_are_sorted() {
        let mut c = Checkpoint::new("s1", fingerprint());
        c.complete(range(4096, 512));
        c.complete(range(0, 512));
        assert_eq!(c.completed[0].offset, 0);
        assert_eq!(c.completed[1].offset, 4096);
    }

    #[test]
    fn reports_remaining_work() {
        let mut c = Checkpoint::new("s1", fingerprint());
        c.complete(range(0, 4096));
        let remaining = c.remaining(8192);
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].offset, 4096);
        assert_eq!(remaining[0].length, 4096);
    }

    #[test]
    fn unreadable_ranges_are_not_rescanned() {
        let mut c = Checkpoint::new("s1", fingerprint());
        c.complete(range(0, 1000));
        c.mark_unreadable(range(1000, 1000));
        // A permanently bad range counts as accounted for.
        let remaining = c.remaining(3000);
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].offset, 2000);
    }

    #[test]
    fn fully_scanned_source_has_no_remaining_work() {
        let mut c = Checkpoint::new("s1", fingerprint());
        c.complete(range(0, 8192));
        assert!(c.remaining(8192).is_empty());
    }

    #[test]
    fn generation_advances_with_each_update() {
        let mut c = Checkpoint::new("s1", fingerprint());
        assert_eq!(c.generation, 0);
        c.complete(range(0, 512));
        assert_eq!(c.generation, 1);
        c.mark_unreadable(range(512, 512));
        assert_eq!(c.generation, 2);
    }

    #[test]
    fn saves_and_loads_a_checkpoint() {
        let root = workspace("roundtrip");
        let path = checkpoint_path(&root, "s1");
        let mut c = Checkpoint::new("s1", fingerprint());
        c.complete(range(0, 4096));
        c.save(&path).unwrap();

        let loaded = Checkpoint::load(&path, &fingerprint()).unwrap();
        assert_eq!(loaded, c);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn refuses_to_resume_against_a_different_source() {
        let root = workspace("mismatch");
        let path = checkpoint_path(&root, "s1");
        Checkpoint::new("s1", fingerprint()).save(&path).unwrap();

        // Same geometry, different content: not the same media.
        let other = SourceFingerprint { content_digest: 0x9999, ..fingerprint() };
        assert_eq!(
            Checkpoint::load(&path, &other),
            Err(ResumeRejection::SourceMismatch)
        );

        // Different capacity is also a mismatch.
        let resized = SourceFingerprint { capacity: 999, ..fingerprint() };
        assert_eq!(
            Checkpoint::load(&path, &resized),
            Err(ResumeRejection::SourceMismatch)
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn refuses_an_incompatible_schema() {
        let text = Checkpoint::new("s1", fingerprint()).encode().replace("schema=1", "schema=99");
        assert_eq!(
            Checkpoint::decode(&text),
            Err(ResumeRejection::SchemaMismatch { found: 99, expected: SCHEMA_VERSION })
        );
    }

    #[test]
    fn corrupt_checkpoints_fail_closed() {
        // Truncated, garbage, and partially valid input must all be refused.
        for text in ["", "schema=1\n", "not a checkpoint at all", "schema=x\n"] {
            assert!(
                Checkpoint::decode(text).is_err(),
                "input {text:?} must not decode"
            );
        }
    }

    #[test]
    fn rejects_malformed_range_entries() {
        let base = Checkpoint::new("s1", fingerprint()).encode();
        for bad in ["completed=abc", "completed=1:", "completed=1:2:3", "completed=:5"] {
            let text = base.replace("completed=", &format!("{bad}\nignored="));
            assert!(Checkpoint::decode(&text).is_err(), "{bad} must not decode");
        }
    }

    #[test]
    fn rejects_ranges_outside_the_recorded_capacity() {
        let mut c = Checkpoint::new("s1", fingerprint());
        c.completed.push(range(1 << 30, 512));
        // A range beyond the source cannot be trusted to describe it.
        assert!(matches!(
            Checkpoint::decode(&c.encode()),
            Err(ResumeRejection::Corrupt(_))
        ));
    }

    #[test]
    fn an_interrupted_save_leaves_the_previous_checkpoint_intact() {
        let root = workspace("atomic");
        let path = checkpoint_path(&root, "s1");
        let mut first = Checkpoint::new("s1", fingerprint());
        first.complete(range(0, 4096));
        first.save(&path).unwrap();

        // A stray temporary file must not be mistaken for the checkpoint.
        fs::write(path.with_extension("tmp"), "garbage").unwrap();
        let loaded = Checkpoint::load(&path, &fingerprint()).unwrap();
        assert_eq!(loaded.completed[0].length, 4096);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn saving_replaces_an_earlier_generation() {
        let root = workspace("replace");
        let path = checkpoint_path(&root, "s1");
        let mut c = Checkpoint::new("s1", fingerprint());
        c.complete(range(0, 512));
        c.save(&path).unwrap();
        c.complete(range(512, 512));
        c.save(&path).unwrap();

        let loaded = Checkpoint::load(&path, &fingerprint()).unwrap();
        assert_eq!(loaded.generation, 2);
        assert_eq!(loaded.completed[0].length, 1024);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn missing_checkpoints_are_reported_not_panicked() {
        let root = workspace("missing");
        let result = Checkpoint::load(&checkpoint_path(&root, "absent"), &fingerprint());
        assert!(matches!(result, Err(ResumeRejection::Corrupt(_))));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn fingerprints_compare_structurally() {
        assert!(fingerprint().matches(&fingerprint()));
        assert!(!fingerprint().matches(&SourceFingerprint { capacity: 1, ..fingerprint() }));
    }
}
