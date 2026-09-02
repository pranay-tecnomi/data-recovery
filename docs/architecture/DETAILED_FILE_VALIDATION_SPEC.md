# Detailed File Validation Specification

## Purpose
Determine whether a candidate's reconstructed bytes are structurally plausible.

## Result
Valid, PartiallyValid, Invalid, Indeterminate plus evidence and bounded diagnostics.

## Validation layers
1. extent bounds
2. size consistency
3. format structure
4. checksums when available
5. semantic consistency where bounded

Validation must stream data when possible and must not require unbounded memory.

## Confidence
Validation evidence modifies but does not erase source provenance. A valid carved file and a metadata-recovered file retain different evidence histories.

## Tests
Valid corpus, corruption corpus, truncation, hostile size fields and cancellation.