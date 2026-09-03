# Pre-Coding Master Plan

## Goal
Freeze an MVP implementation baseline for a macOS-first, read-only-source data recovery system before feature implementation expands.

## MVP boundary
Supported first: file-backed images, MBR/GPT, FAT32, exFAT, controlled raw carving, validated recovery output.
Platform: macOS-first.
Not MVP-complete: APFS deleted-file recovery, HFS+ recovery, repair/write-back.

## Exit gates
No milestone starts until its inputs, contracts, test plan and acceptance criteria are satisfied.

| Gate | Evidence |
|---|---|
| Requirements | traceability matrix |
| Architecture | module contracts and diagrams |
| Safety | invariants and threat model |
| Interfaces | versioned API contracts |
| Tests | corpus and acceptance plan |
| Operations | CI and observability plan |

## Dependency order
0 core/storage safety
1 partition discovery
2 filesystem probing
3 FAT32/exFAT metadata recovery
4 validation/confidence
5 output/persistence
6 carving
7 macOS integration
8 end-to-end hardening

## Change control
Architecture changes require an ADR. Public contracts require compatibility notes and tests.