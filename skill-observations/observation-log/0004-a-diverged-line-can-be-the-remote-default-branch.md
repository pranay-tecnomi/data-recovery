---
id: 4
title: Re-check the remote before pushing; the divergence may be three-way
status: open
type: open-source
skill: []
proposes_skill: [repository-handoff-triage]
siblings_checked: repository-handoff-triage: proposed in [[0001-verify-branch-topology-before-trusting-a-handoff-brief]] — same proposed skill, both entries belong to it
area: integrating work when parallel lines exist
date: 2026-09-07
session_context: After merging two diverged lines locally, the push was rejected; origin/main had meanwhile advanced and was itself a third line
---

**Issue:** Two diverged development lines were reconciled locally and
verified green. The push was then rejected: `origin/main` had advanced by
three commits during the session. Inspecting it showed it was not a newer
version of the local default branch at all - it was the *other* line
(filesystem engine only, with none of the pipeline, output, carving or
application crates), which had been published while the local work
proceeded. The divergence was three-way, not two-way, and the topology
established at session start had gone stale.

One of the three new commits carried a genuine logic fix; another had
reformatted a source file into single-line declarations that the
project's own `fmt --check` gate would reject. Taking either the local or
the remote side wholesale would have lost the fix or broken the build.

**Suggested improvement:** Treat a push rejection as a topology change, not
a mechanical obstacle: re-run the same ahead/behind and content checks used
at session start against the *new* remote head before merging, and diff its
commits individually. When resolving, separate a commit's logic from its
incidental formatting - take the behaviour change, keep the formatting the
project's gates enforce. Re-verify every CI gate after the merge rather
than relying on the pre-merge green run.

**Principle:** Repository topology is a live value, not a fact established
once at session start. Any long-running task must re-read it before acting
on it, because the assumption that a rejection means "just pull" hides the
possibility that the remote is a different line of development rather than
a later version of yours.
