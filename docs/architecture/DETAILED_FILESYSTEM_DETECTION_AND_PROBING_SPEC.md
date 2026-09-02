# Detailed Filesystem Detection and Probing Specification

## 1. Purpose

Define how the recovery engine identifies likely filesystems on a partition or bounded source range before selecting a filesystem-specific recovery strategy.

Detection is evidence gathering, not proof. A valid signature alone does not guarantee that the filesystem metadata is internally consistent.

## 2. Scope

MVP probes:
- FAT32
- exFAT
- unknown/unsupported classification

Deferred:
- HFS+
- APFS recovery
- NTFS recovery
- automatic filesystem repair

## 3. Inputs and Boundaries

A probe receives:
- read-only partition-scoped BlockDevice view
- partition byte range
- logical sector size
- probe policy and resource limits

The probe must not read outside the partition-scoped range.

Filesystem modules never receive unrestricted mutable source access.

## 4. Probe Result

A result contains:

- filesystem kind
- confidence level
- validation status
- evidence list
- diagnostics
- optional parsed boot metadata safe for reuse

Possible outcomes:

- DetectedValidated
- DetectedWithWarnings
- Ambiguous
- NotDetected
- Unsupported
- ProbeFailed

Probe failure is distinct from NotDetected.

## 5. Probe Pipeline

```text
Partition candidate
       |
       v
Read bounded boot metadata
       |
       v
Signature checks
       |
       v
Structural checks
       |
       +--> inconsistent --> diagnostics
       |
       v
Candidate scoring
       |
       +--> unique best --> filesystem-specific plan
       |
       +--> ambiguous ----> conservative fallback policy
       |
       +--> unknown ------> raw/deep scan eligible
```

## 6. Probe Safety Rules

1. All reads are bounded.
2. All offsets use checked arithmetic.
3. Metadata-controlled lengths are capped.
4. Probe code must not allocate from untrusted values without limits.
5. Invalid metadata must not panic.
6. Probes do not modify source state.
7. A probe must complete within configured byte and operation budgets.

## 7. FAT32 Detection

FAT32 detection begins with the boot sector and evaluates multiple evidence fields.

Evidence includes:
- boot signature
- plausible bytes-per-sector
- plausible sectors-per-cluster
- reserved sector count
- FAT count
- FAT size fields
- root cluster
- total sector count
- FAT32-specific structural consistency

The implementation must not classify FAT32 solely from a filesystem label string.

Derived region boundaries must fit inside the partition.

## 8. exFAT Detection

exFAT detection begins with the exFAT boot region.

Evidence includes:
- exFAT signature
- bytes-per-sector shift plausibility
- sectors-per-cluster shift plausibility
- FAT and cluster heap offsets
- cluster count
- root directory cluster
- volume length consistency

The implementation must validate derived offsets against partition capacity.

## 9. Confidence

Confidence is derived from evidence.

Suggested dimensions:
- signature evidence
- structural consistency
- checksum/boot-region validation where supported
- boundary consistency
- contradictory evidence

A corrupted but recognizable filesystem may be reported as DetectedWithWarnings rather than rejected outright.

The confidence model must preserve evidence sufficient to explain the result.

## 10. Ambiguity

If more than one probe reports credible evidence:

- do not silently choose based only on probe registration order
- compare confidence and validation severity
- preserve competing evidence
- let scan policy decide whether to probe further or proceed with raw scanning

The UI must be able to explain ambiguity.

## 11. Corrupted Boot Metadata

Corrupted boot metadata does not automatically mean recovery is impossible.

Policy:

- record diagnostics
- avoid trusting invalid derived offsets
- allow filesystem-specific fallback scanning only when its algorithm can establish safe boundaries independently
- otherwise mark filesystem metadata recovery unavailable and permit raw/deep scan according to policy

The probe layer does not repair metadata.

## 12. Unsupported Filesystems

Unsupported but recognizable structures may produce:

- filesystem kind if confidently recognized
- Unsupported outcome
- evidence and diagnostics

This prevents unsupported media from being misleadingly classified as unknown.

## 13. Probe Registry

The recovery engine uses an explicit registry of probe implementations.

Conceptually:

```text
FilesystemProbe
    kind()
    probe(context) -> ProbeResult
```

Registry requirements:
- deterministic registration
- no hidden priority decisions
- policy-controlled probe order
- independent result collection where safe

The core owns final strategy selection.

## 14. Probe Resource Limits

Probe policy defines:
- maximum bytes read
- maximum metadata structures parsed
- maximum allocation size
- maximum diagnostics retained per probe
- cancellation behavior

A filesystem probe must return a typed limit error rather than exceed configured resources.

## 15. Error and Diagnostic Taxonomy

Examples:

- FS_001 ProbeReadFailed
- FS_002 InvalidBootSignature
- FS_003 InvalidGeometry
- FS_004 DerivedRangeOutOfBounds
- FS_005 MetadataTooLarge
- FS_006 UnsupportedFilesystem
- FS_007 AmbiguousDetection
- FS_008 CorruptRecognizedFilesystem
- FS_009 ProbeBudgetExceeded

Human-readable messages are not API identifiers.

## 16. Integration with Recovery Planning

Probe results feed recovery-core.

Planning examples:

```text
Validated FAT32
    -> metadata recovery + deleted-entry analysis

FAT32 with warnings
    -> conservative metadata scan + optional raw scan

Validated exFAT
    -> exFAT metadata recovery plan

Unsupported recognized filesystem
    -> raw/deep scan policy

Ambiguous
    -> collect more evidence or conservative raw scan
```

No probe directly starts recovery output.

## 17. Testing Requirements

### Unit tests
- valid FAT32 boot metadata
- FAT32 label spoofing
- invalid FAT32 derived regions
- valid exFAT metadata
- invalid exFAT shifts
- partition boundary violations
- ambiguous evidence
- unsupported recognized structures

### Integration tests
Disk-image fixtures containing:
- clean FAT32
- clean exFAT
- corrupted boot metadata
- valid partition with unknown bytes
- truncated partition image

### Property tests
Generate bounded boot metadata combinations and assert:
- no panic
- no accepted range outside capacity
- resource limits are respected

### Fuzzing
Fuzz boot sectors and probe contexts.

## 18. Definition of Done

Filesystem probing is complete when:

- FAT32 and exFAT probes produce evidence-based results
- signatures are not the only classification mechanism
- all derived ranges are bounds checked
- ambiguous outcomes are representable
- corrupted metadata produces diagnostics without panic
- unsupported recognition is distinguishable from unknown data
- resource budgets are enforced
- corpus, property and fuzz tests pass

## 19. Implementation Order

1. Probe domain types.
2. Partition-scoped reader abstraction.
3. Probe registry.
4. FAT32 boot-sector probe.
5. FAT32 tests.
6. exFAT boot-region probe.
7. exFAT tests.
8. Confidence/evidence aggregation.
9. Ambiguity handling.
10. Corpus and fuzz integration.

Do not begin deleted-file recovery until the relevant probe can establish a safe filesystem context or the recovery strategy explicitly documents its fallback assumptions.
