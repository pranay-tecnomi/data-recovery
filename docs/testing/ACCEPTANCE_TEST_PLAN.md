# Acceptance Test Plan

## Milestone 0
Build, formatting, linting, unit tests, fault injection, cancellation and no source-write API regression.

## Milestone 1
MBR/GPT fixtures; corrupted headers; CRC mismatches; backup GPT; boundary/overlap cases.

## Filesystems
Golden images with active/deleted/fragmented files; corruption; loops; out-of-range metadata.

## System
Cancellation, resume mismatch, destination conflict, device disconnect simulation, partial recovery.

## Release
Clean CI, reproducible build, dependency audit, corpus pass, fuzz budget, manual macOS smoke test.