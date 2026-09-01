# exFAT Recovery Specification

## Scope
Read-only analysis of exFAT volumes and recovery of files where metadata/allocation evidence remains.

## Structures
Validate boot region, parse allocation bitmap, file directory entry sets, stream extension and filename entries.

## Entry sets
A file is reconstructed only from a validated set of related entries. Checksums and ordering are used as evidence, not as a reason to overclaim certainty.

## Allocation
Use allocation bitmap and stream metadata to determine whether contiguous or fragmented reconstruction is supported.

## Deleted recovery
Search inactive/deleted entry evidence conservatively and validate associated clusters against current allocation state.

## Fragmentation
Honor extent/allocation information when available. Do not assume contiguous allocation without evidence.

## Tests
Deleted entry sets, damaged checksums, bitmap conflicts, fragmented files, malformed name entries and bounds attacks.