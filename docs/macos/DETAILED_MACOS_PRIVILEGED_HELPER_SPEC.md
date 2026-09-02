# Detailed macOS Privileged Helper Specification

## Boundary
The GUI runs unprivileged. Privileged operations are isolated behind a narrow authenticated IPC boundary.

## Principles
- least privilege
- explicit operation allowlist
- caller authentication
- typed requests
- no arbitrary command execution
- no source-write API

## Operations
Enumerate authorized storage metadata and open approved raw sources read-only where platform authorization permits.

## XPC
Requests are versioned and validated before privileged execution. Paths and identifiers are canonicalized and revalidated server-side.

## Testing
Authorization denial, malformed requests, confused-deputy attempts, disconnects and privilege-boundary integration tests.