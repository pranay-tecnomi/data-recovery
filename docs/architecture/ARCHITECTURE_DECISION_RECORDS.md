# Architecture Decision Records

## ADR-001 Layered architecture
Separate UI, orchestration, domain, recovery core, and platform access.

## ADR-002 Generic BlockDevice
Treat physical devices and images through one read abstraction.

## ADR-003 Read-only source policy
No intentional writes to recovery sources.

## ADR-004 Rust recovery core
Use Rust for low-level parsing and memory-safety benefits; isolate FFI.

## ADR-005 External-first MVP
Reduce APFS/startup-disk complexity during initial validation.

## ADR-006 Modular filesystem and carver plugins
Allow incremental support without coupling the entire engine.

## ADR-007 Image-first for unstable media
Prefer preserving readable data before repeated analysis.

## ADR-008 Evidence-based confidence
Use qualitative classes backed by validation evidence rather than misleading percentages.