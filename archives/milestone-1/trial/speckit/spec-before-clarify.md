# Feature Specification: Row-Cap Rewriting Proxy for Doris

**Feature Branch**: `001-l7-mysql-proxy`

**Created**: 2026-08-08

**Status**: Draft

**Input**: User description: "L7 MySQL proxy in front of Apache Doris that parses each incoming SQL statement and appends LIMIT 200 to queries and sub-queries before forwarding, to stop unbounded scans from hammering the cluster"

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Unbounded exploratory query is capped (Priority: P1)

An analyst connects a SQL client to what they believe is the Doris cluster and runs
`SELECT * FROM events` against a table with four billion rows. They forgot a `WHERE` clause and
forgot a `LIMIT`. Instead of the cluster spending minutes scanning and streaming the whole table,
the analyst gets 200 rows back promptly, notices the result looks truncated, and refines the query.

**Why this priority**: This is the entire reason the proxy exists. Unbounded exploratory queries are
the dominant source of accidental cluster load. If only this story ships, the proxy already delivers
its value, because the overwhelming majority of ad-hoc statements are single-level selects with no
sub-queries at all.

**Independent Test**: Point a SQL client at the proxy, run a bare `SELECT * FROM <large table>` with
no limit, and confirm exactly 200 rows are returned and that the cluster's reported scan volume for
that statement is bounded rather than full-table. No other story needs to exist for this to work.

**Acceptance Scenarios**:

1. **Given** a table with more than 200 rows, **When** the analyst submits `SELECT * FROM events`,
   **Then** the client receives exactly 200 rows.
2. **Given** a table with 12 rows, **When** the analyst submits `SELECT * FROM small_lookup`,
   **Then** the client receives all 12 rows and no error.
3. **Given** a statement that returns no rows to the client at all (for example `SET` or `USE`),
   **When** it is submitted, **Then** it is forwarded unchanged and its response is relayed unchanged.

---

### User Story 2 - Aggregates and joins keep returning true values (Priority: P2)

A data engineer runs `SELECT COUNT(*) FROM (SELECT user_id FROM events WHERE day = '2026-08-01') t`
to size a backfill. The answer must be the real count — 4.2 million — not 200. The same engineer runs
a query whose sub-query is one side of a join, and expects the join to consider every matching row,
not the first 200 the storage layer happened to produce.

**Why this priority**: A wrong number is worse than a slow query, because the analyst cannot tell it is
wrong. This story is what makes the proxy safe to deploy transparently rather than as an opt-in tool.
It ranks below P1 only because P1 is what delivers the load reduction; without P2 the proxy is not
deployable to real traffic.

**Independent Test**: Run a corpus of statements containing aggregates over sub-queries, joins on
sub-queries, `WITH` clauses, `UNION` branches, `EXISTS`/`IN` predicates, and window functions, directly
against the database and again through the proxy. Compare the returned values cell by cell. Every value
outside the final truncated row set must be identical.

**Acceptance Scenarios**:

1. **Given** a sub-query producing 4.2 million rows, **When** the analyst submits
   `SELECT COUNT(*) FROM (SELECT user_id FROM events) t`, **Then** the returned count is 4,200,000.
2. **Given** a `WITH` clause whose result is joined to a second table, **When** the statement is
   submitted, **Then** the join sees the full common table expression and the row values match those
   returned by the database without the proxy.
3. **Given** `SELECT a FROM t1 UNION ALL SELECT a FROM t2` where each branch returns 5,000 rows,
   **When** the statement is submitted, **Then** the client receives 200 rows drawn from the combined
   result, not 200 rows per branch.
4. **Given** a sub-query with its own `ORDER BY` feeding an outer filter, **When** the statement is
   submitted, **Then** the rows surviving the outer filter are the same rows the database would have
   produced without the proxy.

---

### User Story 3 - A limit the user wrote is honored (Priority: P3)

An analyst who already knows what they want writes `SELECT * FROM events ORDER BY ts DESC LIMIT 500`,
or pages through results with `LIMIT 50 OFFSET 1000`. The proxy must not quietly turn a deliberate
request into a different one, and must not break pagination.

**Why this priority**: This affects a smaller share of traffic than P1 and P2, and a user who wrote an
explicit limit has already demonstrated they are not running an unbounded scan. But getting it wrong
produces visible, confusing breakage for the users most likely to complain loudly.

**Independent Test**: Submit statements with an explicit `LIMIT` above the cap, below the cap, and with
an `OFFSET`, and confirm the row counts and the specific rows returned match the documented rule for
each case.

**Acceptance Scenarios**:

1. **Given** a table with 10,000 rows, **When** the analyst submits `SELECT * FROM events LIMIT 50`,
   **Then** the client receives 50 rows.
2. **Given** a table with 10,000 rows, **When** the analyst submits `SELECT * FROM events LIMIT 500`,
   **Then** the client receives the number of rows defined by the existing-limit rule (see FR-006).
3. **Given** a paginated read `SELECT * FROM events ORDER BY ts LIMIT 50 OFFSET 1000`, **When** the
   statement is submitted, **Then** the client receives rows 1001 through 1050 in `ts` order.

---

### User Story 4 - Operators can see what the proxy did (Priority: P4)

An analyst opens a ticket saying "my query returns 200 rows and I don't know why" or "my query got
slower and I don't think the cap applied". A platform engineer answers from the proxy's records in a
few minutes, without reproducing the query and without asking the analyst to re-run anything.

**Why this priority**: This story does not change what any query returns, so it is last. It is still
required before production rollout, because the proxy's whole behavior is invisible from both sides of
the connection and unexplained truncation is the ticket the team will receive most often.

**Independent Test**: Submit one statement of each kind — capped, not capped because it already had a
limit, not capped because it could not be parsed, not capped because capping was unsafe — then read the
proxy's records alone and correctly classify all four without access to the client or the database.

**Acceptance Scenarios**:

1. **Given** a statement the proxy rewrote, **When** an operator inspects the records for that statement,
   **Then** they can see the original statement's identity, that a rewrite occurred, and the exact SQL
   text forwarded to the database.
2. **Given** a statement the proxy could not parse, **When** an operator inspects the records,
   **Then** they can see that parsing failed and that the statement was forwarded unchanged.
3. **Given** a statement the proxy parsed but chose not to cap, **When** an operator inspects the
   records, **Then** they can see which reason applied.

---

### Edge Cases

- What happens when a single client submission contains several statements separated by semicolons?
  Each statement is evaluated independently; a decision on one does not carry to the next.
- What happens when the statement is a write that carries a query inside it, such as
  `INSERT INTO summary SELECT ... FROM events`? Capping would silently write fewer rows than intended.
- What happens when the statement is a `SELECT` with no table at all, such as `SELECT 1` or
  `SELECT NOW()`? A cap is harmless but pointless.
- What happens when a sub-query appears in a scalar position, such as
  `SELECT (SELECT MAX(ts) FROM events) AS latest`? The database already requires that to yield one row;
  imposing a cap changes nothing and risks changing an error into a wrong answer.
- How does the system handle a statement that parses but uses a Doris-specific construct the parser
  models incorrectly rather than rejecting outright?
- How does the system handle an outermost query that is itself inside a set operation, where the
  natural place for the cap is the whole set operation rather than any one branch?
- What happens when the client uses a prepared statement or a stored procedure body, where the text the
  proxy sees is not the text the database ultimately executes?
- How does the system handle a query whose result the client stops reading part way through?

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: System MUST evaluate every statement a client submits and decide, per statement, whether a
  row cap can be applied.
- **FR-002**: System MUST apply a row cap of 200 to the outermost result of a statement that returns rows
  to the client and does not already constrain its own row count.
- **FR-003**: System MUST NOT apply a row cap in any position where the capped rows are consumed by
  another part of the same statement rather than returned to the client. This includes, at minimum: a
  sub-query feeding an aggregate, a sub-query on either side of a join, a common table expression
  referenced elsewhere in the statement, an individual branch of a set operation, a sub-query inside an
  `EXISTS` or `IN` predicate, and a sub-query in a scalar position.
- **FR-004**: System MUST forward a statement unchanged when it cannot be parsed, and MUST NOT return an
  error to the client on account of the parse failure alone.
- **FR-005**: System MUST forward a statement unchanged when it parses but contains a construct whose
  capping safety has not been established.
- **FR-006**: When a statement's outermost result already specifies its own row count, System MUST
  [NEEDS CLARIFICATION: leave the user's limit untouched regardless of its size, replace it with the
  200-row cap, or apply the smaller of the two? And when an offset accompanies the limit, is the cap
  applied to the rows after the offset or to the offset and limit combined?]
- **FR-007**: For statements that write rows as a result of an inner query, such as
  `INSERT INTO ... SELECT`, System MUST [NEEDS CLARIFICATION: cap the inner query, which silently writes
  fewer rows than the author intended, or leave such statements entirely alone, which means a large
  backfill through the proxy is uncapped? A third option is to reject them so the author reconnects
  directly.]
- **FR-008**: When a result has been truncated by the proxy rather than by the user's own request,
  System MUST [NEEDS CLARIFICATION: signal the truncation to the client in some way the client can
  observe, or truncate silently? Signalling costs the analyst a moment of confusion; silence means a
  200-row answer is indistinguishable from a complete one.]
- **FR-009**: System MUST produce the forwarded statement by serializing its parsed representation, so
  that a rewrite cannot corrupt statement text by textual manipulation.
- **FR-010**: System MUST record, for every statement, an identity for the statement, whether a rewrite
  was applied, the reason when no rewrite was applied, and the forwarded text when a rewrite was applied.
- **FR-011**: System MUST treat each statement in a multi-statement submission independently.
- **FR-012**: System MUST maintain exactly one database connection per client connection for the lifetime
  of that client connection, and MUST NOT share it with any other client.
- **FR-013**: System MUST relay the client's authentication exchange to the database without interpreting
  or storing any credential.
- **FR-014**: Users MUST be able to obtain a full, uncapped result by writing an explicit row count in
  their statement, without any proxy-side configuration change. (Dependent on the resolution of FR-006.)
- **FR-015**: System MUST relay statements that return no row set — session settings, database selection,
  administrative commands — unchanged in both directions.

### Key Entities *(include if data involved)*

- **Statement**: One SQL statement as submitted by a client. Carries the original text, an identity
  derived from that text, and the connection it arrived on. The unit at which every decision is made.
- **Cap Position**: A location within a parsed statement where a row cap could syntactically be placed.
  Each position is classified as safe (its rows go to the client) or unsafe (its rows are consumed by
  another operator). Only one safe position exists per statement, and many statements have none.
- **Cap Decision**: The outcome for one statement — applied, not applied because the statement already
  constrains its rows, not applied because no safe position exists, not applied because parsing failed,
  not applied because the statement returns no rows. Carries exactly one reason.
- **Rewrite Record**: The durable account of one statement's handling: statement identity, cap decision
  and its reason, forwarded text when rewritten, and parse outcome. The only artifact linking what the
  client asked for to what the database received.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: For every statement the proxy reports as capped, the client receives no more than 200 rows.
- **SC-002**: Across a reference corpus of at least 200 real statements drawn from recorded traffic,
  every value returned through the proxy is identical to the value returned without the proxy, except
  for rows absent solely because the final result was truncated. Zero discrepancies is the pass mark;
  one is a failure.
- **SC-003**: At least 90% of statements in a 30-day sample of recorded traffic receive a cap decision
  other than "could not be parsed".
- **SC-004**: The proxy adds no more than 5 milliseconds at the 99th percentile to the time between a
  client submitting a statement and that statement reaching the database.
- **SC-005**: Total rows returned to clients across a representative day drop by at least 80% compared
  with the same workload run without the proxy, with no increase in the number of statements submitted
  that fail with an error.
- **SC-006**: Given a statement identity, an operator can determine whether and how the proxy altered
  that statement within 5 minutes, using the proxy's records alone.
- **SC-007**: No statement that the proxy forwards unchanged is slower to reach the database than it
  would have been without the proxy by more than the SC-004 budget.

## Assumptions

- Clients connect to the proxy using the same protocol and tooling they would use to connect to Doris
  directly, and no client is modified in order to use the proxy.
- The cluster retains its own resource controls — query timeouts and resource groups. The proxy reduces
  the frequency with which those controls are hit; it is not the last line of defense, which is why
  forwarding an uncapped statement on parse failure is acceptable rather than negligent.
- 200 is the correct cap for the initial deployment and is the same for every user and every statement.
  Per-user or per-table caps are out of scope for this feature.
- Some valid Doris SQL will not parse, because no parser used here models the Doris dialect exactly.
  The share of such statements is expected to be small but nonzero, and SC-003 sets the tolerance.
- Analysts who need complete results are expected to write an explicit row count, and will be told so.
  The proxy is not required to offer a bypass mechanism of its own.
- Recorded traffic representative enough to build the SC-002 corpus and measure SC-003 is available
  before implementation begins.
- The proxy runs as a single process per host and holds no state that must survive a restart; a restart
  drops in-flight connections and clients reconnect.
