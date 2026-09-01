# Architecture Review Checklist

## Safety
- [ ] Recovery path exposes no source write operation.
- [ ] Source and destination identity are independently validated.
- [ ] Same physical source/destination is rejected.
- [ ] Unstable-source policy is explicit.

## Correctness
- [ ] Every offset/length is overflow and bounds checked.
- [ ] Endianness is explicit at binary parsing boundaries.
- [ ] Partial reads and disconnects have typed outcomes.
- [ ] Resume validates source fingerprint.

## Security
- [ ] All media bytes are treated as untrusted.
- [ ] Parser resource limits exist.
- [ ] Privileged helper exposes no shell execution.
- [ ] XPC clients and requests are authenticated/validated.

## Reliability
- [ ] Cancellation is cooperative and deterministic.
- [ ] Terminal state is emitted once.
- [ ] Checkpoint writes are atomic.
- [ ] Crash/restart behavior is tested.

## Maintainability
- [ ] Dependency direction is respected.
- [ ] Public contracts are versioned where needed.
- [ ] New parser features include corpus fixtures.

A checklist pass is required before each milestone merge.