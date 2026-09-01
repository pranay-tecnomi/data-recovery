# Core State Machines

## ScanSession
Created → Profiling → Ready → Running ↔ Paused → Completing → Completed
Running → Failed; Ready/Running/Paused → Cancelled.

## RecoveryJob
Created → ValidatingDestination → Running → Verifying → Completed.
Running → Paused/Failed/Cancelled.

## ImageJob
Created → Validating → Reading → RetryingFailedRanges → Finalizing → Completed.

Terminal states are immutable. Progress is monotonic within a phase. Cancellation never reports Completed. Disconnect never silently switches source.