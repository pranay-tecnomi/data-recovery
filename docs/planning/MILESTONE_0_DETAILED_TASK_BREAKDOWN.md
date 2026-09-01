# Milestone 0 Detailed Task Breakdown

## Goal
Create a compilable, testable workspace with no recovery algorithms yet.

## M0.1 Workspace
- Create Cargo workspace.
- Add crate manifests.
- Configure formatting and linting.
Acceptance: clean build and test from repository root.

## M0.2 recovery-core domain
- Implement opaque IDs.
- Implement ByteRange and capacity validation.
- Implement typed errors.
- Implement cancellation token abstraction.
Acceptance: unit/property tests for range arithmetic and cancellation.

## M0.3 storage-io contract
- Define read-only BlockDevice trait.
- Define ReadResult and read statuses.
- Prohibit write method from public source contract.
Acceptance: compile-time API review plus tests.

## M0.4 File image adapter
- Open regular image file read-only.
- Read bounded ranges.
- Report EOF/out-of-range correctly.
Acceptance: integration tests with generated images.

## M0.5 Fault injection adapter
- Simulate partial reads, transient failures, permanent failures and disconnect.
Acceptance: deterministic tests.

## M0.6 Quality automation
- CI runs format, lint and tests.
- Dependency/license/security checks are configured where practical.
Acceptance: pull request gate is green.

## Exit criteria
No warnings in owned code, documented public APIs, all acceptance tests green.