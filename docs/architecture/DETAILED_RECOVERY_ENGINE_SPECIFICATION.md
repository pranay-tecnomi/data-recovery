# Detailed Recovery Engine Specification

## 1. Purpose
Define a safe, modular engine for analyzing block-addressable sources and reconstructing recoverable files.

## 2. Pipeline
Source → Profile → Safety Gate → Read Plan → Filesystem Analysis and/or Carving → Normalize → Deduplicate → Validate → Confidence Score → Result Store → Recovery.

## 3. Core abstractions
### BlockDevice
Read-only interface: metadata(), size(), sector_size(), read(offset,length).
Implementations: PhysicalDevice, DiskImage, PartitionView.

### ReadScheduler
Plans bounded sequential/random reads, adapts chunk size after failures, records error regions, and supports cancellation.

### ScanSession
Persistent state: source fingerprint, plan, progress, errors, checkpoints, results metadata.

## 4. Scan modes
Quick Scan uses filesystem metadata and deleted-entry analysis.
Deep Scan combines filesystem analysis with raw signature scanning.
Image-first mode is recommended for unstable media.

## 5. Carving
Chunks overlap sufficiently to detect signatures crossing boundaries.
Signature detection creates candidates.
Format-specific carvers determine extent/boundaries.
Candidates are reconstructed to a temporary destination or streamed to validation.
No temporary data may be written to the source.

## 6. Fragmentation
Filesystem metadata is preferred when extents are available.
Raw carving must mark fragmentation uncertainty rather than silently claiming integrity.

## 7. Validation
Validate magic bytes, container structure, internal offsets, size consistency, checksums where available, and parser acceptance.

## 8. Confidence
HIGH: strong metadata/structure evidence.
MEDIUM: valid structure with uncertainty.
LOW: signature found but incomplete validation.
UNKNOWN: insufficient evidence.

## 9. Failure behavior
Disconnect → pause and persist.
Read error → bounded retries, smaller reads, record bad range.
Cancellation → checkpoint safely.
Memory pressure → reduce concurrency/chunk retention.

## 10. Exit criteria
Every parser/carver has fixtures, malformed-input tests, bounds checks, and regression coverage.