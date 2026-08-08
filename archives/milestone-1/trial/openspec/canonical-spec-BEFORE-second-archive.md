# query-limit-injection Specification

## Purpose
Defines the row cap the proxy imposes on queries travelling to Doris: which query nodes receive a `LIMIT`, how an existing `LIMIT` is reconciled with the cap, and which constructs the proxy must never rewrite because doing so would change results rather than merely truncate them.
## Requirements
### Requirement: Cap the top-level result set

The proxy SHALL ensure every statement it forwards that produces a result set to the client carries a top-level row bound no greater than the configured cap.

#### Scenario: Unbounded select gains a cap

- **WHEN** the client sends `SELECT id, ts FROM events WHERE ts > '2026-01-01'`
- **THEN** the proxy forwards `SELECT id, ts FROM events WHERE ts > '2026-01-01' LIMIT 200`

#### Scenario: Aggregate over a base table is unaffected in practice

- **WHEN** the client sends `SELECT COUNT(*) FROM events`
- **THEN** the proxy forwards `SELECT COUNT(*) FROM events LIMIT 200`
- **AND** the returned count is the true count, because the query produces a single row

### Requirement: Cap row-producing sub-queries

The proxy SHALL apply the cap to each sub-query that contributes a row stream to its parent: derived tables in `FROM`, common table expression bodies, and every branch of a set operation.

Each such node is capped independently; capping the outer query does not exempt an inner one, because Doris materialises the inner scan before the outer bound takes effect.

#### Scenario: Derived table is capped

- **WHEN** the client sends `SELECT t.id FROM (SELECT id FROM events) t`
- **THEN** the proxy forwards `SELECT t.id FROM (SELECT id FROM events LIMIT 200) t LIMIT 200`

#### Scenario: Every union branch is capped

- **WHEN** the client sends `SELECT id FROM events_a UNION ALL SELECT id FROM events_b`
- **THEN** each branch carries `LIMIT 200`
- **AND** the combined statement also carries a top-level `LIMIT 200`

#### Scenario: CTE body is capped

- **WHEN** the client sends `WITH recent AS (SELECT id FROM events) SELECT * FROM recent`
- **THEN** the body of `recent` carries `LIMIT 200`
- **AND** the outer `SELECT` carries `LIMIT 200`

#### Scenario: Capping a derived table changes an aggregate result

- **WHEN** the client sends `SELECT COUNT(*) FROM (SELECT id FROM events) t` and `events` holds 5,000,000 rows
- **THEN** the proxy forwards a statement whose inner query carries `LIMIT 200`
- **AND** the client receives `200` rather than `5000000`
- **AND** the proxy records the capped inner node in the rewrite record so the discrepancy is attributable

### Requirement: Never loosen an existing bound

Where a query node already carries a `LIMIT`, the proxy SHALL forward the smaller of that value and the configured cap, and SHALL preserve any `OFFSET` unchanged.

#### Scenario: Larger existing limit is tightened

- **WHEN** the client sends `SELECT id FROM events LIMIT 500`
- **THEN** the proxy forwards `SELECT id FROM events LIMIT 200`

#### Scenario: Smaller existing limit is kept

- **WHEN** the client sends `SELECT id FROM events LIMIT 50`
- **THEN** the proxy forwards the statement unchanged

#### Scenario: Offset survives the rewrite

- **WHEN** the client sends `SELECT id FROM events LIMIT 500 OFFSET 1000`
- **THEN** the proxy forwards `SELECT id FROM events LIMIT 200 OFFSET 1000`
- **AND** the proxy records that the effective scan depth remains 1,200 rows, since `OFFSET` is not itself bounded by the cap

### Requirement: Leave predicate sub-queries unrewritten

The proxy SHALL NOT insert a `LIMIT` into a sub-query that appears in a predicate or expression position: the operand of `IN`, `NOT IN`, `EXISTS`, `NOT EXISTS`, `ANY`, `ALL`, or a scalar sub-query in a `SELECT` list, `WHERE`, `HAVING`, or `ON` clause.

Capping such a sub-query changes which rows the outer query matches, producing a result that is wrong rather than short.

#### Scenario: IN operand is left alone

- **WHEN** the client sends `SELECT id FROM events WHERE user_id IN (SELECT id FROM banned_users)`
- **THEN** the sub-query is forwarded without a `LIMIT`
- **AND** only the outer query carries `LIMIT 200`

#### Scenario: Scalar sub-query is left alone

- **WHEN** the client sends `SELECT id, (SELECT MAX(ts) FROM events) AS latest FROM users`
- **THEN** the scalar sub-query is forwarded without a `LIMIT`

### Requirement: Never truncate a write

The proxy SHALL NOT apply the cap to the `SELECT` that supplies rows to a write statement, including `INSERT INTO ... SELECT`, `CREATE TABLE ... AS SELECT`, and `INSERT OVERWRITE`.

A truncated read is recoverable by re-running the query; a truncated write silently persists incomplete data.

#### Scenario: Insert-select passes through uncapped

- **WHEN** the client sends `INSERT INTO events_archive SELECT * FROM events WHERE ts < '2026-01-01'`
- **THEN** the proxy forwards the statement with no `LIMIT` added at any level
- **AND** the proxy emits a warning-level rewrite record noting that an uncapped write was allowed through

### Requirement: Forward non-query statements unchanged

The proxy SHALL forward statements that do not produce a scannable result set byte-for-byte unchanged. This includes DDL, `INSERT ... VALUES`, `UPDATE`, `DELETE`, `SET`, `USE`, `SHOW`, `DESCRIBE`, and administrative commands.

#### Scenario: Session variable assignment is untouched

- **WHEN** the client sends `SET query_timeout = 300`
- **THEN** the proxy forwards the exact bytes it received

#### Scenario: DDL is untouched

- **WHEN** the client sends `CREATE TABLE t (id BIGINT) DISTRIBUTED BY HASH(id) BUCKETS 8`
- **THEN** the proxy forwards the exact bytes it received

### Requirement: Make every rewrite auditable

For each statement it modifies, the proxy SHALL emit a structured record containing the original SQL text, the forwarded SQL text, and the set of query nodes that received a cap.

Without this, a truncated result set is indistinguishable to the client from a genuinely small table, and an operator cannot tell whether a wrong number came from the data or from the proxy.

#### Scenario: Record identifies the capped nodes

- **WHEN** the proxy rewrites `SELECT t.id FROM (SELECT id FROM events) t`
- **THEN** the emitted record names both the top-level query and the derived table as capped nodes

#### Scenario: Unmodified statements produce no rewrite record

- **WHEN** the proxy forwards `SET query_timeout = 300` unchanged
- **THEN** no rewrite record is emitted

### Requirement: Cap value is configurable

The cap SHALL be a single operator-set integer applied uniformly to every capped node, defaulting to 200. Changing it SHALL NOT require rebuilding the proxy.

#### Scenario: Operator raises the cap

- **WHEN** the operator sets the cap to 1000 and the client sends `SELECT id FROM events`
- **THEN** the proxy forwards `SELECT id FROM events LIMIT 1000`

#### Scenario: Cap must be positive

- **WHEN** the operator sets the cap to 0 or a negative value
- **THEN** the proxy refuses to start and reports the invalid configuration

