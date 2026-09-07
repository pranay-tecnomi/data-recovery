---
id: 5
title: Code placed where tests cannot reach it will be untested, however important
status: open
type: open-source
skill: []
proposes_skill: [test-coverage-triage]
siblings_checked: test-coverage-triage: proposed in [[0003-passing-test-suites-can-hide-untested-integration-seams]] — same proposed skill, both entries belong to it
area: code placement and its effect on what can be verified
date: 2026-09-07
session_context: The orchestration layer implementing the product's actual workflow lived inside a binary target and so had zero test coverage; extracting it to a library immediately exposed a data-corruption bug
---

**Issue:** The module orchestrating the entire product workflow - detect,
scan, recover - lived inside a binary target. Binary targets cannot be
imported by integration tests, so the layer that actually implemented the
product had no coverage at all, while the individual libraries beneath it
were well tested. Nobody had decided to leave it untested; the placement
decided it, silently.

Extracting it into a library was a mechanical change (move the file, add a
manifest, depend on it from the binary) and immediately allowed a workflow
test that found a corruption bug affecting every filesystem: recovered
files were padded to the block boundary because the writer ignored the
declared file size.

A second instance in the same session: a fixture builder marked
`#[cfg(test)]` was invisible to other crates' tests, so a second filesystem
had no end-to-end coverage. Moving it behind a `test-support` feature fixed
that, and also subjected it to the linter for the first time, which found
two real issues.

**Suggested improvement:** Treat reachability as a design constraint, not
an afterthought. Keep binary targets to argument parsing and wiring, with
all logic in libraries. Share fixtures across crates through an optional
feature rather than `#[cfg(test)]`. When auditing coverage, ask which code
*can* be tested from outside before asking which code *is* tested - the
first question explains most of the second, and its answer is usually a
five-minute fix that unlocks everything behind it.

**Principle:** Testability is determined by structure long before anyone
writes a test. Code in a location no test can import will be untested
regardless of how important it is or how disciplined the team is, and its
absence from coverage looks identical to a deliberate decision. The highest-
leverage coverage work is often not writing tests but moving code to where
tests can see it.
