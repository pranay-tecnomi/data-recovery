# Non-Functional Requirements

Safety: zero intentional writes to source.
Correctness: every recovery claim backed by evidence.
Performance: bounded memory and cancellation for long operations.
Reliability: checkpoint long jobs where feasible.
Security: untrusted-media parsing is hardened and fuzz-tested.
Privacy: minimize logs and telemetry.
Accessibility: core workflows usable with macOS accessibility features.
Maintainability: modular filesystem/carver boundaries and documented contracts.