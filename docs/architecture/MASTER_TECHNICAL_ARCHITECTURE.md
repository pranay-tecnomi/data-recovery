# Master Technical Architecture

## Layers
Presentation (Tauri: dark, minimal web UI)
→ Application (orchestration, jobs, safety policies)
→ Domain (BlockDevice, ScanSession, FileCandidate)
→ Recovery Engine (imaging, filesystem, carving, validation)
→ Platform (PlatformDevice adapters: macOS and Windows, authorization, IPC)
→ Hardware (SSD, HDD, USB, SD, disk images)

## Technology direction
Tauri hosts the dark, minimal UI on macOS and Windows over one Rust binary.
Rust handles binary parsing, block processing, carving, filesystem modules, validation, and recovery algorithms.
Platform-specific raw-device access and elevation are isolated behind the PlatformDevice seam; no engine crate references a platform API directly.

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
9. Cross-platform macOS and Windows target.
10. Tauri presentation layer.
11. PlatformDevice adapter seam.
5. Modular filesystem implementations.