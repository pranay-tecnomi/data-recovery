# Data Recovery

A macOS-first data recovery system.

## Development status

Milestone 0 is in progress.

Current foundation:
- Rust workspace
- recovery-core
- read-only storage I/O contract
- file-backed disk image adapter
- range validation
- cancellation primitive

No physical-device write functionality is exposed by the recovery path.
