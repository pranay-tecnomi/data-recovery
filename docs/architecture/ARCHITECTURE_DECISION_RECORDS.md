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
## ADR-009 Cross-platform macOS and Windows target
Supersedes the macOS-only scope in ADR-001's platform layer.

The product ships as an installable/portable application on both macOS and
Windows. The Rust recovery engine is already platform-agnostic: it parses
on-disk formats (MBR/GPT, FAT32, exFAT), which do not depend on the host OS.
Platform variance is confined to two seams: device access and privilege
elevation.

Consequence: SwiftUI is withdrawn as the presentation layer (ADR-010), and the
macOS privileged-helper/XPC specifications become one of two platform adapters
rather than the only one.

## ADR-010 Tauri presentation layer
Replaces SwiftUI. SwiftUI cannot target Windows, so retaining it would require
building and maintaining two separate UIs.

Tauri hosts a web-technology UI over the same Rust binary, so one codebase
serves both platforms and the UI links directly against the existing engine
crates rather than through an FFI boundary. The interface is dark and minimal;
the ten screens defined in the UI/UX specification remain authoritative for
information architecture, and its accessibility requirements are restated in
web terms (keyboard-first navigation, screen-reader labelling, scalable text,
contrast, and status never conveyed by colour alone).

Consequence: the FFI/ABI event-schema specifications describe an interface that
is no longer on the critical path for the desktop app; they remain relevant only
if a non-Rust host is reintroduced.

## ADR-011 PlatformDevice adapter seam
Raw-device access is isolated behind the existing BlockDevice contract, which
already states that platform adapters enforce native alignment internally.

macOS uses the /dev/rdiskN character device with an authorization-gated helper.
Windows uses \\.\PhysicalDriveN and \\.\X: handles, which require elevation and
enforce sector-aligned reads; the adapter performs aligned read-modify-trim
internally so callers keep passing arbitrary byte ranges.

No engine crate may reference a platform API directly. Source read-only access
(ADR-003) is unchanged and applies identically on both platforms.
