# Discussion: What actually caught defects in milestone 2

**Date:** 2026-08-10 · **Status:** open — update as milestone 2 continues
**Context:** building the row-filter proxy MVP with OpenSpec, four agents in parallel

Milestone 1 concluded that neither OpenSpec nor Spec Kit enforces anything
semantic, and that their value lies in the artifacts they make you write. This
records what happened when that was tested against real work, and it is the
milestone's actual output — the proxy is the exercise.

---

## 1. The headline: the spec caught nothing directly

Not one defect was found by writing a spec, by `openspec validate`, or by a
tool gate. `validate --strict` passed continuously, including at moments when a
live bypass was present in the code.

What the artifacts did was make **disagreement legible**. Every real defect was
found when someone held an implementation against a written contract and saw
they diverged. That is a narrower claim than "SDD prevents defects", and it is
the one milestone 1 predicted.

The clearest instance: the lead told one agent that a load-time rejection was
"still open, hold". The lead then decided it, wrote it into
`specs/policy-config`, and never sent the follow-up. The agent read the spec,
found a requirement no message had mentioned, implemented it, **and flagged the
discrepancy**. Had it followed the message instead of the spec, the hazard
would still be open and the lead would have believed it closed.

The written contract outlived an inconsistent instruction. That is the whole
mechanism, and it required no enforcement.

---

## 2. Every defect lived in a seam

Not one was inside a component. All five were at a boundary between two things
that were each individually correct.

| Defect | The seam |
|---|---|
| D4's protocol claim was false | Design doc vs. what the crate actually does |
| `Unresolvable` bypass | Two-state interface over a three-state decision |
| Escaping could widen a permitted set | Two individually-sound decisions composed |
| Non-ASCII identifier leak | A stated fail-closed rationale vs. an implementation that inverted it for characters the rationale never mentioned |
| `/*!` executed unconditionally | A parser that discards a version gate vs. a rewriter that re-renders |

Each was found by a party **other than the one who wrote the code**, and in
every case the tests were green throughout.

---

## 3. Test doubles bound what a suite can ever check

Three times a gap was invisible from inside the tests because everything
passed, and each time the shape of a stand-in had already decided what could be
checked.

| Stand-in | What its shape excluded | What that hid |
|---|---|---|
| Two-state `PolicyLookup` mock | The third decision state | An unqualified reference with no current database, forwarded unconstrained |
| Mock comparing table names by exact equality | Case and encoding folding | The non-ASCII identifier leak |
| Fixture whose `with()` took `&[&str]` | Integer permitted values | A documented feature with **zero** coverage |

> A mock that is simpler than the real collaborator hides exactly the class of
> defect the real one has — and so does a fixture whose parameter types are
> narrower than the configuration it stands in for.

**Operational form: check what a fixture cannot express, not only what it does
express.** That question would have found all three before mutation testing
did.

---

## 4. A test that cannot fail in the direction the bug lies

Found independently four times, in three files, by three different people
before anyone noticed it was one shape.

| Where | The test that could not fail |
|---|---|
| `policy.rs` | A dozen assertions on `is_restricted`, all asserting the positive answer |
| `rewrite.rs` | Four near-miss tests, all failing recognition at the same late gate — one gate tested four times, five earlier exits never |
| `rewrite.rs` | The fail-closed re-check, only ever reached *after* a correct wrapping, so it always answered "yes" |
| `analyze.rs` | `SELECT 1 / 2` as a negative case, with no marker for the bug to swallow |

A predicate only ever asked the question it answers "yes" to is
indistinguishable from one that always answers "yes". The useful question, and
the one nobody had asked of any file: **which direction does the suite drive
this check?**

Mutation testing found all four. Nothing else did — not the spec, not review,
not 200 passing tests.

---

## 5. Three variants of one timing mistake

All three were made, by two different people, in one afternoon.

1. **Read mid-run** — a conclusion drawn from an empty `missed.txt`; a "1
   missed" reported from a run polled at 45 of 55 mutants.
2. **Compare across runs** — a count of "6 rather than 9" that was line-shift
   from notes added between runs.
3. **Run started before the last edit** — results describing a tree that no
   longer existed.

All three are the same error: **treating a mutation result as a property of the
code rather than of a specific tree at a specific moment.** The counts look
like measurements of the codebase; they are measurements of a snapshot.

The rule is *pin the commit, then run*. But the rule only prevented the error
where someone applied it, and **what actually caught it each time was that
nobody treated a reported count as evidence** — each was found by a second
party re-deriving the number instead of quoting it. The first half is worthless
without the second.

---

## 6. Reasoning about code was unreliable; running it was not

Three separate times someone reasoned carefully about whether a mutant was
equivalent, and was wrong:

- An agent predicted an equivalence, reasoned soundly about *deleting* a check,
  and was wrong about *returning `true`* at the same site.
- The lead wrote a test asserting the right outcome by the wrong code path —
  twice, after warning others about exactly that error.
- A mutant two people cleared by hand-tracing turned out to be **both** a
  fail-open bug and an infinite loop. Both had traced comments containing
  ordinary letters in the two-byte window that mattered; between them they had
  tried the same input.

What broke that deadlock was building an instrument: the scanner extracted
standalone, real and mutated side by side, **with a step budget so an infinite
loop is observable rather than a hang**. A hang otherwise reads as "no result",
which reads as "no difference".

The standard that survived: **call a mutant equivalent only if you can state
why no observable behaviour changes; anything else is unresolved, not
equivalent.** "Unresolved" was recorded once, and it was the one that turned
out to be a real bug.

---

## 7. Multi-agent process notes

- **Ownership must be explicit and written down.** One near-miss came from the
  lead inferring an agent had abandoned work from a stale file read, reassigning
  its file, and nearly having two agents edit it at once. Recovery came from the
  agent stopping on a rejected write to ask who held the pen, rather than
  forcing it.
- **File mtime is a bad proxy for whether an agent is working.** It was wrong
  three times.
- **Restraint prevented more damage than any fix.** Stopping on a rejected
  write, freezing on an ambiguous routing, and waiting out another agent's
  broken build rather than reporting unverified work.
- **A shared `mutants.out` is a coordination hazard** — three agents running
  `cargo mutants` in one workspace clobber each other's output silently.
- **Correcting the lead was routine and load-bearing.** Agents overturned lead
  rulings on at least four occasions, each time with evidence, each time
  correctly.

---

## 8. What this says about the tool

OpenSpec's contribution was the artifacts and their durability, not enforcement.
Specifically:

- A spec that outlived a contradictory instruction (§1).
- Scenario naming that survived archiving: "Multi-statement request is rejected"
  stayed true when the mechanism behind it was disproved, because it was named
  for the behaviour rather than the mechanism. A task named for the mechanism
  had to be rewritten before archiving.
- `design.md` as the place a wrong decision could be corrected *with the
  original wrong text preserved*, so the correction teaches.
- ADRs whose immutability rule was honoured on the first occasion it bit, by the
  agent that wrote the rule, against its own document.

What it did not contribute: any detection. The tests keep the code honest; the
spec keeps the tests honest — and mutation testing keeps the tests honest about
being honest.
