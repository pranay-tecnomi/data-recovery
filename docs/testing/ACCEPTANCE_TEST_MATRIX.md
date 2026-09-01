# Acceptance Test Matrix

P0: source-write attempt is impossible by API and integration test.
P1: bounds, cancellation, image/device I/O and error mapping.
P2: GPT/MBR enumeration and malformed boundary rejection.
P3: FAT32/exFAT known fixtures and correct uncertainty.
P4: JPEG/PNG/PDF/ZIP validated recovery and malformed rejection.
P5: macOS source/destination restrictions and permission failures.

Release gate: all required tests pass, zero source-write violations, no critical known security defect, corpus metrics recorded.