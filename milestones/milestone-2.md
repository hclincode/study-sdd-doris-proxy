# Milestone 2 — Building the proxy with OpenSpec

**Period:** 2026-08-09 → 2026-08-11
**Status:** ✅ **Closed** 2026-08-11. It was paused first — commit `a3e574d`,
*"pause milestone-2 for token consume too large"* — and then closed where it
stood rather than resumed. The change ended at 70/75 tasks with five items
**closed-unaddressed**, and was **not archived**, so `openspec/specs/` is still
empty.
**Outcome:** the proxy works end to end against a real Doris and is verified more
heavily than anything in milestone 1 — but the change grew faster than it closed,
which is the milestone's actual finding.

> **Milestones are independent.** Nothing here is work in progress. The five open
> tasks in §5 are a record of where the compatibility surface ran out, not a
> backlog for milestone 3, and the workspace at
> `milestones/milestone-2/mysql-proxy-project/openspec-workspace/` is frozen
> reference. What carries forward is the lessons in §3, and nothing else.
**Purpose of this file:** a single place to re-orient. Read this instead of
re-reading `openspec/changes/add-row-filter-proxy-mvp/`,
`notes/adr-material.md` and `discussions/milestone-2/`.

> **Verification note.** Numbers marked ✅ were re-run while writing this report
> (2026-08-11). Numbers marked ↩ are quoted from `tasks.md` and
> `notes/adr-material.md` and were **not** re-run — in particular the mutation
> counts and every result requiring a live Doris container.

---

## 1. What we wanted

Milestone 1 closed by refusing to pick a tool from research: *"which of these
will you still be using at change fifteen?"* is a question about how the loop
feels in your hands. Milestone 2 is that hands-on use, and it carried three
things forward.

| # | Intent | Where it came from |
|---|---|---|
| G1 | **Use OpenSpec for real work, end to end** — propose → design → specs → tasks → implement. A starting point, explicitly not a verdict | `discussions/milestone-1/01-sdd-tool-selection.md` |
| G2 | **Test milestone 1's central claim** — that neither tool enforces anything semantic, and that the value is the artifacts, not the checking | milestone-1.md §3.2 |
| G3 | **Watch for R4** — the process was adopted with no felt pain, the classic setup for abandonment | milestone-1.md §1 |

And it changed the exercise itself. The proxy's rewriting purpose was
**replaced**, not refined:

- **Milestone 1:** append `LIMIT 200` to every query and sub-query. Abandoned —
  `LIMIT` caps rows *returned*, not rows *read*, and Doris's `sql_select_limit`
  already did the useful part.
- **Milestone 2:** **row-level filtering by authenticated user.** A config maps
  `(user, table)` to a column and a set of permitted values; the proxy constrains
  that user's `SELECT`s to matching rows.

That swap is the load-bearing decision of the milestone, because it inverts the
failure mode. Under `LIMIT 200` a missed rewrite is a slow query. Under row
filtering **a missed rewrite is a data leak**, so the whole design becomes
fail-closed: anything the rewriter cannot *prove* it has fully constrained is
refused, never forwarded. A harder exercise was chosen deliberately, on the
theory that a process is only tested by work that can actually go wrong.

One thing we wanted to be honest about from the start, and wrote into the
proposal rather than around: **Doris ships `CREATE ROW POLICY`, and it is the
stronger control.** The proxy is justified by *where policy lives* — versioned,
reviewable, applied identically across clusters — never by claiming better
enforcement.

---

## 2. What we did

### 2.1 Setup

- OpenSpec v1.8.0 installed globally via npm (milestone 1 used `npx`).
- Rust 1.97.1 installed via rustup with the `default` profile — rustfmt and
  clippy included. `cargo-mutants` added later.
- Workspace at `milestones/milestone-2/mysql-proxy-project/openspec-workspace/`,
  holding `openspec/` **and** the crate. `openspec init --tools none`, because
  the CLI resolves its root by searching *upward* and Claude Code only loads
  `.claude/` from the repo root.
- `openspec/config.yaml` carries the project context, the fail-closed rule, the
  invariants and the known bypass vectors, so the tool restates them on every
  artifact it generates. **It fails silently** — a YAML error prints
  `could not parse … ignoring it` and continues with *no context and no rules at
  all*, and `validate --strict` does not catch it. Both failure modes bit here.

### 2.2 Planning — one change, four artifacts, three capabilities

`add-row-filter-proxy-mvp`, generated through `/opsx:propose`:

| Artifact | Content |
|---|---|
| `proposal.md` | Why, what changes, the REJECTED statement list, non-goals, the honest comparison against `CREATE ROW POLICY` |
| `design.md` | D1–D7 — the load-bearing decisions |
| `specs/` | `connection-routing`, `policy-config`, `row-filter-rewrite` |
| `tasks.md` | 75 tasks in 10 sections |

The seven design decisions, since everything downstream refers to them by number:

| | Decision |
|---|---|
| **D1** | Connect to the backend first and relay **Doris's own auth challenge**, so credentials pass through and the proxy never holds a password |
| **D2** | Constrain by **wrapping each policy-bearing relation in a derived table**, never by appending to the user's `WHERE` |
| **D3** | Enumerate table references with an **allowlist** walk; refuse anything unrecognised |
| **D4** | Refuse the capabilities the design cannot police |
| **D5** | Rejections are MySQL error packets and disclose nothing about policy |
| **D6** | Parse failures are a **category**, not an exception |
| **D7** | Decisions that are not spec-shaped are deferred to ADRs |

✅ `openspec status` reports 4/4 artifacts complete and `openspec validate --all
--strict` passes.

### 2.3 Implementation — four agents in parallel

✅ `git diff d362400..HEAD -- milestones/`: **44 files, 16,252 insertions** —
5,885 lines of `src/`, 6,054 lines of `tests/`, the rest specs, ADRs and notes.

✅ `cargo test`: **245 tests across 18 targets, 0 failed, 0 ignored.**

| Module | What it owns |
|---|---|
| `src/policy.rs` | Policy model, load, validation, `(user, table)` lookup returning **three** outcomes — `Unrestricted` / `Restricted` / `Unresolvable` |
| `src/analyze.rs` | Parse, then the allowlist walk. No `_ => {}` arm continues the walk |
| `src/rewrite.rs` | Derived-table wrapping, plus an independent re-check of the **rendered** output before forwarding |
| `src/session.rs` | 1:1 backend mapping, passthrough auth. The backend connection phase is hand-rolled on a raw `TcpStream` |
| `src/error.rs` | Refusal reasons — deliberately with no "could not analyse, forwarded anyway" variant |
| `src/main.rs` | Startup ordering enforced by types: `serve` takes `PolicySet` **by value**, so a bind before validation does not compile |

Two implementation findings worth carrying (↩ `notes/adr-material.md`):

- **`mysql_async` could not do D1** — three independent blocks, each sufficient:
  the backend nonce is a private field with no accessor, the handshake response
  is derived from a plaintext password held in `Opts`, and both public
  constructors run the handshake internally. Resolution: hand-roll the backend
  connection phase. Recorded as ADR 0006.
- **D4's stated rationale was wrong.** Clearing the advertised
  `CLIENT_MULTI_STATEMENTS` bit is advisory — `opensrv-mysql` never checks
  whether the client honoured it. Multi-statement rejection had to move to the
  SQL level. The design document was corrected **with the wrong text preserved**,
  which is the point of having one.

### 2.4 Verification — three layers, and only the third found the leaks

1. **Tests** (245 ✅) — including property tests and one test per injection
   position from the D2 table.
2. **Mutation testing** (↩ task 7.8, scoped to `rewrite.rs` / `analyze.rs` /
   `policy.rs` / `error.rs` at commit `598ccaf`): **155 caught, 11 missed, 7
   timeout, 182 unviable**, with a written reason for every one of the 11. It
   found **five real defects**, not a metric — chief among them that the
   fail-closed re-check was a **no-op in every test that went through
   `rewrite_statement`**.
3. **A real Doris** (↩ tasks 8.1–8.6, 10.1–10.6, `apache/doris-all-in-one-2.1.0`)
   — reported blocked for most of the change, then found to have been available
   all along.

What the real backend settled:

- ✅↩ **Passthrough auth works.** Every D1 test before this ran against a fake
  frontend that does no hashing. A real Doris accepting the relayed scramble was
  the project's largest unverified claim.
- ✅↩ **The bypass is real and demonstrable.** The same user, same credentials,
  straight to port 9030 returns all five rows including the restricted ones. The
  README's first operator precondition is now demonstrated rather than argued.
- ↩ **The largest recorded compatibility cost was not one.** `GROUP BY … WITH
  ROLLUP` is refused because `MySqlDialect` cannot parse it — and **Doris cannot
  parse it either**, so refusing costs nothing.
- ↩ **The sharpest parser-differential worry is closed.** Doris resolves
  `WITH orders AS (…) SELECT * FROM orders` to the CTE exactly as MySQL does.
- ↩ **And it broke the proxy for real clients**, twice, then leaked — see §3.

### 2.5 Documentation

- `docs/adr/0001`–`0006` — protocol crate choice, task topology, buffer
  ownership, cancellation, error-enum shape, removing `mysql_async`. Written
  **after the fact**, so they cannot drift.
- `README.md` — operator-facing, with a **"What this does not protect against"**
  table of 13 rows. Listing only what is closed would read as a claim that the
  rest is handled.
- `discussions/milestone-2/01-what-actually-caught-defects.md` — the process
  record, and the milestone's real output.

---

## 3. What we learned

### 3.1 Milestone 1's prediction held exactly

**Not one defect was found by writing a spec, by `openspec validate`, or by any
tool gate.** `validate --strict` passed continuously, *including at moments when
a live bypass was present in the code*.

What the artifacts did was make **disagreement legible**. The clearest instance:
the lead told an agent a load-time rejection was "still open, hold", then decided
it, wrote it into `specs/policy-config`, and never sent the follow-up. The agent
read the spec, found a requirement no message had mentioned, implemented it, and
flagged the discrepancy. **The written contract outlived an inconsistent
instruction** — with no enforcement involved.

### 3.2 Every defect lived in a seam

All five were at a boundary between two things that were each individually
correct, each was found by a party **other than the one who wrote the code**, and
the tests were green throughout.

| Defect | The seam |
|---|---|
| D4's protocol claim was false | Design doc vs. what the crate actually does |
| `Unresolvable` bypass | A two-state interface over a three-state decision |
| Escaping could widen a permitted set | Two individually-sound decisions composed |
| Non-ASCII identifier leak | A stated fail-closed rationale vs. an implementation that inverted it |
| `/*!` executed unconditionally | A parser that discards a version gate vs. a rewriter that re-renders |

### 3.3 The one that leaked, and why it is the most useful

A restricted user could read the unfiltered contents of a policy table:

```
SHOW VARIABLES WHERE (SELECT COUNT(*) FROM sales.orders) = 5   -> returns rows
SHOW VARIABLES WHERE (SELECT COUNT(*) FROM sales.orders) = 3   -> returns nothing
```

A general-purpose oracle: any predicate over restricted rows, one bit at a time.
**It was introduced by a ruling made an hour earlier**, and it is the only
finding here where the written artifact was **right** and a live disclosure
followed anyway.

The cause was not plumbing. *"Names are forwarded" had been implemented by not
enumerating the name at all* — a disposition decision taken inside the layer
whose only job is to report what is there.

> An analyser that quietly declines to report something is indistinguishable
> from one that found nothing, and no care downstream can recover the difference.

Fixed by recording provenance per reference (`Named` vs `Relation`) and making
the decision at **lookup** rather than at disposition. The spec sentence turned
out to be implementable exactly as written; it needed the distinction made where
the information still existed.

### 3.4 Four failure shapes that recurred

1. **A test that cannot fail in the direction the bug lies.** Found five times,
   in three files, by three people, before anyone noticed it was one shape. *A
   predicate only ever asked the question it answers "yes" to is
   indistinguishable from one that always answers "yes".* Mutation testing found
   four of them; nothing else did — not the spec, not review, not 200 passing
   tests.
2. **A test double's shape bounds what can ever be tested.** A two-state mock
   over a three-state decision, a mock matching names by exact equality, a
   fixture whose `with()` took `&[&str]` and so could not express integer
   values — each hid exactly one defect. **Check what a fixture cannot express,
   not only what it does express.**
3. **An instrument shaped by your own assumptions returns them.** "Nearly every
   connector issues `SET NAMES` on connect" was recorded by two people across
   four messages, and a blocker scoped around it — on the evidence of a statement
   the lead had asked to be typed. A packet sniffer showed the real client sends
   `select @@version_comment limit 1` and nothing else. *The measurement was
   real. It was measuring us.*
4. **The stale artifact.** Four instances: a mutation report read mid-run, counts
   compared across runs, a suite committed without being read, and — worst — a
   security fix re-measured against a **binary started before the change**. That
   last one ran in the harmless direction by luck. The habit that survived, and
   it is a habit rather than a rule because a rule is something you can forget:
   **for anything security-relevant, rebuild in the same command that measures.**

### 3.5 What OpenSpec actually contributed

- A spec that outlived a contradictory instruction (§3.1).
- Scenario naming that survives archiving, when named for the **behaviour**
  rather than the mechanism: "Multi-statement request is rejected" stayed true
  when the mechanism behind it was disproved. A task named for the mechanism had
  to be rewritten.
- `design.md` as a place a wrong decision could be corrected **with the original
  wrong text preserved**, so the correction teaches.
- ADRs whose immutability rule was honoured the first time it bit — by the agent
  that wrote the rule, against its own document.

> **What it did not contribute: any detection.** The tests keep the code honest;
> the spec keeps the tests honest — and mutation testing keeps the tests honest
> about being honest.

---

## 4. The author's own account

From `study-note.md`, the human-authored journal (quoted, not paraphrased; that
file is the primary source and is never edited by an agent):

> **install openspec** — I refer to the OpenSpec installation doc.
>
> **propose project** — 應該要先好好看一下 openspec 的 readme. 原生 openspec 可以使用
> `/opsx:propose` 或是 `/opsx:explore` 開始一個發想. study 過程中看到有另一個專案叫做
> superpower, 似乎也是合作專案發想. 但先放旁邊，使用 openspec 的 workflow.
>
> **implementing** — 跑得比過去沒有使用 openspec 還更自動. 期待結果如何.
> 結果看起來 task 不斷放大. 兩三次 token limit 後都還沒辦法結束任務.
> 猜想是任務太大了，打算重做從 explore 開始先給比較小的任務，再放大成 sql rewrite.

Four things in that, worth separating because only one of them is about OpenSpec:

1. **Read the tool's README first.** The same lesson as milestone 1's
   `instructions`-is-the-ground-truth finding, arrived at again from the other
   side.
2. **`superpowers` was seen and deliberately shelved** to avoid running two
   workflows at once. Surveyed in `surveys/superpowers.md`; the path-collision
   note there is the open question if it is ever picked up.
3. **The loop ran more autonomously than any previous non-SDD attempt.** This is
   the milestone's most positive result and it came from the author, not from
   measurement.
4. **The tasks kept expanding.** Two or three token limits and the change still
   would not close. The author's own diagnosis — *the task was too big* — and the
   intended remedy: **restart from `/opsx:explore` with a smaller first task, and
   grow into SQL rewriting.**

Point 4 is the same phenomenon §5 records from the artifact side, reached
independently. It is the finding milestone 3 has to act on.

---

## 5. Where it stopped

✅ `openspec list` — **70/75 tasks**, change **not archived**, `openspec list
--specs` reports **no specs**: nothing has been folded into canonical form yet.

The five remaining tasks are all in section 10, *"Found by integration, not yet
addressed"* — a section that **did not exist when the change was written**. It
was created by contact with a real backend and grew to eleven items while the
first nine sections were closing. They are **closed-unaddressed** and left
unticked on purpose: they were not done, and a ticked box recording nothing is
the exact false-signal shape §3.4 keeps describing.

| Task | Open item |
|---|---|
| 10.7 | `mysqldump` cannot run through the proxy at all — and the gap had been recorded at the wrong depth, as a `--single-transaction` problem, when it actually dies on statement one |
| 10.8 | A **client-corpus test** — assert that what real clients and tools actually send is analysable. *The fix that prevents the next one* |
| 10.9 | Change the **default**, not the list: an unclassified statement kind should walk and refuse only if a policy-bearing reference turns up, rather than refusing without looking |
| 10.10 | Log refusals with the statement-kind name, so compatibility loss is visible in a deployment rather than invisible until someone complains |
| 10.11 | DDL disposition, and stating in the README that a policy-bearing user can `CREATE TABLE` |

**The shape of the stall is the finding.** Section 10 was opened by the *first*
real client, and three of its entries (10.1 `SET`/`SHOW`, 10.3 transaction
control, 10.7 `mysqldump`) are the same defect found three times by three
different clients. Each was closed as a one-off before anyone wrote down that
they were one class — which is exactly what 10.8 and 10.9 are for, and they are
the two still open.

So the change did not stall on difficulty. It stalled because **fail-closed
against a foreign SQL dialect has an open-ended compatibility surface**, and
every real client discovers another piece of it. The MVP boundary was drawn
around the security property, and the compatibility work it implies has no
natural edge.

R4 (§1, G3) is therefore answered, but not in the form it was asked. The process
was not abandoned for lack of felt pain — it was paused for **cost**.

---

## 6. If you read only four things

1. `discussions/milestone-2/01-what-actually-caught-defects.md` — the process
   record. The proxy is the exercise; this is the output.
2. `openspec/changes/add-row-filter-proxy-mvp/design.md` — D1 and D2 are
   load-bearing, and the corrected D4 shows the artifact working as intended.
3. `README.md` §"What this does not protect against" — 13 rows, and the honest
   version of what was built.
4. `tasks.md` §10 — the compatibility surface that stopped the change, still open.

---

## 7. Milestone closed

Nothing is pending. The change is frozen at 70/75, the workspace is reference
material, and **milestone 3 starts from its own scope** — it inherits no tasks,
no change and no backlog from here.

Only three things are worth carrying, and none of them is work:

1. **The scoping unit is what failed, not the tool.** The author's own remedy —
   start smaller and grow into SQL rewriting — is a lesson about how to size a
   first task, available to milestone 3 whatever it decides to build.
2. **The SDD tool choice is still open.** Milestone 1 deferred it to hands-on
   use; milestone 2 gave OpenSpec that use and produced a clear picture of what
   it does (artifacts, durability, legible disagreement) and what it does not
   (any detection). Spec Kit and the no-framework baseline remain untried in
   anger.
3. **The finding that survives whichever tool wins**, now with evidence behind it
   rather than prediction:

> A spec catches nothing on its own. It makes disagreement visible, which is
> only useful if someone holds an implementation against it. Every defect in
> this milestone was found that way, by a party other than the one who wrote the
> code — and the two nastiest were found by a real client and a mutation run,
> with every test green and `validate --strict` passing throughout.
