# API and FFI Interface Specification

## Boundary
Swift application communicates with the Rust core through a narrow, versioned interface.

## Commands
- profile_source(source)
- start_scan(session_config)
- pause_scan(session_id)
- resume_scan(session_id)
- cancel_scan(session_id)
- list_results(session_id, query)
- preview(candidate_id)
- recover(job_config)
- create_image(image_config)

## Events
Progress, phase changes, candidate batches, warnings, recoverable errors, terminal completion.

## Data rules
Use stable IDs and explicit enums.
No raw pointers cross the public boundary without ownership rules.
All byte buffers have explicit lifetime and size contracts.
Errors are typed, not string-parsed.

## Compatibility
Version the interface. New fields must be backward compatible where possible. Integration tests must exercise Swift↔Rust serialization and cancellation.