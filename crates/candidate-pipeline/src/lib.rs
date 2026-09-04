//! The candidate pipeline: normalise, deduplicate, validate, score.
//!
//! Filesystem modules and carvers emit candidates with provenance and
//! metadata evidence. This crate turns that raw set into the ranked, explained
//! list the UI presents, without any module asserting its own confidence.

#![forbid(unsafe_code)]

pub mod normalize;
pub mod validate;

pub use normalize::{deduplicate, extents_overlap, normalize, normalize_name};
pub use validate::{validate, validate_bytes, ValidationReport, VALIDATION_WINDOW};

use recovery_core::{
    CancellationToken, Confidence, Evidence, FileCandidate, RecoveryResult,
};
use storage_io::BlockDevice;

/// Runs the full pipeline over a raw candidate set.
///
/// Candidates are normalised, deduplicated, validated against their content
/// and ranked strongest first. Cancellation is checked per candidate, so a
/// long scan stays responsive.
pub fn run<D: BlockDevice>(
    device: &D,
    candidates: Vec<FileCandidate>,
    cancel: &CancellationToken,
) -> RecoveryResult<Vec<FileCandidate>> {
    let mut candidates = candidates;
    for candidate in &mut candidates {
        normalize(candidate);
    }
    let mut candidates = deduplicate(candidates);

    for candidate in &mut candidates {
        cancel.check()?;
        let report = validate::validate(device, candidate, cancel)?;
        candidate.validation = report.validation;
        candidate.evidence.extend(report.evidence);
        // A format that contradicts the name is worth recording, since the
        // user chose what to recover partly by extension.
        if let Some(detected) = report.detected_format
            && !name_matches_format(&candidate.name, detected)
        {
            candidate.evidence.push(Evidence::detracting(format!(
                "content is {detected}, which does not match the file name"
            )));
        }
    }

    flag_overlapping_candidates(&mut candidates);

    // Strongest first; ties broken by name so the order is reproducible.
    candidates.sort_by(|a, b| {
        b.confidence()
            .cmp(&a.confidence())
            .then_with(|| a.display_path().cmp(&b.display_path()))
    });
    Ok(candidates)
}

/// Whether a file name's extension agrees with the detected format.
fn name_matches_format(name: &str, format: &str) -> bool {
    let Some((_, extension)) = name.rsplit_once('.') else {
        // No extension makes no claim to contradict.
        return true;
    };
    let extension = extension.to_ascii_lowercase();
    match format {
        "jpeg" => matches!(extension.as_str(), "jpg" | "jpeg" | "jpe"),
        "png" => extension == "png",
        "pdf" => extension == "pdf",
        "gif" => extension == "gif",
        // Many formats are ZIP containers, so a mismatch here means little.
        "zip" => true,
        _ => true,
    }
}

/// Records contradictory evidence when two candidates claim the same bytes.
///
/// The same source range cannot belong to two files, so at least one
/// reconstruction is wrong. Both are flagged rather than one being silently
/// discarded, since the pipeline cannot tell which is correct.
fn flag_overlapping_candidates(candidates: &mut [FileCandidate]) {
    let extents: Vec<Vec<recovery_core::Extent>> =
        candidates.iter().map(|c| c.extents.clone()).collect();
    let mut overlapping = vec![false; candidates.len()];
    for i in 0..extents.len() {
        for j in (i + 1)..extents.len() {
            if extents_overlap(&extents[i], &extents[j]) {
                overlapping[i] = true;
                overlapping[j] = true;
            }
        }
    }
    for (candidate, overlaps) in candidates.iter_mut().zip(overlapping) {
        if overlaps {
            candidate.evidence.push(Evidence::detracting(
                "extents overlap another candidate, so at least one is wrong",
            ));
        }
    }
}

/// Groups candidates by confidence band, strongest first, for presentation.
pub fn group_by_confidence(candidates: &[FileCandidate]) -> Vec<(Confidence, Vec<&FileCandidate>)> {
    let bands = [
        Confidence::High,
        Confidence::Medium,
        Confidence::Low,
        Confidence::Unknown,
    ];
    bands
        .into_iter()
        .map(|band| {
            let members = candidates.iter().filter(|c| c.confidence() == band).collect();
            (band, members)
        })
        .filter(|(_, members): &(Confidence, Vec<&FileCandidate>)| !members.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use recovery_core::{
        ByteRange, CandidateId, Completeness, Completeness::Complete, Extent, Origin,
        RecoveryResult, Validation,
    };

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

    fn jpeg() -> Vec<u8> {
        let mut v = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10];
        v.extend_from_slice(b"JFIF\0");
        v.resize(30, 0);
        v.extend_from_slice(&[0xFF, 0xD9]);
        v
    }

    /// A device holding a valid JPEG at offset 0 and junk at 4096.
    fn device() -> Mem {
        let mut disk = vec![0u8; 8192];
        let j = jpeg();
        disk[..j.len()].copy_from_slice(&j);
        Mem(disk)
    }

    fn candidate(
        name: &str,
        offset: u64,
        size: u64,
        origin: Origin,
        completeness: Completeness,
    ) -> FileCandidate {
        FileCandidate {
            id: CandidateId::new(name),
            name: name.into(),
            path: Vec::new(),
            origin,
            extents: vec![Extent::new(ByteRange::new(offset, size).unwrap(), 0).unwrap()],
            declared_size: size,
            completeness,
            validation: Validation::NotAttempted,
            evidence: Vec::new(),
        }
    }

    fn run_pipeline(candidates: Vec<FileCandidate>) -> Vec<FileCandidate> {
        run(&device(), candidates, &CancellationToken::default()).unwrap()
    }

    #[test]
    fn validated_active_file_reaches_high_confidence() {
        let size = jpeg().len() as u64;
        let out = run_pipeline(vec![candidate(
            "photo.jpg",
            0,
            size,
            Origin::ActiveFilesystem,
            Complete,
        )]);
        assert_eq!(out[0].validation, Validation::Valid);
        assert_eq!(out[0].confidence(), Confidence::High);
    }

    #[test]
    fn failed_validation_demotes_a_pristine_record() {
        // Metadata is perfect, but the bytes are not a JPEG.
        let out = run_pipeline(vec![candidate(
            "photo.jpg",
            4096,
            100,
            Origin::ActiveFilesystem,
            Complete,
        )]);
        assert_eq!(out[0].validation, Validation::Invalid);
        assert_eq!(out[0].confidence(), Confidence::Unknown);
    }

    #[test]
    fn validation_promotes_nothing_beyond_its_provenance() {
        let size = jpeg().len() as u64;
        // The same valid bytes, found by carving rather than a record.
        let out = run_pipeline(vec![candidate("carved.jpg", 0, size, Origin::Carved, Complete)]);
        assert_eq!(out[0].validation, Validation::Valid);
        // Provenance is not erased by a clean validation.
        assert_eq!(out[0].confidence(), Confidence::Low);
    }

    #[test]
    fn results_are_ranked_strongest_first() {
        let size = jpeg().len() as u64;
        let out = run_pipeline(vec![
            candidate("carved.jpg", 0, size, Origin::Carved, Complete),
            candidate("bad.jpg", 4096, 100, Origin::ActiveFilesystem, Complete),
            candidate("good.jpg", 0, size, Origin::ActiveFilesystem, Complete),
        ]);
        let bands: Vec<Confidence> = out.iter().map(|c| c.confidence()).collect();
        // Sorted descending, with no band out of order.
        assert!(bands.windows(2).all(|w| w[0] >= w[1]));
        assert_eq!(out[0].confidence(), Confidence::High);
    }

    #[test]
    fn duplicates_are_collapsed_to_the_best_evidenced() {
        let size = jpeg().len() as u64;
        let out = run_pipeline(vec![
            candidate("carved.jpg", 0, size, Origin::Carved, Complete),
            candidate("photo.jpg", 0, size, Origin::ActiveFilesystem, Complete),
        ]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name, "photo.jpg");
    }

    #[test]
    fn overlapping_candidates_are_flagged_on_both_sides() {
        let size = jpeg().len() as u64;
        // Two distinct files claiming intersecting bytes.
        let a = candidate("a.jpg", 0, size, Origin::ActiveFilesystem, Complete);
        let b = candidate("b.jpg", 8, size, Origin::ActiveFilesystem, Complete);
        let out = run_pipeline(vec![a, b]);
        assert_eq!(out.len(), 2);
        for c in &out {
            assert!(
                c.evidence.iter().any(|e| !e.supporting && e.detail.contains("overlap")),
                "both candidates must carry the contradiction"
            );
        }
    }

    #[test]
    fn mislabelled_content_is_recorded_as_detracting() {
        let size = jpeg().len() as u64;
        // JPEG bytes carrying a .png name.
        let out = run_pipeline(vec![candidate(
            "mislabelled.png",
            0,
            size,
            Origin::ActiveFilesystem,
            Complete,
        )]);
        assert!(out[0]
            .evidence
            .iter()
            .any(|e| !e.supporting && e.detail.contains("does not match")));
        // The contradiction costs it the top band.
        assert_eq!(out[0].confidence(), Confidence::Medium);
    }

    #[test]
    fn names_are_normalised_before_output() {
        let size = jpeg().len() as u64;
        let mut c = candidate("photo.jpg", 0, size, Origin::ActiveFilesystem, Complete);
        c.name = "../../escape.jpg".into();
        let out = run_pipeline(vec![c]);
        assert!(!out[0].name.contains('/'));
    }

    #[test]
    fn partial_content_is_capped_regardless_of_validation() {
        let size = jpeg().len() as u64;
        let out = run_pipeline(vec![candidate(
            "photo.jpg",
            0,
            size,
            Origin::ActiveFilesystem,
            Completeness::Partial,
        )]);
        assert_eq!(out[0].validation, Validation::Valid);
        assert_eq!(out[0].confidence(), Confidence::Low);
    }

    #[test]
    fn pipeline_honours_cancellation() {
        let size = jpeg().len() as u64;
        let token = CancellationToken::default();
        token.cancel();
        let result = run(
            &device(),
            vec![candidate("photo.jpg", 0, size, Origin::ActiveFilesystem, Complete)],
            &token,
        );
        assert!(result.is_err());
    }

    #[test]
    fn pipeline_output_is_reproducible() {
        let size = jpeg().len() as u64;
        let build = || {
            vec![
                candidate("b.jpg", 0, size, Origin::ActiveFilesystem, Complete),
                candidate("a.jpg", 4096, 100, Origin::Carved, Complete),
                candidate("c.jpg", 2048, 100, Origin::DeletedFilesystem, Complete),
            ]
        };
        assert_eq!(run_pipeline(build()), run_pipeline(build()));
    }

    #[test]
    fn groups_candidates_into_populated_bands() {
        let size = jpeg().len() as u64;
        let out = run_pipeline(vec![
            candidate("good.jpg", 0, size, Origin::ActiveFilesystem, Complete),
            candidate("bad.jpg", 4096, 100, Origin::ActiveFilesystem, Complete),
        ]);
        let groups = group_by_confidence(&out);
        // Only bands with members appear, strongest first.
        assert!(!groups.is_empty());
        assert!(groups.windows(2).all(|w| w[0].0 > w[1].0));
        assert!(groups.iter().all(|(_, members)| !members.is_empty()));
    }

    #[test]
    fn empty_input_is_handled() {
        assert!(run_pipeline(Vec::new()).is_empty());
        assert!(group_by_confidence(&[]).is_empty());
    }
}
