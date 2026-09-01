# Data Model and Persistence Specification

## Entities
Device: id, fingerprint, display_name, capacity, sector_size, connection, filesystem summary.
Partition: device_id, offset, length, type, filesystem.
ScanSession: id, source_fingerprint, mode, status, timestamps, checkpoint.
ReadError: session_id, offset, length, error_code, retry_count.
FileCandidate: id, session_id, origin, name, type, size, extents, confidence, validation_status.
RecoveryJob: id, destination, status, timestamps.
RecoveryItem: job_id, candidate_id, output_path, status, error.

## Persistence rules
Persist metadata and checkpoints, not unnecessary file contents.
Use transactional updates for session state.
Version schemas and migrations.
Source identity must include enough attributes to detect accidental resume against a different device.

## Retention
Allow users to delete sessions and logs. Sensitive paths and metadata should be minimized and protected by normal macOS file permissions.