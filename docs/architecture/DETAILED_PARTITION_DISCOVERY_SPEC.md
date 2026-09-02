# Detailed Partition Discovery Specification

## 1. Purpose

Define the implementation contract for discovering and validating MBR and GPT partition structures from a read-only block source.

This specification is an immediate dependency of Milestone 1.

## 2. Scope

MVP support:
- protective MBR detection
- legacy MBR parsing
- primary GPT header parsing
- GPT partition entry array parsing
- backup GPT header discovery and validation
- partition range validation
- overlap detection
- typed diagnostics

Out of scope:
- partition repair
- modifying partition tables
- filesystem recovery logic
- hybrid MBR recovery heuristics beyond reporting detected structures

## 3. Safety Invariants

1. The partition module only receives a read-only BlockDevice.
2. No partition parser performs writes.
3. Every offset and length calculation is checked for overflow.
4. Parsed ranges must remain inside source capacity.
5. Untrusted metadata never controls allocation without explicit limits.
6. Invalid structures produce diagnostics, not panics.

## 4. Public Domain Model

### PartitionTableKind

- Unknown
- Mbr
- Gpt
- ProtectiveMbr

### PartitionCandidate

Contains:
- table kind
- partition index
- byte range
- starting and ending logical block addresses where applicable
- type identifier
- optional name
- attributes
- evidence
- validation status

### DiscoveryResult

Contains:
- detected table structures
- partition candidates
- diagnostics
- confidence/evidence metadata

Discovery may succeed with diagnostics.

## 5. Discovery Pipeline

```text
BlockDevice
    |
    v
Read LBA 0
    |
    v
Validate MBR signature
    |
    +---- invalid ----> emit diagnostic / probe GPT cautiously
    |
    v
Parse four MBR entries
    |
    +---- protective entry? ----> GPT discovery
    |
    +---- normal entries -------> validate MBR partitions
                                    |
                                    v
                              DiscoveryResult
```

GPT discovery:

```text
Read primary GPT header
        |
        v
Validate signature and header size
        |
        v
Validate header CRC
        |
        v
Validate usable LBA ranges
        |
        v
Read bounded partition-entry array
        |
        v
Validate entry-array CRC
        |
        v
Parse non-empty entries
        |
        v
Validate ranges and overlaps
        |
        v
Optionally validate backup GPT
```

## 6. MBR Parsing

### Sector assumptions

The conventional MBR structure occupies the first 512 bytes. The implementation must not assume that the underlying device logical sector size is always 512 bytes.

The parser reads the minimum required metadata range explicitly.

### Validation

The parser checks:
- boot signature 0x55AA at the expected MBR location
- entry offsets and sizes
- checked conversion from LBA counts to byte ranges
- non-zero sector counts for usable entries
- source-capacity bounds

### Partition range calculation

Given:
- start_lba
- sector_count
- logical_sector_size

Compute:

start = start_lba * logical_sector_size
length = sector_count * logical_sector_size

Both multiplications use checked arithmetic.

### Extended partitions

Extended partition chains are not required for the first Milestone unless explicitly added to the implementation plan.

If encountered, report a typed diagnostic indicating unsupported extended partition traversal rather than treating contained logical partitions as discovered.

## 7. GPT Parsing

### Header location

Primary GPT header is normally located at LBA 1.

The implementation derives the byte offset from the source geometry and validates that the complete header lies within source capacity.

### Header checks

Validate:
- signature
- revision support policy
- header size minimum and maximum
- header size within containing logical block
- current LBA
- alternate LBA
- usable LBA ordering
- partition entry LBA
- entry count
- entry size
- checked entry-array size
- all ranges against source capacity

### CRC policy

CRC verification is required for:
- GPT header
- partition entry array

A CRC mismatch is diagnostic evidence. The engine must not silently report corrupted metadata as fully validated.

### Resource limits

Before allocating or reading the entry array:
- cap entry count
- cap entry size
- cap total bytes read per metadata structure

If metadata requests excessive resources, reject the structure with a typed validation error.

## 8. Backup GPT

The backup header is normally located at the alternate end of the disk.

When primary GPT is valid:
- validate backup consistency when requested by the scan policy.

When primary GPT is invalid:
- attempt backup discovery using safe bounded reads.

Primary and backup disagreements produce diagnostics. The module does not repair either copy.

## 9. Partition Entry Validation

For each non-empty entry:

1. Ensure first LBA is not greater than last LBA.
2. Ensure range lies inside usable GPT bounds.
3. Convert inclusive LBA range to byte range with checked arithmetic.
4. Ensure byte range lies inside source capacity.
5. Preserve type GUID and unique partition GUID as opaque identifiers.
6. Decode partition name defensively and bound resulting string size.

## 10. Overlap Detection

Partition candidates are sorted by starting byte offset.

For each adjacent validated pair:

previous.end > current.start

indicates overlap.

Overlapping structures are retained as evidence but marked with a validation warning. The module does not silently discard potentially useful structures.

## 11. Error and Diagnostic Contract

Examples:

- PART_001 InvalidTable
- PART_002 InvalidMbrSignature
- PART_003 InvalidGptHeader
- PART_004 HeaderCrcMismatch
- PART_005 EntryArrayCrcMismatch
- PART_006 RangeOverflow
- PART_007 RangeOutOfBounds
- PART_008 ExcessiveMetadataSize
- PART_009 OverlappingPartitions
- PART_010 BackupHeaderMismatch
- PART_011 UnsupportedExtendedPartition

Errors represent inability to perform required operations. Diagnostics represent suspicious or partially valid structures.

## 12. Recovery Engine Integration

Partition discovery does not decide recovery strategy.

It provides partition candidates and evidence to recovery-core.

Recovery-core may then:
- probe each partition for supported filesystems
- choose raw scanning where partition metadata is unreliable
- expose uncertainty to the user

A partition parse failure must not prevent a later raw carving strategy when scan policy permits it.

## 13. API Shape

Conceptually:

```text
discover(device, geometry, policy)
    -> DiscoveryResult

probe_mbr(device, geometry)
    -> MbrEvidence

probe_gpt(device, geometry, policy)
    -> GptEvidence
```

The concrete Rust API must avoid exposing mutable source access.

## 14. Testing Requirements

### Unit tests

- valid MBR
- invalid signature
- zero-sized entry
- range overflow
- range beyond capacity
- valid GPT header
- invalid GPT signature
- invalid header size
- header CRC mismatch
- entry-array CRC mismatch
- malformed entry range
- overlapping partitions

### Integration tests

Use generated disk images containing:
- MBR-only layout
- GPT-only layout
- protective MBR + valid GPT
- corrupted primary GPT + valid backup GPT
- corrupted backup GPT
- partitions near source boundary

### Property tests

Generate bounded combinations of:
- sector size
- LBA values
- entry counts
- entry sizes

Assert:
- no arithmetic panic
- no out-of-bounds source range is accepted
- all emitted byte ranges satisfy capacity constraints

### Fuzzing

Fuzz:
- MBR sector bytes
- GPT headers
- GPT entry arrays

The parser must not panic, allocate unbounded memory or perform writes.

## 15. Milestone 1 Definition of Done

Milestone 1 partition discovery is complete when:

- MBR parsing passes the corpus.
- GPT primary parsing passes the corpus.
- CRC validation is implemented and tested.
- backup GPT discovery is tested.
- malformed input does not panic.
- all arithmetic is checked.
- overlap diagnostics are emitted.
- source-write impossibility remains intact.
- integration tests run in CI.

## 16. Implementation Order

1. Partition domain types and diagnostics.
2. Geometry-aware read helpers.
3. MBR parser.
4. MBR corpus tests.
5. GPT header parser.
6. CRC verification.
7. GPT entry-array parser.
8. Range and overlap validation.
9. Backup GPT support.
10. Integration and fuzz tests.

Do not begin filesystem parsing until the partition discovery acceptance criteria are met.
