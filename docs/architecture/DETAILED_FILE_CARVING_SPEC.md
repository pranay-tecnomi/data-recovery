# Detailed File Carving Specification

## Purpose
Recover candidate files directly from raw byte ranges when filesystem metadata is missing or unreliable.

## Architecture
Signature scanner -> candidate boundary finder -> format validator -> extent model -> confidence scoring.

## Rules
- Scan bounded ranges only.
- Signatures are hints, never sufficient proof.
- Parsers impose explicit maximum lengths and nesting limits.
- Carving never overwrites source data.
- Overlapping candidates retain provenance.

## MVP
Implement a plugin registry and support only formats with explicit boundary validation. Do not claim arbitrary-file recovery.

## Fragmentation
A simple contiguous carve must report low confidence when fragmentation cannot be excluded. Fragment reconstruction is a separate strategy requiring format-specific evidence.

## Tests
Signature false positives, truncated files, embedded signatures, overlapping candidates and adversarial lengths.