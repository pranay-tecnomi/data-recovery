# FFI ABI and Event Schema

## Envelope
Request: version, request_id, operation, payload.
Response: version, request_id, status, payload, error.

## Events
PhaseChanged, Progress, Warning, CandidateBatch, ReadError, Completed, Failed, Cancelled.

Cancellation is idempotent. Exactly one terminal event is emitted. Unknown required versions fail explicitly. No Rust ownership type crosses the ABI directly.