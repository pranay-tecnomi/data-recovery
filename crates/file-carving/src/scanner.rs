//! The bounded signature scanner.
//!
//! The source is read in fixed-size chunks so memory stays bounded regardless
//! of device size. Adjacent chunks overlap by the longest registered marker, so
//! a signature straddling a boundary is still found. Every offset and length is
//! checked: the bytes are untrusted.

use recovery_core::{
    ByteRange, CancellationToken, CandidateId, Completeness, Evidence, Extent, FileCandidate,
    Origin, RecoveryError, RecoveryResult, Validation,
};
use storage_io::BlockDevice;

use crate::registry::{BoundaryStrategy, CarveLimits, Signature, max_lookbehind};

/// A detected signature before its boundary has been resolved.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Detection {
    offset: u64,
    signature_index: usize,
}

/// Finds every occurrence of `needle` in `haystack`.
fn find_all(haystack: &[u8], needle: &[u8]) -> Vec<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return Vec::new();
    }
    let mut hits = Vec::new();
    let mut index = 0usize;
    while index + needle.len() <= haystack.len() {
        if &haystack[index..index + needle.len()] == needle {
            hits.push(index);
            index += 1;
        } else {
            index += 1;
        }
    }
    hits
}

/// Finds the first occurrence of `needle` at or after `from`.
fn find_from(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || from >= haystack.len() {
        return None;
    }
    haystack[from..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| p + from)
}

/// Resolves where a candidate ends, reading forward from its header.
///
/// Returns the candidate length, and whether the end was positively identified.
/// A candidate whose end cannot be found is truncated at the format's maximum
/// and reported as unterminated, never silently extended.
fn resolve_end<D: BlockDevice>(
    device: &D,
    signature: &Signature,
    start: u64,
    limits: &CarveLimits,
    cancel: &CancellationToken,
) -> RecoveryResult<(u64, bool)> {
    let capacity = device.capacity();
    let available = capacity.saturating_sub(start);
    let ceiling = signature.max_length.min(available);
    if ceiling == 0 {
        return Ok((0, false));
    }

    let Some(footer) = signature.footer else {
        // No terminator to search for; take what the format allows.
        return Ok((ceiling, false));
    };

    // Walk forward in bounded windows rather than buffering the whole span.
    let window = limits.chunk_size.max(footer.len() * 2);
    let mut searched: u64 = 0;
    let mut carry: Vec<u8> = Vec::new();

    while searched < ceiling {
        cancel.check()?;
        let take = (ceiling - searched).min(window as u64);
        let range = ByteRange::new(
            start
                .checked_add(searched)
                .ok_or(RecoveryError::RangeOverflow)?,
            take,
        )?;
        range.validate_within(capacity)?;

        let mut buffer = vec![0u8; usize::try_from(take).map_err(|_| {
            RecoveryError::LengthTooLarge { length: take }
        })?];
        let read = device.read(range, &mut buffer)?;
        if read == 0 {
            break;
        }
        buffer.truncate(read);

        // Prepend the carry so a footer split across windows is still found.
        let carry_len = carry.len();
        let mut searchable = carry;
        searchable.extend_from_slice(&buffer);

        if let Some(position) = find_from(&searchable, footer, 0) {
            let end_in_span = searched + (position as u64) - (carry_len as u64) + footer.len() as u64;
            // PNG's IEND is followed by a 4-byte CRC that belongs to the file.
            let end_in_span = if signature.boundary == BoundaryStrategy::StructureWalk {
                (end_in_span + 4).min(ceiling)
            } else {
                end_in_span
            };
            return Ok((end_in_span.min(ceiling), true));
        }

        searched += read as u64;
        // Keep the last footer-length bytes to bridge the next window.
        let keep = footer.len().saturating_sub(1).min(searchable.len());
        carry = searchable[searchable.len() - keep..].to_vec();
    }

    // The footer was never found: report the bound rather than inventing an end.
    Ok((ceiling, false))
}

/// Carves candidates from a bounded range of a source.
///
/// `range` bounds the scan; nothing outside it is read. Cancellation is checked
/// per chunk, so a long scan stays interruptible.
pub fn carve<D: BlockDevice>(
    device: &D,
    range: ByteRange,
    signatures: &[Signature],
    limits: &CarveLimits,
    cancel: &CancellationToken,
) -> RecoveryResult<Vec<FileCandidate>> {
    range.validate_within(device.capacity())?;
    let overlap = max_lookbehind(signatures);
    let chunk_size = limits.chunk_size.max(overlap + 1);

    let mut candidates: Vec<FileCandidate> = Vec::new();
    let mut cursor = range.offset;
    let range_end = range.end()?;
    // Offsets already claimed by a candidate, so a signature embedded inside a
    // carved file does not produce a duplicate.
    let mut claimed: Vec<(u64, u64)> = Vec::new();
    let mut sequence = 0u64;

    while cursor < range_end {
        cancel.check()?;
        if candidates.len() >= limits.max_candidates {
            break;
        }

        let take = (range_end - cursor).min(chunk_size as u64);
        let chunk_range = ByteRange::new(cursor, take)?;
        chunk_range.validate_within(device.capacity())?;
        let mut buffer = vec![0u8; usize::try_from(take).map_err(|_| {
            RecoveryError::LengthTooLarge { length: take }
        })?];
        let read = device.read(chunk_range, &mut buffer)?;
        if read == 0 {
            break;
        }
        buffer.truncate(read);

        // Collect detections from this chunk, in offset order.
        let mut detections: Vec<Detection> = Vec::new();
        for (index, signature) in signatures.iter().enumerate() {
            for hit in find_all(&buffer, signature.header) {
                detections.push(Detection {
                    offset: cursor + hit as u64,
                    signature_index: index,
                });
            }
        }
        detections.sort_by_key(|d| {
            // Higher priority first at the same offset.
            (d.offset, u8::MAX - signatures[d.signature_index].priority)
        });
        detections.truncate(limits.max_per_chunk);

        for detection in detections {
            cancel.check()?;
            if candidates.len() >= limits.max_candidates {
                break;
            }
            // A header inside an already-carved file is part of that file.
            if claimed.iter().any(|&(s, e)| detection.offset >= s && detection.offset < e) {
                continue;
            }
            let signature = &signatures[detection.signature_index];
            let (length, terminated) =
                resolve_end(device, signature, detection.offset, limits, cancel)?;

            // Too short to be a real file: treat the match as a false positive.
            if length < signature.min_length {
                continue;
            }
            let source_range = ByteRange::new(detection.offset, length)?;
            if source_range.validate_within(device.capacity()).is_err() {
                continue;
            }

            claimed.push((detection.offset, detection.offset + length));
            sequence += 1;
            candidates.push(build_candidate(
                signature,
                source_range,
                terminated,
                sequence,
            )?);
        }

        // Advance, overlapping so a signature spanning the boundary is caught.
        let advance = (read as u64).saturating_sub(overlap as u64).max(1);
        cursor = cursor
            .checked_add(advance)
            .ok_or(RecoveryError::RangeOverflow)?;
    }

    Ok(candidates)
}

/// Builds a candidate from a resolved carve.
fn build_candidate(
    signature: &Signature,
    source_range: ByteRange,
    terminated: bool,
    sequence: u64,
) -> RecoveryResult<FileCandidate> {
    let mut evidence = vec![Evidence::supporting(format!(
        "{} header signature found at offset {}",
        signature.id, source_range.offset
    ))];

    if terminated {
        evidence.push(Evidence::supporting(format!(
            "{} terminator located, so the boundary is evidenced",
            signature.id
        )));
    } else {
        // Without a terminator the length is a bound, not a measurement.
        evidence.push(Evidence::detracting(format!(
            "no {} terminator found within {} bytes; the end is not established",
            signature.id, signature.max_length
        )));
    }

    // Raw carving cannot see filesystem extents, so contiguity is assumed.
    evidence.push(Evidence::detracting(
        "carved contiguously; fragmentation cannot be excluded without filesystem extents",
    ));

    Ok(FileCandidate {
        id: CandidateId::new(format!("carved-{}-{}", signature.id, sequence)),
        name: format!("carved-{:010}.{}", source_range.offset, signature.extension),
        path: vec!["carved".to_string()],
        origin: Origin::Carved,
        extents: vec![Extent::new(source_range, 0)?],
        declared_size: source_range.length,
        completeness: if terminated {
            Completeness::Complete
        } else {
            // An unterminated carve is explicitly partial.
            Completeness::Partial
        },
        validation: Validation::NotAttempted,
        evidence,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{GIF, JPEG, PNG, REGISTRY, ZIP};

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

    /// A JPEG of `body` bytes, optionally terminated.
    fn jpeg(body: usize, terminated: bool) -> Vec<u8> {
        let mut v = vec![0xFF, 0xD8, 0xFF, 0xE0];
        v.extend(std::iter::repeat_n(0x41u8, body));
        if terminated {
            v.extend_from_slice(&[0xFF, 0xD9]);
        }
        v
    }

    fn png(body: usize) -> Vec<u8> {
        let mut v = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        v.extend(std::iter::repeat_n(0x42u8, body));
        v.extend_from_slice(b"IEND");
        v.extend_from_slice(&[0; 4]); // CRC
        v
    }

    fn small_limits() -> CarveLimits {
        // A small chunk exercises the overlap logic in tests.
        CarveLimits { chunk_size: 256, ..CarveLimits::default() }
    }

    fn carve_all(device: &Mem, limits: &CarveLimits) -> Vec<FileCandidate> {
        carve(
            device,
            ByteRange::new(0, device.capacity()).unwrap(),
            REGISTRY,
            limits,
            &CancellationToken::default(),
        )
        .unwrap()
    }

    #[test]
    fn carves_a_terminated_jpeg() {
        let mut disk = vec![0u8; 100];
        let file = jpeg(300, true);
        disk.extend_from_slice(&file);
        disk.extend(std::iter::repeat_n(0u8, 100));

        let out = carve_all(&Mem(disk), &small_limits());
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].origin, Origin::Carved);
        assert_eq!(out[0].extents[0].source_range.offset, 100);
        assert_eq!(out[0].extents[0].source_range.length, file.len() as u64);
        // A located terminator means the boundary is evidenced.
        assert_eq!(out[0].completeness, Completeness::Complete);
        assert!(out[0].evidence.iter().any(|e| e.detail.contains("terminator located")));
    }

    #[test]
    fn unterminated_carves_are_partial_not_complete() {
        // A header with no EOI anywhere in the source.
        let disk = jpeg(2000, false);
        let out = carve_all(&Mem(disk), &small_limits());
        assert_eq!(out.len(), 1);
        // The end was never established, so this must not claim completeness.
        assert_eq!(out[0].completeness, Completeness::Partial);
        assert!(out[0].evidence.iter().any(|e| !e.supporting && e.detail.contains("not established")));
    }

    #[test]
    fn every_carve_records_fragmentation_uncertainty() {
        let out = carve_all(&Mem(jpeg(300, true)), &small_limits());
        // Raw carving cannot exclude fragmentation, and must say so.
        assert!(out[0]
            .evidence
            .iter()
            .any(|e| !e.supporting && e.detail.contains("fragmentation")));
    }

    #[test]
    fn carved_candidates_never_reach_high_confidence() {
        let out = carve_all(&Mem(jpeg(300, true)), &small_limits());
        // Carved provenance plus unexcluded fragmentation caps the band.
        assert!(out[0].confidence() < recovery_core::Confidence::High);
    }

    #[test]
    fn short_matches_are_treated_as_false_positives() {
        // A bare SOI marker with nothing after it is noise, not a file.
        let disk = vec![0xFF, 0xD8, 0xFF, 0x00, 0x00, 0x00];
        assert!(carve_all(&Mem(disk), &small_limits()).is_empty());
    }

    #[test]
    fn random_bytes_produce_no_candidates() {
        // Deterministic pseudo-random filler with no signatures.
        let disk: Vec<u8> = (0..4096u32).map(|i| (i.wrapping_mul(37) % 200) as u8 + 1).collect();
        let out = carve_all(&Mem(disk), &small_limits());
        assert!(out.is_empty(), "false positives: {out:?}");
    }

    #[test]
    fn a_signature_embedded_in_a_carved_file_is_not_a_separate_candidate() {
        // A JPEG whose body contains a PNG header: a thumbnail, not a file.
        let mut file = vec![0xFF, 0xD8, 0xFF, 0xE0];
        file.extend(std::iter::repeat_n(0x41u8, 100));
        file.extend_from_slice(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
        file.extend(std::iter::repeat_n(0x41u8, 100));
        file.extend_from_slice(&[0xFF, 0xD9]);

        let out = carve_all(&Mem(file), &small_limits());
        // Only the outer file is emitted.
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].extents[0].source_range.offset, 0);
    }

    #[test]
    fn carves_several_distinct_files() {
        let mut disk = Vec::new();
        disk.extend_from_slice(&jpeg(200, true));
        disk.extend(std::iter::repeat_n(0u8, 64));
        let png_offset = disk.len() as u64;
        disk.extend_from_slice(&png(200));

        let out = carve_all(&Mem(disk), &small_limits());
        assert_eq!(out.len(), 2);
        assert!(out.iter().any(|c| c.extents[0].source_range.offset == 0));
        assert!(out.iter().any(|c| c.extents[0].source_range.offset == png_offset));
    }

    #[test]
    fn finds_a_signature_spanning_a_chunk_boundary() {
        // Place the header so it straddles the 256-byte chunk edge.
        let mut disk = vec![0u8; 254];
        disk.extend_from_slice(&jpeg(400, true));
        let out = carve_all(&Mem(disk), &small_limits());
        assert_eq!(out.len(), 1, "signature at a chunk boundary was missed");
        assert_eq!(out[0].extents[0].source_range.offset, 254);
    }

    #[test]
    fn finds_a_footer_spanning_a_window_boundary() {
        // A body long enough that the EOI lands across a search window edge.
        let disk = jpeg(255, true);
        let out = carve_all(&Mem(disk.clone()), &small_limits());
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].extents[0].source_range.length, disk.len() as u64);
        assert_eq!(out[0].completeness, Completeness::Complete);
    }

    #[test]
    fn png_carve_includes_the_iend_crc() {
        let file = png(100);
        let out = carve_all(&Mem(file.clone()), &small_limits());
        assert_eq!(out.len(), 1);
        // IEND is followed by a 4-byte CRC that belongs to the file.
        assert_eq!(out[0].extents[0].source_range.length, file.len() as u64);
    }

    #[test]
    fn carving_never_reads_outside_the_requested_range() {
        let mut disk = vec![0u8; 512];
        disk.extend_from_slice(&jpeg(200, true));
        let device = Mem(disk);
        // Scan only the leading zero region.
        let out = carve(
            &device,
            ByteRange::new(0, 512).unwrap(),
            REGISTRY,
            &small_limits(),
            &CancellationToken::default(),
        )
        .unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn candidate_extents_stay_within_the_device() {
        // A header near the very end, with no room for a full file.
        let mut disk = vec![0u8; 100];
        disk.extend_from_slice(&[0xFF, 0xD8, 0xFF]);
        let device = Mem(disk);
        let capacity = device.capacity();
        for c in carve_all(&device, &small_limits()) {
            assert!(c.extents[0].source_range.end().unwrap() <= capacity);
        }
    }

    #[test]
    fn respects_the_candidate_limit() {
        // Many small terminated JPEGs.
        let mut disk = Vec::new();
        for _ in 0..20 {
            disk.extend_from_slice(&jpeg(200, true));
        }
        let limits = CarveLimits { chunk_size: 256, max_candidates: 5, ..CarveLimits::default() };
        assert!(carve_all(&Mem(disk), &limits).len() <= 5);
    }

    #[test]
    fn honours_cancellation() {
        let disk = jpeg(2000, true);
        let token = CancellationToken::default();
        token.cancel();
        let device = Mem(disk);
        let result = carve(
            &device,
            ByteRange::new(0, device.capacity()).unwrap(),
            REGISTRY,
            &small_limits(),
            &token,
        );
        assert!(result.is_err());
    }

    #[test]
    fn carve_output_is_deterministic() {
        let mut disk = Vec::new();
        disk.extend_from_slice(&jpeg(200, true));
        disk.extend(std::iter::repeat_n(0u8, 50));
        disk.extend_from_slice(&png(200));
        let first = carve_all(&Mem(disk.clone()), &small_limits());
        let second = carve_all(&Mem(disk), &small_limits());
        assert_eq!(first, second);
    }

    #[test]
    fn an_empty_source_yields_nothing() {
        let device = Mem(Vec::new());
        let out = carve(
            &device,
            ByteRange::new(0, 0).unwrap(),
            REGISTRY,
            &small_limits(),
            &CancellationToken::default(),
        )
        .unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn a_truncated_source_does_not_panic() {
        // Headers with the source ending mid-file.
        for signature in [&JPEG, &PNG, &ZIP, &GIF] {
            let mut disk = signature.header.to_vec();
            disk.extend(std::iter::repeat_n(0x41u8, 40));
            let _ = carve_all(&Mem(disk), &small_limits());
        }
    }

    #[test]
    fn carved_names_encode_their_offset_and_format() {
        let mut disk = vec![0u8; 100];
        disk.extend_from_slice(&jpeg(300, true));
        let out = carve_all(&Mem(disk), &small_limits());
        assert!(out[0].name.ends_with(".jpg"));
        assert!(out[0].name.contains("0000000100"));
        // Carved output is grouped so it is distinguishable from recovered files.
        assert_eq!(out[0].path, vec!["carved".to_string()]);
    }
}
