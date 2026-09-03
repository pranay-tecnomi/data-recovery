# Decisions Required Before Their Affected Code

## Must be decided before physical-device release
- supported macOS version matrix
- signing/notarization/distribution
- physical-device MVP access model

## Must be decided before public persistence/IPC stabilization
- source fingerprint evidence set and algorithm
- checkpoint serialization/migration policy
- event backpressure transport for FFI
- retry backoff/subdivision constants
- confidence weighting policy

These decisions do not block pure parser implementation, but affected public interfaces must not be frozen until resolved.