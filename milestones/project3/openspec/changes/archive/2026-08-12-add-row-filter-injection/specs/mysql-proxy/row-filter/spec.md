## Purpose

Defines how a configured row-filter predicate is chosen for a statement, when the proxy may insert it into a read-only statement before forwarding, where it goes, and what happens to the statements it cannot rewrite.

## ADDED Requirements

### Requirement: Filter rules are configured per listener and per table

Each listener MAY define row-filter rules mapping a table name to a predicate expression. A table with no rule SHALL NOT be filtered. A listener with no rules SHALL behave exactly as a listener that predates this capability.

#### Scenario: Listener with no rules

- **WHEN** a listener defines no row-filter rules
- **THEN** every statement is forwarded unchanged

#### Scenario: Table has a rule

- **WHEN** a statement reads a table for which the listener defines a predicate
- **THEN** that predicate is the one considered for injection

#### Scenario: Table has no rule

- **WHEN** a statement reads a table for which the listener defines no predicate
- **THEN** the statement is forwarded unchanged
- **AND** the absence of a rule is not reported as a skipped rewrite

#### Scenario: Two listeners filter the same table differently

- **WHEN** two listeners define different predicates for the same table
- **THEN** each connection is filtered using the predicate of the listener that accepted it

### Requirement: Predicates are validated when configuration is loaded

The proxy SHALL validate every configured predicate at startup as exactly one complete boolean expression, and SHALL refuse to start when one is not. A predicate SHALL NOT contain a statement terminator, a trailing comment, an unbalanced quote or parenthesis, or a parameter placeholder.

#### Scenario: Valid predicate

- **WHEN** a predicate is a complete boolean expression such as a comparison of a column to a literal
- **THEN** the proxy starts normally

#### Scenario: Predicate containing a statement terminator

- **WHEN** a configured predicate contains a statement terminator or trailing comment, which would let it alter the statement it is spliced into
- **THEN** the proxy fails to start and names the listener, the table, and the reason
- **AND** does not begin accepting client connections

#### Scenario: Structurally incomplete predicate

- **WHEN** a configured predicate has unbalanced quotes or parentheses, or is a fragment rather than a whole expression
- **THEN** the proxy fails to start and names the listener, the table, and the reason

#### Scenario: Predicate containing a placeholder

- **WHEN** a configured predicate contains a parameter placeholder
- **THEN** the proxy fails to start, because injecting one would change the parameter count a prepared statement reported to its client

### Requirement: Eligible read-only statements have the predicate injected

The proxy SHALL inject the configured predicate into a statement when all of the following hold: the command carries SQL, the statement is a `SELECT`, it reads exactly one table, that table has a rule, and the statement was tokenized successfully. Injection SHALL apply equally to statements submitted directly and to statements being prepared, so that a client using prepared statements is filtered identically to one that is not.

#### Scenario: Select with no existing WHERE clause

- **WHEN** an eligible statement has no `WHERE` clause
- **THEN** the proxy inserts a `WHERE` clause containing the predicate, enclosed in parentheses

#### Scenario: Select with an existing WHERE clause

- **WHEN** an eligible statement already has a `WHERE` clause
- **THEN** the forwarded statement restricts rows to those matching both the original condition and the predicate

#### Scenario: Existing condition combines alternatives

- **WHEN** an eligible statement's existing `WHERE` clause combines conditions with `OR`
- **THEN** the original condition and the injected predicate are each parenthesized, so that the result set is narrowed rather than widened

#### Scenario: Statement being prepared

- **WHEN** an eligible statement is submitted for preparation rather than direct execution
- **THEN** the predicate is injected before the backend prepares it
- **AND** executing the prepared statement returns only rows matching the predicate
- **AND** the number of parameters and columns the client was told about is unchanged

#### Scenario: Filtered rows are absent from the result

- **WHEN** a table contains rows that do not match the listener's predicate
- **THEN** a client reading that table through the listener does not receive them

### Requirement: The predicate is inserted at the correct position

The proxy SHALL insert the predicate at the end of the statement's table reference, before any clause that must follow `WHERE`. Text outside the inserted region SHALL be forwarded exactly as the client submitted it, including comments, hints, casing, and whitespace.

#### Scenario: Statement with trailing clauses

- **WHEN** an eligible statement has clauses that must follow `WHERE`, such as grouping, ordering, or a row limit
- **THEN** the predicate is inserted before those clauses
- **AND** the statement remains valid and those clauses still apply

#### Scenario: Statement with a locking clause

- **WHEN** an eligible statement ends with a locking clause
- **THEN** the predicate is inserted before it

#### Scenario: Clause keyword appears inside a string or comment

- **WHEN** an eligible statement contains text resembling a clause keyword inside a string literal, a quoted identifier, or a comment
- **THEN** that text is not mistaken for a clause boundary
- **AND** the predicate is inserted at the real one

#### Scenario: Surrounding text is preserved

- **WHEN** an eligible statement contains comments, optimizer hints, or unusual whitespace
- **THEN** the forwarded statement retains them unchanged

#### Scenario: Table referenced by an alias

- **WHEN** an eligible statement gives its table an alias
- **THEN** the injected predicate still applies, because a single-table statement has no ambiguous column references

### Requirement: Statements that cannot be rewritten are forwarded unchanged

The proxy SHALL forward a statement unchanged when it cannot rewrite it with confidence, and SHALL NOT return an error in place of executing it. This applies to statements that are not `SELECT`, that reference more than one table or reference tables indirectly, that carry more than one statement in a single command, that fail to tokenize, whose clause boundary cannot be located, and whose rewrite would change the number of packets the command occupies. Each occurrence SHALL be counted and attributed to a reason.

#### Scenario: Statement joining several tables

- **WHEN** a statement reads more than one table and at least one has a rule
- **THEN** the statement is forwarded unchanged and the skip is recorded with a reason
- **AND** the client receives rows the predicate would have excluded

#### Scenario: Statement containing a subquery, union, or common table expression

- **WHEN** a statement's structure places its tables beyond a single table reference
- **THEN** the statement is forwarded unchanged and the skip is recorded with a reason

#### Scenario: Statement that cannot be tokenized

- **WHEN** a statement cannot be tokenized
- **THEN** it is forwarded unchanged and the skip is recorded with a reason
- **AND** the connection remains usable

#### Scenario: Several statements in one command

- **WHEN** a command carries more than one statement
- **THEN** it is forwarded unchanged and the skip is recorded with a reason

#### Scenario: Write statement

- **WHEN** a statement modifies data or schema
- **THEN** it is forwarded unchanged and no rule is applied

#### Scenario: Rewrite would change the packet count

- **WHEN** inserting the predicate would make the command occupy more packets than the client sent
- **THEN** the statement is forwarded unchanged and the skip is recorded with a reason
- **AND** the sequence identifiers the client expects are unaffected

#### Scenario: Command carries no SQL

- **WHEN** a command carries no statement text
- **THEN** it is forwarded unchanged and no skip is recorded
