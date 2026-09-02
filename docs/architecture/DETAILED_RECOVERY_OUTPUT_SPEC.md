# Detailed Recovery Output Specification

## Safety
Recovered output must never target the source device. Destination identity checks occur before job execution.

## Pipeline
Candidate selection -> destination validation -> collision policy -> streaming copy -> validation metadata -> atomic finalization.

## Naming
Original names are untrusted metadata. Names are sanitized and collisions resolved deterministically.

## Partial files
Partial recovery is explicitly labeled and never silently presented as complete.

## Atomicity
Write to temporary destination, flush according to policy, then atomically finalize where supported.

## Manifest
Record candidate ID, provenance, source ranges, validation state, confidence and output status.

## Tests
Source/destination conflict, collisions, short writes, cancellation and partial candidates.