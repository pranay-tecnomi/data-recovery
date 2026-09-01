# HFS+ Recovery Specification

## Status
Post-MVP filesystem module.

## Scope
Read-only parsing of volume header, allocation file, catalog B-tree, extents overflow structures and forks.

## Recovery
Use catalog/extents evidence to reconstruct reachable and deleted metadata where it remains available. Deleted-record behavior varies with reuse and metadata changes; results require validation.

## Safety
B-tree node links, record offsets and lengths are untrusted and strictly bounded.

## Fragmentation
Resolve extents from catalog and overflow structures with loop and overlap checks.

## Tests
Volume header corruption, malformed B-tree nodes, fragmented forks, deleted records and truncated images.