# Disk Imaging and Bad Sector Specification

## Goal
Capture as much readable source data as possible while minimizing repeated stress on unstable media.

## Pipeline
Profile → Create destination image → Sequential pass → Retry failed ranges → Reduce range size → Record unreadable ranges → Finalize manifest.

## Read policy
Start with configurable bounded chunks. On failure, retry a limited number of times; reduce range size before declaring a range unreadable. Never retry indefinitely.

## Error map
Persist ranges, error class, attempts and final status. Adjacent failures may be coalesced.

## Image format
MVP may use a raw image plus sidecar manifest containing source fingerprint, capacity, chunk map, bad ranges, timestamps and optional hashes.

## Resume
Resume only after destination and source identity validation. Completed ranges are never reread unless explicitly requested.

## Integrity
Hash completed image regions where practical. Do not claim that a hash proves unreadable regions were recovered.

## Destination rules
Separate from source, writable, sufficient capacity or sparse-file support, and validated before long operations.

## Acceptance
Fault-injected tests prove bounded retries, accurate bad-range maps, safe cancellation and deterministic resume.