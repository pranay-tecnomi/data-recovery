//! The recovery manifest: what was recovered, from where, and how trustworthy.
//!
//! The manifest is the audit record for a job. It is written as line-delimited
//! JSON so a partially written manifest still parses up to its last complete
//! record, rather than becoming unreadable in its entirety.

use recovery_core::{Confidence, FileCandidate, Validation};

use crate::writer::{ItemOutcome, RecoveredItem};

/// Escapes a string for embedding in JSON.
fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            // Control characters must be escaped to keep the JSON valid.
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
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
        Validation::PartiallyValid => "partially_valid",
        Validation::Invalid => "invalid",
        Validation::Indeterminate => "indeterminate",
        Validation::NotAttempted => "not_attempted",
    }
}

/// One manifest record, pairing a candidate with what happened to it.
pub fn manifest_line(candidate: &FileCandidate, item: &RecoveredItem) -> String {
    let (status, path, bytes, declared) = match &item.outcome {
        ItemOutcome::Written { path, bytes } => {
            ("written", path.display().to_string(), *bytes, *bytes)
        }
        ItemOutcome::WrittenPartial {
            path,
            bytes,
            declared,
        } => (
            "written_partial",
            path.display().to_string(),
            *bytes,
            *declared,
        ),
        ItemOutcome::Skipped { path } => ("skipped", path.display().to_string(), 0, 0),
        ItemOutcome::NoContent => ("no_content", String::new(), 0, 0),
    };

    let ranges: Vec<String> = candidate
        .extents
        .iter()
        .map(|e| format!("[{},{}]", e.source_range.offset, e.source_range.length))
        .collect();
    let evidence: Vec<String> = candidate
        .evidence
        .iter()
        .map(|e| {
            format!(
                r#"{{"supporting":{},"detail":"{}"}}"#,
                e.supporting,
                escape(&e.detail)
            )
        })
        .collect();

    format!(
        r#"{{"candidate_id":"{}","name":"{}","origin":"{:?}","status":"{}","output_path":"{}","bytes_written":{},"declared_size":{},"confidence":"{}","validation":"{}","source_ranges":[{}],"evidence":[{}]}}"#,
        escape(&item.candidate_id),
        escape(&candidate.display_path()),
        candidate.origin,
        status,
        escape(&path),
        bytes,
        declared,
        confidence_label(candidate.confidence()),
        validation_label(candidate.validation),
        ranges.join(","),
        evidence.join(",")
    )
}

/// Builds a full manifest from paired candidates and outcomes.
pub fn build_manifest(entries: &[(&FileCandidate, &RecoveredItem)]) -> String {
    let mut out = String::new();
    for (candidate, item) in entries {
        out.push_str(&manifest_line(candidate, item));
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::writer::{ItemOutcome, RecoveredItem};
    use recovery_core::{
        ByteRange, CandidateId, Completeness, Evidence, Extent, FileCandidate, Origin,
    };
    use std::path::PathBuf;

    fn candidate() -> FileCandidate {
        FileCandidate {
            id: CandidateId::new("c1"),
            name: "photo.jpg".into(),
            path: vec!["DCIM".into()],
            origin: Origin::DeletedFilesystem,
            extents: vec![Extent::new(ByteRange::new(4096, 1024).unwrap(), 0).unwrap()],
            declared_size: 1024,
            completeness: Completeness::Complete,
            validation: Validation::Valid,
            evidence: vec![Evidence::supporting("chain resolved cleanly")],
        }
    }

    fn written() -> RecoveredItem {
        RecoveredItem {
            candidate_id: "c1".into(),
            outcome: ItemOutcome::Written {
                path: PathBuf::from("/out/DCIM/photo.jpg"),
                bytes: 1024,
            },
        }
    }

    #[test]
    fn records_provenance_and_evidence() {
        let line = manifest_line(&candidate(), &written());
        assert!(line.contains(r#""candidate_id":"c1""#));
        assert!(line.contains(r#""name":"DCIM/photo.jpg""#));
        assert!(line.contains(r#""origin":"DeletedFilesystem""#));
        assert!(line.contains(r#""status":"written""#));
        assert!(line.contains(r#""source_ranges":[[4096,1024]]"#));
        assert!(line.contains("chain resolved cleanly"));
        // A deleted record cannot claim high confidence.
        assert!(line.contains(r#""confidence":"medium""#));
    }

    #[test]
    fn partial_recovery_is_recorded_as_partial() {
        let item = RecoveredItem {
            candidate_id: "c1".into(),
            outcome: ItemOutcome::WrittenPartial {
                path: PathBuf::from("/out/photo.jpg.partial"),
                bytes: 200,
                declared: 1024,
            },
        };
        let line = manifest_line(&candidate(), &item);
        // Never presented as complete.
        assert!(line.contains(r#""status":"written_partial""#));
        assert!(line.contains(r#""bytes_written":200"#));
        assert!(line.contains(r#""declared_size":1024"#));
    }

    #[test]
    fn escapes_hostile_names() {
        let mut c = candidate();
        c.name = format!("quote\"slash\\newline\nctrl{}.jpg", '\u{1}');
        let line = manifest_line(&c, &written());
        // The record must stay parseable whatever the name contained.
        assert!(line.contains("\\\""));
        assert!(line.contains("\\\\"));
        assert!(line.contains("\\n"));
        assert!(line.contains("\\u0001"));
        // A newline in a name must not split the line-delimited record.
        assert!(!line.contains('\n'));
    }

    #[test]
    fn records_skipped_and_empty_outcomes() {
        let skipped = RecoveredItem {
            candidate_id: "c1".into(),
            outcome: ItemOutcome::Skipped {
                path: PathBuf::from("/out/photo.jpg"),
            },
        };
        assert!(manifest_line(&candidate(), &skipped).contains(r#""status":"skipped""#));

        let empty = RecoveredItem {
            candidate_id: "c1".into(),
            outcome: ItemOutcome::NoContent,
        };
        assert!(manifest_line(&candidate(), &empty).contains(r#""status":"no_content""#));
    }

    #[test]
    fn builds_line_delimited_records() {
        let c = candidate();
        let item = written();
        let manifest = build_manifest(&[(&c, &item), (&c, &item)]);
        let lines: Vec<&str> = manifest.lines().collect();
        assert_eq!(lines.len(), 2);
        // Each line stands alone, so a truncated manifest still parses.
        assert!(lines.iter().all(|l| l.starts_with('{') && l.ends_with('}')));
        assert!(build_manifest(&[]).is_empty());
    }

    #[test]
    fn failed_validation_is_visible_in_the_record() {
        let mut c = candidate();
        c.validation = Validation::Invalid;
        let line = manifest_line(&c, &written());
        assert!(line.contains(r#""validation":"invalid""#));
        assert!(line.contains(r#""confidence":"unknown""#));
    }
}
