# Threat Model

## Assets
Source evidence, recovered output, user privacy, privileged helper boundary, session metadata.

## Trust boundaries
Untrusted disk bytes -> parser; GUI -> privileged helper; recovery candidate -> output writer; checkpoint -> resume loader.

## Threats
Malformed metadata causing overflow/OOM/panic; path confusion; source/destination identity collision; privilege escalation; corrupted checkpoints; sensitive data leakage in logs.

## Required controls
Checked arithmetic, bounded allocation, read-only source contracts, typed IPC allowlist, canonical identity checks, atomic checkpoints, structured redacted logs, fuzzing of parsers.

## Non-goals
Filesystem repair and arbitrary privileged command execution.