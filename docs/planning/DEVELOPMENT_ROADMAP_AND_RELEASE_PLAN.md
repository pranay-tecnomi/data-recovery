# Development Roadmap and Release Plan

## Phase 0 — Foundation
Finalize specifications, ADRs, threat model, test corpus design, repository conventions.

## Phase 1 — Core I/O
BlockDevice, disk-image source, bounded reader, cancellation, error map, tests.

## Phase 2 — Scan Framework
Sessions, progress events, chunk scheduler, signature registry.

## Phase 3 — MVP Recovery
FAT32/exFAT analysis, selected file carvers, validation, result store.

## Phase 4 — macOS Integration
Device discovery, authorization boundary, Tauri workflows on macOS and Windows.

## Phase 5 — Imaging and Reliability
Image creation, resume/checkpoints, fault injection.

## Phase 6 — Security and Beta
Fuzzing, threat-model remediation, performance profiling, closed beta.

## Release gates
No known source-write defect.
Critical security issues resolved.
Target corpus metrics met.
Crash and disconnect behavior tested.
Upgrade, signing, and rollback procedures validated.