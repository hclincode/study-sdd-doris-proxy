## Context

See proposal.md - Why. The relevant prior decision is in the archived `add-limit-injection` design: the aggregate hazard was left in place deliberately, pending traffic evidence. This change spends that evidence.

The constraint that shapes the implementation is that classification is a property of the **parent**, not the sub-query. The rewriter currently walks the AST top-down and decides at each row-producing node in isolation. It now needs to know, at the moment it reaches a child, what its parent's projection looks like.

## Goals / Non-Goals

**Goals:**

- Make the exemption rule small enough to state in one sentence to a user asking why their number changed.
- Fail toward capping. An unrecognised parent shape gets the old behaviour, so extending the classifier later can only ever remove caps, never add them.

**Non-Goals:**

- Bounding the cost of an exempt scan. An exempt sub-query is unbounded by construction; that is the price of a correct count, and it is stated in the spec rather than hidden.
- Handling nesting beyond one level of parent. `SELECT COUNT(*) FROM (SELECT x FROM (SELECT y FROM big) a) b` exempts only the node whose immediate parent is aggregate-only; the deeper one is still capped. Correct-by-the-rule, arguably surprising, and left alone until traffic says otherwise.

## Decisions

### Exempt on the parent's shape, not the sub-query's

The wrong-count problem is caused by what the parent does with the rows, not by what the sub-query selects. `SELECT id FROM events` is the same sub-query in both `SELECT COUNT(*) FROM (...)` and `SELECT * FROM (...)`; only the first is safe to leave uncapped.

*Alternative considered*: exempt any sub-query whose own projection is aggregate. That catches `SELECT COUNT(*) FROM (SELECT COUNT(*) ...)` and misses the actual case entirely. Rejected as solving the wrong problem.

### `GROUP BY` in the parent disqualifies

A grouped parent can return one row per distinct key, which is unbounded in the same way the raw scan was. Exempting it would reintroduce the failure mode this whole feature exists to prevent, in a shape that looks superficially like an aggregate.

*Alternative considered*: exempt grouped parents whose key has low cardinality. Requires statistics the proxy does not have, and cardinality is a runtime property. Rejected.

### Window functions disqualify

`COUNT(*) OVER ()` is an aggregate in name and a row-preserving operator in behaviour - it emits one output row per input row. Treating it as an aggregate would exempt a sub-query that then streams every row to the client. The classifier tests for a window clause explicitly rather than matching on function names.

### Conservative default

Where the classifier cannot make sense of the parent, it declines to exempt. This is the direction that preserves the previous change's guarantee: nothing that was capped before becomes uncapped by accident. The cost is that some correct-count cases stay wrong until the classifier learns their shape, and the rewrite record makes those countable.

## Risks / Trade-offs

- **Exempt scans are unbounded** → Stated in the spec as its own scenario so nobody discovers it later. The remaining backstop is Doris' query timeout. If exempt scans become a load problem, the answer is a cost ceiling, not a re-tightened cap.
- **Dashboard numbers will move** → Aggregates that have been returning the cap value start returning true values. Communicate before deploying; the rewrite records from the previous change identify which queries are affected, by name.
- **Classifier drift** → Every new shape the classifier learns removes a cap. Each addition needs a test that pins the previously-capped behaviour as intentionally changed.
- **One-level-deep rule is surprising** → Documented in Non-Goals. Nested derived tables under an aggregate get partial exemption, which is defensible but not obvious.
