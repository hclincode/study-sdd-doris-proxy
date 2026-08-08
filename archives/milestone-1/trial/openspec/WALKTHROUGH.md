# OpenSpec, worked end to end

Every command and every block of output below was actually run. Nothing here is
reconstructed from documentation. The only editing is removal of shell noise from
the local zsh profile (`command not found: _encode`) and npm's `npm notice` lines.

- Tool: `@fission-ai/openspec` **1.8.0**, run as `npx -y @fission-ai/openspec@latest`
- Node v26.0.0, macOS
- Working directory: a throwaway `git init` repo
- Date stamped by the tool on the artifacts: 2026-08-08

The exercise is an L7 MySQL proxy in front of Apache Doris that parses incoming SQL
and injects a row cap before forwarding. The software is a vehicle. The thing to
watch is what the tool does with the documents.

---

## 1. What OpenSpec is, in one paragraph

OpenSpec keeps two bodies of text. `openspec/specs/` holds the **current truth** —
what the system does, stated as requirements. `openspec/changes/<name>/` holds a
**proposed delta** — what a specific piece of work will change about that truth.
You write the delta, you build the thing, then you `archive` the change, and the
tool merges the delta into the canonical specs and moves the change folder into
`changes/archive/`. The specs are never edited by hand in the normal flow; they are
the accumulated result of every archived delta.

That merge is the entire point. Everything else — the templates, the validator, the
status command — exists to make sure the delta is well enough formed that the merge
can be mechanical.

---

## 2. Setup

```
$ npx -y @fission-ai/openspec@latest init --tools claude --no-animation
- Creating OpenSpec structure...
▌ OpenSpec structure created
- Setting up Claude Code...
✔ Setup complete for Claude Code

OpenSpec Setup Complete

Created: Claude Code
6 skills and 6 commands in .claude/
Config: openspec/config.yaml (schema: spec-driven)

Getting started:
  Start your first change: /opsx:propose "your idea"
```

`init` creates exactly two things: `openspec/config.yaml` (32 lines, almost all
commented-out examples) and twelve files under `.claude/` — six slash commands
under `commands/opsx/` and six skills. No `specs/` directory, no `changes/`
directory. Those appear when you first need them.

The `.claude/` payload is 2,266 lines of prompt scaffolding. It is how the tool
expects you to drive it: you type `/opsx:propose`, an agent reads the skill, and the
skill tells the agent to run the same CLI commands used below. This walkthrough
skips the agent layer and drives the CLI directly, because that is where you can
see what is actually enforced.

The `config.yaml` is worth a look before you start. It has a `context:` key for
tech stack and domain notes, and a `rules:` key for per-artifact house style
("keep proposals under 500 words"). Both are fed to whatever agent writes the
artifacts. Both were left empty here.

---

## 3. Creating a change

```
$ npx -y @fission-ai/openspec@latest new change add-limit-injection
- Creating change 'add-limit-injection' with schema 'spec-driven'...
Created change 'add-limit-injection' at openspec/changes/add-limit-injection/
Schema: spec-driven
Next: openspec status --change add-limit-injection
```

This creates one file, `openspec/changes/add-limit-injection/.openspec.yaml`, whose
entire contents are:

```yaml
schema: spec-driven
created: 2026-08-08
```

That is the whole scaffolding. The four artifacts are yours to write. `status`
tells you what is expected and in what order:

```
$ npx -y @fission-ai/openspec@latest status --change add-limit-injection
Change: add-limit-injection
Schema: spec-driven
Change root: .../openspec/changes/add-limit-injection
Progress: 0/4 artifacts complete

[ ] proposal
[-] specs (blocked by: proposal)
[-] design (blocked by: proposal)
[-] tasks (blocked by: specs, design)
```

The dependency graph is advisory — nothing stops you writing `tasks.md` first — but
it encodes the intended order and it is a good order.

---

## 4. The four artifacts and why they are separate files

This is the part a reader new to spec-driven development most needs, so it gets
prose rather than a table.

**`proposal.md` — why, and what capabilities are touched.** It is the argument for
doing the work at all. Motivation, a bullet list of what changes, impact. Its
load-bearing section is `## Capabilities`, split into `### New Capabilities` and
`### Modified Capabilities`. That list is a contract: every capability named there
needs a matching delta spec file, and the paths must match exactly. The proposal is
the document you would show someone deciding whether to fund the work; nobody
reading it should need to know how you plan to build it.

**`specs/<capability>/spec.md` — what the system will do, as a behaviour contract.**
This is separate from the proposal because it outlives it. The proposal is about a
moment ("we should do this, now, because X"); the spec is about a steady state
("the proxy SHALL NOT cap a sub-query in predicate position"). Six months from now
the proposal is history and the spec is still the answer to "what is this supposed
to do?". Mechanically, the separation is what makes archiving possible: the tool
merges the spec file into canonical truth and files the proposal away as a record.
If they were one document you could not do that.

The spec file is written as a *delta*, not as a finished spec. Its top-level
headings are `## ADDED Requirements`, `## MODIFIED Requirements`,
`## REMOVED Requirements`, `## RENAMED Requirements`. Under each, requirements are
`### Requirement: <name>` and scenarios are `#### Scenario: <name>` with
`- **WHEN**` / `- **THEN**` bullets. A brand-new capability's delta additionally
opens with `## Purpose`.

**`design.md` — how, and what was decided against.** The proposal says the proxy
will cap sub-queries; the design says *which* sub-queries and why capping a
predicate sub-query was rejected as a silent-corruption risk. It is the only one of
the four that is optional in spirit — the tool's instructions list conditions under
which it is worth writing — but `status` counts it, so in practice you write it.
Its value in this exercise was concentrating the genuinely contentious decisions in
one place where they could be argued rather than discovered in code.

**`tasks.md` — the checklist.** Numbered groups `## 1. Name` with
`- [ ] 1.1 description` items. The format is parsed: `openspec list` reads the
checkboxes and reports `0/25 tasks`. Anything not matching `- [ ]` is invisible to
the tool.

### Getting the format right

Do not guess at the format. The CLI will hand you the exact instructions and the
exact template for each artifact:

```
$ npx -y @fission-ai/openspec@latest instructions specs --change add-limit-injection
```

It prints an XML-ish block containing `<task>`, the paths of any dependency
artifacts you should read first, the output path, a long `<instruction>` section,
and the literal `<template>`. It is written to be pasted into an agent's context,
but it reads fine as documentation. `openspec templates` prints the raw template
file paths if you want them without the surrounding prose.

The instructions carry warnings the validator does not enforce. For instance:

> **CRITICAL**: Scenarios MUST use exactly 4 hashtags (`####`). Using 3 hashtags
> or bullets will fail silently.

Hold onto that. Section 9 tests it, and the answer is "half right".

---

## 5. Validating

The written artifacts for `add-limit-injection`: a 35-line proposal declaring two
new capabilities, an 80-line design, a 42-line task list of 25 items, and two delta
specs — 143 lines for `query-limit-injection` (8 requirements) and 54 lines for
`sql-parse-fallback` (3 requirements).

```
$ npx -y @fission-ai/openspec@latest validate --all --strict
- Validating...
✓ change/add-limit-injection
Totals: 1 passed, 0 failed (1 items)

$ npx -y @fission-ai/openspec@latest status --change add-limit-injection
Progress: 4/4 artifacts complete

[x] proposal
[x] specs
[x] design
[x] tasks

All planning artifacts complete!

$ npx -y @fission-ai/openspec@latest list
Changes:
  add-limit-injection     0/25 tasks    just now

$ npx -y @fission-ai/openspec@latest list --specs
No specs found.
```

`No specs found` is correct and important: at this point the delta exists but
nothing has been merged into canonical truth. The change is a proposal, not a fact.

---

## 6. Archiving — the merge

This is the step that no static example of OpenSpec shows, and it is the whole
thesis of the tool.

```
$ npx -y @fission-ai/openspec@latest archive add-limit-injection --yes < /dev/null

Proposal warnings in proposal.md (non-blocking):
  ⚠ Consider splitting changes with more than 10 deltas
Task status: 0/25 tasks
Warning: 25 incomplete task(s) found. Continuing due to --yes flag.

Specs to update:
  query-limit-injection: create
  sql-parse-fallback: create
Applying changes to openspec/specs/query-limit-injection/spec.md:
  + 8 added
Applying changes to openspec/specs/sql-parse-fallback/spec.md:
  + 3 added
Totals: + 11, ~ 0, - 0, → 0
Specs updated successfully.
Change 'add-limit-injection' archived as '2026-08-08-add-limit-injection'.
```

Note that archive warns about the 25 unfinished tasks and proceeds anyway under
`--yes`. Task completion is not a gate. If you want it to be one, you have to not
pass `--yes`, which means answering an interactive prompt.

The tree before:

```
openspec/changes/add-limit-injection/.openspec.yaml
openspec/changes/add-limit-injection/design.md
openspec/changes/add-limit-injection/proposal.md
openspec/changes/add-limit-injection/specs/query-limit-injection/spec.md
openspec/changes/add-limit-injection/specs/sql-parse-fallback/spec.md
openspec/changes/add-limit-injection/tasks.md
openspec/config.yaml
```

and after:

```
openspec/changes/archive/2026-08-08-add-limit-injection/.openspec.yaml
openspec/changes/archive/2026-08-08-add-limit-injection/design.md
openspec/changes/archive/2026-08-08-add-limit-injection/proposal.md
openspec/changes/archive/2026-08-08-add-limit-injection/specs/query-limit-injection/spec.md
openspec/changes/archive/2026-08-08-add-limit-injection/specs/sql-parse-fallback/spec.md
openspec/changes/archive/2026-08-08-add-limit-injection/tasks.md
openspec/config.yaml
openspec/specs/query-limit-injection/spec.md          ← new
openspec/specs/sql-parse-fallback/spec.md             ← new
```

The change folder moved wholesale, date-prefixed. Nothing was deleted. The delta
specs still sit inside the archived change exactly as written — the archive is the
audit trail, and you can always read back what a given change claimed.

The two new canonical specs are the delta with three edits. Here is the actual diff
between the archived delta and the canonical spec it produced:

```diff
-## Purpose
+# query-limit-injection Specification

+## Purpose
 Defines the row cap the proxy imposes on queries travelling to Doris: ...
-
-## ADDED Requirements
-
+## Requirements
 ### Requirement: Cap the top-level result set
```

That is it. A title line is added, `## ADDED Requirements` becomes `## Requirements`,
`## Purpose` is retained and its blank line collapsed. Requirement and scenario
bodies are copied verbatim. The merge is textual and predictable, which is what you
want — nothing is being interpreted.

```
$ npx -y @fission-ai/openspec@latest list --specs
Specs:
  query-limit-injection     requirements 8
  sql-parse-fallback        requirements 3

$ npx -y @fission-ai/openspec@latest list
No active changes found.
```

The change is gone from the active list. The specs are now truth.

---

## 7. A second change that modifies the first

The first change shipped a deliberate, documented flaw: it caps derived tables, so
`SELECT COUNT(*) FROM (SELECT id FROM events) t` returns 200 instead of the real
count. The design document said so explicitly and said the fix would wait for
production evidence. The second change is that fix.

```
$ npx -y @fission-ai/openspec@latest new change exempt-aggregate-subqueries \
    --description "Stop capping derived tables that feed only aggregates"
Created change 'exempt-aggregate-subqueries' at openspec/changes/exempt-aggregate-subqueries/
```

`--description` writes a two-line `README.md` in the change folder. It has no other
effect.

Its proposal declares no new capabilities and one modified one:

```markdown
### Modified Capabilities

- `query-limit-injection`: sub-query capping gains an aggregate-only exemption, and
  the audit record must report exemptions alongside caps.
```

Its delta spec has `## ADDED Requirements` (one new requirement defining the
classifier) and `## MODIFIED Requirements` (two requirements rewritten). Note there
is **no** `## Purpose` — the capability already has one in the canonical spec, and
the instructions are explicit that a delta for an existing capability must not
carry one.

The MODIFIED rule that bites: a MODIFIED block replaces the entire requirement, so
you must copy the whole existing requirement out of the canonical spec and edit it,
not write only the changed sentence. First attempt at validation:

```
$ npx -y @fission-ai/openspec@latest validate --all --strict
- Validating...
✗ change/exempt-aggregate-subqueries
✓ spec/query-limit-injection
✓ spec/sql-parse-fallback
Totals: 2 passed, 1 failed (3 items)

$ npx -y @fission-ai/openspec@latest validate exempt-aggregate-subqueries --type change --strict
Change 'exempt-aggregate-subqueries' has issues
✗ [ERROR] query-limit-injection/spec.md: MODIFIED "Cap row-producing sub-queries"
  omits scenario(s) the current spec still has: "Capping a derived table changes an
  aggregate result". Copy them into the MODIFIED block (a MODIFIED requirement
  replaces the whole block, so archive refuses to drop them).
```

This is a genuinely good check and it caught a real authoring error. Section 9
covers what it does *not* let you do about it.

After fixing:

```
$ npx -y @fission-ai/openspec@latest validate --all --strict
- Validating...
✓ change/exempt-aggregate-subqueries
✓ spec/query-limit-injection
✓ spec/sql-parse-fallback
Totals: 3 passed, 0 failed (3 items)
```

Notice `validate --all` now covers the canonical specs too, which it could not
before the first archive.

```
$ npx -y @fission-ai/openspec@latest archive exempt-aggregate-subqueries --yes < /dev/null
Task status: 0/16 tasks
Warning: 16 incomplete task(s) found. Continuing due to --yes flag.

Specs to update:
  query-limit-injection: update
Applying changes to openspec/specs/query-limit-injection/spec.md:
  + 1 added
  ~ 2 modified
Totals: + 1, ~ 2, - 0, → 0
Specs updated successfully.
Change 'exempt-aggregate-subqueries' archived as '2026-08-08-exempt-aggregate-subqueries'.
```

`create` became `update`. `+ 1 added, ~ 2 modified` matches the delta exactly.

---

## 8. The payoff: canonical spec before and after

`openspec/specs/query-limit-injection/spec.md` went from 143 lines / 8 requirements
to 197 lines / 9 requirements. Full diff also saved as
`canonical-spec-BEFORE-AFTER.diff`, with the pre-merge file as
`canonical-spec-BEFORE-second-archive.md`.

```diff
@@ -24,6 +24,8 @@
 Each such node is capped independently; capping the outer query does not exempt an
 inner one, because Doris materialises the inner scan before the outer bound takes effect.

+The proxy SHALL NOT cap a row-producing sub-query that is aggregate-only. Such a
+sub-query yields a single row to its parent, so the cap protects nothing while
+changing the answer.
+
 #### Scenario: Derived table is capped

@@ -44,10 +46,26 @@
 #### Scenario: Capping a derived table changes an aggregate result

 - **WHEN** the client sends `SELECT COUNT(*) FROM (SELECT id FROM events) t` and
   `events` holds 5,000,000 rows
-- **THEN** the proxy forwards a statement whose inner query carries `LIMIT 200`
-- **AND** the client receives `200` rather than `5000000`
-- **AND** the proxy records the capped inner node in the rewrite record so the
-  discrepancy is attributable
+- **THEN** the inner query is forwarded without a `LIMIT`
+- **AND** the client receives `5000000`, not `200`
+- **AND** only the outer query carries `LIMIT 200`

+This scenario keeps the name it was given when the outcome was the opposite. ...
+
+#### Scenario: Grouped parent keeps the cap on its derived table
+
+- **WHEN** the client sends `SELECT region, COUNT(*) FROM (SELECT region FROM events) t GROUP BY region`
+- **THEN** the inner query carries `LIMIT 200`
+- **AND** the returned per-region counts are computed over at most 200 rows
+
+#### Scenario: Exempt sub-query is unbounded
+
+- **WHEN** a sub-query is exempted as aggregate-only
+- **THEN** no row bound is imposed on its scan by the proxy
+- **AND** Doris' query timeout is the only remaining limit on its cost
+
 ### Requirement: Never loosen an existing bound

@@ -113,15 +131,21 @@
 ### Requirement: Make every rewrite auditable

-For each statement it modifies, the proxy SHALL emit a structured record containing
-the original SQL text, the forwarded SQL text, and the set of query nodes that
-received a cap.
+For each statement it modifies, the proxy SHALL emit a structured record containing
+the original SQL text, the forwarded SQL text, the set of query nodes that received
+a cap, and the set of row-producing nodes that were exempted from the cap together
+with the reason for each exemption.

+#### Scenario: Record identifies exempted nodes and why
+
+- **WHEN** the proxy rewrites `SELECT COUNT(*) FROM (SELECT id FROM events) t`
+- **THEN** the emitted record names the derived table as exempted
+- **AND** gives the reason as aggregate-only
+
@@ -141,3 +165,33 @@
+### Requirement: Classify a sub-query as aggregate-only
+
+The proxy SHALL classify a row-producing sub-query as aggregate-only when, and only
+when, its immediate parent query satisfies all of the following: ...
```

Two things to notice about the merge mechanics.

First, MODIFIED requirements are patched **in place** — the requirement keeps its
position in the file, so the spec stays readable rather than accumulating a change
log. ADDED requirements are **appended to the end**. Over many changes the file's
ordering drifts toward "original requirements in original order, then everything
ever added, in the order it was added". There is no reordering facility.

Second, the canonical spec after two changes contains no trace of the fact that
there were two changes. It reads as one coherent document. The history lives in
`changes/archive/`, where both proposals and both designs — including the first
design's explicit "this is wrong and here is why we're shipping it anyway" — remain
readable. Splitting proposal from spec is what buys you that.

---
## 9. What is actually enforced

I deliberately broke things and ran the tool. Everything below is observed
behaviour in 1.8.0, not a reading of the documentation. The malformed-input probes
were run in a separate throwaway repo so the deliverable state stays clean.

The first thing to understand is that there are **three** gates, not one, and they
check different things:

1. `openspec validate <change> --type change` — structural checks on the delta.
2. `openspec validate <spec> --type spec` — structural checks on canonical truth.
   `validate --all` runs this over `openspec/specs/`, so it only has anything to say
   after your first archive.
3. `openspec archive` — re-runs gate 2 against the **rebuilt** spec before writing,
   and aborts if it fails. This is the strictest gate and the last one you reach.

That layering explains most of the results.

| Malformation | change `--strict` | `archive` |
|---|---|---|
| A. `## Purpose` deleted from a new capability's delta | pass | **writes a `TBD` placeholder into canonical truth** |
| B. `## Purpose` present but 10 chars | pass | pass, with a warning |
| C. `#### Scenario:` demoted to `### Scenario:` | pass (scenario silently vanishes) | **aborts** |
| D. Requirement with zero scenarios | **fail** | — |
| E. `## Capabilities` deleted from the proposal entirely | pass | pass — creates an undeclared capability |
| F. Delta spec files deleted (zero deltas) | **fail** | — |
| G. MODIFIED block drops a scenario the base spec still has | **fail** | — |
| H. MODIFIED names a requirement that does not exist in the base spec | pass | **aborts cleanly** |

### The ones that fail early, and fail well

D:

```
✗ [ERROR] sql-parse-fallback/spec.md: ADDED "Orphan requirement with no scenario"
  must include at least one scenario
```

F:

```
✗ [ERROR] file: Change must have at least one delta. No deltas found. Ensure your
  change has a specs/ directory with capability folders (e.g. specs/http-server/spec.md)
  containing .md files that use delta headers (## ADDED/MODIFIED/REMOVED/RENAMED
  Requirements) ... If this change intentionally modifies no specs (pure refactor,
  tooling, docs), set "skip_specs: true" in the change's .openspec.yaml instead.
```

G is the one that caught a real mistake I made while writing the second change, and
it is the best check in the tool:

```
✗ [ERROR] query-limit-injection/spec.md: MODIFIED "Cap row-producing sub-queries"
  omits scenario(s) the current spec still has: "Capping a derived table changes an
  aggregate result". Copy them into the MODIFIED block (a MODIFIED requirement
  replaces the whole block, so archive refuses to drop them).
```

### The ones that only fail at archive

H — I renamed a MODIFIED header to `Cap row-producing subqueries` (one hyphen
dropped). `validate --strict` reported the change as valid. Archive did not:

```
$ npx -y @fission-ai/openspec@latest archive exempt-aggregate-subqueries --yes < /dev/null
Specs to update:
  query-limit-injection: update
query-limit-injection MODIFIED failed for header "### Requirement: Cap row-producing subqueries" - not found
Aborted. No files were changed.
```

The abort is atomic — I checked the canonical spec afterwards and all eight original
requirement headers were intact.

C is the subtler one, and it is where the `instructions` output's capitalised
warning ("Using 3 hashtags or bullets will **fail silently**") turns out to be half
right. I wrote a requirement with one well-formed `#### Scenario:` and one demoted
`### Scenario:`. The change validated clean, and `show --json --deltas-only` shows
what the tool actually parsed:

```json
"requirement": {
  "text": "The system SHALL probe twice.",
  "scenarios": [
    { "rawText": "- **WHEN** a probe happens\n- **THEN** it is recorded" }
  ]
}
```

One scenario. The demoted block is simply not there — no warning, no trace. So the
content loss is real and silent at the delta stage. But archive rebuilds the spec,
where a bare `###` header sits at requirement level, and catches it:

```
Validation errors in rebuilt spec for probe-cap (will not write changes):
  ✗ Requirement must have at least one scenario
  ⚠ Requirement must have at least one scenario. Scenarios must use level-4 headers.
    Convert bullet lists into:
#### Scenario: Short name
- **WHEN** ...
- **THEN** ...
- **AND** ...
Aborted. No files were changed.
```

Good outcome, arrived at by accident of parsing rather than by design — and only at
the last possible moment.

### Purpose: the check exists, just not where the change is

A and B look like the documentation lying, and are more interesting than that. The
`instructions specs` text says a brief `## Purpose` is something
"`openspec validate --strict` reports". It does — but only when validating a
**spec**, never a **change**:

```
$ npx -y @fission-ai/openspec@latest validate probe-b-e --type change --strict
Change 'probe-b-e' is valid

$ npx -y @fission-ai/openspec@latest validate probe-two --type spec --strict
Specification 'probe-two' has issues
⚠ [WARNING] overview: Purpose section is too brief (less than 50 characters)
```

Since the delta is a change and the spec does not exist yet, a short or missing
Purpose is unreachable by validation until after you have archived it. Archive
itself does warn, and then proceeds:

```
Specs to update:
  probe-two: create
⚠️  Warning: probe-two - carried Purpose is under 50 characters; openspec validate
   --strict reports it as too brief.
Applying changes to openspec/specs/probe-two/spec.md:
  + 1 added
Specs updated successfully.
```

Case A is worse, because it is quiet. Delete `## Purpose` entirely and archive
succeeds with no warning at all, writing this into canonical truth:

```markdown
# probe-cap Specification

## Purpose
TBD - created by archiving change probe-no-purpose. Update Purpose after archive.
## Requirements
```

And that placeholder is 79 characters, so it clears the 50-character check
permanently:

```
$ npx -y @fission-ai/openspec@latest validate probe-cap --type spec --strict
Specification 'probe-cap' is valid
```

A capability whose entire stated purpose is "TBD" validates forever. The only fix is
to hand-edit `openspec/specs/<capability>/spec.md`, which the instructions do tell
you — but nothing will ever remind you.

### The proposal is barely checked at all

E: I deleted the `## Capabilities` section from a proposal — the section the tool's
own instructions call "critical", the one described as "the contract between
proposal and specs phases" — and shipped a delta for a capability the proposal never
mentions. Change validation passed. Archive created the capability without comment.
The contract is a discipline, not a constraint.

The only proposal content check I saw fire is a length check on `## Why`, and it
appears only at archive, as a non-blocking warning:

```
Proposal warnings in proposal.md (non-blocking):
  ⚠ Why section must be at least 50 characters
```

Alongside the one my real proposal triggered:

```
  ⚠ Consider splitting changes with more than 10 deltas
```

### Summary

What is enforced is **structural completeness of the spec deltas**: every
requirement has a scenario, a change has at least one delta, a MODIFIED block does
not silently discard scenarios, and a MODIFIED header must actually exist. What is
not enforced is **whether you wrote the right things, or wrote them where you said
you would**. The proposal is essentially free text. Purpose text is checked only
after it has already become permanent. And the last two gates fire at `archive`, the
single command you run once at the end.

### A limitation worth knowing before you hit it

Check G prevents accidental scenario loss, which is good. But there is no way to
*deliberately* retire a scenario from a requirement that survives. I tried three
routes on the scenario "Capping a derived table changes an aggregate result", whose
name became false once the behaviour was inverted:

- `REMOVED` + `MODIFIED` the same requirement →
  `✗ Requirement present in both MODIFIED and REMOVED`
- `REMOVED` + `ADDED` the same requirement →
  `✗ Requirement present in both ADDED and REMOVED`
- `RENAMED` the requirement, then `MODIFIED` under the new name → the scenario check
  follows the rename and fires anyway

So a scenario name, once archived, is permanent for as long as its requirement lives.
Meanwhile the check is name-only: I kept the name and inverted every `WHEN`/`THEN`
bullet beneath it, and validation passed. You may gut a scenario's meaning freely;
you may not retire its title. The spec in this repo carries the awkward result — a
scenario called "Capping a derived table changes an aggregate result" that now
asserts the capping does not happen — with a sentence explaining why the name stayed.
That is the tool shaping the document, and not for the better.

## 10. Counts

Final state in this directory:

| | files | lines |
|---|---|---|
| `openspec/specs/` (canonical truth) | 2 | 251 |
| `openspec/changes/archive/` (2 changes) | 12 | 565 |
| `openspec/config.yaml` | 1 | 32 |
| **`openspec/` total** | **15** | **848** |
| `dot-claude/` (agent scaffolding from `init`) | 12 | 2,266 |

Canonical `query-limit-injection`: 197 lines, 9 requirements, 25 scenarios.
Canonical `sql-parse-fallback`: 54 lines, 3 requirements, 6 scenarios.

Change 1: 35-line proposal, 197 lines of delta spec across 2 files, 80-line design,
25 tasks. Change 2: 28-line proposal, 103-line delta spec, 46-line design, 16 tasks.

The ratio to notice: the agent scaffolding is 2.7× the size of everything the
process produced. OpenSpec is primarily a prompt library with a validator attached,
and if you drive it by hand as this walkthrough does, most of what `init` installed
goes unused.

---

## 11. Things that are awkward

- The word `validate` covers two different validators — one for changes, one for
  specs — and `--strict` means different things to each. The Purpose checks live on
  the spec side, so they cannot reach a delta. This is not documented anywhere I
  saw; I found it by getting a warning from `archive` that named a check
  `validate --strict` had just declined to perform.
- The last line of defence is `archive`, which is the one command you run once, at
  the end, when you least want a surprise. It aborts cleanly and atomically, but a
  `--dry-run` would let you find these earlier. There isn't one. The closest thing
  is running `archive` in a scratch copy of the repo, which is what I did.
- A missing `## Purpose` is silently converted into a permanent `TBD` placeholder in
  canonical truth, and the placeholder is long enough to pass the length check that
  would otherwise flag it. This is the one failure mode with no gate anywhere in the
  pipeline.
- `openspec show <change>` prints the proposal and nothing else. Not the specs, not
  the design, not the tasks. To see the delta you open the file or use
  `show --json --deltas-only`.
- Scenario names are immutable once archived, as covered above.
- Task completion does not gate archiving; `--yes` walks past 25 unfinished tasks
  with a warning. The "spec first, then build, then archive" loop is a convention
  the tool describes but does not enforce.
- The proposal warning `⚠ Consider splitting changes with more than 10 deltas` fired
  on 11 requirements. It counts requirements, not conceptual scope, so a
  well-factored capability with many small requirements trips it for no reason.
- `openspec new change --description` writes a README nothing else reads.
- ADDED requirements are appended to the end of the canonical spec, so file order
  drifts over time and there is no way to reorder without hand-editing.
- The CLI surface is large — `store`, `workset`, `schema`, `doctor`, `context`,
  `view`, `completion` — and none of it is needed for the core loop. `doctor` on a
  healthy two-spec repo reports only "OpenSpec root: ok" and "References: (none
  declared)".

## What actually earns its keep

Strip away the surface area and what remains is worth having. Writing the delta
against the existing spec forces you to read the existing spec. The MODIFIED rule —
copy the whole requirement, edit it, and the validator checks you did not silently
drop scenarios — is the mechanism that keeps a spec from rotting into a pile of
amendments. And the archive step produces something no changelog gives you: a
current-state document that reads as though it were written all at once, with the
full argument for every clause preserved separately in the archive. The first
change's design document says, in writing, "this returns the wrong count and we are
shipping it anyway, here is why, here is what would fix it". The second change is
that fix. Six months later both are still on disk, and the canonical spec shows
neither — just what the proxy does.
