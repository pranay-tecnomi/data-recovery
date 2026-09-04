//! The cross-cutting candidate model shared by every recovery source.
//!
//! Filesystem modules and carvers all emit `FileCandidate`. Confidence is
//! aggregated here, from evidence, so a score is reproducible for identical
//! evidence rather than asserted by whichever module happened to build it.

use crate::{extent, ByteRange, CandidateId, Extent, RecoveryResult};

/// Where a candidate came from. Validation modifies confidence but never
/// erases provenance: a valid carved file and a metadata-recovered file retain
/// different evidence histories.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Origin {
    /// Recovered from an intact, in-use filesystem record.
    ActiveFilesystem,
    /// Recovered from a deleted filesystem record.
    DeletedFilesystem,
    /// Found by signature scanning with no filesystem record.
    Carved,
}

/// How much of the candidate's content the extents represent.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Completeness {
    /// Extents cover the declared size.
    Complete,
    /// Extents cover part of the declared size.
    Partial,
    /// A record survives but no content could be located.
    MetadataOnly,
}

/// Outcome of running a format validator over a candidate's bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Validation {
    /// Structure parsed cleanly end to end.
    Valid,
    /// Structure parsed but is truncated or partially damaged.
    PartiallyValid,
    /// Structure contradicts the claimed format.
    Invalid,
    /// Validation could not reach a conclusion. Per the invariants, a timeout
    /// or resource limit is inconclusive, never invalid.
    Indeterminate,
    /// No validator was run.
    NotAttempted,
}

/// Categorical confidence band. Deliberately not a percentage: the UI must not
/// display invented precision.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Confidence {
    Unknown,
    Low,
    Medium,
    High,
}

/// One reason contributing to a classification. Append-only, so the UI can
/// explain a score rather than restate it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Evidence {
    pub detail: String,
    /// Whether this evidence supports or undermines the candidate.
    pub supporting: bool,
}

impl Evidence {
    pub fn supporting(detail: impl Into<String>) -> Self {
        Self { detail: detail.into(), supporting: true }
    }

    pub fn detracting(detail: impl Into<String>) -> Self {
        Self { detail: detail.into(), supporting: false }
    }
}

/// A recoverable file with the evidence behind it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileCandidate {
    pub id: CandidateId,
    /// Name as recovered. May be incomplete; never invented.
    pub name: String,
    /// Path from the volume root, when the source provided one.
    pub path: Vec<String>,
    pub origin: Origin,
    pub extents: Vec<Extent>,
    /// Size recorded by the source. Untrusted.
    pub declared_size: u64,
    pub completeness: Completeness,
    pub validation: Validation,
    pub evidence: Vec<Evidence>,
}

impl FileCandidate {
    /// Bytes actually covered by the extents.
    pub fn recovered_size(&self) -> RecoveryResult<u64> {
        extent::total_length(&self.extents)
    }

    /// Source ranges in logical order, for streaming the file out.
    pub fn source_ranges(&self) -> Vec<ByteRange> {
        self.extents.iter().map(|e| e.source_range).collect()
    }

    /// Full display path including the name.
    pub fn display_path(&self) -> String {
        if self.path.is_empty() {
            self.name.clone()
        } else {
            format!("{}/{}", self.path.join("/"), self.name)
        }
    }

    /// Aggregates the candidate's evidence into a confidence band.
    ///
    /// The rules are deliberately conservative and ordered so that structural
    /// corruption dominates: no cosmetic indicator can lift a candidate whose
    /// content failed validation.
    pub fn confidence(&self) -> Confidence {
        // A validator that read the bytes and found them contradictory
        // outranks any metadata quality.
        if self.validation == Validation::Invalid {
            return Confidence::Unknown;
        }
        // Without content there is nothing to recover, whatever the record says.
        if self.completeness == Completeness::MetadataOnly {
            return Confidence::Unknown;
        }

        let base = match self.origin {
            Origin::ActiveFilesystem => Confidence::High,
            // Deleted records are weaker evidence: the content may have been
            // reused even when the record is pristine.
            Origin::DeletedFilesystem => Confidence::Medium,
            // A carved file has no filesystem record vouching for it.
            Origin::Carved => Confidence::Low,
        };

        let base = match self.validation {
            // Content confirming the format is the strongest evidence there is.
            Validation::Valid => base,
            // Truncation or damage caps the result regardless of provenance.
            Validation::PartiallyValid => base.min(Confidence::Medium),
            Validation::Indeterminate | Validation::NotAttempted => {
                // Unvalidated content cannot claim the top band.
                base.min(Confidence::Medium)
            }
            Validation::Invalid => Confidence::Unknown,
        };

        // Incomplete content is a structural shortfall, not a cosmetic one.
        let base = match self.completeness {
            Completeness::Complete => base,
            Completeness::Partial => base.min(Confidence::Low),
            Completeness::MetadataOnly => Confidence::Unknown,
        };

        // Any recorded contradiction pulls the candidate down a band, so
        // detracting evidence can never be silently outvoted.
        if self.evidence.iter().any(|e| !e.supporting) {
            return match base {
                Confidence::High => Confidence::Medium,
                Confidence::Medium => Confidence::Low,
                other => other,
            };
        }
        base
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(origin: Origin, completeness: Completeness, validation: Validation) -> FileCandidate {
        FileCandidate {
            id: CandidateId::new("c1"),
            name: "photo.jpg".into(),
            path: vec!["DCIM".into()],
            origin,
            extents: vec![Extent::new(ByteRange::new(0, 1024).unwrap(), 0).unwrap()],
            declared_size: 1024,
            completeness,
            validation,
            evidence: Vec::new(),
        }
    }

    #[test]
    fn active_validated_files_reach_high() {
        let c = candidate(Origin::ActiveFilesystem, Completeness::Complete, Validation::Valid);
        assert_eq!(c.confidence(), Confidence::High);
    }

    #[test]
    fn deleted_records_cannot_reach_high_on_metadata_alone() {
        let c = candidate(Origin::DeletedFilesystem, Completeness::Complete, Validation::Valid);
        assert_eq!(c.confidence(), Confidence::Medium);
    }

    #[test]
    fn carved_files_start_low() {
        let c = candidate(Origin::Carved, Completeness::Complete, Validation::Valid);
        assert_eq!(c.confidence(), Confidence::Low);
    }

    #[test]
    fn failed_validation_dominates_perfect_metadata() {
        // An active, complete record whose bytes contradict the format.
        let c = candidate(Origin::ActiveFilesystem, Completeness::Complete, Validation::Invalid);
        assert_eq!(c.confidence(), Confidence::Unknown);
    }

    #[test]
    fn unvalidated_content_cannot_reach_high() {
        let c = candidate(Origin::ActiveFilesystem, Completeness::Complete, Validation::NotAttempted);
        assert_eq!(c.confidence(), Confidence::Medium);
    }

    #[test]
    fn indeterminate_validation_is_not_treated_as_invalid() {
        // A validator timeout must not reject an otherwise good candidate.
        let c = candidate(Origin::ActiveFilesystem, Completeness::Complete, Validation::Indeterminate);
        assert_eq!(c.confidence(), Confidence::Medium);
    }

    #[test]
    fn partial_content_caps_confidence() {
        let c = candidate(Origin::ActiveFilesystem, Completeness::Partial, Validation::Valid);
        assert_eq!(c.confidence(), Confidence::Low);
    }

    #[test]
    fn metadata_only_candidates_are_unknown() {
        let c = candidate(Origin::ActiveFilesystem, Completeness::MetadataOnly, Validation::Valid);
        assert_eq!(c.confidence(), Confidence::Unknown);
    }

    #[test]
    fn detracting_evidence_lowers_the_band() {
        let mut c = candidate(Origin::ActiveFilesystem, Completeness::Complete, Validation::Valid);
        assert_eq!(c.confidence(), Confidence::High);
        c.evidence.push(Evidence::detracting("read error inside an extent"));
        assert_eq!(c.confidence(), Confidence::Medium);
    }

    #[test]
    fn supporting_evidence_does_not_inflate_the_band() {
        let mut c = candidate(Origin::Carved, Completeness::Complete, Validation::Valid);
        for i in 0..10 {
            c.evidence.push(Evidence::supporting(format!("marker {i}")));
        }
        // No amount of cosmetic evidence promotes a carved candidate.
        assert_eq!(c.confidence(), Confidence::Low);
    }

    #[test]
    fn scores_are_reproducible_for_identical_evidence() {
        let a = candidate(Origin::DeletedFilesystem, Completeness::Complete, Validation::Valid);
        let b = a.clone();
        assert_eq!(a.confidence(), b.confidence());
    }

    #[test]
    fn reports_sizes_and_paths() {
        let c = candidate(Origin::ActiveFilesystem, Completeness::Complete, Validation::Valid);
        assert_eq!(c.recovered_size().unwrap(), 1024);
        assert_eq!(c.display_path(), "DCIM/photo.jpg");
        assert_eq!(c.source_ranges().len(), 1);
    }

    #[test]
    fn display_path_handles_root_level_files() {
        let mut c = candidate(Origin::ActiveFilesystem, Completeness::Complete, Validation::Valid);
        c.path.clear();
        assert_eq!(c.display_path(), "photo.jpg");
    }
}
