# Storage I/O and Block Device Specification

## Purpose
Define the exact read contract used by every recovery component.

## Invariants
1. Recovery sources are opened read-only.
2. Offsets and lengths are bounds-checked before I/O.
3. No implicit retries occur below the scheduler layer.
4. Partial reads are explicit.
5. Device identity is revalidated before resume.

## BlockDevice contract
Metadata: stable identifier, capacity, logical/physical sector sizes where available, source kind, read-only capability.
read(offset,length): returns bytes read plus status for the requested range.
Reads beyond capacity fail without wraparound.

## Alignment
The abstraction accepts byte offsets. Platform adapters enforce native alignment requirements internally. Callers must never assume 512-byte sectors.

## Errors
OutOfRange, PermissionDenied, Disconnected, TransientReadFailure, PermanentReadFailure, Cancelled, Unsupported.

## Concurrency
Implementations declare whether concurrent reads are supported. The scheduler owns parallelism.

## Cancellation
Cancellation is cooperative. No successful read may be discarded silently; callers receive a terminal Cancelled status.

## Resume
A session stores source fingerprint, capacity, sector geometry and selected ranges. Mismatch requires explicit restart.

## Tests
Boundary offsets, zero-length reads, overflow attempts, disconnects, partial reads, cancellation, images and partition views.