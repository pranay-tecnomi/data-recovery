# Security Threat Model

## Assets
User data, source devices, recovery destination, authorization credentials, logs, application integrity.

## Threats
- Malformed filesystem/parser exploitation.
- Integer overflow and out-of-bounds access.
- Malicious filenames/paths.
- Privilege escalation through helper IPC.
- Symlink/path traversal at recovery destination.
- Resource exhaustion from crafted media.
- Supply-chain compromise.
- Sensitive metadata leakage.

## Controls
Memory-safe implementation where practical, strict bounds checks, fuzzing, size limits, least privilege, authenticated IPC, canonicalized destination paths, atomic output handling, dependency review, signed releases, and structured redacted logs.

## Trust boundaries
External media and disk images are untrusted input.
UI is not authorization for arbitrary privileged operations.
Recovery destinations are untrusted until validated.

## Security testing
Fuzz filesystem parsers and carvers, test privilege boundaries, inject malformed metadata, test cancellation/disconnect, and review dependencies before release.