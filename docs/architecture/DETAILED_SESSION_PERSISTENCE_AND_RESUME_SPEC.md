# Detailed Session Persistence and Resume Specification

## Session identity
Every persisted session has schema version, immutable source fingerprint, scan configuration and generation number.

## Checkpoints
Persist only consistent state. Use temporary write plus atomic replacement where supported.

## Resume
Validate schema compatibility, source fingerprint and capacity before reconstructing work queues.

## Corruption
Invalid checkpoints fail closed. The engine must not resume from partially decoded state.

## Range accounting
Completed, pending and terminal-unreadable ranges must be normalized and non-overlapping under their accounting policy.

## Tests
Interrupted writes, corrupt JSON/binary metadata, version migration, source mismatch and cancellation at checkpoint boundaries.