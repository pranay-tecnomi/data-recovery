# Definition of Done and Quality Gates

## Story done
1. Specification updated or referenced.
2. Unit tests cover normal and boundary cases.
3. Malformed-input behavior is tested for parsers.
4. No source-write path is introduced.
5. Cancellation behavior is defined for long operations.
6. Errors use stable taxonomy.
7. Code passes formatter and lint rules.
8. Relevant integration/corpus tests pass.

## Milestone done
- All planned tasks complete.
- Traceability rows have passing verification.
- Architecture checklist passes.
- Performance measurements recorded where relevant.
- Known limitations documented.
- Deferred scope explicitly remains deferred.

## Release gate
No critical safety/security defect; zero known source-write violations; reproducible build; signed/notarized workflow validated for distribution target.