# macOS Platform and Privilege Specification

This specification covers the macOS adapter only; see
WINDOWS_PLATFORM_SPECIFICATION.md for the Windows equivalent. Both realise the
PlatformDevice seam defined in ADR-011.

## Principles
Use public supported macOS mechanisms, least privilege, explicit authorization, and strict separation between UI and elevated operations.

## Responsibilities
App: UI, orchestration, result presentation.
Platform adapter: device discovery, mount state, authorization integration.
Privileged helper: only narrowly scoped operations requiring elevated access.

## Rules
- Do not run the recovery engine wholesale as root.
- Validate every IPC request and source identity.
- Prefer read-only handles.
- Never expose arbitrary command execution through IPC.
- Treat the active startup disk as a special restricted workflow.
- Encryption must not be bypassed; recovery depends on legitimately available decrypted access/keys.

## Device lifecycle
Discover → identify stable attributes → authorize if required → open read-only → monitor disconnect → close deterministically.

## Packaging
Code signing, sandbox/entitlement decisions, helper authorization, notarization, and update security must be designed before release.

## Acceptance
Privilege boundaries are tested, IPC is authenticated and schema-validated, and no normal recovery path requires broad persistent root access.