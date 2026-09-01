# Partition Analysis and Recovery Specification

## Supported tables
MVP: GPT and MBR, including protective MBR recognition.

## Pipeline
Read initial sectors → Parse table → Validate boundaries → Enumerate partitions → Probe filesystems.
If metadata is missing or inconsistent: scan for partition/filesystem signatures and score candidate boundaries.

## Safety
Partition candidates are analysis results only. The engine never rewrites a partition table.

## Validation
Offsets, lengths, overlap, capacity bounds, signatures and backup GPT consistency where available.

## Lost partitions
Infer candidates from coherent filesystem structures and plausible boundaries; expose evidence and confidence.

## Tests
Valid GPT/MBR, corrupt headers, overlap, truncated images, stale signatures and mixed-disk fixtures.