# Detailed Recovery Engine Specification

## 1. Purpose and scope

The recovery engine is the orchestration layer for safe, read-only analysis of storage sources and controlled writing of recovered output to a separately validated destination. It owns scan planning, source identity checks, cancellation, progress, checkpointing, candidate normalization, validation aggregation, confidence calculation, and terminal outcomes.

The engine does not parse filesystem structures directly and does not expose source-write operations.

### MVP scope
- File-backed disk images as the first source adapter.
- Partition discovery delegated to `partition`.
- FAT32 and exFAT recovery delegated to filesystem modules.
- Raw carving delegated to `carving`.
- Candidate validation delegated to `validators`.
- Persistent session support delegated to `session-store`.

Physical-device access and additional filesystems remain outside this document's implementation commitments until their platform adapters and safety requirements are finalized.

## 2. Normative safety invariants

1. The engine MUST treat every source byte and metadata field as untrusted.
2. The engine MUST NOT provide an API that writes to the selected recovery source.
3. Every range MUST be validated before it reaches a source reader.
4. Arithmetic used to derive offsets, lengths, sector counts, cluster ranges, or progress totals MUST be checked for overflow.
5. Resume MUST be rejected when the current source fingerprint does not match the persisted session fingerprint.
6. Recovery output MUST NOT target the same source identity or any overlapping backing object.
7. Cancellation MUST be cooperative and observable at bounded work intervals.
8. A terminal session state MUST be emitted exactly once.
9. Parser or validator failure MUST become evidence/diagnostics, not process termination.
10. Progress percentages MUST never decrease for a single phase and MUST remain within 0..=100.

## 3. Core responsibilities

### 3.1 Source assessment
The engine accepts a `SourceDescriptor`, obtains stable capacity and read capability, and requests a source fingerprint. Display names, mount names, and paths are not sufficient identity evidence.

### 3.2 Safety policy
Before scanning, the engine evaluates:
- source accessibility;
- source capacity consistency;
- read-error history observed during the session;
- requested scan depth;
- whether an image-first workflow is required by policy.

Policy output is one of `Permit`, `RequireImaging`, `Restrict`, or `Reject`, with typed reasons.

### 3.3 Scan planning
A `ScanPlan` is immutable after transition to `Running`. It specifies:
- target source or partition ranges;
- enabled strategies;
- chunk size bounds;
- checkpoint cadence;
- validation budget;
- cancellation polling budget;
- memory budget.

The planner may divide work into deterministic `WorkUnit`s. Each work unit has a stable identifier and bounded byte range.

### 3.4 Strategy orchestration
The engine invokes strategies through contracts rather than concrete filesystem implementations:
- partition enumeration;
- filesystem analysis;
- raw carving;
- candidate validation.

A strategy returns candidates, evidence, diagnostics, and progress. It cannot mutate session state directly.

## 4. State model

### 4.1 Session states
`Created -> Assessing -> Ready -> Running -> Completing -> Completed`

Alternative paths:
- `Assessing -> Failed`
- `Ready -> Cancelled`
- `Running -> Paused -> Running`
- `Running -> Cancelled`
- `Running -> Failed`

`Completed`, `Cancelled`, and `Failed` are terminal.

### 4.2 Transition rules
- `start` is valid only from `Ready`.
- `pause` is valid only from `Running` and completes the current atomic work unit first.
- `resume` is valid only from `Paused` after fingerprint verification.
- `cancel` is idempotent and may be requested from any non-terminal state.
- a second terminal transition is rejected as an internal invariant violation.

## 5. Work scheduling and resource bounds

The MVP scheduler is single-session and deterministic. Parallelism is optional and MUST NOT be introduced until ordering, memory accounting, and checkpoint semantics are explicitly tested.

Each work unit MUST declare estimated input bytes. The engine enforces:
- maximum in-flight bytes;
- maximum candidate batch size;
- maximum validator input size;
- maximum diagnostic payload size.

Unbounded buffering of source data is prohibited.

## 6. Read semantics

The engine uses the read-only `BlockDevice` contract. A read request consists of a validated `ByteRange` and caller-provided output storage.

Outcomes are classified conceptually as:
- full read;
- partial read;
- transient read failure;
- permanent read failure;
- disconnect;
- cancellation.

The concrete `storage-io` error taxonomy MUST preserve enough structure for retry policy; string-only classification is insufficient for production policy.

### Retry policy
The engine retries only errors classified retryable. Retry count, backoff, and minimum subdivision size are plan parameters. On repeated failure, it records a bad range and continues only when the active strategy permits sparse/error-aware progress.

The engine MUST NOT retry indefinitely.

## 7. Candidate pipeline

Strategy output flows through:

`Discover -> Normalize -> Deduplicate -> Validate -> Score -> Store`

### 7.1 Normalization
Normalization assigns stable session-local IDs, canonicalizes type hints, validates extent ordering, and rejects impossible declared sizes.

### 7.2 Deduplication
Candidates are compared using source extents first, then content/structural evidence when available. Name equality is not deduplication evidence.

### 7.3 Validation
Validators return evidence with explicit status such as `Valid`, `PartiallyValid`, `Inconclusive`, or `Invalid`. A validator timeout or resource-limit hit is `Inconclusive`, not `Invalid`.

### 7.4 Confidence
Confidence is derived from weighted evidence and policy. The score MUST retain its evidence components so a UI can explain why a candidate received its classification.

The engine MUST NOT claim a candidate is fully recoverable solely because it was detected.

## 8. Progress and events

Events are ordered per session and include a monotonic sequence number.

Minimum event classes:
- `StateChanged`
- `PhaseStarted`
- `Progress`
- `CandidateBatch`
- `Diagnostic`
- `CheckpointSaved`
- `Completed`
- `Cancelled`
- `Failed`

`Progress` includes phase, completed units, total units when known, bytes processed when meaningful, and optional rate estimate. Unknown totals MUST remain explicitly unknown rather than fabricated.

Candidate batches are bounded; a slow consumer cannot force unbounded memory growth.

## 9. Checkpoint and resume

A checkpoint contains:
- schema version;
- session ID;
- source fingerprint;
- immutable plan digest;
- completed work-unit IDs or deterministic frontier;
- candidate-store reference;
- diagnostics summary;
- checkpoint sequence.

Checkpoint publication MUST be atomic from the session-store perspective. A torn or corrupt checkpoint is ignored in favor of the previous valid checkpoint.

Resume performs this order:
1. load and validate schema;
2. verify source fingerprint;
3. verify plan compatibility;
4. restore candidate-store reference;
5. restore frontier;
6. enter `Ready` or `Paused` before `Running`.

Resume never assumes a source path alone proves identity.

## 10. Imaging integration

When policy requires imaging, the engine treats imaging as a distinct job with its own checkpoints and read-error map. The original source remains read-only. Subsequent scans use the completed image when available.

The engine records unreadable ranges explicitly. An image containing unreadable regions MUST carry completeness metadata; downstream strategies must be able to distinguish absent data from successfully read zero bytes.

## 11. Recovery job boundary

Scanning and writing recovered files are separate state machines.

A `RecoveryJob` begins only after destination validation succeeds. Destination validation checks identity, available capacity, writability, and path collision policy.

For each item:
1. open a new destination file according to collision policy;
2. stream extents without loading the whole file when possible;
3. check cancellation between bounded writes;
4. optionally perform post-write validation;
5. commit item result.

A failed item does not automatically fail the entire job unless policy marks it fatal.

## 12. Error taxonomy requirements

Errors crossing module boundaries MUST be typed and contextual. At minimum, distinguish:
- invalid argument;
- range overflow;
- source out of range;
- retryable I/O;
- permanent I/O;
- disconnect;
- permission;
- cancellation;
- malformed structure;
- resource limit;
- checkpoint corruption;
- source identity mismatch;
- destination safety violation;
- internal invariant violation.

Human-readable messages are supplementary and are not the primary control mechanism.

## 13. Public orchestration contracts

Conceptual API:

```text
create_session(source, request) -> SessionHandle
assess(session) -> Assessment
configure(session, plan) -> Ready
start(session) -> EventStream
pause(session)
resume(session)
cancel(session)
checkpoint(session)
restore(checkpoint, source) -> SessionHandle
list_candidates(session) -> paged candidates
create_recovery_job(session, selection, destination) -> JobHandle
start_recovery(job) -> EventStream
```

Concrete Rust types may differ, but they MUST preserve the ownership and safety boundaries above.

## 14. Concurrency rules

- Session state has a single logical transition authority.
- Event publication must not hold source I/O locks.
- A blocking source read must not prevent cancellation from being requested.
- Multiple consumers may observe events, but no consumer may mutate engine state through the event stream.
- Candidate persistence and checkpoint persistence must define ordering explicitly.

## 15. Testing requirements

### Unit
- state transition legality;
- range arithmetic;
- confidence aggregation;
- retry classification;
- checkpoint compatibility.

### Integration
- normal scan over generated disk image;
- malformed partition metadata;
- partial reads;
- transient failure followed by success;
- permanent bad ranges;
- disconnect during scan;
- cancellation during long scan;
- pause/checkpoint/resume;
- source fingerprint mismatch;
- destination equals source rejection.

### Property/fuzz
Binary parsers and range construction are high-priority fuzz targets. Malformed input must not panic, loop indefinitely, or allocate without bound.

## 16. Milestone mapping

Milestone 0 establishes the domain types, read-only I/O boundary, cancellation primitives, deterministic faults, and CI required by this specification.

Milestone 1 adds partition discovery behind the `partition` contract.

Filesystem recovery, carving, validation, persistence, and UI integration are added only after their contracts and corpus tests exist.

## 17. Open decisions

The following require ADRs before implementation affects public contracts:
- exact source fingerprint algorithm and evidence set;
- checkpoint serialization format and migration policy;
- event transport/backpressure mechanism for FFI;
- concrete retry backoff and range subdivision policy;
- confidence weighting model;
- physical-device adapter and macOS privilege boundary details.
