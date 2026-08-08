# GitHub Spec Kit: a worked example

This is a record of running GitHub Spec Kit end to end on one feature, with the real CLI and the real
scripts. Every artifact in `specs/` was produced by copying the tool's own templates and filling them; no
file here was written to imitate the format. Command output is real, trimmed only of unrelated shell noise.

The feature: **an L7 MySQL proxy in front of an Apache Doris cluster that parses each incoming SQL
statement and appends a row limit before forwarding, so that unbounded scans stop hammering the cluster.**
Fixed constraints: one backend connection per client, passthrough authentication, SQL parsed with the
`sqlparser` crate (`MySqlDialect`; there is no Doris dialect, so some valid Doris SQL will not parse).

The software is the exercise. What is worth your attention is where the process pushed back.

## Contents of this directory

| Path | What it is |
|------|------------|
| `specs/001-l7-mysql-proxy/` | The complete artifact set for one feature |
| `dot-specify/` | The `.specify/` directory the CLI installed — scripts, templates, constitution |
| `dot-claude/` | The `.claude/skills/speckit-*/` directory — the actual command definitions |
| `lean-preset/` | A copy of the `lean` preset's files, kept for reference after it was uninstalled |
| `spec-before-clarify.md` | `spec.md` as it stood before clarification, for the diff below |

`.specify` and `.claude` are renamed so they do not register as live config in this repository.

---

## 1. Install

```sh
uvx --from git+https://github.com/github/spec-kit.git specify init --here \
    --integration claude --script sh --force --ignore-agent-tools
```

```
Selected coding agent integration: claude
Selected script type: sh
Initialize Specify Project
├── ● Check required tools (ok)
├── ● Select coding agent integration (claude)
├── ● Select script type (sh)
├── ● Install integration (Claude Code)
├── ● Install shared infrastructure (scripts (sh) + templates)
├── ● Ensure scripts executable (5 updated)
├── ● Constitution setup (copied from template)
├── ● Install bundled workflow (speckit installed)
└── ● Finalize (project ready)

Project ready.
```

The flag is `--integration`, not `--ai`. What lands is 35 files: five bash scripts, five markdown templates,
ten skill definitions, and a constitution template. 4,648 lines of tooling before you have written anything.

Two halves matter and they work differently:

- `.specify/scripts/bash/*.sh` — deterministic. They resolve paths, allocate feature numbers, copy templates
  and check preconditions. They never write content.
- `.claude/skills/speckit-*/SKILL.md` — prompts for the agent. `speckit-specify` is 348 lines, `speckit-plan`
  169, `speckit-clarify` 294. These are where the actual judgment lives.

This split is the thing to understand about Spec Kit. The scripts are the enforceable part; everything else
is instructions to a model, which means it holds exactly as well as the model follows them.

## 2. The constitution

`init` copies `.specify/templates/constitution-template.md` to `.specify/memory/constitution.md` as a
placeholder skeleton — `[PRINCIPLE_1_NAME]`, `[PRINCIPLE_1_DESCRIPTION]`, and so on. Filling it is
authoring work, guided by `.claude/skills/speckit-constitution/SKILL.md`.

Result: `dot-specify/memory/constitution.md`, 143 lines, five principles. The one that ends up mattering:

> ### I. Semantic Preservation (NON-NEGOTIABLE)
>
> A query that this proxy rewrites MUST return the same rows, in the same order, with the same computed
> values, as the original query would have returned — except that the final result set MAY be truncated to
> the first N rows.
>
> […] When this principle conflicts with reducing cluster load, this principle wins.

The constitution is written once for the project, not per feature. Its job is to be a thing a later document
has to argue against. Keep reading — it does that in section 6.

**What it is for**: it is the only artifact that outlives the feature. Everything else in `specs/` describes
one change; the constitution describes what is always true, so that the next feature's plan has something to
fail against.

## 3. Create the feature

```sh
.specify/scripts/bash/create-new-feature.sh --json "L7 MySQL proxy in front of Apache Doris that
parses each incoming SQL statement and appends LIMIT 200 to queries and sub-queries before forwarding,
to stop unbounded scans from hammering the cluster"
```

```
# To persist: export SPECIFY_FEATURE=001-l7-mysql-proxy
#              export SPECIFY_FEATURE_DIRECTORY=/…/sk3/specs/001-l7-mysql-proxy
{"BRANCH_NAME":"001-l7-mysql-proxy","SPEC_FILE":"/…/sk3/specs/001-l7-mysql-proxy/spec.md","FEATURE_NUM":"001"}
```

The script derives a short name from the description by stripping stop words, allocates the next free number
by scanning `specs/`, creates the directory, copies the spec template into it, and writes
`.specify/feature.json` so later commands can find the feature without relying on the git branch name. It
does not create a branch — that is a separate hook, off by default.

Then `spec.md` gets filled from the template. **The spec is about behavior, not implementation.** No Rust, no
crate names, no module layout. Four user stories, prioritized and each independently testable; functional
requirements; success criteria that must be technology-agnostic and measurable; assumptions. 251 lines.

The template's own checklist enforces the boundary — `speckit-specify` generates
`checklists/requirements.md` with items including "No implementation details (languages, frameworks, APIs)"
and "Success criteria are technology-agnostic". It is a self-graded checklist, so it is a prompt to the model
rather than a gate, but it is specific enough to be checkable by a human reader in a minute.

## 4. `[NEEDS CLARIFICATION]` — the mechanism worth seeing

The spec template invites markers for decisions the author cannot make alone:

```markdown
- **FR-006**: System MUST authenticate users via [NEEDS CLARIFICATION: auth method not specified …]
```

`speckit-specify` caps them at three and says to use one only when "multiple reasonable interpretations
exist with different implications". This feature had more than three candidates. The design tensions in
appending a limit to every sub-query are:

- `SELECT COUNT(*) FROM (SELECT … FROM big) t` — capping the inner query gives a **wrong count**
- `ORDER BY` in a sub-query — capping changes *which* rows survive, not just how many
- `UNION` branches, `WITH` clauses, derived tables, `IN`/`EXISTS` sub-queries — same problem
- a query that already says `LIMIT 500` — replace, take the minimum, or leave alone? and what of `OFFSET`?
- `INSERT INTO … SELECT` — capping silently writes fewer rows than the author asked for
- parse failure — forward unchanged, or reject?

Most of these are not judgment calls at all once the constitution exists. Principle I settles every
sub-query case outright, and Principle II settles parse failure ("forward unchanged, never reject what you
merely failed to understand"). That is the constitution doing useful work: it converts what look like open
questions into decided ones, so they never reach the user.

Three genuinely remained the user's call, and were marked:

```markdown
- **FR-006**: When a statement's outermost result already specifies its own row count, System MUST
  [NEEDS CLARIFICATION: leave the user's limit untouched regardless of its size, replace it with the
  200-row cap, or apply the smaller of the two? And when an offset accompanies the limit, is the cap
  applied to the rows after the offset or to the offset and limit combined?]
```

### Running the clarification workflow

`.claude/skills/speckit-clarify/SKILL.md` (294 lines) specifies a strict interaction: run
`check-prerequisites.sh --json --paths-only` once, scan the spec against an eleven-category ambiguity
taxonomy, queue at most five questions, ask **exactly one at a time**, and after each accepted answer write
it into the spec immediately before asking the next.

```sh
.specify/scripts/bash/check-prerequisites.sh --json --paths-only
```

```
{"REPO_ROOT":"/…/sk3","BRANCH":"001-l7-mysql-proxy","FEATURE_DIR":"/…/specs/001-l7-mysql-proxy",
 "FEATURE_SPEC":"/…/spec.md","IMPL_PLAN":"/…/plan.md","TASKS":"/…/tasks.md"}
```

The question format is prescribed down to the punctuation: lead with `**Question:**`, end with a `?`, add one
plain-language "why it matters" sentence, then a **recommended option with reasoning** before the table of
alternatives. The recommendation is required, not optional — the workflow is built so the user can answer
"yes" and move on.

The first question as produced:

> **Question:** When a statement already specifies its own row limit, how should the 200-row cap interact
> with it? (FR-006)
>
> Why it matters: it decides whether `LIMIT 5000000` — the exact "give me everything" case the proxy exists
> to stop — passes through untouched.
>
> **Recommended:** Option C — one invariant ("no result exceeds 200 rows") is far easier to explain, test
> and reason about than a rule with exceptions.
>
> | Option | Description |
> |--------|-------------|
> | A | Leave the user's limit untouched, whatever its size |
> | B | Always replace it with 200 |
> | C | Apply the smaller of the user's limit and 200; offset is applied first and never altered |

Answers: **C**, then **B** for `INSERT INTO … SELECT` (forward unchanged, record a distinct reason), then
**B** for truncation signalling (a retrievable advisory that does not alter rows or columns).

*Honest note: there was no live human in this run. The answers were chosen by the agent acting as the
stakeholder. The workflow, the question format and the write-back are real; the decisions are stand-ins.*

### Before and after

The real diff, `spec-before-clarify.md` against `specs/001-l7-mysql-proxy/spec.md`:

```diff
+## Clarifications
+
+### Session 2026-08-08
+
+- Q: When a statement already specifies its own row limit, how should the proxy's 200-row cap interact
+  with it? → A: Apply the smaller of the user's limit and 200; the offset is applied first and is never altered.
+- Q: For statements that write rows as a result of an inner query, such as `INSERT INTO ... SELECT`, should
+  the proxy cap the inner query, leave the statement alone, or reject it? → A: Leave it alone and record a
+  distinct reason; never cap and never reject.
+- Q: When the proxy truncates a result the user did not ask to truncate, should the client be able to tell?
+  → A: Yes — a retrievable advisory attached to the statement's response, which does not alter the rows or
+  columns returned.
```

```diff
 - **FR-006**: When a statement's outermost result already specifies its own row count, System MUST
-  [NEEDS CLARIFICATION: leave the user's limit untouched regardless of its size, replace it with the
-  200-row cap, or apply the smaller of the two? And when an offset accompanies the limit, is the cap
-  applied to the rows after the offset or to the offset and limit combined?]
+  return the smaller of that row count and 200. Any offset the user specified MUST be applied first and
+  MUST NOT be altered, so that the rows returned are the first 200 or fewer of the rows the user's own
+  limit and offset select.
```

The part that makes this more than bookkeeping is the knock-on edit. The skill says: *"If the clarification
invalidates an earlier ambiguous statement, replace that statement instead of duplicating; leave no obsolete
contradictory text."* Answer C invalidated a requirement written before the question was asked:

```diff
-- **FR-014**: Users MUST be able to obtain a full, uncapped result by writing an explicit row count in
-  their statement, without any proxy-side configuration change. (Dependent on the resolution of FR-006.)
+- **FR-014**: System MUST NOT provide any in-statement means of exceeding the 200-row cap. A user who
+  needs more rows than the cap allows must obtain them by a route other than this proxy; the proxy MUST
+  make that visible through the FR-008 advisory rather than by silently honoring a larger request.
```

An assumption, a success criterion and a user-story acceptance scenario changed with it. Four sections
touched by one answer. That is what the "integrate after EACH accepted answer" rule is for — it forces the
consequences to be chased down while the reason for the change is still in front of you.

The checklist is then re-validated. It went from 13/16 to 16/16 items passing, with no regressions. Verified:

```sh
grep -c 'NEEDS CLARIFICATION' specs/001-l7-mysql-proxy/spec.md
0
```

**What the spec is for**: it is the artifact you can hand to someone who will never read the code. Every
question it leaves open is a question that will otherwise be answered silently, by whoever writes the code
first, at the point where it is most expensive to revisit.

## 5. The plan

```sh
.specify/scripts/bash/setup-plan.sh --json
```

```
Copied plan template to /…/specs/001-l7-mysql-proxy/plan.md
{"FEATURE_SPEC":"/…/spec.md","IMPL_PLAN":"/…/plan.md","SPECS_DIR":"/…/specs/001-l7-mysql-proxy",
 "BRANCH":"001-l7-mysql-proxy"}
```

**What the plan is for, and why it is not the spec**: the spec says what must be true for the user; the plan
says how this codebase will make it true. The separation earns its keep in exactly one place — the plan is
where the constitution gets applied, and the constitution can only constrain a design, not a behavior. You
cannot check "does this violate Semantic Preservation" against a spec that never mentions an AST.

Technical Context in the plan is where Rust, `sqlparser`, `tokio` and `tracing` appear for the first time.

## 6. The Constitution Check, doing real work

This is the section worth the whole exercise. The plan template contains only a stub:

```markdown
## Constitution Check
*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*
[Gates determined based on constitution file]
```

Evaluated principle by principle against the feature **as requested**:

| Principle | Result | Finding |
|-----------|--------|---------|
| I. Semantic Preservation | **FAIL** | Appending `LIMIT 200` to every query *and sub-query* violates this outright. `SELECT COUNT(*) FROM (SELECT … FROM big) t` returns 200 instead of the true count. This is not an edge case; it is the stated design. |
| II. Fail Open, Never Fail Wrong | PASS | FR-004/005/007 already forward what cannot be handled. |
| III. Every Rewrite Is Observable | PASS | FR-010 and SC-006 require per-statement records. |
| IV. Rewriter Is Test-First | PASS (with obligation) | Imposes an ordering constraint carried into `tasks.md`. |
| V. One Job Only | **AT RISK** | Recorded in Complexity Tracking rather than waved through. |

The gate could not be passed by rewording. The design had to change. What Principle I **deleted** from the
plan, in the plan's own words:

- a recursive visitor appending a limit at every `Query` node — deleted, it is the direct cause of the wrong count
- capping individual `UNION` branches — deleted, the cap moves to the set operation as a whole
- capping a CTE body — deleted, a CTE's rows are consumed elsewhere
- capping the inner `SELECT` of `INSERT INTO … SELECT` — deleted, it would silently write 200 rows where the
  author asked for millions, and unlike a truncated read the damage persists

And then the part a process is usually too polite to write down:

> What is left is a much weaker feature than the request: on a statement like
> `SELECT COUNT(*) FROM (SELECT … FROM events) t`, the proxy caps the outer `COUNT` query, which already
> returns one row, and therefore does nothing useful at all. The scan still happens. Principle I says that
> is the correct outcome and that the load problem must be solved for those statements elsewhere — by Doris
> resource groups, not by this proxy.

The feature shipped is *smaller than the one asked for*, and the plan says so in the artifact rather than in
a conversation someone will forget. That is the mechanism working. A constitution that never fails a gate is
decoration.

Complexity Tracking then records the cost of complying — a three-column table, filled only when there is
something to justify:

| Violation | Why Needed | Simpler Alternative Rejected Because |
|-----------|------------|-------------------------------------|
| A dedicated AST position classifier, roughly a third of the rewrite module, where the request implied a one-line append | Principle I requires knowing, per position, whether its rows reach the client or feed another operator | Appending at every `Query` node is one visitor and about twenty lines. Rejected: it returns 200 for `SELECT COUNT(*) FROM (…) t` |
| A verification pass that re-parses the serialized output before forwarding | The proxy forwards SQL the user never wrote; Principle I's guarantee is only as strong as the serializer | Trusting the serializer round-trip. Rejected: fidelity is documented as best-effort and the failure mode is silent |

Note the second-order effect: Principle I forced complexity that Principle V objects to. The template has a
place for that, and the Governance section says the deviation is re-examined when the feature completes.

## 7. Phase 1 artifacts — count them honestly

`speckit-plan` mandates four more artifacts before any task exists: `research.md`, `data-model.md`,
`contracts/`, `quickstart.md`. Real line counts as filled:

| File | Lines | Verdict |
|------|-------|---------|
| `spec.md` | 251 | Earns its place |
| `plan.md` | 159 | Earns its place |
| `tasks.md` | 269 | Earns its place |
| `research.md` | 121 | Earns its place — six decisions with rejected alternatives, including why not to fork the parser |
| `contracts/cap-position-classifier.md` | 59 | Earns its place — the position table *is* the specification of Principle I |
| `contracts/rewrite-decision.md` | 69 | Earns its place |
| `contracts/rewrite-record.md` | 59 | Earns its place |
| `data-model.md` | 117 | **Half of it is padding.** See below |
| `quickstart.md` | 131 | Useful, but overlaps `tasks.md` |
| `checklists/requirements.md` | 41 | Thin |
| **Total, one feature** | **1,276** | plus a 143-line constitution written once |

Estimated production Rust for this feature: roughly 1,500–2,000 lines, plus 800–1,200 of tests. So the
specification set is somewhere near 50–70% of the code volume it describes, written before the code exists.

Whether that is proportionate depends on the feature. Here, arguably yes — the whole risk is a silent
wrongness that no runtime signal catches, and about 190 lines of contract are what pin it down. For a
CRUD endpoint it would be absurd.

### Where a mandated artifact fits badly

`data-model.md` is the clear case, and its closing note says so in the file itself:

> Roughly half of what a data model is normally for does not apply here. There is no persistence, no schema,
> no migration, no identity across time, and no state machine — the "lifecycle" of every entity above is
> "created, used once, dropped". […] What did earn its place is the middle of the document: the
> `CapPosition` safety classification and the closed set of `ForwardedUnchanged` reasons.

The template's frame — entities, fields, validation rules, relationships, state transitions — is built for
something with a database. This feature has none. The useful 40 lines could have lived in `contracts/`; the
other 77 exist because the plan template lists the file.

`quickstart.md` is a milder case. It is genuinely useful as a validation guide, but a fair amount of it
restates what `tasks.md` already says about which tests exist and why.

Also worth noticing: `contracts/` has no schema, no format, and no validation. `speckit-plan` says only
"Document the contract format appropriate for the project type". They are freeform markdown. Their quality
here comes from the effort put into the position table, not from anything the tool enforces.

## 8. Tasks

```sh
.specify/scripts/bash/setup-tasks.sh --json
```

```
{"FEATURE_DIR":"/…/specs/001-l7-mysql-proxy",
 "AVAILABLE_DOCS":["research.md","data-model.md","contracts/","quickstart.md"],
 "TASKS_TEMPLATE":"/…/.specify/templates/tasks-template.md"}
```

Note the asymmetry with `setup-plan.sh`: that one **copies** the template into place and says so
("Copied plan template to …"). `setup-tasks.sh` only *resolves and reports the path*. It creates nothing.
The agent must read the template and write `tasks.md` itself. Easy to get wrong if you assume symmetry.

`tasks-template.md` is 252 lines and prescribes real structure: `[ID] [P?] [Story] Description` format, a
Setup phase, a Foundational phase that blocks everything, one phase per user story in priority order, a
Polish phase, then dependency and parallel-execution sections. The result is 61 tasks, T001–T061, in
269 lines.

**What tasks.md is for, and why it is not the plan**: the plan is a design; `tasks.md` is an order of
operations with file paths. The grouping by user story is the substantive part — each story's phase ends at
a checkpoint where that story alone is complete and testable, so the work can stop at any checkpoint and
still have shipped something.

Constitution Principle IV shows up here as concrete ordering. Every classifier rule gets its tests written
first, and each test task pairs a positive case with a negative one:

```
- [ ] T027 [P] [US2] P4 pair in tests/contract/classifier_positions.rs:
      SELECT * FROM (SELECT a FROM big) t caps the outer query only;
      SELECT COUNT(*) FROM (SELECT a FROM big) t places no cap on the derived table
…
- [ ] T036 [US2] Classify DerivedTable, CteBody, InPredicate, ExistsPredicate, ScalarSubquery
      as Unsafe with their causes in src/rewrite/classify.rs (P4–P8)
```

The negative half is what a normal test suite omits, and it is the only thing that catches the failure this
feature was reshaped to avoid.

## 9. Enforcement — what is actually checked

This is where to be precise, because the answer is narrower than the ceremony suggests.

**Before any feature existed:**

```sh
.specify/scripts/bash/check-prerequisites.sh --json --require-tasks
echo $?
```

```
ERROR: Feature directory not found. Set SPECIFY_FEATURE_DIRECTORY or run the specify command to create .specify/feature.json.
ERROR: Failed to resolve feature paths
1
```

**After `plan.md` existed but before `tasks.md`:**

```sh
.specify/scripts/bash/check-prerequisites.sh --json --require-tasks --include-tasks
echo $?
```

```
ERROR: tasks.md not found in /…/specs/001-l7-mysql-proxy
Run /speckit-tasks first to create the task list.
1
```

**The same call, after `tasks.md` was written:**

```sh
.specify/scripts/bash/check-prerequisites.sh --json --require-tasks --include-tasks
echo $?
```

```
{"FEATURE_DIR":"/…/specs/001-l7-mysql-proxy","AVAILABLE_DOCS":["research.md","data-model.md","contracts/","quickstart.md","tasks.md"]}
0
```

Real exit codes, real messages. Now the honest part:

**What is mechanically enforced** — that `spec.md` exists before a plan, that `plan.md` exists before tasks,
that `tasks.md` exists before implementation, and that a feature directory has been allocated. `setup-plan.sh`
and `setup-tasks.sh` refuse to proceed without their inputs. That is the whole list.

**What is not** — nothing checks that a template's mandatory sections were filled. Nothing checks that
`[NEEDS CLARIFICATION]` markers were resolved; the `grep` above is something I ran, not something the tool
runs. Nothing validates that Success Criteria are technology-agnostic, that the Constitution Check was
performed rather than left as `[Gates determined based on constitution file]`, or that `contracts/` contains
anything at all. `check-prerequisites.sh` tests file existence — an empty `tasks.md` passes.

Every quality property in this walkthrough is enforced by a prompt telling a model to enforce it. The
checklist in `checklists/requirements.md` is self-graded by the same agent that wrote the spec. The
Constitution Check is a section the agent is asked to fill in honestly. These work — the FAIL in section 6 is
real and it did change the design — but they work the way a code review works, not the way a compiler does.
Spec Kit gives you the discipline of a scaffold, plus exit code 1 on four file-existence checks.

## 10. The lean preset

```sh
specify preset add lean
```

```
Installing bundled preset lean...
✓ Preset 'Lean Workflow' v1.0.0 installed (priority 10)
```

What it adds on disk: `.specify/presets/lean/` with five command files, a `preset.yml`, a README, a
`.registry`, and a catalog cache. Twelve new files.

What it actually changes: it **overwrites five skill definitions in place**. Measured before and after:

| Skill | Before | After lean | Change |
|-------|-------:|-----------:|--------|
| `speckit-specify` | 348 | 33 | −91% |
| `speckit-plan` | 169 | 29 | −83% |
| `speckit-tasks` | 217 | 29 | −87% |
| `speckit-implement` | 226 | 32 | −86% |
| `speckit-constitution` | 171 | 43 | −75% |
| **Total for these five** | **1,131** | **166** | **−85%** |

The other five skills — `clarify`, `analyze`, `checklist`, `converge`, `taskstoissues` — are untouched.

The lean `speckit.plan` in full is 19 lines:

```markdown
## Outline
1. Read `.specify/feature.json` to get the feature directory path.
2. **Load context**: `.specify/memory/constitution.md` and `<feature_directory>/spec.md`.
3. Create an implementation plan and store it in `<feature_directory>/plan.md`.
   - Technical context: tech stack, dependencies, project structure
   - Design decisions, architecture, file structure
```

**So: does lean remove the artifacts that fit badly?** Yes, and more than you may want. There is no Phase 0
and no Phase 1 in the lean plan command — `research.md`, `data-model.md`, `contracts/` and `quickstart.md`
simply are not produced. That is 426 of the 1,276 lines gone. But the Constitution Check goes with them:
lean's plan command loads the constitution and says nothing about evaluating gates against it. The FAIL in
section 6, which is the single most valuable thing that happened in this run, would not have occurred under
lean. Lean also drops `create-new-feature.sh` from the flow entirely — its `speckit.specify` **asks the user
to type a feature directory path** instead.

**Does lean change the templates?** No. Verified:

```sh
specify preset resolve spec-template
```

```
  spec-template: /…/.specify/templates/spec-template.md
    (top layer from: core)
```

Lean replaces *commands*, not templates. If you want a shorter `spec.md` rather than a shorter workflow, the
lever is elsewhere.

### `templates/overrides/` is the sharper tool

`resolve_template()` in `.specify/scripts/bash/common.sh` searches in this order:

```
1. .specify/templates/overrides/<name>.md     ← project override, always wins
2. .specify/presets/<id>/templates/<name>.md  ← installed presets, by priority
3. .specify/extensions/*/templates/           ← extensions
4. .specify/templates/<name>.md               ← core
```

Dropping a trimmed `plan-template.md` into `.specify/templates/overrides/` beats both presets and core, and
applies to every future feature. That is the way to delete the Phase 1 artifacts that do not suit your domain
while keeping the Constitution Check that does — a precision lean cannot offer, because lean is
all-or-nothing on five commands at once.

### A bug worth knowing about

```sh
specify preset remove lean
```

```
✓ Preset 'lean' removed successfully
```

Removal reports success, but the five restored skill files come back **with corrupted YAML frontmatter**.
Each grew by exactly 2 lines, and the `description` value has been re-folded across lines with the
continuation landing after `argument-hint`:

```yaml
---
name: speckit-plan
description: Execute the implementation planning workflow using the plan template
argument-hint: "Optional guidance for the planning phase"
  to generate design artifacts.
compatibility: Requires spec-kit project structure with .specify/ directory
```

The original:

```yaml
---
name: "speckit-plan"
description: "Execute the implementation planning workflow using the plan template to generate design artifacts."
argument-hint: "Optional guidance for the planning phase"
compatibility: "Requires spec-kit project structure with .specify/ directory"
```

The description is truncated mid-sentence and a stray continuation line sits inside the mapping. All five
lean-replaced skills are affected; `speckit-clarify`, which lean never touched, is intact. The install/remove
round trip is not lossless — it re-serializes frontmatter rather than restoring the original bytes.

Re-running `specify init --here --force` restores the files correctly (169/348/217 again) and does **not**
overwrite an existing `.specify/memory/constitution.md` — the second run's step list omits "Constitution
setup" entirely, and `specs/` is left alone. That is the recovery path. The artifacts in `dot-claude/` here
are the restored clean versions.

## 11. What to take from this

The commands that did real work: `specify init`, `create-new-feature.sh`, `setup-plan.sh`, `setup-tasks.sh`,
`check-prerequisites.sh`, `specify preset add|remove|list|resolve|info`. All five scripts behaved exactly as
documented, with clean JSON output and correct exit codes.

Three things Spec Kit gave that free-form work would not have:

1. **`[NEEDS CLARIFICATION]` plus a workflow that resolves it.** A marker is only half of it; the value came
   from the rule that each answer is written back and its consequences chased immediately. One answer here
   changed four sections including a requirement written before the question was asked.
2. **A constitution the plan has to argue against.** The Principle I FAIL deleted four rewrite behaviors and
   shrank the feature below what was asked for, in writing, in the artifact. No amount of care produces that
   without a prior document to fail against.
3. **Complexity Tracking.** A three-column table that forces the rejected simpler alternative to be named.
   "We built a classifier" is a decision; "we built a classifier because the twenty-line version returns 200
   for `COUNT(*)`" is a decision someone can overturn later on evidence.

Two things that are awkward:

1. **The mandated Phase 1 set does not fit every domain.** `data-model.md` for a stateless SQL rewriter is
   half padding, and the template gives no way to say "not applicable" other than writing the sections
   anyway. `contracts/` has no defined format at all — its quality is entirely the author's.
2. **Enforcement is thinner than the volume implies.** Four file-existence checks are the mechanical part.
   1,276 lines of artifact for one feature rests on a model following 4,648 lines of instructions, graded by
   itself. It works, and it worked well here, but it is discipline, not verification. Read the Constitution
   Check yourself; it is the section where a hurried agent has the most to gain from writing "PASS".
