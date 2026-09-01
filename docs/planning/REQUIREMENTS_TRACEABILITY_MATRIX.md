# Requirements Traceability Matrix

| Requirement | Architecture/Module | Specification | Verification |
|---|---|---|---|
| Source is never written | storage-io, recovery-core | Storage I/O, NFR | API + integration negative tests |
| Read arbitrary valid range | storage-io | Block Device | unit + property tests |
| Resume only same source | recovery-core, session-store | Storage I/O, persistence | fingerprint mismatch tests |
| Enumerate partitions | partition | Partition spec | GPT/MBR corpus |
| Recover supported FAT32 cases | filesystem-fat | FAT32 spec | golden corpus |
| Recover supported exFAT cases | filesystem-exfat | exFAT spec | golden corpus |
| Image unstable media | recovery-core, storage-io | Imaging spec | fault injection |
| Carve supported formats | carving, validators | Carving specs | format corpus |
| Explain uncertainty | recovery-core | Confidence spec | scoring golden tests |
| Long operations cancellable | core, io | State machines | cancellation tests |
| Privileged access minimized | macOS adapter/helper | XPC specs | unauthorized-client tests |
| Destination differs from source | app/core | MVP contract | integration tests |

No requirement is considered implemented without linked automated verification.