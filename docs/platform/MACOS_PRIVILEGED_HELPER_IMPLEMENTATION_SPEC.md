# macOS Privileged Helper Implementation Specification

## Purpose
Define a minimal privileged boundary for operations that cannot safely be performed by the app process.

## Process model
SwiftUI App → authenticated IPC/XPC → narrowly scoped Helper → approved storage operations.

## Rules
- The recovery engine is not automatically run as root.
- The helper exposes no arbitrary shell execution.
- Every request uses typed schema validation.
- Client identity/code-signing requirements are verified by the platform mechanism.
- Device identifiers are resolved and revalidated server-side.
- Write-capable operations are excluded from recovery paths.

## IPC operations
Open approved source read-only; query permitted device metadata; close handles; report typed errors.

## Authorization
Use supported macOS authorization/service-management mechanisms appropriate to the shipping target. Do not implement custom privilege escalation.

## Lifecycle
Install/update/remove according to supported platform mechanisms. Failed helper calls are recoverable and logged.

## Security tests
Unauthorized client attempts, malformed messages, path substitution, TOCTOU source changes, disconnects and helper crashes.