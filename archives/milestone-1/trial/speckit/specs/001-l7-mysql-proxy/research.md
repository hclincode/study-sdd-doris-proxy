# Phase 0 Research: Row-Cap Rewriting Proxy for Doris

**Feature**: 001-l7-mysql-proxy | **Date**: 2026-08-08

Unknowns extracted from the plan's Technical Context, plus one best-practices question per fixed
dependency. Each entry follows Decision / Rationale / Alternatives considered.

## R1: Where the cap can be placed without changing a result

**Decision**: Exactly one position per statement is eligible: the node whose rows are handed to the
client. For a plain `SELECT` that is the outermost query. For a set operation (`UNION`, `UNION ALL`,
`INTERSECT`, `EXCEPT`) it is the set operation as a whole, never a branch. For everything else —
sub-queries in `FROM`, in `IN`/`EXISTS`, in scalar position, CTE bodies, the query inside a write — there
is no eligible position and the statement is forwarded unchanged.

**Rationale**: A limit changes which rows exist at that point in the plan. That is harmless only when
"that point" is the end. Anywhere else, some other operator consumes the rows, and truncating its input
changes its output: an aggregate computes over fewer rows, a join loses matches, a filter selects from a
different candidate set, `ORDER BY` inside a sub-query decides *which* rows survive rather than merely
their order. Constitution Principle I forbids all of these.

**Alternatives considered**:
- *Cap every query node.* The literal feature request. Rejected: produces wrong aggregates. See the plan's
  Constitution Check.
- *Cap sub-queries only when the outer statement is a plain projection.* Tempting, and correct in a narrow
  set of cases, but the safety condition is a whole-statement property that must be re-derived per shape.
  Rejected as a large increase in classifier surface for a small increase in coverage; it can be added
  later as individual SAFE positions once each shape is proven.
- *Cap CTE bodies when the CTE is referenced exactly once and only in the outermost `FROM`.* Genuinely
  safe under those conditions, and genuinely useful. Rejected for this feature because reference counting
  across a statement is a second analysis pass; deferred, not dismissed.

## R2: Interaction with a limit the user already wrote

**Decision**: Take the minimum of the user's row count and 200. The user's offset is applied first and is
never modified. If the user's count is already at or below 200, the statement is left structurally
unchanged and recorded as `AlreadyBounded`.

**Rationale**: Settled by the clarification session, not by research; recorded here because the
implementation needs the offset rule stated precisely. `LIMIT 500 OFFSET 1000` becomes
`LIMIT 200 OFFSET 1000` — rows 1001 to 1200. Rewriting the offset instead would silently move the user's
page, which Principle I forbids just as much as changing an aggregate.

**Alternatives considered**: Leave larger user limits untouched (rejected in clarification: `LIMIT 5000000`
is precisely the case the proxy exists to stop). Always replace with 200 (rejected: enlarges a user's
`LIMIT 50` to 200 in the reverse direction, returning rows they did not ask for).

## R3: What `sqlparser` with `MySqlDialect` will and will not parse

**Decision**: Use `MySqlDialect` and treat parse failure as an expected, measured operating condition
rather than a defect. Budget for it explicitly: SC-003 sets the tolerance at no more than 10% of real
traffic failing to parse. Measure against recorded traffic before implementation, not after.

**Rationale**: `sqlparser` has no Doris dialect. Doris accepts syntax that MySQL does not, and the gap is
not a fixed list — it moves with Doris versions. Known-risky areas to measure: Doris-specific hints and
`PROPERTIES` clauses, `SELECT ... INTO OUTFILE` variants, materialized-view and rollup DDL, some window
frame spellings, and functions whose argument syntax is not MySQL-shaped. None of these are worth forking
a parser for, because Principle II makes an unparsed statement a pass-through rather than an outage.

**Alternatives considered**:
- *Fork `sqlparser` and add a Doris dialect.* Rejected: unbounded maintenance against a moving target, in
  exchange for coverage of statements that are mostly DDL and administrative — not the unbounded analyst
  `SELECT`s this feature targets.
- *Regex or lexical detection of a trailing `LIMIT` instead of parsing.* Rejected outright by Principle I
  and FR-009: a lexer cannot tell an outermost `LIMIT` from one inside a sub-query, and string splicing
  can corrupt a statement containing a `LIMIT` in a string literal or a comment.
- *Try `GenericDialect` as a fallback when `MySqlDialect` fails.* Rejected for the first version: a second
  dialect that parses a statement differently is a second way to be confidently wrong, and the resulting
  AST would be classified by rules written for MySQL semantics. Worth revisiting only with a measured
  parse-failure rate that exceeds the SC-003 budget.

## R4: Serializing the modified AST safely

**Decision**: Produce forwarded SQL only via `sqlparser`'s `Display` implementation on the modified AST,
then re-parse that output and compare it structurally to the AST that was intended. On any mismatch,
discard the rewrite and forward the original text unchanged, recording the reason.

**Rationale**: FR-009 forbids string splicing, which leaves serialization as the only route. But
`sqlparser`'s round-trip fidelity is best-effort: it normalizes whitespace and quoting, and can drop
constructs it parsed loosely. Silent loss of a construct changes results, so Principle I requires a check.
Principle II decides what to do when the check fails: downgrade to pass-through, never error.

**Alternatives considered**: Trust the serializer (rejected — see the plan's Complexity Tracking). Compare
the serialized string to the original string textually (rejected: normalization makes every rewrite look
like a difference, so the check would have no signal).

## R5: Structured records without a store

**Decision**: Emit one `tracing` event per statement, with fields rather than a formatted message, and let
the existing platform log pipeline collect them. The proxy stores nothing.

**Rationale**: Principle V forbids adding state, and Principle III requires the record to exist. A
structured log event satisfies both: the proxy is still restart-clean, and SC-006's "operator can answer
in 5 minutes" is a query against infrastructure the team already runs. Statement identity is a hash of the
normalized statement text so that records can be correlated without logging the SQL of every pass-through.

**Alternatives considered**: An embedded store or ring buffer with a query endpoint (rejected: state, plus
a second interface to secure and operate). Logging full SQL text on every statement (rejected: high volume,
and it puts user data in logs for statements the proxy did not even touch).

## R6: Connection model and its consequence for the rewriter

**Decision**: One Doris connection per client connection, established on client connect, closed on client
disconnect, never reused. Each statement is decided independently with no state carried between statements.

**Rationale**: Fixed by the constitution's Technical Constraints and by FR-012. The consequence that
matters for this feature is that the rewriter needs no session context — no current database, no prepared
statement table, no user identity — which is what allows `src/rewrite/` to be a pure function of the
statement text and testable without a network.

**Alternatives considered**: None. This is a given constraint, recorded here so the boundary it implies is
traceable to a stated reason rather than to habit.

## Remaining unknowns

None blocking. Two items are deliberately deferred rather than resolved:

- The exact parse-failure rate on this cluster's real traffic (R3). Measurable only against recorded
  traffic; it is a task in the setup phase, and it gates the SC-003 claim rather than the design.
- Whether single-reference CTE bodies should become a SAFE position (R1). Deferred to a later feature; the
  classifier is structured so adding a position is a new rule plus its paired negative test, not a rewrite.
