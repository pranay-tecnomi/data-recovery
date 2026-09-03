# Global Invariants

1. No recovery-source write API.
2. All external metadata is untrusted.
3. All derived arithmetic is checked.
4. All reads are bounded and range-validated.
5. Short reads are explicit failures unless a contract explicitly models partial progress.
6. Parsers must not panic, loop indefinitely, or allocate from unbounded metadata.
7. Source identity is stronger than a path or display name.
8. Output must not overlap the source backing object.
9. Cancellation is cooperative and checked at bounded work intervals.
10. Terminal session transitions occur exactly once.
11. Validator timeout/resource exhaustion is inconclusive, not invalid.
12. Corruption produces diagnostics/evidence where safe; it does not terminate the process.
13. Persisted and IPC contracts carry versions.
14. No subsystem mutates global session state except the session transition authority.