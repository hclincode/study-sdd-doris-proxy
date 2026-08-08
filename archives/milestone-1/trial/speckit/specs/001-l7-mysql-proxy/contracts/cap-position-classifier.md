# Contract: Cap Position Classifier

**Feature**: 001-l7-mysql-proxy | **Date**: 2026-08-08
**Implements**: FR-002, FR-003, FR-005 | **Enforces**: Constitution Principle I

The classifier is the interface between "we parsed some SQL" and "we may modify it". It is the place where
Principle I is either upheld or broken, so its behavior is specified position by position rather than
described in prose.

## Interface

```
classify(ast: &Statement) -> Vec<CapPosition>
```

Total function. Never fails, never panics on any AST the parser produced. Returns every position at which
a row limit could syntactically be attached, each labelled `Safe` or `Unsafe`.

**Invariant C1**: at most one returned position is `Safe`.
**Invariant C2**: an AST shape not enumerated below is returned as `Other` / `Unsafe`. The default is
unsafe; adding a safe position requires an explicit rule and its paired negative test.
**Invariant C3**: `classify` does not mutate the AST.

## Position table

| # | Position kind | Example | Safety | Cause |
|---|---------------|---------|--------|-------|
| P1 | `OutermostQuery` | `SELECT a FROM t` → the whole query | **Safe** | Its rows are the rows the client receives. Truncating here is the one truncation the spec permits. |
| P2 | `SetOperationRoot` | `SELECT a FROM t1 UNION ALL SELECT a FROM t2` → the union node | **Safe** | The set operation's output is what reaches the client. |
| P3 | `SetOperationBranch` | either `SELECT` inside the union above | Unsafe | Capping a branch changes which rows enter the set operation, so `UNION` deduplication and the final row content both change. |
| P4 | `DerivedTable` | `FROM (SELECT … ) t` | Unsafe | Its rows are consumed by the enclosing query — most destructively by an aggregate, where a cap yields a wrong number with no visible symptom. |
| P5 | `CteBody` | the body of `WITH t AS (…)` | Unsafe | Referenced elsewhere in the statement; the reference site consumes the rows. Also potentially referenced more than once, where a cap would be applied to each use. |
| P6 | `InPredicate` | `WHERE id IN (SELECT …)` | Unsafe | Capping shrinks the membership set, so rows that should match the predicate stop matching. Silently changes which rows the outer query returns. |
| P7 | `ExistsPredicate` | `WHERE EXISTS (SELECT …)` | Unsafe | Same as P6. A cap here can turn a true `EXISTS` into a false one. |
| P8 | `ScalarSubquery` | `SELECT (SELECT MAX(ts) FROM e) AS latest` | Unsafe | The engine already requires one row. A cap either does nothing or converts a "subquery returned more than one row" error into a wrong value. |
| P9 | `WriteSourceQuery` | the `SELECT` in `INSERT INTO s SELECT …` | Unsafe | Capping writes fewer rows than the author asked for, and unlike a truncated read the effect persists after the connection closes. Reported as `WriteCarryingQuery`, not `NoSafePosition`. |
| P10 | `Other` | anything unenumerated | Unsafe | Invariant C2. |

## Required test pairs

Principle IV requires each rule to have a case proving it fires and a case proving it does not. Each row
below is one pair, written before the rule it covers.

| Rule | Positive case (cap IS placed) | Negative case (cap is NOT placed) |
|------|-------------------------------|-----------------------------------|
| P1 | `SELECT a FROM t` yields one `Safe` position | `SELECT 1` (no table) yields no `Safe` position |
| P2 | `SELECT a FROM t1 UNION ALL SELECT a FROM t2` yields exactly one `Safe` position, at the union root | the same statement yields zero `Safe` positions among its branches |
| P4 | `SELECT * FROM (SELECT a FROM big) t` yields a `Safe` position at the outer query only | `SELECT COUNT(*) FROM (SELECT a FROM big) t` yields no cap on the derived table; the count is unaffected |
| P5 | `WITH c AS (SELECT a FROM big) SELECT * FROM c` caps the outer query | the CTE body has no cap |
| P6/P7 | `SELECT * FROM t WHERE id IN (SELECT id FROM big)` caps the outer query | the `IN` sub-query has no cap |
| P8 | `SELECT (SELECT MAX(ts) FROM e) AS l, a FROM t` caps the outer query | the scalar sub-query has no cap |
| P9 | — | `INSERT INTO s SELECT a FROM big` yields no cap at all, and the decision reason is `WriteCarryingQuery` |
| C1 | — | no statement in the corpus yields two `Safe` positions |

## What this contract does not cover

Placement is not application. Whether the cap becomes `LIMIT 200`, or is reduced to the user's smaller
existing limit, or is skipped as `AlreadyBounded`, belongs to `apply.rs` and is specified in
[rewrite-decision.md](./rewrite-decision.md).
