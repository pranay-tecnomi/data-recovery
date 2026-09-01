# FAT32 Recovery Specification

## Scope
Logical analysis and deleted-file recovery for valid or partially damaged FAT32 volumes.

## Structures
Parse BPB, FSInfo where relevant, FAT copies, root/subdirectories and directory entries.

## Detection
Validate boot-sector signatures and geometry conservatively. Derived offsets must be bounds-checked.

## Active files
Resolve first cluster and FAT chains with loop detection and maximum traversal bounds.

## Deleted files
Analyze deleted directory entries and associated cluster chains. A deleted file is recoverable only when required metadata/data remains sufficiently intact.

## Fragmentation
If the original chain remains inferable, reconstruct it. Otherwise contiguous inference is explicitly low-confidence and must be validated.

## Corruption
Compare FAT copies when available; never blindly trust either copy.

## Validation
File size, chain capacity, cluster bounds and format-specific validation.

## Tests
Clean, deleted, fragmented, overwritten, cyclic FAT, invalid cluster and truncated-image fixtures.