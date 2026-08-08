# Survey: Automatic `LIMIT` Injection in an L7 SQL Proxy (Doris)

> **Template version:** 1.0 · **Surveyed:** 2026-08-08 · **Author:** agent (research subagent)
> **Scope:** Covers automatic appending/capping of `LIMIT` by a MySQL-protocol proxy in front of Apache Doris — prior art, native alternatives, sub-query semantics, existing-`LIMIT` interaction, `sqlparser` (datafusion-sqlparser-rs) mechanics, and parse-failure policy. Deliberately excludes all MySQL wire-protocol concerns (handshake, capability flags, packet framing) and excludes authz/row-level-security rewriting.

---

## ⚠️ Lead finding — read before specifying anything

**Apache Doris already implements the `sql_select_limit` session variable.** It is present in the FE source as `SessionVariable.SQL_SELECT_LIMIT = "sql_select_limit"`, backing field `sqlSelectLimit`, default `Long.MAX_VALUE` [S3]. It was requested in 2021 [S2] and 2023 [S1] for BI-tool compatibility, shipped, and has since been maintained (PR #34177 added `"DEFAULT"`/string input [S5]; PR #40106 fixed a materialized-view interaction bug, landing in 2.1.7 and 3.0.3 [S4]).

A proxy can therefore issue **one `SET sql_select_limit = 200` per session** and obtain the outermost-`SELECT` row cap with **zero parsing, zero rewriting, and zero semantic hazard**. Everything in §4's hazard table disappears.

**Second, harder finding: neither approach achieves the stated goal.** The premise is "stop unbounded scans from hammering the cluster." A `LIMIT` — injected or native — bounds *rows returned*, not *rows scanned*. `SELECT COUNT(*) FROM huge_fact` returns one row and scans everything; `LIMIT 200` changes nothing about the load. Doris pushes `LIMIT` into scan nodes and stops early **only** when the limit sits directly above a scan [S26][S29] — precisely the case that was never the problem. Any aggregation, join, sort, or `DISTINCT` between the scan and the `LIMIT` restores the full scan.

The mechanisms that actually bound scan work are Doris-native and already exist:

| Goal | Correct Doris mechanism | Not: |
|---|---|---|
| Cap rows returned to client | `sql_select_limit` session var [S3] | `LIMIT` injection |
| Cap rows/partitions/tablets **scanned** | `SQL_BLOCK_RULE` `cardinality` / `partition_num` / `tablet_num` (planning-time reject) [S6][S7] | `LIMIT` anything |
| Cap runtime scan volume | Workload Policy `be_scan_rows`, `be_scan_bytes` (runtime, 500 ms poll, `cancel_query`) [S6] | `LIMIT` anything |
| Cap wall-clock | `query_timeout` (900 s default), `max_execution_time` (900 000 ms) [S3] | `LIMIT` anything |
| Cap concurrency / protect from stampede | Workload group `max_concurrency`, `max_queue_size`, `queue_timeout` [S8] | `LIMIT` anything |

**Recommendation:** treat SQL-rewriting `LIMIT` injection as a *last-resort, narrow* control, not the primary one. See §7 for what remains spec-shaped if the project proceeds anyway.

---

## 1. Snapshot

*(Adapted: this characterises a **technique**, not a product. Field names retained from the template; values reinterpreted.)*

| Field | Value |
|---|---|
| Name | Automatic `LIMIT` injection (a.k.a. implicit row cap, query-rewrite row guard) |
| Vendor / owner | No owner — a folk technique. Nearest standardised form is MySQL's `sql_select_limit` (Oracle/MySQL, ~2001-era), reimplemented by MariaDB, Vitess, and Apache Doris |
| License | N/A (technique). Reference implementations: Doris Apache-2.0; `sqlparser` Apache-2.0 |
| First release | `sql_select_limit` predates MySQL 4.x; Doris implementation traceable to issues #5326 (2021-02) and #21063 (2023-06) [S1][S2] |
| Latest version (as of survey date) | Doris `sql_select_limit` current in 2.1.7+/3.0.3+ [S4]; `sqlparser` 0.62.0 [S16] |
| Popularity signal | Universal as a *native* feature (MySQL, MariaDB, Vitess, Doris). **Rare as a proxy-side AST rewrite** — no major MySQL proxy ships it as a first-class feature. ProxySQL offers only regex rewrite [S19]; MaxScale's `maxrows` truncates *results*, not queries [S21]; Vitess enforces `queryserver-config-max-result-size` by **erroring**, not truncating [S22] |
| Maintenance signal | Doris variable actively maintained (bugfix PRs 2024–2025). `sqlparser` very active under ASF governance, ~72 M downloads [S16][S17] |
| Primary interface | Three distinct surfaces: (a) session variable `SET`; (b) regex rewrite (ProxySQL); (c) parse → mutate AST → re-serialize (the proposed approach) |
| Agent compatibility | N/A |
| Language/stack neutrality | Technique is neutral. The AST-rewrite variant is **strongly** coupled to parser dialect coverage — see §8 |
| Greenfield vs brownfield bias | Heavily brownfield: the entire justification is "we cannot change the clients." If clients are controllable, this is the wrong layer |

## 2. One-paragraph thesis

A proxy sitting on the MySQL wire between analysts/BI tools and an OLAP cluster claims it can protect the cluster by silently bounding every query's result size — parse the incoming SQL, walk the AST, and append `LIMIT 200` wherever a query block lacks one, so that no client can accidentally ask for a hundred million rows. The core bet is that *result-set size is a good proxy for cluster cost*, and that *silently altering a query is preferable to rejecting it*. Both halves of that bet are weak. Result size correlates with cluster cost only for the narrow case of a bare scan; for aggregations, joins, and sorts — the queries that actually hurt an OLAP cluster — the correlation is near zero. And silent alteration converts a loud, diagnosable failure (a query that times out) into a quiet, undiagnosable one (a dashboard that reports 200 instead of 4.2 million). The technique's real, defensible value is narrower than the pitch: it protects the *proxy's own* memory and the client's network pipe from a runaway `SELECT *`, and it makes exploratory `SELECT * FROM fact_table` in a SQL console cheap. That value is real. It is also entirely delivered by `sql_select_limit`, without touching a single byte of SQL.

## 3. Workflow / artifact model

Three implementation strategies exist, in ascending order of cost and risk. A spec should choose one explicitly.

```
STRATEGY A — session variable (no SQL touched)
  client connects
    └─ proxy issues:  SET sql_select_limit = 200
    └─ proxy rejects any client statement matching ^\s*SET\b.*sql_select_limit
    └─ every subsequent statement forwarded BYTE-FOR-BYTE unchanged
  cost: one round-trip per session · risk: near zero · parser needed: none

STRATEGY B — regex rewrite (ProxySQL-style)
  statement
    └─ match_pattern  ^(SELECT .*[^)])$        (no trailing LIMIT)
    └─ replace_pattern  \1 LIMIT 200
  cost: trivial · risk: HIGH — no comment/string/paren awareness
  known to mangle: LIMIT inside string literals, trailing comments,
                   multi-statement, UNION precedence

STRATEGY C — AST rewrite (the proposed design)
  bytes ──► sqlparser::Parser::parse_sql(&MySqlDialect{}, sql)
             │
             ├─ Err(_) ──────────────────────────► FAIL-OPEN: forward verbatim,
             │                                      increment metric, log (see §6)
             └─ Ok(Vec<Statement>)
                  │
                  ├─ len() != 1 ────────────────► forward verbatim
                  ├─ Statement::Insert / CreateTable{query} /
                  │  Update / Delete / Explain ─► forward verbatim  (NEVER rewrite)
                  └─ Statement::Query(q)
                       │
                       ├─ q.fetch.is_some() ────► forward verbatim
                       ├─ q.limit_clause  ──► Some(LimitOffset{limit: Some(n), offset, ..})
                       │                        └─ cap policy (§4b)
                       │                     ──► Some(OffsetCommaLimit{offset, limit})
                       │                        └─ cap policy (§4b) — SEPARATE VARIANT,
                       │                                              easy to forget
                       │                     ──► None
                       │                        └─ inject LimitOffset{limit: Some(200),
                       │                                             offset: None,
                       │                                             limit_by: vec![]}
                       └─ ***outermost Query node ONLY*** — never recurse into
                          body.SetOperation branches, TableFactor::Derived,
                          Query.with (CTEs), or Expr::Subquery / InSubquery / Exists
                  │
                  └─ q.to_string()  ──► COMMENTS AND WHITESPACE ARE LOST (§8)
```

**Command surface** (the knobs a spec must name, not CLI commands):
`cap_rows` (the 200), `strategy` (A/B/C), `on_parse_error` (`pass_through` | `reject`), `existing_limit_policy` (`leave` | `replace` | `min`), `offset_policy` (`ignore` | `cap_offset_plus_limit`), `statement_allowlist` (which `Statement` variants are eligible), `bypass` (users/comment-tag that opt out).

**Artifact lifecycle** — the per-query path: bytes arrive → statement classified → eligible/ineligible decision → (maybe) mutated → forwarded → response streamed back. Two lifecycle facts matter for the spec: (1) the decision must be **observable** — a client must be able to learn its query was capped, otherwise silent truncation is undebuggable; the conventional mechanism is a MySQL warning or a response comment, plus a per-query log line; (2) the cap must be **idempotent** — re-injection on a query the proxy already capped must be a no-op, which it is, since `limit_clause` is then `Some`.

## 4. What it enforces vs. what it suggests

### 4a. Claims audit

| Concern | Enforced (mechanism genuinely blocks it) | Suggested (appears to, does not) |
|---|---|---|
| Bounded rows to client | Yes — for a plain top-level `SELECT` | — |
| Bounded rows **scanned** | Only when `LIMIT` sits directly above a scan node [S26][S29] | Any query with `GROUP BY` / `JOIN` / `ORDER BY` / `DISTINCT` above the scan — full scan still happens |
| Bounded memory on Doris BE | No — use `exec_mem_limit` / workload policy `query_be_memory_bytes` [S3][S6] | Injected `LIMIT` implies it, delivers nothing |
| Bounded wall-clock | No — use `query_timeout` / `max_execution_time` [S3] | — |
| Protection against a *malicious* client | No — a client can send `SELECT /*...*/` shapes that defeat the parser and hit fail-open (§6) | This is a **resource** control, not a security control. It must never be specified as the latter |
| Result correctness preserved | Only under the narrow rule in §4b | The naive "every query and sub-query" design silently corrupts results |

### 4b. Sub-query compatibility table — **the core deliverable**

Assume the proxy appends `LIMIT 200` to a query block that has no `LIMIT`. "Doris reaction" reflects documented Doris planner restrictions [S9][S10].

| # | Construct | Example | Result changed? | Doris planner reaction | Verdict |
|---|---|---|---|---|---|
| 1 | **Outermost plain `SELECT`** | `SELECT * FROM t WHERE x=1` | Yes — by design; this *is* the contract | Pushes limit into scan, stops early [S26] | ✅ **SAFE — inject here** |
| 2 | **Outermost `SELECT` with `GROUP BY`** | `SELECT k, count(*) FROM t GROUP BY k` | Yes — truncates group list | Plans fine; **full scan still occurs** | ⚠️ **SAFE but INEFFECTIVE** — no scan reduction |
| 3 | **Outermost with `ORDER BY`** | `SELECT * FROM t ORDER BY ts DESC` | Yes — becomes Top-N | Converted to TopN node [S29]. Note Doris **already** caps unlimited `ORDER BY` at 65 535 rows by default [S10] | ✅ **SAFE — and largely redundant** |
| 4 | **Aggregate over derived table** | `SELECT COUNT(*) FROM (SELECT x FROM t) d` | **YES — catastrophically.** `COUNT` returns `min(n, 200)` | Plans fine — no error | 🚫 **UNSAFE — silent wrong answer.** The canonical disaster case |
| 5 | **`ORDER BY` inside derived table** | `SELECT * FROM (SELECT * FROM t ORDER BY ts DESC) d WHERE region='EU'` | **YES** — filter now applied to an arbitrary top-200 | Plans fine | 🚫 **UNSAFE — silent wrong answer** |
| 6 | **Derived table feeding outer sort** | `SELECT * FROM (SELECT * FROM t) d ORDER BY amt DESC LIMIT 10` | **YES** — top-10 chosen from an arbitrary 200 rows | Plans fine | 🚫 **UNSAFE — silent wrong answer** |
| 7 | **`IN (SELECT …)`** | `WHERE id IN (SELECT id FROM u WHERE …)` | Yes — arbitrary 200-element IN-list | **Hard error.** Doris: "The subquery cannot have `LIMIT`" for IN/NOT IN [S9] | 🚫 **UNSAFE — breaks a previously working query** |
| 8 | **`NOT IN (SELECT …)`** | `WHERE id NOT IN (SELECT …)` | Yes — and inverted logic makes it worse | Same restriction as #7 [S9] | 🚫 **UNSAFE — hard error** |
| 9 | **`EXISTS (SELECT …)`** | `WHERE EXISTS (SELECT 1 FROM u WHERE u.id=t.id)` | No — non-empty stays non-empty for any cap ≥ 1 | Doris permits `LIMIT` alone, **forbids `LIMIT` + `OFFSET` together** [S9]; a `LIMIT` may block de-correlation into a semi-join | ⚠️ **Semantically safe, operationally pointless** — do not inject |
| 10 | **Correlated scalar sub-query** | `SELECT (SELECT max(v) FROM u WHERE u.id=t.id) FROM t` | No (1-row result) | Doris requires equality predicates + a single aggregate with **no `GROUP BY`** for de-correlation [S9]; adding `LIMIT` risks "unsupported" | 🚫 **UNSAFE — breaks planning, zero benefit** |
| 11 | **Non-aggregate scalar sub-query** | `SET x = (SELECT v FROM u WHERE …)` | Masks the multi-row runtime error Doris would raise [S9] | Plans, then behaves differently | 🚫 **UNSAFE — hides a real bug** |
| 12 | **`UNION` / `UNION ALL` branch** | `SELECT a FROM t1 UNION ALL SELECT a FROM t2` | **YES** — each branch truncated; total ≤ 200 × N, and `UNION` de-dup then loses more | Plans fine | 🚫 **UNSAFE per branch.** ✅ SAFE on the *outer* `Query` — note `LIMIT` after the last branch binds to the whole set operation, and `sqlparser` models this correctly as `Query{body: SetOperation, limit_clause}` |
| 13 | **`INTERSECT` / `EXCEPT` branch** | `SELECT a FROM t1 EXCEPT SELECT a FROM t2` | **YES** — truncating the subtrahend *adds* rows to the result | Plans fine | 🚫 **UNSAFE — silent wrong answer, wrong direction** |
| 14 | **CTE body (`WITH x AS (…)`)** | `WITH x AS (SELECT … ) SELECT COUNT(*) FROM x` | **YES** — identical to #4 | Plans fine | 🚫 **UNSAFE — silent wrong answer** |
| 15 | **CTE referenced N times** | `WITH x AS (…) SELECT … FROM x a JOIN x b …` | **YES** — truncation propagates to every reference, and may differ per reference if not materialised | Plans fine | 🚫 **UNSAFE — non-deterministic wrong answer** |
| 16 | **Recursive CTE** | `WITH RECURSIVE r AS (… UNION ALL …)` | **YES** — truncating the recursive term halts the fixpoint early | Not supported before 4.1 [S11][S12][S13]; will fail to parse/plan on 2.x/3.x/4.0 | 🚫 **UNSAFE — never inject** |
| 17 | **`INSERT INTO … SELECT`** | `INSERT INTO agg SELECT … FROM fact` | **YES — silent data loss.** 200 rows written instead of millions | Plans fine, writes 200 rows | 🚫🚫 **UNSAFE — worst case in the table. Must be hard-excluded.** |
| 18 | **`CREATE TABLE AS SELECT`** | `CREATE TABLE snap AS SELECT * FROM fact` | **YES — silent data loss.** 200-row table created | Plans fine | 🚫🚫 **UNSAFE — hard-exclude** |
| 19 | **`SELECT … INTO OUTFILE` / export** | `SELECT * FROM t INTO OUTFILE 's3://…'` | **YES — silent truncated export** | Plans fine | 🚫🚫 **UNSAFE — hard-exclude** |
| 20 | **`UPDATE`/`DELETE` with sub-query** | `DELETE FROM t WHERE id IN (SELECT …)` | **YES** — wrong rows deleted/retained | See #7 | 🚫🚫 **UNSAFE — hard-exclude the whole statement** |
| 21 | **Window function, outermost** | `SELECT ROW_NUMBER() OVER (ORDER BY v) rn, * FROM t` | Values correct; fewer rows returned | Window computed over full partition, then truncated | ✅ **SAFE at outermost** |
| 22 | **Window function over a capped input** | `SELECT SUM(v) OVER (ORDER BY ts) FROM (SELECT … ) d` | **YES** — running totals, ranks, percentiles all computed over 200 rows | Plans fine | 🚫 **UNSAFE — silent wrong answer** |
| 23 | **Query against a view** | `SELECT * FROM v` | View body expanded server-side; proxy cannot reach it | Fine | ✅ **SAFE** — outer cap applies, body untouched |
| 24 | **`EXPLAIN <query>`** | `EXPLAIN SELECT * FROM t` | Changes the plan being explained | Fine | ⚠️ **Exclude** — misleads the user |

**The rule that falls out of this table, stated as one sentence:**

> Inject `LIMIT` into **exactly one** node — the outermost `Query` of a standalone read-only `SELECT` statement — and into **no** nested query block, ever.

Rows 4–22 are not edge cases; #4, #5, #14 and #17 are the shapes BI tools and ETL jobs emit constantly. The design as briefed ("every query and sub-query") produces silent wrong answers on the most common analytical query shapes in the catalogue, and hard planner errors on `IN (SELECT …)`.

### 4c. Existing `LIMIT` already present

| System | Policy when a `LIMIT` is already there |
|---|---|
| MySQL `sql_select_limit` | **Leave alone** — an explicit `LIMIT` takes precedence over the variable [S27] |
| Doris `sql_select_limit` | Issue #5326 explicitly warns the implementation "may not be completely consistent with MySQL… the limit is not determined by whether there is a limit in the select statement" [S2] — **must be tested, do not assume MySQL parity** |
| MaxScale `maxrows` | No query rewrite at all; if the *result* exceeds `max_resultset_rows` an **empty** result is returned [S21] |
| Vitess | Errors out ("row count exceeded N") rather than truncating; this divergence from MySQL was filed as a bug and fixed [S23] |
| ProxySQL | Regex has no notion of an existing `LIMIT`; the rule author must exclude them in `match_pattern` [S19] |

**Recommendation for a proxy:** `min(existing, cap)`. It is the only policy that both preserves the protective invariant and respects intent. But be honest in the spec that `min()` **is** a result change for `LIMIT 1000` under a cap of 200, and must be surfaced as a warning — otherwise you have reintroduced silent truncation for the one class of user who explicitly thought about row counts. `leave` is the MySQL-compatible choice and the weaker one.

**`LIMIT n OFFSET m` — the trap.** Capping `n` to 200 does not bound the work: `LIMIT 200 OFFSET 5000000` still requires the engine to produce and discard 5 000 000 rows. A cap on `n` alone provides *no* protection against deep pagination, which is one of the more common ways a BI tool actually hammers an OLAP cluster. Options: (a) bound `m + n` against the cap; (b) bound `m` separately with a distinct, much larger threshold; (c) accept and document the gap. Option (a) is the honest resource control but silently breaks any legitimate paginator past page N.

**`LIMIT a, b` — the second trap.** Doris supports both `LIMIT n OFFSET m` and MySQL's two-argument `LIMIT offset, count` ("limit m,n means output n records starting from the mth line") [S10]. In `sqlparser` these are **two distinct enum variants** — `LimitClause::LimitOffset` and `LimitClause::OffsetCommaLimit` [S15]. An implementation that pattern-matches only `LimitOffset` will treat `SELECT … LIMIT 100, 50000` as "no limit present" and inject a **second** `LIMIT`, producing invalid SQL or a wrong cap. This is the single most likely implementation bug and belongs in the spec as an explicit requirement with a test case.

## 5. Strengths

- **Genuinely stops the console `SELECT * FROM fact` accident.** For a bare scan with no aggregation, Doris pushes the limit into the scan node and halts early [S26][S29] — real, measurable protection for the exact case a human typing into a SQL client causes.
- **Protects the proxy itself.** Whatever happens on the cluster, a capped result bounds the proxy's own buffering and the client's network pipe. This is a legitimate reason for a *proxy* to care that a database-side control does not give you.
- **Tamper-resistant in a way `sql_select_limit` is not.** A client can undo a session variable with `SET sql_select_limit = DEFAULT` [S5]; it cannot undo a rewrite. This is the strongest — arguably the only — argument for Strategy C. Note it is also served far more cheaply by intercepting `SET` statements (a prefix match) than by rewriting every query.
- **Per-query-shape policy is possible.** A session variable is one number per connection; a rewriter can apply different caps to different tables, users, or query shapes. No native Doris mechanism offers this.
- **Enumerable rule set.** The safe/unsafe boundary in §4b is finite, closed, and testable — which makes it unusually good spec material (see §7).

## 6. Weaknesses / friction

- **Silent wrong answers are the default failure mode.** Rows 4, 5, 6, 14, 15, 22 of §4b all plan cleanly and return incorrect data. There is no error, no warning, no log line unless the proxy manufactures one. A finance dashboard reporting 200 instead of 4.2 M is a worse outcome than the slow query it prevented.
- **Silent data loss on write paths.** `INSERT … SELECT` and `CTAS` (rows 17–18) turn a resource control into a data-corruption engine. Excluding them is mandatory, and "we'll remember to exclude them" is exactly the kind of thing that regresses.
- **Solves the wrong problem.** See lead finding: it bounds returned rows, not scanned rows. The queries that hammer an OLAP cluster — large aggregations, exploding joins, `ORDER BY` over billions — are precisely the ones a `LIMIT` does not help.
- **Parse coverage is an unbounded liability.** `sqlparser` ships no Doris dialect [S17]; `MySqlDialect` is the closest fit. Doris-specific syntax — `SET_VAR` hints in comments, `PARTITION(p)` clauses, `INTO OUTFILE` with broker properties, bitmap/array/variant types, `STREAM LOAD` adjacents — will fail to parse or mis-parse. Every parse failure is either a bypass (fail-open) or an outage (fail-closed).
- **Comment loss on re-serialization.** `to_string()` recovers SQL "with comments removed, normalized whitespace and keyword capitalization" [S17]. Doris carries per-query hints in comments (`/*+ SET_VAR(query_timeout=60) */`); BI tools and observability stacks carry routing/attribution tags there. Stripping them silently changes behaviour and destroys attribution. **This alone argues against full re-serialization** — see §8.
- **Doris's own implementation of the equivalent feature has had correctness bugs.** PR #40106 fixed `sql_select_limit`/`default_order_by_limit` being incorrectly re-applied after materialized-view query rewrite, "producing erroneous results" [S4]. If Doris got this wrong inside its own optimizer, a proxy doing it from outside with less information should expect to get it wrong too.
- **Session-state leakage if connections are pooled.** Strategy A is trivial only if the proxy owns backend session state. If backend connections are multiplexed across clients, a `SET` on a shared connection leaks. This is a real constraint on Strategy A, and it is the second-best argument for rewriting.
- **Undebuggable for the end user.** Without an explicit warning channel, a user cannot distinguish "my table has 200 rows" from "the proxy capped me at 200." Every hour spent on that confusion is a cost the survey should attribute to this technique.
- **Prepared statements and multi-statement.** `COM_STMT_PREPARE` payloads and `;`-separated batches each need an explicit policy; the natural safe answer (pass through) is another bypass.

## 7. Fit signals

**Strong fit when:**
- The cap is applied at exactly one AST node (outermost `Query` of a standalone `SELECT`) and the goal is *client/proxy* protection, not cluster protection.
- Clients are uncontrollable, the workload is interactive/ad-hoc (SQL consoles, notebooks), and the dominant failure is a human typing `SELECT *`.
- The cap must survive a client that would otherwise reset a session variable.
- Doris version predates the `sql_select_limit` fixes (< 2.1.7 / < 3.0.3) [S4].

**Poor fit when:**
- The actual goal is bounding scan volume or cluster load — use `SQL_BLOCK_RULE` + workload policies [S6][S7].
- ETL/ELT traffic shares the port with analyst traffic — `INSERT … SELECT` and `CTAS` make the blast radius data loss.
- The workload is BI-tool-generated: derived tables and CTEs wrapping aggregates (rows 4, 14) are the house style of Looker, Tableau extracts, and dbt.
- Correctness matters more than latency — which, for an analytics database, it usually does.

### Spec-shaped vs. exploratory

**Spec-shaped — write these as normative requirements now:**
1. **The injection-site rule.** "Exactly one node: the outermost `Query` of a standalone read-only `SELECT`." Binary, testable, and it is the whole safety story.
2. **The §4b compatibility table**, converted to a statement/expression allowlist. Each row becomes one requirement and one test. This is the single most valuable artefact this survey produces.
3. **The statement-type exclusion list.** `Insert`, `CreateTable{query: Some(_)}`, `Update`, `Delete`, `Explain`, `SELECT … INTO OUTFILE`, any multi-statement batch — enumerable, closed.
4. **Existing-`LIMIT` policy.** `min(existing, cap)` with both `LimitClause` variants handled; `LIMIT ALL` treated as absent; `fetch: Some(_)` treated as present. Table-driven tests, one row per syntax form.
5. **`OFFSET` policy.** Whichever of (a)/(b)/(c) in §4c is chosen, it is a one-line normative statement plus tests.
6. **Parse-failure policy.** Fail-open, with a required counter metric and log line. Binary.
7. **Observability contract.** "When a query is capped or modified, the proxy MUST emit a warning to the client and a structured log record containing the original and rewritten statement digests." Testable.
8. **Idempotency.** Re-processing an already-capped statement is a no-op. One property test.

**Exploratory — do not freeze these into a spec yet; run experiments:**
- **Whether the rewrite is needed at all.** Bench `SET sql_select_limit = 200` against the real workload on the real Doris version. This experiment can delete the entire feature and should run before the spec is written.
- **Doris's actual `sql_select_limit` semantics** vs. an explicit `LIMIT`, given the #5326 caveat [S2]. Empirical, version-specific.
- **Parse success rate** of `MySqlDialect` against a captured production query log. Everything about fail-open's acceptability depends on this number, and it is currently unknown.
- **Round-trip fidelity** on that same corpus: parse → `to_string()` → re-parse → compare, plus a check that Doris returns identical results for original and re-serialized text.
- **Whether `min()` or `leave` is right for existing limits** — depends on how many real queries carry a `LIMIT` above the cap.
- **Deep-pagination frequency** — determines whether the `OFFSET` gap matters.

## 8. Rust / systems-software notes

**Crate.** `sqlparser` 0.62.0, Apache-2.0, ~72 M lifetime downloads [S16]. Now under ASF governance as `apache/datafusion-sqlparser-rs` [S17]; DataFusion is the anchor consumer, so the crate tracks that project's needs rather than proxy use cases.

**AST shape (0.62.0)** [S14][S15]:

```rust
pub struct Query {
    pub with: Option<With>,
    pub body: Box<SetExpr>,
    pub order_by: Option<OrderBy>,
    pub limit_clause: Option<LimitClause>,   // ← the only limit surface
    pub fetch: Option<Fetch>,                // FETCH FIRST n ROWS — separate!
    // …
}

pub enum LimitClause {
    LimitOffset {
        limit: Option<Expr>,      // LIMIT { <N> | ALL }
        offset: Option<Offset>,   // OFFSET n [ROW|ROWS]
        limit_by: Vec<Expr>,      // ClickHouse LIMIT … BY — irrelevant here
    },
    OffsetCommaLimit {            // MySQL LIMIT <offset>, <limit>
        offset: Expr,
        limit: Expr,
    },
}
```

**Version pinning is load-bearing.** Older `sqlparser` exposed flat `Query.limit` and `Query.offset` fields; these were consolidated into `limit_clause` in a breaking change that rippled through DataFusion's visitors [S17]. Any code sample, blog post, or LLM recollection predating that change will not compile, and — worse — a partial migration that handles `LimitOffset` but not `OffsetCommaLimit` compiles fine and is wrong. Pin exactly and put both variants in the test matrix.

**Round-trip fidelity — the decisive implementation concern.** Three distinct problems:

1. **Comments are dropped.** The crate's own framing is that `to_string()` recovers SQL "with comments removed, normalized whitespace and keyword capitalization" [S17]. For Doris this is not cosmetic: `/*+ SET_VAR(...) */` hints are semantically significant, and BI/observability tooling puts attribution tags in comments. Dropping them changes query behaviour and breaks tracing.
2. **`Display` has no dialect context.** Issue #2153 [S18] documents this structurally: the ClickHouse `Display` impl uppercases type names because `Display` cannot know which dialect it is rendering for, producing SQL the source engine rejects. The bug is ClickHouse-specific; the *defect class* is not. Any place where Doris's accepted spelling differs from `sqlparser`'s canonical rendering is a latent instance.
3. **Identifier quoting and case normalization.** Keyword capitalization is normalized on output; identifier quoting is preserved via `Ident.quote_style` but only when the parse captured it correctly.

**Consequence — a design recommendation stronger than "be careful":** do **not** parse-mutate-`to_string()` the whole statement. Parse for *analysis only* (decide: eligible? where does the outermost query block end? is a `LIMIT` present?), then perform a **minimal textual splice** on the original bytes — append ` LIMIT 200` at the computed offset, or rewrite just the numeric literal in an existing clause. The original SQL, comments, hints, whitespace and all, is forwarded byte-identical apart from the splice. This eliminates the entire re-serialization fidelity class at the cost of needing token spans. `sqlparser` exposes token locations via its tokenizer, though span coverage on AST nodes has historically been incomplete — **verify this against 0.62.0 before committing to the design** (flagged in §11).

**Dialect.** There is no Doris dialect [S17]; supported dialects are Generic, Hive, MsSql, MySql, PostgreSql, Redshift, SQLite, Snowflake. Doris is MySQL-protocol and largely MySQL-syntax, so `MySqlDialect` is the starting point — but Doris's SQL surface is a superset with OLAP extensions. `GenericDialect` is more permissive and correspondingly more likely to mis-parse. Measure the failure rate on a real query log before choosing; this is not a decision to make from the armchair.

**Concurrency/perf.** Parsing is CPU-bound and per-query on the hot path. `sqlparser` allocates freely; a large `IN (…)` list or a wide `SELECT` will allocate proportionally, which is a DoS-adjacent property worth an input-size cap. Strategy A costs one round-trip per *session*; Strategy C costs a parse per *statement*. That is a real, measurable difference in a proxy's tail latency and should be benchmarked, not assumed negligible.

**Property/mutation testing.** This feature is unusually well suited to it. Two properties worth encoding: *(idempotence)* `rewrite(rewrite(q)) == rewrite(q)`; *(equivalence)* for every statement class marked SAFE in §4b, `results(rewrite(q))` is a prefix of `results(q)` under a deterministic ordering. `cargo-mutants` (already anticipated in the repo's `.gitignore`) will find the classic bugs here — an inverted eligibility check, a missing `OffsetCommaLimit` arm, a `>` where `>=` belongs in the `min()` logic.

## 9. Cost & lock-in

- **Money:** Zero. All mechanisms discussed are open source (Doris Apache-2.0, `sqlparser` Apache-2.0, ProxySQL GPLv3).
- **Engineering cost:** Strategy A ≈ hours. Strategy C ≈ weeks, dominated not by the rewrite but by (a) building the parse-failure corpus, (b) the §4b test matrix, (c) the observability/warning channel, (d) ongoing dialect-drift maintenance as Doris adds syntax.
- **Ongoing cost:** Every Doris release that adds syntax is a potential new parse failure. This is a permanent tax with no natural end, and it scales with Doris's release cadence, not with your feature work.
- **Lock-in:** Low in the artefact sense — no proprietary formats. High in the behavioural sense: once clients are built against a silently-capped view of the data, removing the cap changes results for every downstream consumer, and any consumer that "corrected" for the cap will now double-count. Removing this feature later is harder than adding it.
- **Exit path:** Strategy A exits with a config flag. Strategy C exits by deleting the rewriter — but see behavioural lock-in above. Prefer Strategy A partly *because* its exit is trivial.

## 10. Evidence & sources

| # | Source | Type | Date | Notes |
|---|---|---|---|---|
| S1 | https://github.com/apache/doris/issues/21063 | issue | 2023-06-21 | "[Feature] add sql_select_limit session variable" — **closed**. BI-tool motivation |
| S2 | https://github.com/apache/doris/issues/5326 | issue | 2021-02 | Earlier request; **explicitly warns** the implementation "may not be completely consistent with MySQL… the limit is not determined by whether there is a limit in the select statement" |
| S3 | https://github.com/apache/doris/blob/master/fe/fe-core/src/main/java/org/apache/doris/qe/SessionVariable.java | source | 2026-08 (master) | **Primary evidence.** `SQL_SELECT_LIMIT = "sql_select_limit"`, default `Long.MAX_VALUE`; also `DEFAULT_ORDER_BY_LIMIT` (-1), `QUERY_TIMEOUT` (900 s), `MAX_EXECUTION_TIME` (900 000 ms), `EXEC_MEM_LIMIT` |
| S4 | https://github.com/apache/doris/pull/40106 | PR | 2024 | `sql_select_limit`/`default_order_by_limit` wrongly re-applied after MV rewrite → "erroneous results". Landed 2.1.7+, 3.0.3+ |
| S5 | https://www.mail-archive.com/commits@doris.apache.org/msg279454.html | commit | 2024 | PR #34177 — `SqlSelectLimitConverter`; accepts `"DEFAULT"` → `Long.MAX_VALUE`. Shows clients can reset the cap |
| S6 | https://doris.apache.org/docs/dev/admin-manual/workload-management/sql-blocking/ | docs | current | SQL Block Rule (`cardinality`, `partition_num`, `tablet_num`, planning-time reject) and Workload Policy (`be_scan_rows`, `be_scan_bytes`, `query_time`, `query_be_memory_bytes`, 500 ms poll, `cancel_query`) |
| S7 | https://doris.apache.org/docs/3.x/sql-manual/sql-statements/data-governance/CREATE-SQL_BLOCK_RULE/ | docs | current | `CREATE SQL_BLOCK_RULE rule_card PROPERTIES ("cardinality"="1000","global"="true","enable"="true")` |
| S8 | https://doris.apache.org/docs/3.x/admin-manual/workload-management/concurrency-control-and-queuing/ | docs | current | `max_concurrency`, `max_queue_size`, `queue_timeout`; **queuing is per-FE**, multiplying effective concurrency across FEs |
| S9 | https://doris.apache.org/docs/3.x/query-data/subquery/ | docs | current | **Critical.** IN/NOT IN sub-query "cannot have LIMIT"; EXISTS "cannot have both OFFSET and LIMIT"; de-correlation constraints |
| S10 | https://doris.apache.org/docs/3.x/sql-manual/sql-statements/data-query/SELECT/ | docs | current | `LIMIT n`, `LIMIT offset, count`, `LIMIT n OFFSET m` all supported; unlimited `ORDER BY` returns first 65 535 rows by default |
| S11 | https://doris.apache.org/docs/4.x/query-data/cte/ | docs | current | CTE support; recursive CTE **not** supported in 4.0 |
| S12 | https://doris.apache.org/releases/v4.1/release-4.1.0/ | release notes | 2026 | Recursive CTE listed for 4.1 |
| S13 | https://github.com/apache/doris/issues/26614 | issue | 2023 | "[Feature] With recursive syntax support" |
| S14 | https://docs.rs/sqlparser/latest/sqlparser/ast/struct.Query.html | API docs | 0.62.0 | `limit_clause: Option<LimitClause>`, `fetch: Option<Fetch>`; no flat `limit`/`offset` |
| S15 | https://docs.rs/sqlparser/latest/sqlparser/ast/enum.LimitClause.html | API docs | 0.62.0 | `LimitOffset{limit, offset, limit_by}` and `OffsetCommaLimit{offset, limit}` |
| S16 | https://crates.io/crates/sqlparser | registry | 2026 | 0.62.0 latest; ~71.98 M downloads; Apache-2.0 |
| S17 | https://github.com/apache/datafusion-sqlparser-rs | repo | current | Dialect list (no Doris); "comments removed, normalized whitespace and keyword capitalization" on re-serialization; `limit`/`offset` → `limit_clause` migration |
| S18 | https://github.com/apache/datafusion-sqlparser-rs/issues/2153 | issue | 2026-01-08 | `Display` has no dialect context → emits SQL the source engine rejects. Defect class, not a one-off |
| S19 | https://proxysql.com/documentation/query-rewrite/ | docs | current | `mysql_query_rules`, `match_pattern`/`replace_pattern`, backreferences. Regex only — no AST |
| S20 | https://github.com/sysown/proxysql/issues/1728 | issue | — | "Unable to parse query" — users expect pass-through (fail-open) on proxy parse failure |
| S21 | https://mariadb.com/kb/en/mariadb-maxscale-24-maxrows/ | docs | current | `maxrows` filter: exceeding `max_resultset_rows` returns an **empty** result. Result-side, not query-rewrite |
| S22 | https://vitess.io/docs/23.0/reference/programs/vttablet/ | docs | current | `--queryserver-config-max-result-size`, default 10 000; **errors** rather than truncates |
| S23 | https://github.com/vitessio/vitess/issues/8899 | issue | 2021 | "Vitess implementation of `sql_select_limit` does not match MySQL" — Vitess errored, MySQL truncates. Fixed in PR #8944. Direct precedent for the hard-vs-soft cap decision |
| S24 | https://trino.io/docs/current/admin/properties-query-management.html | docs | current | `query.max-scan-physical-bytes` / `query_max_scan_physical_bytes` — Trino bounds **bytes scanned**, not rows returned |
| S25 | https://doris.apache.org/docs/3.x/sql-manual/sql-statements/account-management/SET-PROPERTY/ | docs | current | Per-user property mechanism; relevant to applying caps without a proxy |
| S26 | https://www.velodb.io/blog/deep-dive-data-pruning-apache-doris | vendor blog | — | For `LIMIT` queries Doris sets scan concurrency to 1 and stops on reaching the limit; engine halts upstream reads once satisfied |
| S27 | https://dev.mysql.com/doc/refman/8.0/en/server-system-variables.html | docs | current | `sql_select_limit`; an explicit `LIMIT` takes precedence over the variable. **Exact wording not retrieved — see §11** |
| S28 | https://shardingsphere.apache.org/blog/en/material/engine/ | blog | — | ShardingSphere rewrites `LIMIT` for sharded execution (correctness of merge), **not** as a resource guard — different motivation, often miscited as prior art |
| S29 | https://doris.apache.org/docs/dev/query-acceleration/optimization-technology-principle/topn-optimization/ | docs | current | `ORDER BY … LIMIT` → TopN node; TopN acceleration principles |

## 11. Confidence & gaps

- **Confidence: high** on the semantics table (§4b) — it follows from documented Doris planner restrictions [S9][S10] plus relational algebra, not from opinion. **High** on `sqlparser` AST shape [S14][S15]. **High** that Doris implements `sql_select_limit` — this rests on the FE source itself [S3] plus three independent maintenance artefacts [S4][S5][S1]. **Medium** on the exact runtime behaviour of Doris's `sql_select_limit` (see below). **Medium** on prior-art completeness.

- **Unverified claims** (flagged explicitly, per brief):
  1. **Doris's `sql_select_limit` behaviour when an explicit `LIMIT` is present is UNVERIFIED.** MySQL's rule is "explicit `LIMIT` wins" [S27], but Doris issue #5326 explicitly disclaims parity on exactly this point [S2]. **This must be tested hands-on before the spec depends on it.**
  2. **Whether Doris's `sql_select_limit` applies only to the outermost query block is UNVERIFIED.** Not found in docs or reachable source. The MV-rewrite bug [S4] implies it is applied as an optimizer rule, which raises the question of where in the plan it attaches. Behaviour on `INSERT … SELECT` and `CTAS` under a non-default `sql_select_limit` is unknown and is the highest-stakes open question — if it truncates writes, Strategy A carries the same data-loss risk as Strategy C.
  3. **No official Doris documentation page for `sql_select_limit` was located.** The variable is evidenced by source and issues, not by a docs entry. It may therefore be undocumented-but-present, which is a maintenance risk (undocumented behaviour changes without a deprecation notice).
  4. **`sqlparser` 0.62.0 release date is inconsistent across sources** — crates.io API reports 2026-05-07, docs.rs surfaced 2026-07-12. Immaterial to the design; noted for accuracy.
  5. **Exact MySQL manual wording for `sql_select_limit` was not retrieved verbatim** (the reference page truncates before that variable). The "explicit `LIMIT` takes precedence" claim comes from secondary summarisation of [S27] and should be confirmed against the manual directly.
  6. **`sqlparser` token-span coverage on AST nodes is UNVERIFIED for 0.62.0.** The §8 minimal-textual-splice recommendation depends on it. Spans have historically been incomplete in this crate. **Verify before committing to that design.**
  7. **`sqlparser` `MySqlDialect` coverage of Doris syntax is UNMEASURED.** No corpus was available. Doris `SET_VAR` comment hints, `PARTITION(p)`, `INTO OUTFILE` with broker properties, and OLAP type syntax are all plausible failures. Everything about fail-open's acceptability rests on this unmeasured number.
  8. **No production proxy was found that does AST-level `LIMIT` injection as a resource guard.** ProxySQL does regex rewrite [S19], MaxScale truncates results [S21], Vitess errors on result size [S22], ShardingSphere rewrites `LIMIT` for sharding correctness rather than protection [S28]. The absence of prior art for the exact proposed technique is itself a finding, and it is consistent with the conclusion that the mature answer is a native session variable.

- **Open questions a decision-maker must test hands-on:**
  1. On the target Doris version: does `SET sql_select_limit = 200` produce the desired behaviour for the real workload? **Run this before writing the spec — it may delete the feature.**
  2. Does `sql_select_limit` truncate `INSERT … SELECT` / `CTAS`? (Blocking for Strategy A.)
  3. Does the proxy pool backend connections? If yes, can it own per-client session state? (Determines whether Strategy A is available at all.)
  4. What is the `MySqlDialect` parse-success rate against a captured production query log?
  5. What fraction of production queries carry an existing `LIMIT`, and what fraction use deep `OFFSET`? (Determines the §4c policies.)
  6. Would `SQL_BLOCK_RULE` `cardinality` + a workload policy on `be_scan_rows` have prevented the specific incidents that motivated this project? If yes, the rewrite is solving a problem that Doris already solves — better, and at the right layer.
