# Persistence Schema

scan_sessions(id, source_fingerprint, mode, state, version, created_at, updated_at)
scan_checkpoints(session_id, phase, cursor, payload)
read_errors(id, session_id, offset, length, code, attempts)
candidates(id, session_id, origin, type, name, size, confidence, validation, evidence_json)
recovery_jobs(id, session_id, destination_fingerprint, state, created_at)
recovery_items(job_id, candidate_id, output_path, state, error_code)

Migrations are forward-only and tested. Checkpoints and state changes are atomic. Sensitive source paths are optional and not required for resume.