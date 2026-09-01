# Corpus and Test Vector Specification

Every fixture has: fixture_id, generator/version, filesystem, capacity, sector geometry, original file hashes, allocation layout, mutation operations, expected candidates and known unreadable ranges.

Required: FAT32/exFAT valid/deleted/fragmented/malformed; GPT/MBR corrupt cases; valid/minimal/malformed/truncated carvers; partial read/disconnect/retry/cancel.

Expected output comes from manifest hashes and evidence classes. No test assumes every deleted file is recoverable. All binary parsers receive fuzz and structure-aware malformed input.