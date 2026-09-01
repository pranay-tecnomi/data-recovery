# Software Requirements Specification

## Functional requirements
- FR-001 Device discovery.
- FR-002 Source profiling.
- FR-003 Read-only source protection.
- FR-004 Startup disk detection.
- FR-005 Filesystem-aware quick scan.
- FR-006 Raw deep scan.
- FR-007 Modular file signatures.
- FR-008 Candidate validation.
- FR-009 Confidence classification.
- FR-010 Preview for supported formats.
- FR-011 Destination validation.
- FR-012 Recovery execution and reporting.
- FR-013 Disk imaging.
- FR-014 Failure detection and reporting.

## Non-functional requirements
- Source safety.
- Responsive UI.
- Stability against malformed metadata.
- Chunked processing and bounded memory.
- Resumability where feasible.
- Structured, privacy-conscious logs.

## Filesystem roadmap
MVP: FAT32, exFAT, raw carving.
Later: NTFS, HFS+.
Advanced research: APFS.

## Completion criteria
A feature is complete only after implementation, unit and integration tests, error-path testing, regression coverage, documentation updates, and review.