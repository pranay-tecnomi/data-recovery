# Concurrency and Cancellation

MVP execution is deterministic and bounded. Parallel reads are deferred until benchmarked and checkpoint ordering is proven.

Cancellation is cooperative and checked at bounded work intervals. Terminal states are emitted once. In-flight ownership is explicit; cancellation never silently marks unfinished ranges complete.

Memory budgets apply to read buffers, candidate batches, diagnostics and validator windows.