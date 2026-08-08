# Phase 1 Data Model: Row-Cap Rewriting Proxy for Doris

**Feature**: 001-l7-mysql-proxy | **Date**: 2026-08-08

This feature has no database and no persisted records. "Entities" here are the in-memory values that pass
between `src/rewrite/`, `src/proxy/` and `src/observe/` during the handling of one statement. They are
modelled because their shape is what the contracts and tests are written against; see the closing note on
how well this artifact fits.

## Entity: Statement

One SQL statement as received from a client, before any decision is made.

| Field | Type | Notes |
|-------|------|-------|
| `text` | string | The statement exactly as received. Never mutated. This is what gets forwarded on any pass-through. |
| `fingerprint` | 16-byte digest | Identity used in records. Computed over the statement text with literals normalized away, so the same query shape with different constants shares a fingerprint. |
| `connection_id` | integer | The proxy-assigned id of the client connection that submitted it. |
| `sequence` | integer | Position within a multi-statement submission, starting at 0. Present so FR-011's independence is visible in records. |

**Validation**: `text` is non-empty after trimming. No other validation — an invalid statement is Doris's
to reject, not the proxy's (Principle II).

**Lifecycle**: Created on receipt, dropped when the statement's response has been relayed. Nothing about a
`Statement` survives to the next statement on the same connection.

## Entity: ParsedStatement

The result of handing `Statement.text` to the parser.

| Field | Type | Notes |
|-------|------|-------|
| `outcome` | `Parsed(ast)` \| `ParseFailed(message)` | A sum type, not a nullable AST. `ParseFailed` is a normal outcome, not an error condition. |
| `statement_count` | integer | Number of statements the parser found. Anything other than 1 for a single submitted statement is treated as unclassifiable. |

**Relationships**: One per `Statement`. `ParseFailed` short-circuits directly to a `CapDecision` of
`ForwardedUnchanged { reason: ParseFailed }`.

## Entity: CapPosition

A location in a parsed statement where a row limit could syntactically be attached, together with its
safety classification. Produced by `src/rewrite/classify.rs`.

| Field | Type | Notes |
|-------|------|-------|
| `path` | node path | Where in the AST the position sits. Used by `apply.rs` to place the cap and by tests to assert on a specific position. |
| `kind` | enum | `OutermostQuery`, `SetOperationRoot`, `SetOperationBranch`, `DerivedTable`, `CteBody`, `InPredicate`, `ExistsPredicate`, `ScalarSubquery`, `WriteSourceQuery`, `Other`. |
| `safety` | `Safe` \| `Unsafe(cause)` | Only `OutermostQuery` and `SetOperationRoot` are `Safe`. Every other kind, including `Other`, is `Unsafe`. |
| `existing_limit` | optional integer | The row count already present at this position, if any. |
| `existing_offset` | optional integer | The offset already present at this position, if any. Read but never modified. |

**Validation rule (the load-bearing one)**: at most one `CapPosition` in a statement may be `Safe`. A
statement yielding two safe positions is a classifier defect; the rewriter must treat it as unclassifiable
and pass through rather than choose one. This invariant is asserted in the classifier's own tests.

**Default rule**: an AST shape that produces no matching `kind` is classified `Other` / `Unsafe`. The
classifier fails closed by construction, so a parser upgrade that introduces new node shapes degrades to
pass-through rather than to a wrong cap.

## Entity: CapDecision

The single outcome for one statement. Exactly one variant, carrying exactly one reason.

| Variant | Carries | Meaning |
|---------|---------|---------|
| `Capped` | applied row count, rewritten SQL text | A safe position existed and the cap was applied. Triggers the FR-008 client advisory. |
| `AlreadyBounded` | the user's row count | A safe position existed but the user's own row count was at or below the cap. Statement forwarded unchanged; no advisory. |
| `ForwardedUnchanged` | reason | No rewrite. See reasons below. |

**Reasons for `ForwardedUnchanged`** — a closed set, each independently reachable and each asserted by a
test in `tests/contract/decision_reasons.rs`:

| Reason | When |
|--------|------|
| `ParseFailed` | The parser rejected the text (FR-004). |
| `NoSafePosition` | Parsed, but every position is `Unsafe` (FR-003, FR-005). |
| `WriteCarryingQuery` | An `INSERT … SELECT`, `CREATE TABLE … AS SELECT`, or similar (FR-007). Deliberately distinct from `NoSafePosition` so writes are countable on their own. |
| `NoRowSet` | The statement returns no rows to the client — session settings, database selection, administrative commands (FR-015). |
| `VerificationFailed` | A cap was applied but the re-parse check disagreed with the intended AST (R4). The rarest reason; a nonzero rate here is a bug signal, not normal operation. |

**State transitions**: none. A `CapDecision` is computed once and never revised. This is deliberate — a
decision that could be revised would need state, which Principle V forbids.

## Entity: RewriteRecord

The durable account of one statement's handling, emitted as one structured event. The only artifact that
links what the client asked for to what Doris received.

| Field | Type | Notes |
|-------|------|-------|
| `fingerprint` | 16-byte digest | From `Statement`. The key an operator searches on. |
| `connection_id`, `sequence` | integer | From `Statement`. |
| `parse_outcome` | `parsed` \| `failed` | Plus the parser's message when failed. |
| `decision` | enum name | `Capped`, `AlreadyBounded`, or `ForwardedUnchanged`. |
| `reason` | enum name or null | Non-null exactly when `decision` is `ForwardedUnchanged`. |
| `applied_limit` | optional integer | Non-null exactly when `decision` is `Capped`. |
| `forwarded_sql` | optional string | Non-null exactly when `decision` is `Capped`. The rewritten text, so an operator can see the SQL Doris actually ran. |
| `elapsed_micros` | integer | Time spent in parse-classify-apply-verify. Feeds SC-004. |

**Validation**: emitted for every statement without exception, including pass-throughs — a missing record
is indistinguishable from a statement that never arrived, which would defeat SC-006. `forwarded_sql` is
recorded only for rewritten statements, so pass-through user SQL never enters the log.

**Relationships**: one `RewriteRecord` per `Statement`. No aggregation, no retention policy of its own; the
platform log pipeline owns both.

## Note on this artifact's fit

Roughly half of what a data model is normally for does not apply here. There is no persistence, no schema,
no migration, no identity across time, and no state machine — the "lifecycle" of every entity above is
"created, used once, dropped". Writing the sections that do not apply would have padded the document
without informing the implementation.

What did earn its place is the middle of the document: the `CapPosition` safety classification and the
closed set of `ForwardedUnchanged` reasons. Those two tables are the real specification of Principle I, and
`tests/contract/` is written directly against them. The rest is scaffolding that exists because the
template asked for it.
