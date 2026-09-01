# NTFS Recovery Specification

## Status
Later module; not required for macOS-only MVP but valuable for external Windows-formatted drives.

## Scope
Boot sector, MFT records, attributes, data runs, $Bitmap and deleted records.

## Recovery
Validate record fixups and attribute bounds. Reconstruct nonresident streams from validated data runs. Deleted records are candidates only until allocation and content evidence are checked.

## Tests
Malformed runlists, sparse/compressed cases, deleted MFT records, fragmented files and corruption fixtures.