# File Carving Framework Specification

## Architecture
Chunk Reader → Overlap Manager → Signature Registry → Candidate Detector → Format Carver → Extent/Reconstruction → Validator → Candidate Store.

## Chunking
Chunks use bounded memory. Adjacent chunks overlap by the maximum required signature look-behind/look-ahead for registered detectors.

## Signature registry
Each format declares identifiers, priority, minimum evidence, parser/carver implementation, validation strategy and maximum candidate limits.

## Candidate lifecycle
Detected → Parsing → Reconstructed/Partial → Validated/Rejected → Ranked.

## Fragmentation
Raw carving cannot reliably infer arbitrary fragmented extents. When fragmentation cannot be resolved, output must be marked partial or low confidence.

## Resource limits
Maximum candidate size, nesting depth, parser recursion, decompression work and per-source candidate count are configurable.

## Safety
External bytes are untrusted. Every offset and length is checked; parsers must not allocate from untrusted sizes.

## Deduplication
Use content/extent evidence where feasible. Do not discard distinct files merely because names or signatures match.