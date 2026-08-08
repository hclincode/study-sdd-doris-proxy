# Milestone 1 — Choosing an SDD tool

**Period:** 2026-08-08 → 2026-08-09
**Status:** ✅ **Closed.** Research and trials complete.
**Outcome:** No tool chosen — **deliberately**. The decision moves to hands-on
use: the next step is trying OpenSpec and Spec Kit manually, in person, rather
than deciding from research.
**Purpose of this file:** a single place to re-orient. Read this instead of
re-reading the 4,600 lines under `surveys/`, `studies/` and `archives/milestone-1/trial/`.

---

## 1. What we wanted

The opening request was:

> "Discuss with me to choose the best SDD tool for this project. Currently I
> choose openspec with no reason."

An interview turned that into something more specific, and the most important
answer inverted the whole framing:

| # | Question | Answer |
|---|---|---|
| R1 | What is the project for? | **Learning SDD.** The Doris proxy is the exercise, not the deliverable |
| R2 | Team | Solo |
| R3 | Tooling | Claude Code CLI; wants portability across agents |
| R4 | What pain drove this? | **None — proactive** |
| R5 | Learning mode | One tool, deeply, end-to-end |
| R6 | Ceremony budget | Moderate — spec+plan+tasks fine; personas and 7-file features not |
| R7 | Proxy scope | Undecided at the time |
| R8 | Cost | Open source only |

**R1 changed the selection criteria.** Normally you pick the SDD tool with the
least friction that still prevents drift. Here, a tool that *hides* the method
teaches nothing — legibility beats efficiency.

**R4 was flagged as the main risk** and remains the main risk. Every one of
these tools is a cure for a specific pain. Process adopted without a felt pain
tends to be abandoned around week three.

---

## 2. What we did

### 2.1 Surveys — 2,345 lines across 9 files

Seven research agents ran in parallel, all following `surveys/_TEMPLATE.md`.

| File | Subject |
|---|---|
| `sdd-fundamentals.md` | SDD as a methodology — lineage, evidence, economics, criticism |
| `openspec.md` | OpenSpec v1.8.x |
| `github-spec-kit.md` | GitHub Spec Kit v0.16.x |
| `ide-native-sdd.md` | Kiro, Antigravity, Cursor, Cosmos |
| `agent-role-frameworks.md` | BMAD-METHOD, Agent OS, CCPM |
| `commercial-and-minimal-sdd.md` | Tessl, ZeroShot, CodeMySpec, **and the no-framework baseline** |
| `domain-fit-rust-proxy.md` | Which parts of *this* project are spec-shaped |
| `sql-limit-injection.md` | (later) whether the trial requirement was sound |
| `studies/2026-08-08_…md` | Head-to-head synthesis of the two finalists |

**The field collapsed fast**, mostly on stated constraints rather than merit:

- Kiro, Antigravity, Cursor, Cosmos — **eliminated by R3/R8** (IDE-locked, paid)
- Tessl, ZeroShot, CodeMySpec — **eliminated by R8** (commercial)
- BMAD and role-frameworks — **eliminated by R6** (persona ceremony)
- **Surviving:** OpenSpec · Spec Kit · no framework at all

### 2.2 Scope — settled after wandering

`discussions/milestone-1/02-proxy-scope.md` records this. The scope moved three times:

1. MySQL wire protocol as the worked example → **wrong**, cost ~50% of the
   reading time on material that was beside the point
2. Tenant/row isolation → security-relevant, forced fail-closed
3. **Final: parse SQL and append `LIMIT 200` to every query and sub-query**

Sections of `02-proxy-scope.md` analysing isolation are marked superseded. What
survived all three: L7 not L4, 1:1 backend connections, passthrough auth, the
`sqlparser` Doris-dialect gap, placeholder stability for prepared statements.

### 2.3 Trial — done twice, because the first was wrong

**Attempt 1 was hand-written from documentation and never run through either
tool.** It was materially malformed in eight ways — missing OpenSpec's mandatory
`## Capabilities` and `## Purpose` sections, wrong task numbering, wrong scenario
keyword, and missing six required Spec Kit sections. It was deleted.

**Attempt 2 installed and ran both tools for real** — OpenSpec via `npx`, Spec
Kit via `uvx`. Every artifact came from the tools' own templates and scripts,
with real command output captured.

- `archives/milestone-1/trial/openspec/WALKTHROUGH.md` — 764 lines, full lifecycle including **two
  changes and two archives**
- `archives/milestone-1/trial/speckit/WALKTHROUGH.md` — 611 lines, full pipeline including
  clarification, Constitution Check, all Phase-1 artifacts, and the lean preset
- `archives/milestone-1/trial/README.md` — the synthesis

---

## 3. What we learned about SDD

### 3.1 The core claim is real, and it was demonstrated on our own requirement

The requirement — *append `LIMIT 200` to every query and sub-query* — is wrong,
and **both tools independently caught it**, with no shared context:

`SELECT COUNT(*) FROM (SELECT … FROM big) t` returns `200` instead of the true
count. Not an edge case — it is what the requirement literally asks for.

- **Spec Kit** caught it at the **Constitution Check**. Principle I (Semantic
  Preservation) returned **FAIL**, and the gate could not be passed by
  rewording. Four behaviours were deleted from the plan, including capping the
  `SELECT` inside `INSERT INTO … SELECT` — which would have silently written 200
  rows where the author asked for millions. Unlike a truncated read, that damage
  persists.
- **OpenSpec** caught it across **two changes**: change 1 shipped the hazard
  knowingly with a design document saying so in writing; change 2 was the fix.

A third, independent finding came from research rather than either tool:

1. **Doris already ships `sql_select_limit`** — one `SET` per session gives the
   outermost row cap with zero parsing and zero semantic hazard.
2. **`LIMIT` does not bound scans.** It caps rows *returned*, not rows *read*.
   `SELECT COUNT(*) FROM huge_fact` returns one row and scans everything. The
   stated goal is not achievable this way; `SQL_BLOCK_RULE` and workload
   policies are.

**This is the milestone's main result.** A written spec caught a flawed premise
before any code existed, for a few hundred lines of markdown. Finding it in
production would have meant a wrong number on a dashboard nobody could explain.

### 3.2 Neither tool enforces anything semantic

Verified by deliberately feeding both malformed input.

| | What is mechanically enforced |
|---|---|
| **OpenSpec** | Structural completeness of deltas: every requirement has a scenario; a `MODIFIED` block carries the full requirement and cannot silently drop scenarios; a `MODIFIED` header must exist in the base spec — **but that last check fires only at `archive`** |
| **Spec Kit** | Four file-existence checks with real exit code 1: feature dir allocated, spec before plan, plan before tasks, tasks before implement. That is the entire list |

Neither checks that mandatory sections were filled, that clarification markers
were resolved, or that anything written is true. An empty `tasks.md` passes.

> **The durable lesson:** every quality property in either tool is enforced by a
> prompt telling a model to enforce it. They work the way a code review works,
> not the way a compiler does. In a Rust project the real enforcement is
> `cargo test`, `proptest` and `cargo-mutants`.
> **The tests keep the code honest; the spec keeps the tests honest.**

Any tool sold on "the spec constrains the agent" is overselling.

### 3.3 The mechanisms worth keeping, regardless of tool

1. **A constitution.** The highest-value artifact in either tool. It failed a
   gate here and changed a design. *A constitution that never fails a gate is
   decoration.*
2. **`[NEEDS CLARIFICATION]` markers.** Record a question *inside* the spec
   instead of guessing. OpenSpec has no equivalent.
3. **Delta-then-archive.** Writing a change against the existing spec forces you
   to read it; archiving leaves a current-state document with every argument
   preserved separately.
4. **An `unknowns.md` register.** Neither tool ships one. Needed for parser and
   dialect surprises, recorded with provenance.

### 3.4 Cost, measured

| | files | lines | covers |
|---|---:|---:|---|
| OpenSpec | 15 | 848 | **two** changes + canonical + archive |
| Spec Kit | 10 | 1,276 | **one** feature |

≈**400 vs 1,276 lines per unit of work — about 3×**, matching published
third-party measurements.

Standing scaffolding neither tool counts as output: OpenSpec installs 2,266
lines of skills and commands — **2.7× everything the process produced**. Both
tools are mostly prompt libraries with a little machinery attached. Their
standing prompt weight is within ~5% of each other (126 KB vs 132 KB), which
corrects an assumption carried through the early surveys.

**`specify preset add lean` cuts five Spec Kit skills by 85%** (1,131 → 166
lines). If verbosity is the objection, measure lean before rejecting the tool.

### 3.5 Process lessons — how this went wrong, twice

Worth recording, because working *with an AI* on SDD is half the exercise.

**Unstated context gets guessed at.** The proxy's purpose was never stated
early, so MySQL wire protocol was chosen as the worked example — technically
reasonable, completely beside the point, and roughly half the early reading time
was wasted on it. *A spec-driven process does not protect you from this; the
spec inherits whatever premise it was given.*

**Imitation is not evidence.** The first trial imitated tool output instead of
running the tools, and was not labelled as such. It was read and discussed as
though it were real. It was malformed in eight ways. **The rule now recorded:
run the real tool, or label the imitation at the point of delivery.**

**Scope creep is bidirectional.** The scope was also *widened* by inference
once — a direction was picked up from a source that turned out not to be a
directive, propagated into three documents and three running agents, and had to
be stopped and reverted wholesale. Re-sourcing a conclusion from elsewhere was
not an acceptable substitute for abandoning it.

---

## 4. The decision: deferred to hands-on use

**No tool was chosen at the end of this milestone, and that is the correct
outcome rather than an unfinished one.**

The research answered what could be answered from the outside: what each tool
installs, what it makes you write, what it enforces, what it costs. It cannot
answer the question that actually decides it — *which of these will you still be
using at change fifteen?* That is a matter of how the loop feels in your own
hands, and reading a walkthrough is not a substitute for running one.

**Next step is manual, hands-on use of both tools.** No further analysis is
pending. The open questions carried at the end of the research — first
milestone, guarding against abandonment, whether the proxy should rewrite SQL at
all — are **closed as deferred**; they are downstream of a tool choice that will
now be made by using the tools.

### 4.1 Evidence to carry into the hands-on trial

The balance, as it stood when research stopped:

**For OpenSpec:** roughly 3× lighter per unit of work; the delta+archive model
keeps a single current-truth document, which matters because the artifacts here
(compatibility matrix, rewrite-rule catalogue, supported-statement table) grow
forever; brownfield-native, so change #15 costs the same as change #2.

**For Spec Kit:** the constitution, which materially changed a design here and
which OpenSpec has no equivalent for; `[NEEDS CLARIFICATION]`; `### Measurable
Outcomes` as a real slot for "100% of…" statements; GitHub-backed, larger
community. Its verbosity is measurable and largely fixable with `lean`.

**Known rough edges, found by use:** OpenSpec makes scenario names permanent
once archived — three routes to retire one were all rejected — while the check
is name-only, so a scenario's *meaning* can be freely inverted. Its last gate is
`archive`, with no `--dry-run`. Spec Kit mandates `data-model.md` and
`contracts/`, which fit a SQL rewriter poorly.

**A third option remains live and under-weighted:** no framework at all —
`CLAUDE.md` plus a `specs/` directory. Given that neither tool enforces anything
semantic, the honest question is what a framework adds over hand-rolled
discipline. `surveys/commercial-and-minimal-sdd.md` covers this. Worth keeping
in view while trying the other two, since it is the baseline they have to beat.

### 4.2 Getting them running

Both were installed and driven successfully during this milestone. The commands
that worked, verified on this machine:

```sh
# OpenSpec — needs Node >= 20.19 (v26 here). No global install required.
npx -y @fission-ai/openspec@latest init --tools claude --no-animation
npx -y @fission-ai/openspec@latest new change <name>
npx -y @fission-ai/openspec@latest instructions <artifact> --change <name>   # format ground truth
npx -y @fission-ai/openspec@latest validate --all --strict
npx -y @fission-ai/openspec@latest archive <name> --yes

# Spec Kit — needs uv (installed via `brew install uv`).
uvx --from git+https://github.com/github/spec-kit.git specify init --here \
    --integration claude --script sh --force        # flag is --integration, NOT --ai
specify preset add lean                             # 85% smaller skills, worth trying early
```

Two things that cost time and are easy to avoid:

- OpenSpec's **`instructions` command is the authoritative format spec** — more
  reliable than the docs. Read it before authoring, not after a validation
  failure.
- Shell output on this machine is polluted by harmless `_encode`/`_decode` zsh
  errors; pipe through `grep -v "_encode\|_decode\|npm notice"` to read it.

The trials under `archives/milestone-1/trial/openspec/` and `archives/milestone-1/trial/speckit/` are real, validated
output from these exact commands — useful as a reference for what correct
artifacts look like, since guessing the formats produced eight distinct errors
(§2.3).

---

## 5. If you read only three things

1. `archives/milestone-1/trial/openspec/WALKTHROUGH.md` §8 — the canonical spec before and after a
   change modified it. The delta model's whole thesis, visible.
2. `archives/milestone-1/trial/speckit/WALKTHROUGH.md` §6 — the Constitution Check failing and
   forcing a design change.
3. `surveys/sql-limit-injection.md` — the lead finding, which is that the
   requirement both tools were fed does not do what it was meant to do.

Everything else is supporting evidence.

---

## 6. Milestone closed

Research is done and nothing is pending. The tool decision is deferred by
choice, to be settled by using OpenSpec and Spec Kit directly.

The one thing worth carrying forward, because it is the finding that survives
whichever tool wins:

> Neither tool enforces anything semantic. Their value is in the artifacts they
> get you to write and keep writing — not in any checking they appear to offer.
> **The tests keep the code honest; the spec keeps the tests honest.**

And the risk that was flagged at the start (R4) is unchanged: this process was
adopted without a felt pain, which is the classic setup for abandonment. Whether
either tool survives contact with real work is exactly what hands-on use will
reveal — and is the right question for milestone 2.
