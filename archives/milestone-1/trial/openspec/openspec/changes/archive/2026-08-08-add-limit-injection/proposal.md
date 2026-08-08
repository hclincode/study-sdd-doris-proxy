## Why

Analysts and BI tools point at our Apache Doris cluster and issue queries with no row bound. A single `SELECT * FROM events` against a multi-billion-row table saturates backend scan threads and degrades every other tenant on the cluster. We cannot fix this by asking people to write better SQL, and Doris' own session limits are per-user settings that clients routinely override.

We already run an L7 MySQL-protocol proxy in front of Doris. Putting a row cap there makes the bound unconditional and invisible to the client, and it is the only point in the path that sees every statement from every tool.

## What Changes

- The proxy parses each incoming statement with the `sqlparser` crate (`MySqlDialect`) and rewrites it to carry a row cap (default 200) before forwarding to Doris.
- The cap is applied to the top-level query and to sub-queries that produce a scannable row stream: derived tables in `FROM`, CTE bodies, and each branch of a set operation (`UNION` / `INTERSECT` / `EXCEPT`).
- An existing `LIMIT` is never loosened. `LIMIT 500` becomes `LIMIT 200`; `LIMIT 50` is left alone. `OFFSET` is preserved.
- Sub-queries used as predicates (`IN (...)`, `EXISTS (...)`, scalar `= (SELECT ...)`) are **not** capped, because capping them changes which rows the outer query returns rather than merely truncating output.
- **BREAKING for clients**: a query that would have returned more than the cap now returns a truncated result set with no error. This is deliberate and is the point of the change; it is called out here so it is not discovered in production.
- **Known semantic hazard, accepted in this change**: capping a derived table changes aggregate results computed over it. `SELECT COUNT(*) FROM (SELECT id FROM big) t` returns 200, not the true count. See design.md - Decisions.
- Statements the parser cannot handle are forwarded byte-for-byte unchanged rather than rejected.
- Every rewrite emits a structured record (original SQL, rewritten SQL, which nodes were capped) so operators can audit what the proxy did.

## Capabilities

### New Capabilities

- `query-limit-injection`: which statements and which query nodes receive a row cap, how an existing `LIMIT` interacts with it, and what the proxy must never rewrite.
- `sql-parse-fallback`: what the proxy does with SQL it cannot parse, given that `sqlparser` has no Doris dialect and therefore rejects some valid Doris SQL.

### Modified Capabilities

<!-- None. This is the first change in the repository; there are no existing specs to modify. -->

## Impact

- New rewrite stage between the proxy's statement-read path and its forward-to-Doris path. Every statement passes through it.
- New dependency: `sqlparser` (parse + rewrite AST + re-serialize).
- New configuration key for the cap value; new counters for rewrites, exemptions, and parse failures.
- No change to connection handling: the proxy keeps one backend connection per client connection and continues to pass authentication through to Doris untouched.
- Downstream: dashboards and extracts that today rely on unbounded result sets will silently return fewer rows. Operations needs the audit record before this ships.
