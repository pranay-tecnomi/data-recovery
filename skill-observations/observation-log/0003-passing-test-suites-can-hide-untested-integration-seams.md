---
id: 3
title: A large passing unit-test suite can hide an entirely untested integration seam
status: open
type: open-source
skill: []
proposes_skill: [test-coverage-triage]
siblings_checked: none — proposes a new skill; no existing family member covers coverage triage
area: assessing whether a green test suite constitutes verification
date: 2026-09-07
session_context: APFS recovery engine with 68 passing unit tests but no whole-container test; building the first integration fixture immediately exposed a bug making most real files unrecoverable
---

**Issue:** A filesystem-recovery crate had 68 passing unit tests and read as
well covered. The tests were almost entirely parser-level - each function
checked against a hand-built byte fixture - and nothing assembled a whole
container end to end. Two tests at the top-level entry point were
tautologies (`assert_eq!(42, 42)`), so that entry point had no coverage at
all while still contributing a green line to the count.

Building the first integration fixture exposed a defect no unit test could
have caught: both the buffered and streaming extent readers rejected any
extent whose logical end exceeded the file size. Extent lengths in this
format are always whole blocks, so the final extent of any file whose size
is not a multiple of the block size legitimately overruns the end. Every
such file - the overwhelming majority - was unrecoverable. The unit tests
missed it because every hand-built fixture happened to use a block-aligned
file size, which is the natural number to pick when writing a fixture by
hand and the rare case in reality.

**Suggested improvement:** Treat "unit tests pass" and "the feature works"
as distinct claims requiring distinct evidence. When auditing coverage,
measure what the tests *compose*, not how many there are: look for a test
that drives the real entry point over a realistic whole artefact. Grep for
tautological assertions (both sides literal or the same variable) as a
cheap way to find entry points with fake coverage. When choosing fixture
values, deliberately pick ones that are unaligned, non-round and not
multiples of the block/page/chunk size - the convenient value is usually
the special case that hides boundary bugs.

**Principle:** A test suite's size measures effort, not coverage. Bugs
concentrate at the seams between correct components, and unit tests are by
construction blind to seams. The most informative test to write next is
almost always the first one that composes the whole path, and its value is
highest exactly where the existing suite looks most reassuring - a green
suite suppresses the suspicion that would otherwise prompt writing it.
