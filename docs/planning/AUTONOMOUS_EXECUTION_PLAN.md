# Autonomous Execution Plan

## Rule

Proceed in dependency order without waiting for approval between documentation and implementation tasks. Stop only when an external permission, irreversible action, missing repository capability, or a material technical decision genuinely requires user input.

## Current baseline

Completed specifications include:
- recovery engine
- partition discovery
- disk imaging
- filesystem detection and probing

Milestone 0 foundation is bootstrapped.

## Documentation completion order

1. Detailed FAT32 Recovery Specification
2. Detailed exFAT Recovery Specification
3. Detailed File Carving Specification
4. Detailed File Validation Specification
5. Detailed Recovery Output Specification
6. Detailed Session Persistence and Resume Specification
7. Detailed Confidence and Candidate Ranking Specification
8. Detailed macOS Privileged Helper and XPC Specification
9. Detailed Test Corpus and Fixture Specification
10. Detailed Observability and Diagnostic Specification
11. Cross-document consistency audit

## Implementation order

### Milestone 0 close
- harden typed I/O errors
- cancellation-aware long operations
- integration fault tests
- verify CI status
- close quality gates

### Milestone 1
- partition domain model
- MBR parser and tests
- GPT parser and CRC tests
- backup GPT
- overlap diagnostics
- corpus and fuzzing

### Milestone 2
- filesystem probe registry
- FAT32 probe
- exFAT probe
- evidence aggregation

### Milestone 3
- FAT32 metadata traversal
- deleted-entry discovery
- cluster-chain reconstruction
- candidate generation

### Milestone 4
- exFAT recovery support

### Milestone 5
- disk imaging implementation and resume

### Milestone 6
- carving and validation

### Milestone 7
- session persistence
- confidence ranking
- recovery output

### Milestone 8
- macOS UI, FFI and privileged helper integration

## Execution discipline

Every completed batch must be:
1. pushed in small commits or verifiable batches;
2. read back from GitHub after writing;
3. linked to its dependency and test requirements;
4. followed by the next dependency rather than arbitrary expansion.

## Definition of autonomous progress

The assistant continues with the next required repository task until:
- a required external credential or permission is missing;
- an operation could cause irreversible user harm;
- a specification conflict requires a product decision;
- repository tooling blocks execution repeatedly.

Otherwise, continue.
