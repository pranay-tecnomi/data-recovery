//! The signature registry.
//!
//! A signature is a hint, never proof. Each format declares how to recognise a
//! start, how to find its end, and the resource limits that bound the work of
//! carving it. Formats without explicit boundary validation are not registered:
//! the MVP does not claim arbitrary-file recovery.

/// How a carver determines where a candidate ends.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BoundaryStrategy {
    /// The format has an explicit terminating marker.
    Footer,
    /// The length is declared in the header and must be validated.
    DeclaredLength,
    /// The end is inferred by walking the format's internal structure.
    StructureWalk,
}

/// One registered format.
#[derive(Clone, Debug)]
pub struct Signature {
    /// Stable identifier used in evidence and manifests.
    pub id: &'static str,
    /// Extension applied to carved output.
    pub extension: &'static str,
    /// Bytes identifying the start of a candidate.
    pub header: &'static [u8],
    /// Bytes identifying the end, where the format has one.
    pub footer: Option<&'static [u8]>,
    pub boundary: BoundaryStrategy,
    /// Refuse to carve beyond this, so a missing footer cannot consume the
    /// rest of the source.
    pub max_length: u64,
    /// Candidates shorter than this are treated as false positives.
    pub min_length: u64,
    /// Higher priority wins when two signatures match at one offset.
    pub priority: u8,
}

impl Signature {
    /// Longest look-behind a detector needs, so chunk overlap can be sized.
    pub fn lookbehind(&self) -> usize {
        self.header.len().max(self.footer.map(|f| f.len()).unwrap_or(0))
    }
}

/// JPEG: SOI header, EOI footer.
pub const JPEG: Signature = Signature {
    id: "jpeg",
    extension: "jpg",
    header: &[0xFF, 0xD8, 0xFF],
    footer: Some(&[0xFF, 0xD9]),
    boundary: BoundaryStrategy::Footer,
    // 100MB covers realistic camera output without unbounded scanning.
    max_length: 100 * 1024 * 1024,
    min_length: 128,
    priority: 10,
};

/// PNG: signature header, IEND chunk terminator.
pub const PNG: Signature = Signature {
    id: "png",
    extension: "png",
    header: &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A],
    footer: Some(b"IEND"),
    boundary: BoundaryStrategy::StructureWalk,
    max_length: 100 * 1024 * 1024,
    min_length: 57,
    priority: 10,
};

/// PDF: header, with the last EOF marker terminating the document.
pub const PDF: Signature = Signature {
    id: "pdf",
    extension: "pdf",
    header: b"%PDF-",
    footer: Some(b"%%EOF"),
    boundary: BoundaryStrategy::Footer,
    max_length: 200 * 1024 * 1024,
    min_length: 32,
    priority: 8,
};

/// ZIP and its container formats: local header, end-of-central-directory.
pub const ZIP: Signature = Signature {
    id: "zip",
    extension: "zip",
    header: b"PK\x03\x04",
    footer: Some(b"PK\x05\x06"),
    boundary: BoundaryStrategy::Footer,
    max_length: 500 * 1024 * 1024,
    min_length: 30,
    priority: 6,
};

/// GIF: header, trailer byte.
pub const GIF: Signature = Signature {
    id: "gif",
    extension: "gif",
    header: b"GIF89a",
    footer: Some(&[0x3B]),
    boundary: BoundaryStrategy::Footer,
    max_length: 50 * 1024 * 1024,
    min_length: 32,
    priority: 9,
};

/// Formats supported by the MVP carver, all with explicit boundary validation.
pub const REGISTRY: &[Signature] = &[JPEG, PNG, PDF, ZIP, GIF];

/// Resource limits bounding a carve run.
#[derive(Clone, Copy, Debug)]
pub struct CarveLimits {
    /// Bytes read per chunk. Bounds memory regardless of source size.
    pub chunk_size: usize,
    /// Maximum candidates emitted, so a pathological source cannot flood the
    /// result set.
    pub max_candidates: usize,
    /// Maximum candidates carried from a single chunk.
    pub max_per_chunk: usize,
}

impl Default for CarveLimits {
    fn default() -> Self {
        Self {
            chunk_size: 4 * 1024 * 1024,
            max_candidates: 100_000,
            max_per_chunk: 10_000,
        }
    }
}

/// Largest look-behind across the registry, used to size chunk overlap so a
/// signature straddling a chunk boundary is still detected.
pub fn max_lookbehind(signatures: &[Signature]) -> usize {
    signatures.iter().map(|s| s.lookbehind()).max().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_registered_format_has_a_boundary_rule() {
        for signature in REGISTRY {
            // The MVP registers only formats whose end can be validated.
            assert!(
                signature.footer.is_some()
                    || signature.boundary != BoundaryStrategy::Footer,
                "{} claims a footer strategy without a footer",
                signature.id
            );
            assert!(signature.max_length > signature.min_length);
            assert!(!signature.header.is_empty());
        }
    }

    #[test]
    fn identifiers_and_headers_are_distinct() {
        for (i, a) in REGISTRY.iter().enumerate() {
            for b in REGISTRY.iter().skip(i + 1) {
                assert_ne!(a.id, b.id);
                // Identical headers would make detection ambiguous.
                assert_ne!(a.header, b.header);
            }
        }
    }

    #[test]
    fn lookbehind_covers_the_longest_marker() {
        assert_eq!(PNG.lookbehind(), 8);
        assert_eq!(max_lookbehind(REGISTRY), 8);
        assert_eq!(max_lookbehind(&[]), 0);
    }

    #[test]
    fn limits_are_bounded_by_default() {
        let limits = CarveLimits::default();
        assert!(limits.chunk_size > 0);
        assert!(limits.max_candidates > 0);
        assert!(limits.max_per_chunk <= limits.max_candidates);
    }
}
