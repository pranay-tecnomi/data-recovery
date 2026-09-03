# Project Contract

## MVP
macOS-first data recovery with file-backed images first, MBR/GPT discovery, FAT32/exFAT recovery, bounded carving, validation, safe output, and resumable sessions.

## Explicit exclusions
APFS deleted-file recovery, HFS+ recovery, filesystem repair, source write-back, unrestricted privileged command execution.

## Non-negotiable outcome
A source is evidence. Recovery paths must never mutate it.

## Coding language
Rust workspace for engine components. macOS integration follows the existing privilege-boundary specification.

## Definition of done
A feature is done only when its acceptance tests pass and integration contracts remain compatible.