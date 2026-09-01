# Module API Contracts

## storage-io
Provides read-only BlockDevice operations and source metadata. Never imports filesystem modules.

## partition
Input: BlockDevice + capacity. Output: validated partition candidates and typed diagnostics.

## filesystem-fat / filesystem-exfat
Input: partition-scoped read abstraction. Output: metadata and FileCandidate evidence. No destination writing.

## carving
Input: bounded byte stream/ranges. Output: candidate evidence and extents where supported.

## validators
Input: candidate bytes/stream under explicit limits. Output: validation evidence, never recovery decisions alone.

## recovery-core
Owns orchestration, scan plans, cancellation, state transitions and confidence aggregation.

## session-store
Persists core-owned serializable state; does not interpret filesystem structures.

## ffi
Maps stable request/response/event contracts only.

## macOS app
Owns presentation, user choices and destination UX; never parses raw filesystem structures.

Cross-module communication uses explicit domain types and typed errors.