# Implementation Plan: Row-Cap Rewriting Proxy for Doris

**Branch**: `001-l7-mysql-proxy` | **Date**: 2026-08-08 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/001-l7-mysql-proxy/spec.md`

**Note**: This template is filled in by the `/speckit-plan` command; its definition describes the execution workflow.

## Summary

Accept client connections, open one Doris connection per client, and relay statements. For each
statement, parse it, decide whether exactly one row cap can be applied without changing any value the
client would otherwise have seen, apply it by editing the parsed representation, serialize, and forward.

The approach is a **single-position cap**, not a blanket rewrite. The parsed statement is walked once to
find the one place whose rows go to the client — the outermost query, or the whole set operation when the
statement is a union. Every other place a limit could syntactically go is a place whose rows feed another
operator, and those are left alone. Statements with no such position (writes carrying a query, statements
returning no row set) and statements that do not parse are forwarded byte-for-byte unchanged.

## Technical Context

**Language/Version**: Rust 1.75+ (2021 edition)

**Primary Dependencies**: `sqlparser` (parse and serialize SQL, `MySqlDialect`); `tokio` (async I/O and
per-connection tasks); `tracing` + `tracing-subscriber` (structured rewrite records)

**Storage**: N/A — no persistent state. Rewrite records are emitted as structured log events and consumed
by the existing platform log pipeline.

**Testing**: `cargo test` for unit and integration; `proptest` for round-trip and idempotence properties
of the rewriter; `cargo-mutants` on the classifier module specifically, to prove the negative tests bite;
a differential harness that runs a corpus against a live Doris with and without the proxy

**Target Platform**: Linux server, same network segment as the Doris frontend nodes

**Project Type**: Single Rust binary (network service) with the rewrite logic as an internal library
module that is testable without any network

**Performance Goals**: Parse-classify-serialize under 1 ms at p99 for statements up to 8 KB; the proxy
adds no more than 5 ms p99 to the client-to-database path (SC-004)

**Constraints**: One backend connection per client connection, never shared (FR-012); passthrough
authentication with no credential ever held by the proxy (FR-013); forwarded SQL produced only by
serializing the AST, never by string splicing (FR-009); no persistent state, restart drops connections

**Scale/Scope**: Hundreds of concurrent analyst connections per proxy instance; statements up to a few
tens of kilobytes; a single cap value (200) shared by all users

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

Evaluated against `.specify/memory/constitution.md` v1.0.0, principle by principle.

### Initial evaluation (before Phase 0)

| Principle | Result | Finding |
|-----------|--------|---------|
| I. Semantic Preservation | **FAIL** | The feature as literally requested — append `LIMIT 200` to every query *and sub-query* — violates this principle outright. `SELECT COUNT(*) FROM (SELECT … FROM big) t` would return 200 instead of the true count, and a capped `UNION` branch or capped join input changes which rows survive. This is not an edge case; it is the stated design. |
| II. Fail Open, Never Fail Wrong | PASS | Spec FR-004 and FR-005 already forward unparsed and unclassifiable statements unchanged, and FR-007 extends the same treatment to writes. No path rejects a statement the proxy merely failed to understand. |
| III. Every Rewrite Is Observable | PASS | FR-010 and SC-006 require a per-statement record with a single reason code, and FR-008 adds a client-visible advisory on truncation. |
| IV. Rewriter Is Test-First | PASS (with obligation) | Nothing in the design prevents test-first, but the principle imposes a hard requirement on task ordering: negative cases for each rule must exist and fail before the rule is written. Carried into tasks.md as an explicit per-rule pairing. |
| V. One Job Only | **AT RISK** | The classifier and the post-serialize verification step below are both machinery beyond "decide whether to cap this statement". Recorded in Complexity Tracking rather than waved through. |

**Resolution of the Principle I failure.** The gate is not passable by adjusting wording; the design had
to change. The plan replaces "cap every query and sub-query" with "cap exactly one position, chosen by
classification". Concretely, this is what Principle I removed from the design:

- A recursive visitor that appends a limit at every `Query` node — **deleted**. It is the direct cause of
  the wrong-`COUNT` failure.
- Capping individual branches of a `UNION` — **deleted**. The cap moves to the set operation as a whole,
  which is the node whose rows reach the client.
- Capping a common table expression body — **deleted**. A CTE is referenced elsewhere in the statement;
  its rows are consumed, not returned.
- Capping the inner `SELECT` of `INSERT INTO … SELECT` — **deleted**. This one would have silently
  written 200 rows where the author asked for millions, and unlike a truncated read the damage persists.

What is left is a much weaker feature than the request: on a statement like
`SELECT COUNT(*) FROM (SELECT … FROM events) t`, the proxy caps the outer `COUNT` query, which already
returns one row, and therefore does nothing useful at all. The scan still happens. Principle I says that
is the correct outcome and that the load problem must be solved for those statements elsewhere — by
Doris resource groups, not by this proxy. The plan accepts a materially smaller win in exchange for never
returning a wrong number. This is the single largest design consequence of the constitution.

### Post-design re-evaluation (after Phase 1)

| Principle | Result | Finding |
|-----------|--------|---------|
| I. Semantic Preservation | PASS | `contracts/cap-position-classifier.md` enumerates every AST position and labels each SAFE or UNSAFE with a reason; the default for an unlisted position is UNSAFE. `contracts/rewrite-decision.md` requires the outcome of a statement with no SAFE position to be a pass-through. The differential test in quickstart.md is the empirical check. |
| II. Fail Open, Never Fail Wrong | PASS | The decision contract makes every failure path terminate in `ForwardedUnchanged` with a reason. There is no path from a proxy-side condition to a client-visible error. |
| III. Every Rewrite Is Observable | PASS | `RewriteRecord` in data-model.md carries statement fingerprint, decision, reason, forwarded text, and parse outcome, and is emitted for every statement including pass-throughs. |
| IV. Rewriter Is Test-First | PASS | Every classifier rule in the contract has a paired positive and negative case named in tasks.md, ordered before the rule's implementation task. |
| V. One Job Only | PASS with recorded deviations | Two additions justified in Complexity Tracking below. Nothing else stateful was added: no pooling, no cache, no credential handling, no routing. |

## Project Structure

### Documentation (this feature)

```text
specs/001-l7-mysql-proxy/
├── plan.md              # This file (/speckit-plan command output)
├── research.md          # Phase 0 output (/speckit-plan command)
├── data-model.md        # Phase 1 output (/speckit-plan command)
├── quickstart.md        # Phase 1 output (/speckit-plan command)
├── contracts/           # Phase 1 output (/speckit-plan command)
└── tasks.md             # Phase 2 output (/speckit-tasks command - NOT created by /speckit-plan)
```

### Source Code (repository root)

```text
src/
├── main.rs                     # binary entry: config load, listener bind, shutdown
├── proxy/
│   ├── mod.rs
│   ├── listener.rs             # accept loop, one task per client
│   ├── session.rs              # pairs one client connection to one Doris connection
│   └── relay.rs                # statement in, response out; calls rewrite once per statement
├── rewrite/
│   ├── mod.rs                  # public entry: decide(sql) -> RewriteDecision
│   ├── classify.rs             # AST position classification (SAFE / UNSAFE), the core of Principle I
│   ├── apply.rs                # place the cap at the single SAFE position; min-with-existing-limit
│   ├── verify.rs               # re-parse the serialized output and compare shape (Complexity row 2)
│   └── reason.rs               # the closed set of decision reasons
├── observe/
│   ├── mod.rs
│   ├── record.rs               # RewriteRecord construction and emission
│   └── fingerprint.rs          # statement identity
└── config/
    └── mod.rs                  # cap value, listen address, upstream address

tests/
├── contract/
│   ├── classifier_positions.rs # one test per position in contracts/cap-position-classifier.md
│   └── decision_reasons.rs     # every reason in contracts/rewrite-decision.md is reachable
├── integration/
│   ├── differential.rs         # corpus through proxy vs direct, values compared (SC-002)
│   └── passthrough.rs          # unparseable, write-carrying, and no-rowset statements
└── unit/
    ├── apply_limit.rs          # min-with-existing, offset preservation
    └── roundtrip_props.rs      # proptest: parse -> serialize -> parse is stable
```

**Structure Decision**: Single Rust project at the repository root, using the default `src/` + `tests/`
layout above. The one structural choice worth stating is that `src/rewrite/` has no dependency on
`src/proxy/` and touches no I/O: it takes a `&str` of SQL and returns a decision. That boundary exists
because Principle IV requires the negative cases — proof that a cap is *not* applied in unsafe positions —
and those tests must be cheap enough to run in the hundreds without a server or a database. `src/proxy/`
is the only module that knows a socket exists; `src/observe/` is called from both and depends on neither.

## Complexity Tracking

> **Fill ONLY if Constitution Check has violations that must be justified**

| Violation | Why Needed | Simpler Alternative Rejected Because |
|-----------|------------|-------------------------------------|
| A dedicated AST position classifier (`src/rewrite/classify.rs`), roughly a third of the rewrite module, where the request implies a one-line append | Principle I requires knowing, for every position in a parsed statement, whether its rows reach the client or feed another operator. That knowledge does not exist anywhere else and cannot be inferred at the point of application. | Walking the AST and appending a limit at every `Query` node is one visitor and about twenty lines. It was rejected because it returns 200 for `SELECT COUNT(*) FROM (SELECT … FROM events) t`, and a wrong count is exactly the silent failure Principle I exists to forbid. |
| A verification pass (`src/rewrite/verify.rs`) that re-parses the serialized output and compares it structurally to the intended AST before anything is forwarded | The proxy forwards SQL the user never wrote. If the serializer drops or reshapes a construct, the client gets wrong results with no signal anywhere. Principle I's guarantee is only as strong as the serializer, which is third-party and not under this project's control. | Trusting `sqlparser`'s serializer round-trip was rejected because its round-trip fidelity is documented as best-effort, and the failure mode is silent. A verification failure downgrades to pass-through under Principle II, so the cost of the check is a missed cap, not an error. |
