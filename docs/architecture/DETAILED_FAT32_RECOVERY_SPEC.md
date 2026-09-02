# Detailed FAT32 Recovery Specification

## Scope
Recover active and deleted directory entries, reconstruct cluster chains, and emit evidence-backed file candidates.

## Pipeline
Boot metadata -> geometry validation -> FAT access -> directory traversal -> deleted entries -> chain reconstruction -> extents -> validation.

## Safety
All cluster-to-byte conversions use checked arithmetic. Cluster numbers are validated against cluster count. FAT values are never trusted without bounds checks.

## Deleted entries
A deleted directory entry is evidence, not proof of recoverability. The original first filename character may be unavailable. Long-file-name fragments are associated only when sequence and checksum evidence are consistent.

## Chain reconstruction
Follow FAT links with:
- visited-cluster detection
- maximum traversal bound
- bad/reserved/end markers handled explicitly
- loop detection
- range validation

Broken chains produce partial candidates with diagnostics rather than fabricated extents.

## Candidate states
Recoverable, PartiallyRecoverable, MetadataOnly, Corrupt, Rejected.

## Tests
Golden images for contiguous files, fragmented files, deleted entries, loops, cross-links, corrupt FAT sectors and out-of-range clusters.

## Definition of done
No malformed filesystem can cause unchecked allocation, out-of-range reads, infinite chain traversal or source writes.