---
id: 2
title: A failing test can indict the fixture, not the code under test
status: open
type: open-source
skill: []
proposes_skill: [binary-format-test-triage]
siblings_checked: none — proposes a new skill; no existing family member covers binary-format test triage
area: debugging failing tests over binary on-disk formats
date: 2026-09-07
session_context: Clearing 6 documented CI blockers in APFS B-tree, OMAP and snapshot code plus one exFAT blocker
---

**Issue:** Six tests were documented in CI as blocking "real logic bugs".
All six were actually malformed hand-built binary fixtures, and every
parser was already spec-correct:
- Two B-tree tests laid values out forward in memory while assigning
  offsets that the format measures *backwards* from the value-area end.
- An OMAP ordering test used `[1u8; 16]` as a key, which decodes to OID
  0x0101010101010101 - vastly greater than the OID 2 it asserted was larger.
- Two snapshot tests built a key with object id `u64::MAX`, overflowing the
  60-bit id field into the 4-bit type tag, silently retyping the record so
  the indexer's catch-all arm discarded it.
- An exFAT test filled 472 bytes where the cluster arithmetic required 476.
Changing the parsers to satisfy these tests would have broken correct code
against the real on-disk format.

**Suggested improvement:** Before editing code to satisfy a red test over a
binary format, derive the expected byte layout independently from the format
spec and check the fixture against it. Prefer computing fixture offsets from
named geometry constants over hardcoded literals, and add a comment stating
the layout rule (which direction offsets grow, which bits a field occupies).
Treat any hand-packed bitfield as suspect: assert the round-trip
(pack-then-unpack) rather than assuming the packing is right.

**Principle:** A red test asserts that code and fixture disagree - it does
not say which one is wrong. For binary formats the fixture is often the
weaker artefact, because it encodes the author's belief about the layout
with no compiler or parser to check it. Resolve the disagreement against the
external specification, never by moving whichever side is easier to edit -
"make the test pass" silently converts correct code into a bug.
