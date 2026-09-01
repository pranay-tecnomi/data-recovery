# Documentation Index and Reading Order

## Status model
IMPLEMENTATION READY: code against this baseline.
DRAFT: direction only.
RESEARCH: experiments required before commitments.

## Capability matrix
| Capability | Status | Target |
|---|---|---|
| Block I/O and disk images | IMPLEMENTATION READY | MVP |
| GPT/MBR analysis | IMPLEMENTATION READY | MVP |
| FAT32 recovery | IMPLEMENTATION READY | MVP |
| exFAT recovery | IMPLEMENTATION READY | MVP |
| JPEG/PNG/PDF/ZIP carving | IMPLEMENTATION READY | MVP |
| macOS privileged boundary | IMPLEMENTATION READY | MVP |
| HFS+ | DRAFT | Post-MVP |
| NTFS | DRAFT | Later |
| APFS deleted recovery | RESEARCH | Research |

## Mandatory reading order
1. PRD and SRS
2. Non-functional requirements
3. Master architecture and ADRs
4. Coding baseline and state machines
5. Storage I/O and partition analysis
6. Imaging and scan/carving framework
7. FAT32/exFAT specifications
8. Validation and confidence
9. Persistence, FFI and macOS XPC
10. Test corpus and acceptance matrix
11. Implementation sequence and backlog

The code-ready baseline is MVP only. HFS+, NTFS and APFS do not block MVP.