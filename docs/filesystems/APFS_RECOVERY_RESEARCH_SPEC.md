# APFS Recovery Research Specification

## Status
Research track. Do not market deleted-file recovery as guaranteed.

## Research areas
APFS container and NX superblock, checkpoint selection, object maps, volume superblocks, B-trees, snapshots, spaceman allocation, copy-on-write history, encryption and Fusion configurations.

## Key limitation
Modern SSD behavior, TRIM, encryption and block reuse can make deleted content unavailable even when metadata can be examined.

## Milestones
1. Read-only container detection.
2. Container/object map validation.
3. Volume enumeration.
4. Metadata exploration on controlled images.
5. Snapshot-aware research.
6. Empirical deleted-file experiments.

## Success criteria
Every claimed capability must be measured against a controlled corpus with known deletion timing, encryption state and storage medium.

## Non-goal
Bypassing encryption or security protections.