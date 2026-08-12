## ADDED Requirements

### Requirement: Records describe what the proxy did to the statement

For every command carrying SQL, the record SHALL state whether the statement was rewritten before being forwarded. When it was, the record SHALL contain the statement as forwarded to the backend and identify the rule applied. When it was not, and a rule existed that might have applied, the record SHALL identify why the rewrite was skipped. The `statement` field SHALL continue to carry the statement exactly as the client submitted it, so a reader can always see both what was asked and what was run.

#### Scenario: Statement was rewritten

- **WHEN** the proxy injects a predicate into a statement
- **THEN** the record marks the statement as rewritten
- **AND** contains the statement as forwarded to the backend
- **AND** identifies the table whose rule was applied
- **AND** the `statement` field still holds the client's original text

#### Scenario: Rewrite was skipped

- **WHEN** a statement reads a table with a rule but could not be rewritten
- **THEN** the record marks the statement as not rewritten
- **AND** records a reason distinguishing why, such as multiple tables, an unsupported structure, several statements in one command, a tokenization failure, or a packet-count change

#### Scenario: No rule applied

- **WHEN** a statement reads no table with a rule, or carries no SQL at all
- **THEN** the record marks the statement as not rewritten
- **AND** records no skip reason, so that skip reasons count only rewrites that were wanted and did not happen

#### Scenario: Skipped rewrites are countable

- **WHEN** a reader groups records by skip reason
- **THEN** they can determine how often each unsupported construct appears in real traffic

#### Scenario: Rewriting does not change the digest

- **WHEN** a statement is rewritten
- **THEN** the digest describes the statement the client submitted, so that grouping is unaffected by whether a rule happened to apply
