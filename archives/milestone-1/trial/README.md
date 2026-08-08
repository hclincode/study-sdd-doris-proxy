# Trial: learning SDD by running both tools on one real requirement

Everything here was produced by **actually installing and running the tools**.
No format was imitated. Both walkthroughs contain the real commands and their
real output, including the errors.

**The requirement, as given:**

> An L7 MySQL proxy in front of an Apache Doris cluster. Parse incoming SQL and
> append `LIMIT 200` to every query and sub-query before forwarding, to stop
> unbounded scans from hammering the cluster.

## Start here

| Read | Why |
|---|---|
| `openspec/WALKTHROUGH.md` (764 lines) | The **delta model** — how a spec stays current as it changes |
| `speckit/WALKTHROUGH.md` (611 lines) | The **phase pipeline** — how a constitution and clarification gates constrain a design |
| `../surveys/sql-limit-injection.md` | Whether the requirement is a good idea at all |

If you read only one thing, read §8 of the OpenSpec walkthrough (the canonical
spec before and after) and §6 of the Spec Kit walkthrough (the Constitution
Check failing). Those two sections are SDD actually working, on your
requirement, rather than a description of SDD working.

## The headline: both tools rejected the requirement as stated

This is the most important thing in the directory, and it was not planned.

Two independent processes, two different tools, no shared context, both
concluded that **appending `LIMIT` to every sub-query is wrong** — because
`SELECT COUNT(*) FROM (SELECT … FROM big) t` returns `200` instead of the true
count. Not an edge case; it is what the requirement literally asks for.

- **Spec Kit** caught it at the **Constitution Check**, a gate in `plan.md`.
  Principle I (Semantic Preservation) returned **FAIL**, and the gate could not
  be passed by rewording — the design changed. Four specific behaviours were
  deleted from the plan: the recursive visitor, capping UNION branches, capping
  CTE bodies, and capping the `SELECT` inside `INSERT INTO … SELECT` (that last
  one would have silently written 200 rows where the author asked for millions,
  and unlike a truncated read the damage persists).
- **OpenSpec** caught it across **two changes**. Change 1 shipped the hazard
  knowingly, with a design document saying in writing "this returns the wrong
  count and we are shipping it anyway, here is why". Change 2 was the fix. The
  canonical spec now shows only the corrected behaviour; both arguments are
  preserved in the archive.

And independently, `../surveys/sql-limit-injection.md` found the same thing from
the other direction, plus two facts neither tool could know:

1. **Doris already ships `sql_select_limit`** — one `SET` per session gives the
   outermost row cap with zero parsing and zero semantic hazard.
2. **`LIMIT` does not bound scans at all.** It caps rows *returned*, not rows
   *read*. `SELECT COUNT(*) FROM huge_fact` returns one row and scans
   everything. The stated goal — "stop unbounded scans" — is not achievable this
   way. Doris's `SQL_BLOCK_RULE` and workload policies are the real mechanisms.

Spec Kit's plan reached this conclusion on its own and wrote it into the
artifact rather than leaving it in a conversation:

> the proxy caps the outer `COUNT` query, which already returns one row, and
> therefore does nothing useful at all. The scan still happens. […] the load
> problem must be solved for those statements elsewhere — by Doris resource
> groups, not by this proxy.

**That is the lesson of this trial.** Not which tool won. A written spec caught
a flawed premise before any code existed. It cost a few hundred lines of
markdown; finding it in production would have cost a wrong number on a
dashboard that nobody could explain.

## What each tool actually enforces

Verified by deliberately feeding both tools malformed input. Neither is a
compiler.

**OpenSpec** enforces *structural completeness of spec deltas*, and this is more
than expected:

- every requirement must have at least one scenario
- a `MODIFIED` block must carry the full requirement, and the validator checks
  you did not silently drop scenarios from it
- a `MODIFIED` header must actually exist in the base spec — but **this fires
  only at `archive`**, the one command you run once, at the end

It does **not** check the proposal (essentially free text), and a missing
`## Purpose` becomes a permanent `TBD` placeholder in canonical truth — long
enough to pass the length check that would have flagged it. That is the one
failure mode with no gate anywhere.

**Spec Kit** enforces *four file-existence checks*, with real exit code 1:
feature directory allocated, `spec.md` before plan, `plan.md` before tasks,
`tasks.md` before implement. That is the entire list. Nothing checks that
mandatory sections were filled, that `[NEEDS CLARIFICATION]` markers were
resolved, or that `contracts/` contains anything. An empty `tasks.md` passes.

> Every quality property in either tool is enforced by a prompt telling a model
> to enforce it. They work the way a code review works, not the way a compiler
> does. In this project the real enforcement is `cargo test`, `proptest` and
> `cargo-mutants`. **The tests keep the code honest; the spec keeps the tests
> honest.**

## Counts, measured

| | files | lines | covers |
|---|---:|---:|---|
| OpenSpec `openspec/` | 15 | 848 | **two** complete changes + canonical specs + archive |
| Spec Kit `specs/001-…/` | 10 | 1,276 | **one** feature |

Per unit of work that is roughly **400 vs 1,276 lines — about 3×**, matching the
published third-party measurement in `../surveys/github-spec-kit.md`.

Scaffolding installed by each tool, which neither walkthrough counts as output:

| | files | lines |
|---|---:|---:|
| OpenSpec `dot-claude/` | 12 | 2,266 |
| Spec Kit `dot-claude/` + `dot-specify/` | 15+ | ~3,900 |

OpenSpec's agent scaffolding is **2.7× everything the process produced.** Both
tools are primarily prompt libraries with a small amount of machinery attached.

**The lean preset is a real escape hatch.** `specify preset add lean` overwrites
five skill definitions in place, cutting them **85%** (1,131 → 166 lines).
`speckit-specify` drops from 348 lines to 33. If Spec Kit's verbosity is the
objection, measure it with lean before rejecting the tool.

## The mechanisms worth stealing, whichever tool you pick

1. **A constitution.** Spec Kit's is the best single artifact in either tool.
   Principle-by-principle evaluation forced a real design change here. A
   constitution that never fails a gate is decoration.
2. **`[NEEDS CLARIFICATION]` markers.** A way to record a question *inside* the
   spec instead of guessing. OpenSpec has no equivalent. See §4 of the Spec Kit
   walkthrough for one being raised and resolved.
3. **Delta-then-archive.** Writing a change against the existing spec forces you
   to read the existing spec, and the archive leaves a current-state document
   with every argument preserved separately. See §8 of the OpenSpec walkthrough.
4. **An `unknowns.md` register.** Neither tool ships one. For a proxy you will
   accumulate parser and dialect surprises that need recording with provenance.

## Known rough edges, found by using them

- **OpenSpec: scenario names are permanent once archived.** Three routes were
  tried to retire one; all were rejected by the validator. Meanwhile the check
  is name-only — you may invert every `WHEN`/`THEN` beneath a scenario and
  validation passes. The canonical spec here carries a scenario called
  "Capping a derived table changes an aggregate result" that now asserts the
  capping does *not* happen, with a sentence explaining why the name stayed.
  The tool shaped the document, and not for the better.
- **OpenSpec: the last gate is `archive`**, and there is no `--dry-run`.
- **Spec Kit: `data-model.md` and `contracts/` are mandated** and fit a SQL
  rewriter poorly. See §7 of its walkthrough for the honest per-file verdict.

## Where this leaves the decision

The tool choice is still open — see `../discussions/01-sdd-tool-selection.md`.
Both walkthroughs end with an honest "what earns its keep" section; read those
two sections against each other and pick.

The *product* question is separate and now needs answering: given that
`sql_select_limit` exists and `LIMIT` does not bound scans, should the proxy
rewrite SQL at all? Either answer leaves the SDD exercise intact — you have
already extracted the lesson.
