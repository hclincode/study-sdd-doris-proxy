# Quickstart: Validating the Row-Cap Rewriting Proxy

**Feature**: 001-l7-mysql-proxy | **Date**: 2026-08-08

How to prove the feature works end to end. These scenarios are the acceptance evidence for the user stories
in [spec.md](./spec.md); each names the story and success criterion it discharges.

## Prerequisites

- Rust 1.75+ toolchain
- A reachable Doris cluster and credentials that can read one large table (millions of rows) and one small
  table (under 200 rows)
- A MySQL-protocol client for manual checks
- Environment: `DORIS_ADDR`, `PROXY_LISTEN` (default `127.0.0.1:9030`), `PROXY_CAP` (default `200`)

## Scenario 1: Rewrite decisions without a network (US1–US3, fastest signal)

The rewrite module is a pure function ([rewrite-decision.md](./contracts/rewrite-decision.md)), so most of
the feature is verifiable with no server and no database.

```sh
cargo test --lib rewrite
cargo test --test classifier_positions
cargo test --test decision_reasons
```

**Expected**: all pass. `classifier_positions` covers every row of the position table in
[cap-position-classifier.md](./contracts/cap-position-classifier.md), including the negative cases — the
`COUNT(*)`-over-derived-table case is the one to check first if anything else is failing, because it is the
failure the whole design exists to prevent. `decision_reasons` proves each reason in the closed set is
reachable, so a reason can never be silently unreachable in production records.

## Scenario 2: Properties of the rewriter (US1–US3)

```sh
cargo test --test roundtrip_props
```

**Expected**: pass. Covers D1 (totality — no input panics), D2 (idempotence — capping an already-capped
statement yields `AlreadyBounded`, not a doubled limit), and D5 (offset never altered).

## Scenario 3: The negative tests actually bite (Constitution Principle IV)

A classifier with only positive tests passes even if every position is labelled `Safe`. Mutation testing is
how that is checked.

```sh
cargo mutants --file src/rewrite/classify.rs
```

**Expected**: zero missed mutants in `classify.rs`. A missed mutant here means a safety label can be
flipped without any test noticing, which is a Principle I hole regardless of the passing suite.

## Scenario 4: Start the proxy and cap a real query (US1, SC-001)

```sh
cargo run --release &
mysql -h 127.0.0.1 -P 9030 -u analyst -e "SELECT * FROM events" | wc -l
```

**Expected**: 201 lines — the header plus 200 rows. Then confirm the small-table case is untouched:

```sh
mysql -h 127.0.0.1 -P 9030 -u analyst -e "SELECT * FROM small_lookup" | wc -l
```

**Expected**: the table's full row count plus one, with no advisory.

## Scenario 5: The aggregate is still correct (US2, SC-002 — the critical one)

```sh
mysql -h 127.0.0.1 -P 9030 -u analyst \
  -e "SELECT COUNT(*) FROM (SELECT user_id FROM events) t"
```

**Expected**: the true count, in the millions. **A result of 200 means the classifier is capping a derived
table and the build must not ship.** This single check is the difference between the design in the plan and
the design that was originally requested.

## Scenario 6: Differential run over the corpus (US2, SC-002)

```sh
cargo test --test differential -- --ignored --nocapture
```

Runs each statement in the reference corpus twice — once directly against `DORIS_ADDR`, once through the
proxy — and compares results cell by cell, ignoring rows absent solely because the final result was
truncated.

**Expected**: zero discrepancies. Not "few". SC-002 makes one discrepancy a failure.

## Scenario 7: Pass-through paths (US4, SC-003)

```sh
mysql -h 127.0.0.1 -P 9030 -u analyst -e "<a statement using Doris syntax MySqlDialect rejects>"
```

**Expected**: the statement succeeds normally — the proxy must not turn its own parse failure into a client
error. Then check the record shows `parse_outcome: "failed"`, `decision: "forwarded_unchanged"`,
`reason: "parse_failed"`.

Repeat for a write: `INSERT INTO summary SELECT … FROM events` must insert every row it would have inserted
without the proxy, and record `reason: "write_carrying_query"`.

## Scenario 8: Records answer the operator's questions (US4, SC-006)

Submit one statement of each decision kind, then — using only the log output, with no access to the client
session or the database — classify all of them correctly:

```sh
cargo run --release 2>&1 | grep rewrite_record
```

**Expected**: one record per statement, no gaps, `reason` present exactly when `decision` is
`forwarded_unchanged`, `forwarded_sql` present exactly when `decision` is `capped`. Field-level rules are in
[rewrite-record.md](./contracts/rewrite-record.md).

## Scenario 9: Latency budget (SC-004)

```sh
cargo run --release 2>&1 | grep rewrite_record   # collect elapsed_micros
```

**Expected**: p99 of `elapsed_micros` under 1000 for statements up to 8 KB. The end-to-end 5 ms budget is
measured separately by comparing client-observed latency with and without the proxy in the path.

## Order to run these in

1, 2, 3 need nothing but a toolchain and should gate every commit. 4, 5, 7 need a running proxy and a
cluster. 6 needs the corpus and is the release gate. 8 and 9 are rollout readiness. If time is short, run
5 — a wrong `COUNT(*)` is the failure that this whole feature was reshaped to avoid.
