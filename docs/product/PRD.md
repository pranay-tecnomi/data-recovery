# Product Requirements Document

## Vision
Build a professional-grade macOS data recovery application capable of safely analyzing and recovering data from supported internal and external storage devices.

## Core workflow
Detect → Assess Risk → Image if Necessary → Scan → Validate → Preview → Recover

## Goals
- Detect supported storage devices.
- Analyze sources without intentional writes.
- Detect supported filesystems.
- Recover deleted or logically inaccessible files when underlying data remains.
- Perform raw file carving.
- Create disk images.
- Preview supported files.
- Prevent recovery to the source.
- Communicate uncertainty.
- Support internal and external storage where technically possible.

## MVP
External storage first; read-only source access; disk imaging; file carving; FAT32 and exFAT analysis; common file formats; recovery to a separate destination; automated testing.

## Non-goals
Physical repair, guaranteed recovery, overwritten-data recovery, encryption bypass, firmware-level recovery, and universal filesystem support.

## Principles
- Never intentionally write to the source.
- Safety before convenience.
- Never guarantee recovery.
- Source and destination must differ.
- Explain uncertainty.