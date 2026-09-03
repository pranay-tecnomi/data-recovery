# Claude Implementation Protocol

For every packet:
1. Read its dependencies and contracts.
2. Identify exact files/types to modify.
3. Do not silently redesign public boundaries.
4. Implement checked arithmetic and explicit resource limits first.
5. Add positive, negative, boundary and corruption tests.
6. Run formatting, compilation, linting and tests required by the repository.
7. Fix failures before the next packet.
8. Record any unresolved contradiction as a blocker.

## Forbidden shortcuts
- unsafe code unless a separately approved ADR permits it
- swallowing read errors
- treating paths as stable source identity
- trusting filesystem labels without structural validation
- unbounded Vec allocation from disk metadata
- infinite chain traversal
- declaring deleted-file content complete without evidence
- source and output overlap

## Required implementation report
For each packet: changed files, tests run/results, remaining limitations, and exact next dependency.