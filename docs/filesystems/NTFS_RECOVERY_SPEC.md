# NTFS Recovery Specification

## Status
Later module. Not in the MVP, which covers FAT32 and exFAT on both platforms. NTFS is the dominant Windows volume format, so it is the highest-priority post-MVP filesystem; a Windows build that reads only FAT32/exFAT will not meet most Windows users' expectations.

## Scope
Boot sector, MFT records, attributes, data runs, $Bitmap and deleted records.

## Recovery
Validate record fixups and attribute bounds. Reconstruct nonresident streams from validated data runs. Deleted records are candidates only until allocation and content evidence are checked.

## Tests
Malformed runlists, sparse/compressed cases, deleted MFT records, fragmented files and corruption fixtures.