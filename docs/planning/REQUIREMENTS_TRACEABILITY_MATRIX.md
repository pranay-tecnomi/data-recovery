# Requirements Traceability Matrix

| ID | Requirement | Design source | Test evidence | Milestone |
|---|---|---|---|---|
| SAFE-001 | Never write recovery source | recovery engine, storage I/O | compile/API review + integration | 0 |
| SAFE-002 | Bounds-check all source ranges | core/storage specs | property tests | 0 |
| PART-001 | Parse MBR | partition spec | corpus | 1 |
| PART-002 | Parse/validate GPT + backup | partition spec | CRC/corruption corpus | 1 |
| FS-001 | Detect FAT32 | probing spec | fixtures | 2 |
| FS-002 | Detect exFAT | probing spec | fixtures | 2 |
| REC-001 | Recover FAT32 candidates | FAT32 spec | golden images | 3 |
| REC-002 | Recover exFAT candidates | exFAT spec | golden images | 4 |
| VAL-001 | Validate candidates | validation spec | corruption corpus | 5 |
| OUT-001 | Safe output destination | output spec | conflict tests | 5 |
| SES-001 | Safe resume | session spec | mismatch tests | 6 |
| CAR-001 | Bounded carving | carving spec | adversarial corpus | 7 |
| MAC-001 | Narrow privilege boundary | helper spec | auth tests | 8 |