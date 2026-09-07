---
id: 1
title: Verify branch topology before trusting a handoff brief's premise
status: open
type: open-source
skill: []
proposes_skill: [repository-handoff-triage]
siblings_checked: none — proposes a new skill; no existing family member covers repo handoff triage
area: session start / inheriting an unfamiliar repository
date: 2026-09-07
session_context: Taking over a data-recovery repo; brief said "continue from the current APFS implementation" but main had no APFS crate at all
---

**Issue:** The handoff brief instructed continuing from "the current APFS
implementation" and listed APFS work as priorities 1-7. Inspecting `main`
showed no `crates/apfs-recovery` at all. The implementation existed only on
an unmerged branch (`ci/verify-green`), which was 120 commits behind main
and had never been an ancestor of it: two parallel lines of development had
diverged, one building APFS and the other building exFAT/pipeline/output.
Had the brief's premise been trusted, the obvious next step would have been
to write an APFS crate from scratch, silently duplicating ~120 commits of
existing work.

**Suggested improvement:** When inheriting a repository, before selecting
any task, establish topology as a distinct step: enumerate all branches,
and for each record commit count, ahead/behind vs the default branch, and
whether it is an ancestor. Then locate the artefacts the brief names
(`git log --all -- <path>`, `git ls-tree -r <branch>`) rather than assuming
they are on the default branch. A brief describes what its author believed
was true at some point; the branch graph is what is actually true now.

**Principle:** A task brief is a claim about the repository, not a
description of it. Where the two disagree, the repository wins — and the
disagreement is itself the most important finding, because it usually means
work exists that the brief cannot see. Verify the premise before executing
against it, since the cost of a wrong premise is duplicated work that looks
like progress.
