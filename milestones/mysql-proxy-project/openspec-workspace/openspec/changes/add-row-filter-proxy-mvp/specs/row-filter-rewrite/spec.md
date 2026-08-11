## Purpose

Defines the parse, rewrite and forward path that enforces row-level policy on statements a user issues. This capability owns the guarantee that any statement the proxy forwards has every policy-bearing table reference constrained to the requesting user's permitted rows, and that any statement for which that cannot be proven is rejected rather than forwarded.

## ADDED Requirements

### Requirement: A forwarded statement constrains every policy-bearing table reference

The proxy SHALL NOT forward a statement unless every reference in it to a table with a policy for the requesting user has been constrained to that user's permitted values. This SHALL hold for references appearing anywhere in the statement, including in `FROM` clauses, every join operand, derived tables, subqueries in any clause, common table expressions, and every branch of a set operation.

#### Scenario: Simple select is constrained

- **WHEN** user `analyst`, restricted on `sales.orders` to `region IN ('APAC','EMEA')`, issues `SELECT * FROM sales.orders`
- **THEN** the statement reaching Doris returns only rows whose `region` is `APAC` or `EMEA`

#### Scenario: Subquery reference is constrained

- **WHEN** `analyst` issues `SELECT COUNT(*) FROM (SELECT * FROM sales.orders) t`
- **THEN** the count reaching the client counts only rows whose `region` is `APAC` or `EMEA`, not all rows of `sales.orders`

#### Scenario: Common table expression reference is constrained

- **WHEN** `analyst` issues `WITH c AS (SELECT * FROM sales.orders) SELECT * FROM c`
- **THEN** only rows whose `region` is `APAC` or `EMEA` are returned

#### Scenario: Every branch of a set operation is constrained

- **WHEN** `analyst` issues a `UNION ALL` whose branches both read `sales.orders`
- **THEN** neither branch contributes rows outside `region IN ('APAC','EMEA')`

#### Scenario: Both operands of a self-join are constrained

- **WHEN** `analyst` joins `sales.orders` to itself under two aliases
- **THEN** both operands are restricted, and no row outside the permitted values contributes to the result

#### Scenario: Restricted rows cannot be recovered through an aggregate

- **WHEN** `analyst` issues a statement whose only output is an aggregate over `sales.orders`, such as a `SUM` or `MAX` with no projection of individual rows
- **THEN** the aggregate is computed only over rows whose `region` is `APAC` or `EMEA`

### Requirement: Injected constraints cannot be defeated by the user's own predicate

The proxy SHALL combine its injected constraint with any user-supplied predicate such that the constraint holds regardless of the structure of the user's predicate. A disjunction written by the user SHALL NOT widen the set of rows beyond the permitted values.

#### Scenario: User disjunction does not widen the permitted set

- **WHEN** `analyst`, restricted to `region IN ('APAC','EMEA')`, issues `SELECT * FROM sales.orders WHERE region = 'AMER' OR 1=1`
- **THEN** no row whose `region` is outside `APAC` or `EMEA` is returned

#### Scenario: Outer join does not reveal restricted rows

- **WHEN** `analyst` issues a `LEFT JOIN` in which `sales.orders` is an operand
- **THEN** no row of `sales.orders` outside the permitted values contributes values to the result, and the result does not disclose the existence of such rows through NULL-extended output

#### Scenario: Correlated subquery cannot test restricted rows

- **WHEN** `analyst` issues a statement whose `WHERE` clause contains a correlated `EXISTS` or `IN` subquery reading `sales.orders`
- **THEN** that subquery evaluates only over rows whose `region` is `APAC` or `EMEA`, so the outer result cannot be used to infer the existence of restricted rows

### Requirement: A statement that cannot be fully analysed is rejected

The proxy SHALL reject, with an error returned to the client, any statement it cannot parse and any statement whose table references it cannot completely enumerate, whenever the requesting user has at least one configured policy. A rejected statement SHALL NOT be forwarded to the backend in any form.

#### Scenario: Unparseable statement from a policy-bearing user is rejected

- **WHEN** user `analyst`, who has at least one configured policy, issues a statement the proxy cannot parse
- **THEN** the proxy returns an error to the client and forwards nothing to the backend

#### Scenario: Unparseable statement from an unrestricted user is forwarded

- **WHEN** user `reporting`, who has no configured policy for any table, issues a statement the proxy cannot parse
- **THEN** the proxy forwards the statement unmodified

#### Scenario: Statement shape the rewriter does not support is rejected

- **WHEN** `analyst` issues a parseable statement containing a construct for which the proxy cannot determine whether a policy-bearing table is referenced
- **THEN** the proxy returns an error to the client and forwards nothing to the backend

#### Scenario: A statement whose execution depends on the backend's identity is refused

- **WHEN** a user with at least one configured policy issues a statement containing a conditionally-executed comment, whose content the backend runs or ignores depending on its own version
- **THEN** the proxy returns an error and forwards nothing, rather than forwarding a statement in which that condition has been resolved on the backend's behalf

#### Scenario: Rewriting never adds to what the original would have executed

- **WHEN** a statement is rewritten
- **THEN** the statement reaching the backend executes no projection, relation, or set-operation branch that the original statement would not have executed

#### Scenario: Rejection names the reason without disclosing policy contents

- **WHEN** the proxy rejects a statement
- **THEN** the client receives an error identifying that the statement was refused by the proxy, and that error does not enumerate other users' policies or permitted values

### Requirement: Write statements against policy-bearing tables are rejected

The proxy SHALL reject any `INSERT`, `UPDATE`, `DELETE` or `REPLACE` statement that references a table with a policy for the requesting user. The proxy SHALL NOT forward such a statement with or without an injected constraint.

#### Scenario: Update against a policy table is rejected

- **WHEN** `analyst` issues an `UPDATE` against `sales.orders`
- **THEN** the proxy returns an error and no rows are modified

#### Scenario: Insert selecting from a policy table is rejected

- **WHEN** `analyst` issues `INSERT INTO scratch.copy SELECT * FROM sales.orders`
- **THEN** the proxy returns an error and no rows are written

#### Scenario: Write against a table with no policy is forwarded

- **WHEN** `analyst` issues an `UPDATE` against `sales.products`, for which no policy is configured for `analyst`
- **THEN** the statement is forwarded

### Requirement: Session-management and metadata statements are analysed, not categorically refused

A statement that configures the session or reports metadata SHALL be analysed by the same rule as any other statement: every table reference in it is enumerated, and the statement is forwarded only if none of them is policy-bearing for the requesting user. Such statements SHALL NOT be refused merely for being of that kind, and SHALL NOT be forwarded merely for being of that kind.

A statement that **reads** a policy-bearing table's rows into session state SHALL be refused. Its result does not reach the client as a row set — it lands somewhere the proxy does not track — so constraining it would not bound what the client can later read.

A statement that merely **names** a policy-bearing table to report its metadata SHALL be forwarded. Refusing it would not withhold anything: the same information is reachable through `information_schema`, which carries no policy and is forwarded, so a refusal would cost compatibility and protect nothing. Metadata disclosure is an accepted and documented limitation of this control, and the two paths to it must not disagree.

#### Scenario: A metadata statement naming a policy table is forwarded

- **WHEN** a restricted user asks for the column list or definition of a policy-bearing table
- **THEN** the statement is forwarded, because the same metadata is already reachable through `information_schema` and refusing it would protect nothing while breaking clients that introspect

#### Scenario: A session statement that reads no table is forwarded

- **WHEN** a restricted user issues a statement that sets a session variable to a literal, such as a character set, autocommit mode, or time zone
- **THEN** the statement is forwarded to the backend unchanged

#### Scenario: A session statement that reads a policy table is refused

- **WHEN** a restricted user issues a statement assigning a session variable from a subquery that reads a policy-bearing table
- **THEN** the proxy refuses it, rather than forwarding a statement whose result would be readable later through session state the proxy cannot constrain

#### Scenario: A metadata statement is forwarded

- **WHEN** a restricted user issues a statement listing tables or databases
- **THEN** the statement is forwarded, consistent with metadata disclosure being an accepted and documented limitation rather than something this control claims to prevent

#### Scenario: An ordinary client can complete a connection

- **WHEN** a client connects and issues the session statements a MySQL connector ordinarily sends before its first query
- **THEN** each is forwarded, and the client reaches a state where it can issue queries

### Requirement: A single request carries a single analysable statement

The proxy SHALL reject a client request containing more than one statement when the requesting user has at least one configured policy, so that no statement reaches the backend without having been analysed in its own right.

#### Scenario: Multi-statement request is rejected

- **WHEN** `analyst` sends a request containing two statements separated by a semicolon
- **THEN** the proxy returns an error and forwards neither statement

#### Scenario: Comment cannot conceal a second statement

- **WHEN** `analyst` sends a request in which a second statement is placed after or within a comment
- **THEN** the proxy either rejects the request or forwards a single statement whose policy-bearing references are all constrained, and never forwards an unanalysed statement

### Requirement: Rewriting preserves everything except restricted rows

A rewritten statement SHALL differ from the original only by excluding rows the requesting user is not permitted to see. The proxy SHALL NOT change the number, order, names or types of result columns, and SHALL NOT change the number or order of parameter placeholders.

#### Scenario: Column shape is unchanged by rewriting

- **WHEN** `analyst` issues `SELECT id, region, total FROM sales.orders`
- **THEN** the client receives exactly the columns `id`, `region`, `total`, in that order, with the types Doris would return for the unrewritten statement

#### Scenario: Ordering among permitted rows is preserved

- **WHEN** `analyst` issues a statement with an `ORDER BY` against `sales.orders`
- **THEN** the permitted rows are returned in the order the unrewritten statement would have produced for those rows

#### Scenario: Placeholder count and order are unchanged

- **WHEN** a statement containing parameter placeholders is rewritten
- **THEN** the rewritten statement contains the same number of placeholders in the same order as the original

#### Scenario: A user permitted all present values sees the same rows as without the proxy

- **WHEN** `analyst` queries `sales.orders` and every row's `region` is within the permitted values
- **THEN** the result is identical to the result of the unrewritten statement
