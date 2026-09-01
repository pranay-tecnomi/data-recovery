# Coding Baseline and Module Boundaries

## Repository target
apps/macos/ — SwiftUI presentation and orchestration
crates/recovery-core/ — domain types and orchestration
crates/storage-io/ — BlockDevice and platform-neutral reads
crates/partition/ — GPT/MBR
crates/filesystem-fat/ — FAT32
crates/filesystem-exfat/ — exFAT
crates/carving/ — registry and shared framework
crates/validators/ — format validation
crates/session-store/ — persistence abstractions
crates/ffi/ — stable Swift/Rust boundary
tests/corpus/ — manifests and generated fixtures

## Dependency rule
UI → FFI → recovery-core → storage/filesystem/carving. No parser depends on UI or macOS frameworks. No recovery module may write to a source abstraction.

## Definition of done
Code + unit tests + malformed-input tests + integration coverage + documentation update.