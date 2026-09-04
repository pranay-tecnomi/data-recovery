//! Candidate normalisation and deduplication.
//!
//! Scans reach the same file by more than one route: a directory walk and a
//! carve may both find it, and two directory records may point at one stream.
//! Normalisation makes candidates comparable; deduplication keeps the
//! best-evidenced representative instead of reporting a file repeatedly.

use recovery_core::{Extent, FileCandidate, Origin};

/// Trims a recovered name to something safe to display and write.
///
/// Names come from untrusted metadata. Path separators and control characters
/// are replaced so a recovered file cannot escape its output directory, and
/// the result is never empty.
pub fn normalize_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c == '/' || c == '\\' || (c as u32) < 0x20 || c == '\u{7f}' {
                '_'
            } else {
                c
            }
        })
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() || trimmed.chars().all(|c| c == '.') {
        "unnamed".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Normalises a candidate in place: its name and each path component.
pub fn normalize(candidate: &mut FileCandidate) {
    candidate.name = normalize_name(&candidate.name);
    candidate.path = candidate
        .path
        .iter()
        // Drop traversal components before normalising, since normalising
        // would otherwise turn them into placeholder directory names.
        .filter(|c| c.as_str() != "." && c.as_str() != "..")
        .map(|c| normalize_name(c))
        .collect();
}

/// The identity used to decide whether two candidates describe one file.
///
/// Content location is the identity, not the name: two records pointing at the
/// same first extent describe the same bytes whatever they are called.
fn identity(candidate: &FileCandidate) -> Option<(u64, u64)> {
    let first = candidate.extents.first()?;
    Some((first.source_range.offset, candidate.declared_size))
}

/// Ranks candidates so the best-evidenced representative survives dedup.
fn quality(candidate: &FileCandidate) -> (u8, u8, u64) {
    let origin_rank = match candidate.origin {
        // A live record is the most trustworthy account of a file.
        Origin::ActiveFilesystem => 2,
        Origin::DeletedFilesystem => 1,
        Origin::Carved => 0,
    };
    (
        origin_rank,
        candidate.confidence() as u8,
        candidate.recovered_size().unwrap_or(0),
    )
}

/// Removes duplicate candidates, keeping the best-evidenced of each group.
///
/// Candidates without extents cannot be compared by content, so they are
/// retained: dropping them would discard metadata-only findings that the user
/// may still want to see.
pub fn deduplicate(mut candidates: Vec<FileCandidate>) -> Vec<FileCandidate> {
    // Strongest first, so the first candidate seen for an identity wins.
    candidates.sort_by_key(|c| std::cmp::Reverse(quality(c)));

    let mut seen: Vec<(u64, u64)> = Vec::new();
    let mut out = Vec::new();
    for candidate in candidates {
        match identity(&candidate) {
            Some(key) if seen.contains(&key) => continue,
            Some(key) => {
                seen.push(key);
                out.push(candidate);
            }
            // No extents: not comparable by content, so keep it.
            None => out.push(candidate),
        }
    }
    out
}

/// Whether two candidates' extents overlap in source space.
///
/// Overlap between distinct files is contradictory evidence: the same bytes
/// cannot belong to two files at once, so at least one reconstruction is wrong.
pub fn extents_overlap(a: &[Extent], b: &[Extent]) -> bool {
    for x in a {
        for y in b {
            let (xs, ys) = (x.source_range.offset, y.source_range.offset);
            let (xe, ye) = match (x.source_range.end(), y.source_range.end()) {
                (Ok(xe), Ok(ye)) => (xe, ye),
                _ => continue,
            };
            if xs < ye && ys < xe {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use recovery_core::{
        ByteRange, CandidateId, Completeness, Confidence, Evidence, Validation,
    };

    fn candidate(name: &str, offset: u64, size: u64, origin: Origin) -> FileCandidate {
        FileCandidate {
            id: CandidateId::new(name),
            name: name.into(),
            path: Vec::new(),
            origin,
            extents: vec![Extent::new(ByteRange::new(offset, size).unwrap(), 0).unwrap()],
            declared_size: size,
            completeness: Completeness::Complete,
            validation: Validation::Valid,
            evidence: Vec::new(),
        }
    }

    #[test]
    fn normalises_ordinary_names_unchanged() {
        assert_eq!(normalize_name("holiday photo.jpg"), "holiday photo.jpg");
    }

    #[test]
    fn replaces_path_separators() {
        // A separator would let a recovered file escape its directory.
        assert_eq!(normalize_name("../../etc/passwd"), ".._.._etc_passwd");
        assert_eq!(normalize_name(r"..\windows"), ".._windows");
    }

    #[test]
    fn replaces_control_characters() {
        assert_eq!(normalize_name("a\u{0}b\u{7f}"), "a_b_");
    }

    #[test]
    fn degenerate_names_get_a_placeholder() {
        assert_eq!(normalize_name(""), "unnamed");
        assert_eq!(normalize_name("   "), "unnamed");
        assert_eq!(normalize_name("..."), "unnamed");
    }

    #[test]
    fn normalises_path_components_and_drops_dot_entries() {
        let mut c = candidate("f.txt", 0, 10, Origin::ActiveFilesystem);
        c.path = vec!["DCIM".into(), "..".into(), "a/b".into(), ".".into()];
        normalize(&mut c);
        assert_eq!(c.path, vec!["DCIM".to_string(), "a_b".to_string()]);
    }

    #[test]
    fn deduplicates_candidates_sharing_content() {
        let carved = candidate("carved.jpg", 4096, 1000, Origin::Carved);
        let active = candidate("photo.jpg", 4096, 1000, Origin::ActiveFilesystem);
        let out = deduplicate(vec![carved, active]);
        assert_eq!(out.len(), 1);
        // The filesystem record is the better-evidenced account.
        assert_eq!(out[0].origin, Origin::ActiveFilesystem);
        assert_eq!(out[0].name, "photo.jpg");
    }

    #[test]
    fn keeps_candidates_at_distinct_offsets() {
        let out = deduplicate(vec![
            candidate("a.jpg", 0, 100, Origin::Carved),
            candidate("b.jpg", 4096, 100, Origin::Carved),
        ]);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn same_offset_different_size_is_not_a_duplicate() {
        let out = deduplicate(vec![
            candidate("a.jpg", 4096, 100, Origin::Carved),
            candidate("b.jpg", 4096, 200, Origin::Carved),
        ]);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn prefers_the_higher_confidence_duplicate() {
        let mut weak = candidate("weak.jpg", 4096, 100, Origin::DeletedFilesystem);
        weak.evidence.push(Evidence::detracting("clusters partly reallocated"));
        let strong = candidate("strong.jpg", 4096, 100, Origin::DeletedFilesystem);
        assert!(strong.confidence() > weak.confidence());
        let out = deduplicate(vec![weak, strong]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name, "strong.jpg");
    }

    #[test]
    fn retains_metadata_only_candidates() {
        let mut a = candidate("gone.txt", 0, 0, Origin::DeletedFilesystem);
        a.extents.clear();
        let mut b = candidate("also-gone.txt", 0, 0, Origin::DeletedFilesystem);
        b.extents.clear();
        // Not comparable by content, so neither is discarded.
        assert_eq!(deduplicate(vec![a, b]).len(), 2);
    }

    #[test]
    fn deduplication_is_deterministic() {
        let build = || {
            vec![
                candidate("a.jpg", 4096, 100, Origin::Carved),
                candidate("b.jpg", 4096, 100, Origin::ActiveFilesystem),
                candidate("c.jpg", 8192, 100, Origin::DeletedFilesystem),
            ]
        };
        let first = deduplicate(build());
        let second = deduplicate(build());
        assert_eq!(first, second);
    }

    #[test]
    fn detects_overlapping_extents() {
        let a = candidate("a", 0, 1000, Origin::Carved);
        let b = candidate("b", 500, 1000, Origin::Carved);
        let c = candidate("c", 4096, 1000, Origin::Carved);
        assert!(extents_overlap(&a.extents, &b.extents));
        assert!(!extents_overlap(&a.extents, &c.extents));
        // Ranges that merely touch do not overlap.
        let d = candidate("d", 1000, 100, Origin::Carved);
        assert!(!extents_overlap(&a.extents, &d.extents));
    }

    #[test]
    fn confidence_ordering_is_usable_for_ranking() {
        assert!(Confidence::High > Confidence::Medium);
        assert!(Confidence::Medium > Confidence::Low);
        assert!(Confidence::Low > Confidence::Unknown);
    }
}
