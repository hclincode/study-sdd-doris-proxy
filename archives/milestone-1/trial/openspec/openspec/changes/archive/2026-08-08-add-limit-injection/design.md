## Context

See proposal.md - Why for the motivation.

Constraints fixed before this design and not up for revision here:

- The proxy holds one backend Doris connection per client connection. There is no pooling, no multiplexing, and therefore no cross-session state to consult when deciding how to rewrite a statement. Every decision is made from the statement text alone.
- Authentication is passed through to Doris untouched. The proxy does not know the user's roles or resource group and cannot vary the cap by identity.
- SQL is parsed with the `sqlparser` crate. It provides `MySqlDialect`; it provides **no** Doris dialect. Doris extends MySQL syntax (materialised view hints, `PARTITION` selectors, `SELECT ... INTO OUTFILE` variants, some window and array functions), so a fraction of legitimate traffic will not parse.
- The rewrite runs on the statement path for every statement, so its cost is on the latency budget of every query.

## Goals / Non-Goals

**Goals:**

- A single, uniform, explainable rule for where the cap lands, so that when a user reports a wrong number the answer is derivable from the SQL text alone.
- Fail toward availability: a parser gap must not take down a working query path.
- Make the truncation attributable. The rewrite record is not an afterthought; it is what makes the whole feature debuggable.

**Non-Goals:**

- Per-user, per-table, or per-resource-group caps. One number, set by the operator. Identity-aware policy would need the auth pass-through to be broken open, which is out of scope.
- Rewriting prepared statements or multi-statement packets. Both are deferred; until then they take the parse-failure path.
- Estimating cost. This is a row cap, not a cost-based governor. A 200-row result from a full-table aggregate is still an expensive query.

## Decisions

### Cap every row-producing node, not just the top level

A `LIMIT` on the outer query does not stop Doris from scanning the inner one. `SELECT t.id FROM (SELECT id FROM huge) t LIMIT 200` still materialises the derived table. Since the entire justification for this feature is bounding backend scan work, capping only the outer query would leave the main failure mode untouched.

*Alternative considered*: cap only the top level. Much safer semantically - result sets are truncated but never wrong - and much less useful. Rejected because the queries that actually hurt the cluster are the ones with expensive inner scans.

### Predicate sub-queries are exempt; derived tables are not

There is a real line between a sub-query whose rows *become* output rows and one whose rows *decide* output rows. Capping the first truncates. Capping the second corrupts: `WHERE user_id IN (SELECT id FROM banned_users LIMIT 200)` silently un-bans everyone past row 200.

*Alternative considered*: cap predicate sub-queries too, on the theory that an `IN` list over a huge table is exactly the pathological pattern. Rejected: the failure is silent and produces confidently wrong answers. A slow query gets noticed; a wrong `WHERE` clause does not.

### The aggregate-over-derived-table hazard is accepted, not solved

`SELECT COUNT(*) FROM (SELECT id FROM big) t` returns 200. This is the sharpest edge in the design and it is knowingly left in place for this change.

The reasoning: detecting "this derived table feeds only aggregates, so capping it is destructive" requires classifying the parent's projection and grouping, and the classification has to be conservative or it will exempt sub-queries that genuinely need capping. Shipping the blunt rule first, with the rewrite record naming every capped node, gives us production evidence about how often the pattern actually occurs before we write the classifier.

*Alternative considered*: exempt derived tables under an aggregate-only parent from the start. This is the better end state and is expected to follow as a separate change; it is not in this one because the exemption rule should be shaped by observed traffic, not guessed at.

### `ORDER BY` inside a capped sub-query changes which rows survive

A sub-query with `ORDER BY` and no `LIMIT` is a top-N pattern missing its N. Injecting `LIMIT 200` yields the top 200 under that ordering, which is usually - not always - the intent. Where the outer query re-sorts or re-filters, the result differs from the unproxied one.

Decision: cap it anyway, and mark the node in the rewrite record as an ordered cap so these are separately countable. There is no rewrite that is both bounded and faithful here; the choice is between a bounded approximation and no bound.

### Writes are never capped

Truncating `INSERT INTO ... SELECT` writes incomplete data that persists after the session ends and after the proxy is reconfigured. A truncated read costs a re-run.

*Alternative considered*: cap writes too - a runaway `INSERT ... SELECT` is genuinely one of the worst things a client can do to the cluster. **This is the decision most likely to need the operator's input.** If silent data loss is unacceptable but unbounded writes are also unacceptable, the third option is to reject uncapped `INSERT ... SELECT` outright rather than truncate it. The default chosen here - allow through, log at warning - is the one that never corrupts data, at the cost of leaving a real load vector open.

### Parse failure forwards rather than rejects

Stated in the spec; the rationale in short is that `sqlparser` has no Doris dialect, so fail-closed would reject valid SQL for reasons that have nothing to do with the user's query. A missed cap is a resource risk with a timeout backstop; a rejected valid query is an outage for that user. The fail-closed switch exists for operators who weigh it the other way.

### Existing `LIMIT` is reduced to the minimum, not replaced

`min(existing, cap)` is the only rule that is monotone: adding the proxy can never make a query return *more* rows than it did before. Replacing outright would turn `LIMIT 5` into `LIMIT 200`, which is both surprising and, for a query the user deliberately bounded, wasteful.

`OFFSET` is preserved, which means `LIMIT 200 OFFSET 1000000` still forces Doris to walk a million rows. The cap bounds output, not work. Bounding `OFFSET` would break pagination, so it is left alone and the residual exposure is recorded rather than fixed.

## Risks / Trade-offs

- **Wrong aggregates over derived tables** → Accepted for this change. Mitigated only by the rewrite record naming capped nodes, so a wrong number can be traced to the proxy. Expected to be narrowed by a follow-up change.
- **Silent truncation looks like real data** → Every rewrite is recorded with original and forwarded SQL. Operators must ship a dashboard for this before enabling the proxy in front of production traffic.
- **Parser gap leaves statements uncapped** → Counted, so the gap is measurable rather than theoretical. If the counter is high, the response is to extend dialect handling, not to flip to fail-closed.
- **Re-serialising the AST changes the SQL text** → `sqlparser`'s `Display` output is not byte-identical to the input (whitespace, quoting, keyword case). Statements that are forwarded unchanged must be forwarded as the *original bytes*, never as a re-serialised AST, or the proxy will mangle SQL it had no reason to touch.
- **Per-statement parse cost** → Parsing is on every statement's latency path. Statements that are structurally ineligible for rewrite should be screened cheaply before a full parse.

## Open Questions

- Should the cap be exposed to clients as a session variable they can read (not write), so a tool can tell whether truncation is possible? Deferable: it changes nothing about the rewrite rule or the task breakdown.
