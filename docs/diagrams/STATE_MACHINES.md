# State Machines

## Scan session
```mermaid
stateDiagram-v2
 [*] --> Created
 Created --> Profiling
 Profiling --> Ready
 Ready --> Running
 Running --> Paused
 Paused --> Running
 Running --> Completing
 Completing --> Completed
 Running --> Failed
 Ready --> Cancelled
 Running --> Cancelled
 Paused --> Cancelled
```

## Recovery job
```mermaid
stateDiagram-v2
 [*] --> Created
 Created --> ValidatingDestination
 ValidatingDestination --> Running
 Running --> Verifying
 Verifying --> Completed
 Running --> Failed
 Running --> Cancelled
```