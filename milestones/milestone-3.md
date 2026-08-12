# Milestone 3 — Learning OpenSpec, end to end

**Period:** 2026-08-11 → 2026-08-12
**Status:** ✅ **Closed** 2026-08-12. Three changes proposed, implemented and
**archived** — `archive` is the one lifecycle step this project had never run on
real work, and it ran three times.
**Outcome:** a working MySQL proxy with statement logging and row-filter
injection, verified against MySQL 8 and demonstrated against Apache Doris as
well, built in three changes that each closed at 100% of their tasks. The
milestone's finding is not about the proxy: **milestone 2's diagnosis was
correct, and acting on it worked.**

> **Milestones are independent.** Nothing here is work in progress. The workspace
> at `milestones/project3/` is milestone 3's, complete; its `openspec/specs/` are
> populated and its `openspec/changes/archive/` holds all three changes. A
> milestone 4 starts from its own scope and inherits no tasks from here.

**Purpose of this file:** a single place to re-orient. Read this instead of
re-reading the three archived changes and the crate.

> **Verification note.** Re-run on 2026-08-12 after the workspace was moved into
> the repository: ✅ `cargo test` — 185 tests pass (154 lib, 18
> `proxy_integration`, 13 `row_filter_integration`), ✅ `openspec doctor` resolves
> the root at the new path, ✅ `openspec list --specs` shows three capabilities.
> **Not re-run:** `scripts/verify-with-mysql.sh` and `demo/01-setup.sh`, both of
> which need live MySQL and Doris containers. Every claim below about real-engine
> behaviour is quoted from the change artifacts, not re-measured.

---

## 1. What we wanted

Milestone 3's goal, as set: **learn OpenSpec** by building a MySQL proxy. The
tool question was closed for the milestone — OpenSpec was the subject of study,
not a candidate. Two constraints came from milestone 2's post-mortem:

1. **Size the first change small enough to reach `archive`.** Milestone 2 closed
   at 70/75 tasks with its change never archived, so `openspec/specs/` stayed
   empty and half the tool went unexercised.
2. **Nothing about row filtering is inherited.** What the proxy did was an open
   question.

Both were met, and the second one interestingly: the milestone arrived back at
row filtering by choice, from a different direction, with the opposite failure
posture.

## 2. What we did

### 2.1 A clean workspace, deliberately outside the repository

The workspace was created **outside this repository** and developed there, then
moved in at the end. The author's reason, stated directly: a model working inside
this repo reads too much accumulated context to start from a clean premise.

That is a real observation about this repository rather than about OpenSpec. Two
milestones of history, a 22 KB `CLAUDE.md`, a frozen crate and a closed change all
argue for continuity, and milestone 3's whole premise was discontinuity. Moving
the finished work in afterwards got both.

The consequence is visible in the workspace: `openspec/config.yaml` is the
**untouched default template** — all comments, no `context`, no `rules`. See §3.5;
this is the milestone's most surprising single fact.

The four commits carried in by `git subtree` are the whole history:

| Commit | |
| --- | --- |
| `415040b` | `opsx prposed for phase 1` — 2026-08-11 |
| `3b85073` | `phase 1` — 2026-08-12 |
| `a11124c` | `phase 2` — 2026-08-12 |
| `8d338ba` | `phase 3: demo` — 2026-08-12 |

### 2.2 Three changes, each closed at 100%

| Change | Artifacts | Tasks | Capabilities |
| --- | --- | --- | --- |
| `add-mysql-proxy-query-logging` | proposal, design, 2 delta specs, tasks | **67/67** | `protocol-relay` (new), `query-logging` (new) |
| `add-row-filter-injection` | proposal, design, 3 delta specs, tasks | **54/54** | `row-filter` (new); the other two modified |
| `add-cross-engine-demo` | proposal, tasks (`skip_specs: true`) | **32/32** | none — demo scripts only |

**153/153 tasks**, all three archived, and the canonical specs they fold into are
520 lines across three capabilities and 19 requirements. Compare milestone 2: one
change, 70/75, never archived, `specs/` empty.

### 2.3 What got built

A `mysql-proxy` crate — 5,509 lines of `src/`, 1,109 of `tests/`, five
dependencies (`tokio`, `bytes`, `serde`, `serde_json`, `toml`), `unsafe_code =
"forbid"`. Notably **no SQL parser**: statement analysis is a hand-written
tokenizer plus a shape scanner over its token stream.

- **Phase 1 — observability.** Terminates the MySQL wire protocol, relays 1:1 to
  a backend, and appends a JSON Lines record per command: statement text, digest,
  digest hash, outcome, row counts, latency. Logging goes through a bounded
  channel to a writer task; when it is full, records are **dropped and counted**
  rather than delaying a query. The log says so in-band with `"type":"dropped"`
  records, and the README states plainly that it is observability, not an audit
  trail.
- **Phase 2 — row filtering.** A per-listener, per-table predicate spliced into
  reads: `SELECT … WHERE (a = 1 OR b = 2) AND (tenant_id = 7)`. Byte splicing at
  an offset taken from a token's recorded span, never from arithmetic on one, so
  a splice cannot land inside a string or a comment. Both operands are
  parenthesized unconditionally.
- **Phase 3 — the demo.** One query, four ways: direct and proxied, against MySQL
  and against Doris, from a single proxy process holding two listeners. No proxy
  changes were permitted, which is why the setup script ends in a readiness table
  that exits non-zero.

### 2.4 Verification

`cargo test` drives the proxy against a scriptable backend in `tests/support`, so
behavioural tests are deterministic and need no database.
`scripts/verify-with-mysql.sh` closes the gap the mock cannot: real MySQL 8, the
`caching_sha2_password` RSA path, prepared statements, multi-result procedures,
TLS refusal, session isolation, log rotation.

The row-filter design is explicit that **the tests that matter assert which rows
come back, not what the rewritten SQL says** — string equality cannot catch a
precedence bug, because `WHERE a = 1 OR b = 2 AND tenant_id = 7` is a
perfectly reasonable-looking string. Row counts can.

---

## 3. What we learned

### 3.1 The remedy worked, and it was a sizing remedy

Milestone 2's author-diagnosis was that the first task was simply too big.
Milestone 3 tested that directly, and the result is as clean as this kind of
evidence gets: same author, same tool, same domain, same machine — three changes
instead of one, all closed, all archived.

Nothing else was changed to make that happen. Not the tool, not the language, not
the process. **The scoping unit was the variable.**

### 3.2 Inverting the failure posture is what made it small

This is the sharpest technical finding, and it is easy to miss because both
milestones ended up building "row filtering".

| | Milestone 2 | Milestone 3 |
| --- | --- | --- |
| Unrecognized statement | **Rejected, never forwarded** | **Forwarded unchanged**, counted, logged |
| Claim | A security control; a missed filter is a data leak | *Not* a security boundary; `GRANT`s remain the access control |
| Scope of rewrite | `SELECT` incl. joins and subqueries | One table, one `SELECT`, no subqueries |
| Rule keyed by | authenticated user | listener |
| Mechanism | full parse, allowlist AST walk, derived-table wrapping | tokenizer, shape scanner, byte splice |
| Compatibility surface | open-ended — every real client found more of it | closed — anything unrecognized is skipped |

Fail-closed is what made milestone 2 unbounded: **rejecting an unrecognized
statement makes every corner of a foreign SQL dialect your problem**, because
each one breaks a client until you handle it. Fail-open makes the same corners
cost a counter increment.

That is a genuine trade, not a free win, and milestone 3 does not pretend
otherwise — the proposal, the design, the README and the demo README each say
in their own words that a statement the proxy cannot rewrite returns unfiltered
rows. The honesty is the mitigation.

What milestone 3 therefore does **not** show is that milestone 2's target was
achievable at any size. It shows that a *different, weaker* target was.

### 3.3 Phase 1 built phase 2's seams on purpose

Phase 1's design has a decision titled *"Structure for phase 2, without building
it"*. Phase 2's design opens by naming three of its inheritances:

- the pipeline stage returns `Cow<'_, [u8]>`, so a stage that replaces a payload
  already has somewhere to put it;
- the tokenizer emits tokens carrying **byte spans**, which is exactly what
  locating a splice point requires;
- packet count is a preserved invariant, so a rewrite that does not change it
  needs no sequence renumbering — phase 2 turned that into a one-comparison guard
  instead of per-connection sequence state.

Phase 2 shipped with **no new dependencies** as a result. This is what "small
first change" is supposed to mean and rarely does: not a change that defers the
hard part, but one that installs the seam the hard part will need and proves the
seam by shipping a no-op stage through it.

### 3.4 `archive` ran — and here is what it actually does

The step milestone 2 never reached. `openspec archive` moves the change to
`openspec/changes/archive/<date>-<name>/` and folds its delta specs into
`openspec/specs/<capability>/spec.md`.

Two things become visible only after it runs:

1. **Delta specs are diffs; canonical specs are the accumulated state.** The
   `protocol-relay` capability appears in two changes — 164 lines of delta in
   phase 1, 33 in phase 2 — and lands as one 176-line canonical spec. Reading the
   deltas alone never shows you the current contract.
2. **The archive is where the reasoning lives.** `design.md` is not folded into
   anything; it stays in the archived change. The crate's README says so
   explicitly: *"The design notes … are in `openspec/changes/archive/`."*
   Canonical specs tell you what the system does; only the archive tells you why.

### 3.5 The context file was empty, and it did not matter

Milestone 2's `config.yaml` was a substantial artifact — project context,
cross-cutting invariants, the fail-closed rule, the bypass vectors — and
`CLAUDE.md` records the care taken to verify it was actually injected, including
two silent-failure modes found the hard way.

**Milestone 3's `config.yaml` is the untouched default: no `context`, no
`rules`.** A populated one was written for the abandoned in-repo workspace and
never used. The three changes were produced with nothing injected but the
conversation.

Be careful what this does and does not say. It is one milestone, and the
conversation carried the context instead — the proposals are visibly informed
about `CLIENT_DEPRECATE_EOF`, `caching_sha2_password` and packet framing, which
did not come from a config file. What it does say is that `config.yaml` is not
load-bearing for artifact quality in a single-session change, and the effort
milestone 2 spent verifying its injection bought less than it appeared to.

### 3.6 Two features found by using the tool, not by reading about it

- **`skip_specs: true`** in `.openspec.yaml`. The demo change adds scripts and
  touches no behaviour, so it has no capability delta. Without the flag the
  workflow expects specs that would have to be invented. A change with proposal
  and tasks only is a legitimate shape.
- **Date-prefixed archive directories** (`2026-08-12-add-…`) — the archive is
  ordered by when work closed, which is the reading order for a history.

### 3.7 The demo tested a branch the test suite did not

Doris is an independent implementation of the MySQL wire protocol, not a fork,
and it **does not advertise `CLIENT_DEPRECATE_EOF`**. A server that does not
negotiate it terminates result sets with explicit EOF packets instead of an OK
packet — so proxying Doris runs the *other* branch of the response state machine,
which until the phase-3 spike had only hand-built unit-test packets behind it.

A presentation aid became the first real-traffic evidence for a code path the
whole test suite had only mocked. That is milestone 2's "check what a fixture
cannot express" lesson, arriving from a direction nobody planned.

---

## 4. What is left open

- **The SDD tool choice is still formally open.** Milestone 3 studied OpenSpec
  rather than comparing it; Spec Kit and the no-framework baseline remain untried
  in anger. What changed is that OpenSpec has now been run end to end, archive
  included, which milestone 2 could not say.
- **No `discussions/milestone-3/` record.** Milestones 1 and 2 each produced one.
  Milestone 3's decisions live in the change artifacts instead — which may be the
  right answer, but it was not a decision anyone recorded making.
- **No mutation testing.** `cargo-mutants` found five real defects in milestone 2
  that nothing else caught; it was not run here.
- **The proxy's own deferred questions** are stated where they belong, in
  `add-row-filter-injection/design.md` § Open Questions: whether multi-table
  support is worth building (the skip counters are meant to answer it with
  evidence), whether to offer an opt-in strict mode, and whether rules should be
  per-user — which would require terminating authentication rather than relaying
  it, and is therefore a different project, not a variation.

---

## 5. If you read only four things

1. `milestones/project3/openspec/changes/archive/2026-08-12-add-row-filter-injection/design.md`
   — the best artifact of the milestone. The unconditional-parenthesization
   decision and the "offset must come from a recorded span" rule are both cases
   of a design document earning its keep.
2. `milestones/project3/README.md` § *This is not a security control* — the
   honest statement of what a best-effort filter is, written where an operator
   will read it.
3. `milestones/project3/demo/README.md` — four cases, two engines, and § *Why two
   engines is the point*.
4. §3.2 above — the posture inversion, which is the reason this milestone closed
   and milestone 2 did not.

---

## 6. Milestone closed

Nothing is pending. Three changes are archived, the canonical specs are
populated, and the crate builds and tests clean at its new location.

Three things carry forward:

1. **Small enough to archive is a real constraint, and it worked.** Not "write
   smaller tasks" — *choose a target whose compatibility surface is closed.* The
   sizing lever was the failure posture, not the task list.
2. **Build the seam, ship it as a no-op, then use it.** Phase 1 → phase 2 is the
   worked example, and it cost phase 2 zero new dependencies.
3. **The finding from milestones 1 and 2 still stands, and milestone 3 neither
   confirmed nor refuted it** — no defect here was attributed to a spec, but
   nothing in this milestone was looking for defects the way milestone 2's
   mutation run and live-client testing were:

> A spec catches nothing on its own. It makes disagreement legible, which is only
> useful if someone holds an implementation against it. The tests keep the code
> honest; the spec keeps the tests honest; and mutation testing keeps the tests
> honest about being honest.
