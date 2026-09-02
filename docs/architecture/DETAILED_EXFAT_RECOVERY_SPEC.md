# Detailed exFAT Recovery Specification

## Scope
Parse exFAT boot metadata, allocation bitmap, file directory entry sets and stream extensions to recover active and recoverable deleted candidates.

## Pipeline
Boot region -> geometry -> root directory -> entry sets -> stream metadata -> allocation evidence -> extents -> validation.

## Entry-set validation
Primary and secondary entry counts are bounded. Entry-set checksums are verified where applicable. Invalid sets remain diagnostic evidence and are not silently treated as valid.

## Allocation
Respect contiguous/no-FAT-chain semantics when indicated. Otherwise validate FAT chains with loop and bounds protection.

## Deleted recovery
Deleted metadata is weaker evidence and must be scored accordingly. Allocation bitmap state is evidence, not a guarantee that original content remains intact.

## Tests
Fixtures for contiguous files, fragmented chains, invalid checksums, corrupt bitmaps, deleted entries and malformed secondary counts.

## Definition of done
All metadata-derived extents are bounded and every confidence decision retains evidence.