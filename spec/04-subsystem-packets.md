# Subsystem Packet Index

## P0 Core/storage
Inputs: existing core and storage contracts.
Done: typed error taxonomy, range invariants, cancellation, deterministic fault model, CI checks.

## P1 Partition discovery
MBR-01 signature/entries; MBR-02 extended partitions if in scope.
GPT-01 primary header; GPT-02 header CRC; GPT-03 entry-array CRC; GPT-04 backup discovery; GPT-05 primary/backup consistency.
Acceptance: corruption corpus and no out-of-range reads.

## P2 Filesystem probing
FAT32- and exFAT-specific structural evidence; return classification plus diagnostics, never a label-only decision.

## P3 FAT32
FAT32-01 boot/geometry
FAT32-02 FAT reader
FAT32-03 bounded chain resolver
FAT32-04 8.3 directory parser
FAT32-05 LFN assembler
FAT32-06 recursive directory walker
FAT32-07 active-file extents
FAT32-08 deleted-entry candidate reconstruction
FAT32-09 fragmented-deleted limitations and evidence
FAT32-10 integration fixtures

## P4 exFAT
Equivalent packetization: boot geometry, allocation metadata, entry sets, names, stream extents, active/deleted candidates, validation fixtures.

## P5 Candidate pipeline
Normalize -> deduplicate -> validate -> score -> persist evidence.

## P6 Output/session
Destination identity safety, streaming output, collision policy, atomic checkpoint publication, fingerprint/plan compatibility.

## P7 Carving
Signature registry, bounded scanner, header/footer rules, overlap policy, structural validators, confidence integration.

## P8 macOS
Read-only physical-device adapter, narrow helper API, typed XPC contract, authorization, disconnect handling.

Every packet must add acceptance tests before being marked complete.