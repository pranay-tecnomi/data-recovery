# Core Domain Model Specification

## Identity
SourceId: opaque runtime identifier.
SourceFingerprint: stable evidence used to reject unsafe resume.
ScanSessionId, CandidateId and RecoveryJobId: opaque UUID-like identifiers.

## Value objects
ByteRange { offset: u64, length: u64 }. Construction fails on overflow or invalid bounds when capacity is known.
SectorGeometry { logical_size: u32, physical_size: Option<u32> }.

## Source
SourceDescriptor contains id, kind, capacity, geometry, display metadata and read capability. Display metadata is never trusted for identity.

## Partition
Partition { range, table_kind, index, attributes }. Range must lie entirely inside source capacity.

## Extent
Extent { source_range, logical_offset }. Extents for a reconstructed stream must be non-overlapping in logical output space.

## FileCandidate
Candidate { id, origin, type_hint, extents, declared_size, validation, confidence, evidence }.
Origin is Filesystem or Carved.

## ScanSession
Contains source fingerprint, plan, state, checkpoint references and immutable creation configuration.

## RecoveryJob
Contains selected candidate IDs, destination identity, state and item results.

## Invariants
IDs are opaque. User-visible names are not keys. Evidence is append-only for auditability. Confidence is derived from evidence, not manually asserted.