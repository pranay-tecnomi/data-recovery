# Test Strategy and Recovery Test Lab

## Test pyramid
Unit tests for parsers and algorithms.
Integration tests for device abstractions and pipelines.
End-to-end tests for user-visible recovery workflows.
Regression tests for every fixed defect.
Fuzzing for binary parsers.

## Corpus
clean/
deleted/
formatted/
corrupted/
fragmented/
encrypted/
read-errors/
malicious/

Use synthetic images with a known manifest of original files, hashes, allocation patterns, and expected recoverability.

## Metrics
Recovery recall, false positives, validated-file rate, source-write violations (must be zero), crash-free runs, throughput, and memory use.

## Rules
Never use a test corpus containing data without rights to store and redistribute.
Tests must run deterministically where practical.