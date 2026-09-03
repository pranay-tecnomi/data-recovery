# Claude Handoff

Start with the earliest incomplete packet in spec/02-dependency-map.md.

Before coding:
- inspect current repository state
- compare code to the applicable packet
- preserve existing working contracts unless a documented defect requires change

During coding:
- work in one logical packet at a time
- prefer small reviewable commits
- do not combine unrelated refactors
- stop on conflicting normative specifications

Completion condition:
- all packets within the defined MVP are implemented
- acceptance and integration tests pass
- unresolved deferred scope remains explicitly excluded
- no completion claim is based solely on compilation