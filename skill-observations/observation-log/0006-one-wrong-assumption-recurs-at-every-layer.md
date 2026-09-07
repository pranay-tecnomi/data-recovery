---
id: 6
title: A wrong domain assumption recurs at every layer that shares it
status: open
type: open-source
skill: []
proposes_skill: [binary-format-test-triage]
siblings_checked: binary-format-test-triage: proposed in [[0002-red-tests-can-indict-the-fixture-not-the-code]] — same proposed skill, both entries belong to it
area: fixing a defect once versus finding every instance of its cause
date: 2026-09-07
session_context: The assumption "extent length equals file length" produced the same corruption bug independently in a filesystem reader and in the output writer, and hid in fixtures across three filesystems
---

**Issue:** One incorrect assumption - that an extent's length equals the
number of file bytes it contributes - produced the same defect in two
unrelated layers written at different times: a filesystem extent reader
rejected files whose final extent overran the declared size, and the output
writer padded recovered files to the block boundary. Both are the same
mistake: storage is allocated in whole blocks, so the last extent of a file
routinely overruns its end.

It survived undetected because every hand-written fixture across three
filesystems used sizes that were exact multiples of the block size - the
natural number to choose when writing a fixture, and the rare case in
reality. Fixing the first instance did not surface the second; the second
was only found by running a complete workflow with a deliberately unaligned
file size.

**Suggested improvement:** When a defect traces to a domain assumption
rather than a coding slip, treat the assumption as the finding and search
for every layer that could share it, rather than closing the issue at the
first fix. Grep for the concept, not the symptom (here: every use of an
extent or cluster length as if it were a byte count). Then encode the
falsifying case permanently in a fixture: at least one deliberately
unaligned, non-round, not-a-multiple size in every test that touches
block-addressed data.

**Principle:** A bug caused by a wrong mental model is rarely a single bug.
The model was applied wherever the concept appears, so the defect is
replicated across layers that never shared code - which is exactly why
fixing one instance does not reveal the others. The unit of investigation
should be the assumption, not the failure that exposed it.
