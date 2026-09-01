# Error Handling and Failure Recovery Specification

## Error classes
Source access, read integrity, parser, authorization, destination, persistence, cancellation, and unexpected internal errors.

## Policy
Never hide a source read failure as successful data.
Never retry indefinitely.
Preserve the original error and affected range.
Prefer partial useful results over corrupting session state.

## Scenarios
Device disconnect: pause, checkpoint, notify, allow validated resume.
Read errors: retry with bounded policy, reduce read size, map bad ranges.
Destination full: stop affected writes cleanly and preserve completed files.
Crash: restore last durable checkpoint.
Permission denial: explain required action without requesting broader privilege than necessary.

## Logging
Timestamp, component, operation, stable IDs, error class, and technical code. Avoid file contents and unnecessary sensitive paths.

## Acceptance
Failure injection tests cover every long-running pipeline stage.