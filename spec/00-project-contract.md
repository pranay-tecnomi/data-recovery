# Project Contract

## MVP
Cross-platform (macOS and Windows) data recovery with file-backed images first, MBR/GPT discovery, FAT32/exFAT recovery, bounded carving, validation, safe output, and resumable sessions. Image-based recovery runs unprivileged on both platforms; raw physical-device access requires elevation and is isolated behind the PlatformDevice seam (ADR-009/010/011).

## Explicit exclusions
APFS deleted-file recovery, HFS+ recovery, NTFS recovery, filesystem repair, source write-back, unrestricted privileged command execution, and volume locking/dismounting.

## Non-negotiable outcome
A source is evidence. Recovery paths must never mutate it.

## Coding language
Rust workspace for engine components; Tauri for the dark, minimal desktop UI on both platforms. Platform integration follows the macOS and Windows privilege-boundary specifications.

## Definition of done
A feature is done only when its acceptance tests pass and integration contracts remain compatible.