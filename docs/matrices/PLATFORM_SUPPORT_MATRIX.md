# Platform Support Matrix

| Platform | Application | Recovery Engine | Status |
|---|---|---|---|
| macOS | Tauri (dark, minimal) | Rust | MVP |
| Windows | Tauri (dark, minimal) | Rust | MVP |
| Linux | No | Portable core possible | Future |

The recovery engine is platform-agnostic: it parses on-disk formats that do not
depend on the host OS. Platform variance is confined to the PlatformDevice
adapter (raw-device access) and privilege elevation. See ADR-009/010/011.

## Minimum versions
| Platform | Minimum | Notes |
|---|---|---|
| macOS | 12 Monterey | Apple silicon and Intel |
| Windows | 10 (1809, 64-bit) | Windows 11 supported |

## Privilege model
| Platform | Device path | Elevation |
|---|---|---|
| macOS | /dev/rdiskN | Authorization-gated helper |
| Windows | \\.\PhysicalDriveN, \\.\X: | UAC elevation, administrator |

Disk-image sources require no elevation on either platform, so the image-first
MVP path runs unprivileged.
