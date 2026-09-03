# Claude Implementation Specification Package

This directory is the normative handoff package for implementation.

## Reading order
1. 00-project-contract.md
2. 01-global-invariants.md
3. 02-dependency-map.md
4. 03-implementation-protocol.md
5. subsystem packets in dependency order

Existing detailed specifications remain authoritative where referenced. If two documents conflict, this package's explicit contract takes precedence and the conflict must be recorded before coding.

## Coding rule
Do not broaden scope. Implement one packet, run its required checks, and stop for a specification contradiction rather than inventing behavior.