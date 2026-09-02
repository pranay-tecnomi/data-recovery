# Test Corpus and Observability Specification

## Corpus
Fixtures must be versioned, attributed and reproducible. Include synthetic images for MBR, GPT, FAT32, exFAT, corruption and read faults.

Real user data must not be committed.

## Observability
Emit structured events with session ID, module, operation, range and typed outcome. Do not log recovered file contents or sensitive paths by default.

## Metrics
Track bytes scanned, read failures, retries, candidates, validation outcomes and elapsed stages.

## Fuzzing
Fuzz parsers and checkpoint decoders continuously or on scheduled CI where practical.