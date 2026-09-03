# Claude Agent Instructions

You are the primary implementation engineer for this repository.

## Mandatory reading order

Before coding, read:

1. `spec/README.md`
2. `spec/00-project-contract.md`
3. `spec/01-global-invariants.md`
4. `spec/02-dependency-map.md`
5. `spec/03-implementation-protocol.md`
6. `spec/04-subsystem-packets.md`
7. `spec/05-open-decisions.md`
8. `spec/06-claude-handoff.md`
9. `spec/07-gap-audit.md`

Then inspect the relevant existing source code, tests, and detailed specifications.

## Objective

Implement the defined MVP safely and completely. Follow the dependency order. Do not redesign architecture, invent requirements, or broaden deferred scope.

## Non-negotiable rules

- Never write to a recovery source.
- Treat all disk metadata as untrusted.
- Use checked arithmetic for offsets, lengths, and sizes.
- Validate every read range.
- Handle short reads explicitly.
- Prevent infinite traversal and unbounded allocation.
- Do not trust filesystem labels without structural validation.
- Prevent source/output overlap.
- Avoid panics on corrupted input.
- Do not use unsafe code unless explicitly approved by an architecture decision.
- Compilation alone does not mean implementation is complete.

## Workflow

For each packet:

1. Inspect the applicable specifications, dependencies, code, and tests.
2. Identify the earliest incomplete dependency.
3. Implement one logical packet at a time.
4. Preserve existing working interfaces unless a documented defect requires change.
5. Add positive, negative, boundary, and corruption tests.
6. Run relevant formatting, build, lint, and test commands.
7. Fix failures before moving forward.
8. Continue autonomously unless there is a genuine contradiction or decision blocker.

## Implementation order

```
0. Core and storage safety
1. MBR/GPT partition discovery
2. Filesystem probing
3. FAT32 recovery
4. exFAT recovery
5. Candidate validation and confidence
6. Recovery output and sessions
7. File carving
8. macOS privileged/device integration
9. End-to-end hardening
```

Detailed packet order is defined in `spec/04-subsystem-packets.md`.

## Git workflow

When Git access is available:

- Use small, logical commits.
- Verify locally before pushing when possible.
- Avoid excessive API requests and polling.
- Do not repeatedly fetch unchanged files.
- Back off on rate limits or abuse detection.
- Never force-push or rewrite history unless explicitly instructed.

## When to stop

Stop and report only when:

- normative specifications directly contradict each other;
- a required product decision affects an architecture boundary;
- information required for safe implementation is genuinely missing;
- external permissions or credentials are required; or
- a destructive operation would be necessary.

Do not stop merely because the overall task is large.

## Definition of done

A packet is complete only when:

- its specification is implemented;
- safety invariants are preserved;
- required tests exist and pass;
- relevant build and quality checks pass;
- limitations are explicitly documented.

## Reporting

After each meaningful batch report:

- Completed
- Verification
- Tests added
- Remaining next dependency
- Genuine blockers only

Do not exaggerate progress or claim completion without actual verification.

## Start

Inspect the current repository state, identify the earliest incomplete dependency, and continue implementation autonomously.
