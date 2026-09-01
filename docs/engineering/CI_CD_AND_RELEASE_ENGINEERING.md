# CI/CD and Release Engineering Specification

## Pull request gates
Formatting, linting, unit tests, integration tests where environment permits, dependency/security checks, and documentation checks.

## Branching
main is protected.
Feature work uses short-lived branches.
Release tags are immutable.

## Build artifacts
Produce reproducible versioned artifacts with commit SHA and dependency lock information.

## macOS release
Automate build, test, code signing, notarization, packaging, and verification using securely stored credentials.

## Release stages
Internal → closed beta → wider beta → stable.

## Rollback
Maintain prior stable release availability and a documented incident process.

## Observability
Crash/error reporting must be opt-in or privacy-respecting and must not upload recovered file contents.