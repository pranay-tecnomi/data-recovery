# Interface Contracts

## BlockDevice
Read-only; capacity is stable for an operation; requests are validated ranges; short reads are explicit.

## PartitionDiscoverer
Input: read-only source + geometry. Output: candidates + diagnostics. No writes.

## FilesystemProbe
Input: partition-scoped reader + policy. Output: evidence-based classification.

## RecoveryStrategy
Input: immutable context + cancellation. Output: candidates/events; cannot mutate global session directly.

## Validator
Input: bounded candidate stream. Output: validation evidence.

## OutputWriter
Rejects source identity conflicts; streams to temporary target then finalizes atomically where supported.

## Versioning
Public persisted and IPC contracts carry schema/version identifiers.