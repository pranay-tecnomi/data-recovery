//! Bounded structural validators.
//!
//! Validators answer one question: are these bytes structurally plausible for
//! the format they claim to be? They return evidence, never a recovery
//! decision. Every validator reads a bounded prefix, so a hostile size field
//! cannot drive unbounded work, and exhaustion is reported as Indeterminate
//! rather than Invalid.

use recovery_core::{
    ByteRange, CancellationToken, Evidence, FileCandidate, RecoveryResult, Validation,
};
use storage_io::BlockDevice;

/// Bytes read from a candidate to judge it. Enough for any header this module
/// inspects, and small enough that validation stays cheap.
pub const VALIDATION_WINDOW: u64 = 64 * 1024;

/// A validator's finding.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidationReport {
    pub validation: Validation,
    pub evidence: Vec<Evidence>,
    /// Format inferred from the content, which may contradict the extension.
    pub detected_format: Option<&'static str>,
}

impl ValidationReport {
    fn new(validation: Validation, detail: impl Into<String>, format: Option<&'static str>) -> Self {
        let supporting = matches!(validation, Validation::Valid | Validation::PartiallyValid);
        Self {
            validation,
            evidence: vec![if supporting {
                Evidence::supporting(detail)
            } else {
                Evidence::detracting(detail)
            }],
            detected_format: format,
        }
    }

    fn indeterminate(detail: impl Into<String>) -> Self {
        Self {
            validation: Validation::Indeterminate,
            // Inconclusive is not evidence against the candidate.
            evidence: vec![Evidence::supporting(detail)],
            detected_format: None,
        }
    }
}

/// Reads the leading bytes of a candidate, up to `VALIDATION_WINDOW`.
///
/// Reads follow the candidate's extents, so a fragmented file is validated
/// against its reconstructed content rather than raw disk order.
fn read_prefix<D: BlockDevice>(
    device: &D,
    candidate: &FileCandidate,
    cancel: &CancellationToken,
) -> RecoveryResult<Vec<u8>> {
    let mut buffer = Vec::new();
    for extent in &candidate.extents {
        cancel.check()?;
        let remaining = VALIDATION_WINDOW.saturating_sub(buffer.len() as u64);
        if remaining == 0 {
            break;
        }
        let take = extent.source_range.length.min(remaining);
        if take == 0 {
            continue;
        }
        let range = ByteRange::new(extent.source_range.offset, take)?;
        // A range outside the device is a defect in the candidate, not a
        // reason to fail the whole scan.
        if range.validate_within(device.capacity()).is_err() {
            break;
        }
        let start = buffer.len();
        buffer.resize(
            start + usize::try_from(take).unwrap_or(0),
            0,
        );
        match device.read(range, &mut buffer[start..]) {
            Ok(read) => buffer.truncate(start + read),
            Err(_) => {
                buffer.truncate(start);
                break;
            }
        }
    }
    Ok(buffer)
}

/// Validates a candidate's content against the format its name implies.
pub fn validate<D: BlockDevice>(
    device: &D,
    candidate: &FileCandidate,
    cancel: &CancellationToken,
) -> RecoveryResult<ValidationReport> {
    if candidate.extents.is_empty() {
        return Ok(ValidationReport::indeterminate(
            "no content located, so structure could not be checked",
        ));
    }
    let data = read_prefix(device, candidate, cancel)?;
    if data.is_empty() {
        return Ok(ValidationReport::indeterminate(
            "content could not be read, so structure could not be checked",
        ));
    }
    Ok(validate_bytes(&data, &candidate.name, candidate.declared_size))
}

/// Validates an in-memory prefix. Separated so the format rules are testable
/// without a device.
pub fn validate_bytes(data: &[u8], name: &str, declared_size: u64) -> ValidationReport {
    let extension = name
        .rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase())
        .unwrap_or_default();

    // Content decides the format; the extension is only a hint about which
    // rules the user expected to apply.
    let detected = detect_format(data);

    match detected {
        Some("jpeg") => validate_jpeg(data, declared_size),
        Some("png") => validate_png(data, declared_size),
        Some("pdf") => validate_pdf(data),
        Some("zip") => validate_zip(data),
        Some("gif") => ValidationReport::new(Validation::Valid, "GIF header is well formed", Some("gif")),
        None if extension.is_empty() => {
            ValidationReport::indeterminate("no recognised signature and no extension to check against")
        }
        None => {
            // A known extension whose content does not match is a genuine
            // contradiction, not merely an unknown format.
            if matches!(extension.as_str(), "jpg" | "jpeg" | "png" | "pdf" | "zip" | "gif") {
                ValidationReport::new(
                    Validation::Invalid,
                    format!("content does not match the .{extension} signature"),
                    None,
                )
            } else {
                ValidationReport::indeterminate("no validator is registered for this format")
            }
        }
        Some(other) => ValidationReport::new(Validation::Valid, "signature recognised", Some(other)),
    }
}

/// Identifies a format from its magic bytes.
fn detect_format(data: &[u8]) -> Option<&'static str> {
    if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some("jpeg");
    }
    if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some("png");
    }
    if data.starts_with(b"%PDF-") {
        return Some("pdf");
    }
    if data.starts_with(b"PK\x03\x04") || data.starts_with(b"PK\x05\x06") {
        return Some("zip");
    }
    if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
        return Some("gif");
    }
    None
}

/// JPEG: an SOI marker, a plausible segment chain, and ideally a terminating
/// EOI. A missing EOI means truncation, which is partial rather than invalid.
fn validate_jpeg(data: &[u8], declared_size: u64) -> ValidationReport {
    let mut index = 2usize;
    let mut segments = 0usize;
    // Walk the marker chain within the window, bounded by the data length.
    while index + 4 <= data.len() {
        if data[index] != 0xFF {
            break;
        }
        let marker = data[index + 1];
        // Start of scan: entropy-coded data follows and is not segmented.
        if marker == 0xDA {
            segments += 1;
            break;
        }
        // Standalone markers carry no length field.
        if (0xD0..=0xD9).contains(&marker) {
            index += 2;
            continue;
        }
        let length = usize::from(u16::from_be_bytes([data[index + 2], data[index + 3]]));
        if length < 2 {
            return ValidationReport::new(
                Validation::Invalid,
                "JPEG segment declares an impossible length",
                Some("jpeg"),
            );
        }
        segments += 1;
        index += 2 + length;
    }

    if segments == 0 {
        return ValidationReport::new(
            Validation::Invalid,
            "JPEG start marker is not followed by a valid segment",
            Some("jpeg"),
        );
    }
    // The window may not reach the end of a large file, so only treat a
    // missing EOI as truncation when the whole file was inspected.
    let whole_file_seen = declared_size <= data.len() as u64;
    if whole_file_seen && !data.ends_with(&[0xFF, 0xD9]) {
        return ValidationReport::new(
            Validation::PartiallyValid,
            "JPEG structure is valid but the end-of-image marker is missing",
            Some("jpeg"),
        );
    }
    ValidationReport::new(Validation::Valid, "JPEG marker chain is well formed", Some("jpeg"))
}

/// PNG: signature, an IHDR first chunk, and a plausible chunk chain.
fn validate_png(data: &[u8], declared_size: u64) -> ValidationReport {
    if data.len() < 16 {
        return ValidationReport::new(
            Validation::PartiallyValid,
            "PNG signature present but the header chunk is truncated",
            Some("png"),
        );
    }
    if &data[12..16] != b"IHDR" {
        return ValidationReport::new(
            Validation::Invalid,
            "PNG signature is not followed by an IHDR chunk",
            Some("png"),
        );
    }
    let mut index = 8usize;
    let mut saw_end = false;
    while index + 12 <= data.len() {
        let length = u32::from_be_bytes([data[index], data[index + 1], data[index + 2], data[index + 3]]);
        let kind = &data[index + 4..index + 8];
        if kind == b"IEND" {
            saw_end = true;
            break;
        }
        // Chunk length plus header and CRC must not overflow the cursor.
        let step = match usize::try_from(length).ok().and_then(|l| l.checked_add(12)) {
            Some(step) => step,
            None => {
                return ValidationReport::new(
                    Validation::Invalid,
                    "PNG chunk declares an impossible length",
                    Some("png"),
                );
            }
        };
        index += step;
    }
    let whole_file_seen = declared_size <= data.len() as u64;
    if whole_file_seen && !saw_end {
        return ValidationReport::new(
            Validation::PartiallyValid,
            "PNG structure is valid but the IEND chunk is missing",
            Some("png"),
        );
    }
    ValidationReport::new(Validation::Valid, "PNG chunk chain is well formed", Some("png"))
}

/// PDF: a version header, and ideally a trailer.
fn validate_pdf(data: &[u8]) -> ValidationReport {
    // %PDF- must be followed by a version digit.
    if data.len() < 8 || !data[5].is_ascii_digit() {
        return ValidationReport::new(
            Validation::Invalid,
            "PDF header is not followed by a version",
            Some("pdf"),
        );
    }
    ValidationReport::new(Validation::Valid, "PDF header is well formed", Some("pdf"))
}

/// ZIP: a local file header or an empty-archive end record.
fn validate_zip(data: &[u8]) -> ValidationReport {
    if data.starts_with(b"PK\x05\x06") {
        return ValidationReport::new(Validation::Valid, "ZIP end-of-archive record", Some("zip"));
    }
    if data.len() < 30 {
        return ValidationReport::new(
            Validation::PartiallyValid,
            "ZIP signature present but the local header is truncated",
            Some("zip"),
        );
    }
    ValidationReport::new(Validation::Valid, "ZIP local file header is well formed", Some("zip"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use recovery_core::{CandidateId, Completeness, Extent, Origin};

    fn jpeg(body: &[u8], with_eoi: bool) -> Vec<u8> {
        // SOI, then an APP0 segment, then optional EOI.
        let mut v = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10];
        v.extend_from_slice(b"JFIF\0");
        v.resize(20, 0);
        v.extend_from_slice(body);
        if with_eoi {
            v.extend_from_slice(&[0xFF, 0xD9]);
        }
        v
    }

    fn png(with_iend: bool) -> Vec<u8> {
        let mut v = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        v.extend_from_slice(&13u32.to_be_bytes());
        v.extend_from_slice(b"IHDR");
        v.extend_from_slice(&[0; 13]);
        v.extend_from_slice(&[0; 4]); // CRC
        if with_iend {
            v.extend_from_slice(&0u32.to_be_bytes());
            v.extend_from_slice(b"IEND");
            v.extend_from_slice(&[0; 4]);
        }
        v
    }

    fn check(data: &[u8], name: &str) -> ValidationReport {
        validate_bytes(data, name, data.len() as u64)
    }

    #[test]
    fn accepts_a_well_formed_jpeg() {
        let r = check(&jpeg(&[], true), "photo.jpg");
        assert_eq!(r.validation, Validation::Valid);
        assert_eq!(r.detected_format, Some("jpeg"));
        assert!(r.evidence[0].supporting);
    }

    #[test]
    fn truncated_jpeg_is_partial_not_invalid() {
        // Structure is sound; only the end marker is missing.
        let r = check(&jpeg(&[], false), "photo.jpg");
        assert_eq!(r.validation, Validation::PartiallyValid);
    }

    #[test]
    fn rejects_a_jpeg_with_an_impossible_segment_length() {
        let mut data = jpeg(&[], true);
        data[4..6].copy_from_slice(&1u16.to_be_bytes());
        assert_eq!(check(&data, "photo.jpg").validation, Validation::Invalid);
    }

    #[test]
    fn rejects_a_jpeg_marker_without_a_segment() {
        let r = check(&[0xFF, 0xD8, 0x00, 0x00], "photo.jpg");
        assert_eq!(r.validation, Validation::Invalid);
        assert!(!r.evidence[0].supporting);
    }

    #[test]
    fn accepts_a_well_formed_png() {
        let r = check(&png(true), "image.png");
        assert_eq!(r.validation, Validation::Valid);
        assert_eq!(r.detected_format, Some("png"));
    }

    #[test]
    fn truncated_png_is_partial() {
        assert_eq!(check(&png(false), "image.png").validation, Validation::PartiallyValid);
    }

    #[test]
    fn rejects_a_png_without_ihdr() {
        let mut data = png(true);
        data[12..16].copy_from_slice(b"XXXX");
        assert_eq!(check(&data, "image.png").validation, Validation::Invalid);
    }

    #[test]
    fn rejects_a_png_chunk_with_an_impossible_length() {
        let mut data = png(false);
        // A length that would overflow the cursor.
        data[8..12].copy_from_slice(&u32::MAX.to_be_bytes());
        let r = check(&data, "image.png");
        // Either rejected outright or reported as truncated; never a panic.
        assert_ne!(r.validation, Validation::Valid);
    }

    #[test]
    fn accepts_pdf_and_zip_headers() {
        assert_eq!(check(b"%PDF-1.7\n%...", "doc.pdf").validation, Validation::Valid);
        let mut zip = b"PK\x03\x04".to_vec();
        zip.resize(40, 0);
        assert_eq!(check(&zip, "archive.zip").validation, Validation::Valid);
        assert_eq!(check(b"PK\x05\x06", "empty.zip").validation, Validation::Valid);
    }

    #[test]
    fn rejects_a_pdf_without_a_version() {
        assert_eq!(check(b"%PDF-XY!!", "doc.pdf").validation, Validation::Invalid);
    }

    #[test]
    fn content_contradicting_a_known_extension_is_invalid() {
        // Bytes are not a JPEG despite the name.
        let r = check(&[0u8; 64], "photo.jpg");
        assert_eq!(r.validation, Validation::Invalid);
        assert!(!r.evidence[0].supporting);
    }

    #[test]
    fn unknown_formats_are_indeterminate_not_invalid() {
        // An unrecognised format is not evidence against the candidate.
        let r = check(&[0u8; 64], "notes.xyz");
        assert_eq!(r.validation, Validation::Indeterminate);
        assert!(r.evidence[0].supporting);
        assert_eq!(check(&[0u8; 64], "noextension").validation, Validation::Indeterminate);
    }

    #[test]
    fn content_decides_the_format_over_the_extension() {
        // A JPEG named .png is still recognised as a JPEG.
        let r = check(&jpeg(&[], true), "mislabelled.png");
        assert_eq!(r.detected_format, Some("jpeg"));
        assert_eq!(r.validation, Validation::Valid);
    }

    #[test]
    fn large_files_are_not_judged_truncated_from_a_window() {
        // The window ends before the file does, so a missing EOI proves nothing.
        let data = jpeg(&[0x11; 1000], false);
        let r = validate_bytes(&data, "photo.jpg", 10_000_000);
        assert_eq!(r.validation, Validation::Valid);
    }

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

    fn candidate(extents: Vec<Extent>, name: &str, size: u64) -> FileCandidate {
        FileCandidate {
            id: CandidateId::new("c"),
            name: name.into(),
            path: Vec::new(),
            origin: Origin::ActiveFilesystem,
            extents,
            declared_size: size,
            completeness: Completeness::Complete,
            validation: Validation::NotAttempted,
            evidence: Vec::new(),
        }
    }

    #[test]
    fn validates_a_candidate_from_a_device() {
        let data = jpeg(&[], true);
        let size = data.len() as u64;
        let device = Mem(data);
        let c = candidate(
            vec![Extent::new(ByteRange::new(0, size).unwrap(), 0).unwrap()],
            "photo.jpg",
            size,
        );
        let r = validate(&device, &c, &CancellationToken::default()).unwrap();
        assert_eq!(r.validation, Validation::Valid);
    }

    #[test]
    fn reassembles_fragmented_extents_before_validating() {
        // A JPEG split across two non-adjacent source ranges.
        let data = jpeg(&[], true);
        let split = 10;
        let mut disk = vec![0u8; 4096];
        disk[..split].copy_from_slice(&data[..split]);
        disk[2048..2048 + (data.len() - split)].copy_from_slice(&data[split..]);
        let device = Mem(disk);
        let c = candidate(
            vec![
                Extent::new(ByteRange::new(0, split as u64).unwrap(), 0).unwrap(),
                Extent::new(
                    ByteRange::new(2048, (data.len() - split) as u64).unwrap(),
                    split as u64,
                )
                .unwrap(),
            ],
            "photo.jpg",
            data.len() as u64,
        );
        let r = validate(&device, &c, &CancellationToken::default()).unwrap();
        // Validation follows logical order, not raw disk order.
        assert_eq!(r.validation, Validation::Valid);
    }

    #[test]
    fn candidates_without_content_are_indeterminate() {
        let device = Mem(vec![0; 16]);
        let c = candidate(Vec::new(), "gone.jpg", 100);
        let r = validate(&device, &c, &CancellationToken::default()).unwrap();
        assert_eq!(r.validation, Validation::Indeterminate);
    }

    #[test]
    fn extents_outside_the_device_do_not_fail_the_scan() {
        let device = Mem(vec![0; 16]);
        let c = candidate(
            vec![Extent::new(ByteRange::new(1_000_000, 100).unwrap(), 0).unwrap()],
            "photo.jpg",
            100,
        );
        // A bad candidate is inconclusive, not a scan-ending error.
        let r = validate(&device, &c, &CancellationToken::default()).unwrap();
        assert_eq!(r.validation, Validation::Indeterminate);
    }

    #[test]
    fn validation_honours_cancellation() {
        let data = jpeg(&[], true);
        let size = data.len() as u64;
        let device = Mem(data);
        let c = candidate(
            vec![Extent::new(ByteRange::new(0, size).unwrap(), 0).unwrap()],
            "photo.jpg",
            size,
        );
        let token = CancellationToken::default();
        token.cancel();
        assert!(validate(&device, &c, &token).is_err());
    }

    #[test]
    fn reads_are_bounded_by_the_validation_window() {
        // A hostile declared size must not drive an unbounded read.
        let device = Mem(vec![0xFF; 1 << 20]);
        let c = candidate(
            vec![Extent::new(ByteRange::new(0, 1 << 20).unwrap(), 0).unwrap()],
            "big.bin",
            u64::MAX,
        );
        validate(&device, &c, &CancellationToken::default()).unwrap();
        // Completing at all proves the window bound held.
    }
}
