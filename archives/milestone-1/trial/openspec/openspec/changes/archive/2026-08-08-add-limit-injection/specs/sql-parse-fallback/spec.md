## Purpose

Defines what the proxy does with statements it cannot parse. The `sqlparser` crate ships a MySQL dialect but no Doris dialect, so a portion of valid Doris SQL will fail to parse; this capability fixes the policy for that gap so it is a stated trade-off rather than an accident of the parser's coverage.

## ADDED Requirements

### Requirement: Forward unparseable statements unchanged

When the parser rejects a statement, the proxy SHALL forward the original bytes to Doris unchanged and SHALL NOT return an error to the client.

A statement that escapes the cap is a resource risk, not a data-exposure risk, and Doris' own query timeout remains as a backstop. Rejecting it would instead break valid Doris SQL that the parser simply does not model, which is a correctness failure the operator did not ask for.

#### Scenario: Doris-specific syntax passes through

- **WHEN** the client sends a statement using Doris syntax the MySQL dialect does not accept
- **THEN** the proxy forwards the original bytes to Doris
- **AND** the client receives whatever Doris returns, success or error

#### Scenario: Syntactically invalid SQL is Doris' problem

- **WHEN** the client sends `SELECT FROM WHERE`
- **THEN** the proxy forwards it unchanged
- **AND** the resulting syntax error originates from Doris, not from the proxy

### Requirement: Parse failures are visible

The proxy SHALL count every parse failure and SHALL emit a record containing the statement text and the parser's error, so the coverage gap can be measured and the dialect handling improved from evidence.

#### Scenario: Failure is counted and logged

- **WHEN** the parser rejects a statement
- **THEN** the parse-failure counter increments
- **AND** a record carrying the statement text and parser error is emitted

#### Scenario: Operator can size the gap

- **WHEN** an operator inspects the counters after a day of traffic
- **THEN** the proportion of statements forwarded uncapped due to parse failure is derivable from them

### Requirement: Fail-closed mode is available but off by default

The proxy SHALL offer a configuration option that rejects unparseable statements with an error instead of forwarding them. This option SHALL default to off.

An operator who values a hard guarantee that nothing reaches Doris uncapped can turn it on; the default favours not breaking valid Doris SQL.

#### Scenario: Fail-closed rejects what it cannot parse

- **WHEN** fail-closed mode is enabled and the parser rejects a statement
- **THEN** the proxy returns an error to the client and forwards nothing to Doris

#### Scenario: Default remains permissive

- **WHEN** no fallback option is configured
- **THEN** unparseable statements are forwarded unchanged
