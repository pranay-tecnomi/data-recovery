# Error Taxonomy

SRC_001 OutOfRange
SRC_002 PermissionDenied
SRC_003 Disconnected
SRC_004 TransientReadFailure
SRC_005 PermanentReadFailure
SRC_006 FingerprintMismatch
PART_001 InvalidTable
FS_001 InvalidStructure
CARVE_001 InvalidCandidate
DST_001 SameAsSource
DST_002 NotWritable
DST_003 InsufficientSpace
AUTH_001 UnauthorizedClient
AUTH_002 AuthorizationDenied
INT_001 Cancelled
INT_002 InvariantViolation

Errors carry code, operation, optional byte range, recoverability and sanitized detail. UI never parses error strings.