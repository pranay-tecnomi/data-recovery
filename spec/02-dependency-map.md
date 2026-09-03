# Dependency Map

0. recovery-core + storage-io safety
1. partition-discovery: MBR, then GPT
2. filesystem-probe
3. FAT32: boot/geometry -> FAT -> directory -> LFN -> traversal -> active reconstruction -> deleted candidates
4. exFAT: boot -> allocation metadata -> directory entry sets -> active reconstruction -> deleted candidates
5. candidate normalization + validation + confidence
6. recovery-output + session persistence/resume
7. file carving
8. macOS physical-device/privileged integration
9. end-to-end hardening

A packet may not depend on a later layer. Shared types belong in recovery-core only when they are genuinely cross-cutting.