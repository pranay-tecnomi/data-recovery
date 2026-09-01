# Master Technical Architecture

## Layers
Presentation (SwiftUI)
→ Application (orchestration, jobs, safety policies)
→ Domain (BlockDevice, ScanSession, FileCandidate)
→ Recovery Engine (imaging, filesystem, carving, validation)
→ Platform (macOS APIs, authorization, IPC)
→ Hardware (SSD, HDD, USB, SD, disk images)

## Technology direction
SwiftUI handles UI and macOS integration.
Rust handles binary parsing, block processing, carving, filesystem modules, validation, and recovery algorithms.

## BlockDevice abstraction
Core operations:
- get metadata
- get size
- get sector size
- read(offset, length)

Implementations may represent physical devices, disk images, or partition views.

## Scan pipeline
Select Source → Device Profile → Safety Policy → Scan Plan → Quick/Deep Scan → Normalize → Validate → Store Results.

## Carving pipeline
Block Reader → Chunk Manager → Signature Detector → Format Carver → Boundary Detection → Reconstruction → Validation → FileCandidate.

## Safety policy
Evaluates startup disk, source/destination conflicts, failing state, encryption, mount state, and destination validity.

## Privilege boundary
The UI remains unprivileged. Elevated operations are isolated behind controlled IPC and minimal scope.

## Key ADRs
1. Layered architecture.
2. Generic BlockDevice abstraction.
3. Read-only source policy.
4. External storage prioritized for MVP.
5. Modular filesystem implementations.