# Contract: Rewrite Decision

**Feature**: 001-l7-mysql-proxy | **Date**: 2026-08-08
**Implements**: FR-001, FR-004, FR-006, FR-007, FR-008, FR-009, FR-015
**Enforces**: Constitution Principles I and II

The public interface of `src/rewrite/`. Everything the proxy knows about SQL enters and leaves through this
one function.

## Interface

```
decide(sql: &str, cap: u64) -> CapDecision
```

**Totality**: `decide` never returns an error and never panics. Every input maps to a `CapDecision`. This
is Principle II expressed as a type: there is no path from "the rewriter had trouble" to "the client sees
an error".

**Purity**: no I/O, no clock, no global state. Same input, same output. This is what allows the negative
tests required by Principle IV to run in the hundreds without a database.

## Decision procedure

1. Parse `sql`. On failure → `ForwardedUnchanged { ParseFailed }`. Stop.
2. If more than one statement was parsed from the input → `ForwardedUnchanged { NoSafePosition }`. Stop.
3. If the statement returns no row set → `ForwardedUnchanged { NoRowSet }`. Stop.
4. If the statement is a write carrying a query → `ForwardedUnchanged { WriteCarryingQuery }`. Stop.
5. Classify positions ([cap-position-classifier.md](./cap-position-classifier.md)).
   Zero `Safe` positions → `ForwardedUnchanged { NoSafePosition }`. More than one → same. Stop.
6. Read `existing_limit` at the safe position:
   - present and ≤ `cap` → `AlreadyBounded { limit }`. The AST is not modified. Stop.
   - present and > `cap` → set the limit to `cap`. Leave `existing_offset` exactly as it was.
   - absent → set the limit to `cap`. Leave `existing_offset` exactly as it was.
7. Serialize the modified AST (FR-009: serialization only, never string manipulation).
8. Re-parse the serialized text and compare structurally with the AST from step 6.
   Mismatch, or re-parse failure → `ForwardedUnchanged { VerificationFailed }`. Stop.
9. → `Capped { applied_limit: cap, forwarded_sql }`.

Steps 1–4 are ordered so that the cheapest and most decisive exits come first, and so that a write is
reported as a write rather than as a generic absence of a safe position.

## Outcome table

| Outcome | Client sees | Advisory (FR-008) | Text sent to Doris |
|---------|-------------|-------------------|--------------------|
| `Capped` | at most `cap` rows | yes | serialized rewritten SQL |
| `AlreadyBounded` | the user's own row count | no | the original text, byte for byte |
| `ForwardedUnchanged` (any reason) | whatever Doris returns | no | the original text, byte for byte |

The middle column is the whole of FR-008: an advisory is attached when and only when the proxy is the cause
of the truncation. `AlreadyBounded` gets none, because the user asked for that row count themselves.

## Offset rule

The offset is read and never written. `LIMIT 500 OFFSET 1000` under a cap of 200 becomes
`LIMIT 200 OFFSET 1000` — rows 1001 through 1200. Rewriting the offset, or folding it into the limit, would
change which rows the client receives rather than merely how many, which Principle I forbids.

## Required properties

| ID | Property | How checked |
|----|----------|-------------|
| D1 | `decide` is total: no input produces a panic or an error | property test over generated and corpus SQL, plus fuzz over arbitrary bytes |
| D2 | `decide` is idempotent: `decide(forwarded_sql_of(decide(s)))` yields `AlreadyBounded` | property test; catches a cap being appended twice on a re-entrant path |
| D3 | `AlreadyBounded` and `ForwardedUnchanged` forward the input unchanged byte for byte | unit assertion on the forwarded text |
| D4 | Every reason in the closed set is reachable | one test per reason in `tests/contract/decision_reasons.rs` |
| D5 | A `Capped` outcome never alters the offset | unit test across present/absent offset with limits above and below the cap |
| D6 | Values returned through a `Capped` rewrite match values returned without it, apart from truncation | differential test against a live Doris (SC-002) |
