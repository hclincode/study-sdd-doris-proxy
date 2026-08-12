## Purpose

Defines what the proxy records about the SQL traffic passing through it, the format and destination of those records, and how the logging path behaves when it cannot keep up so that logging never degrades the availability of the proxied database.

## Requirements

### Requirement: Every command produces one log record

The proxy SHALL emit exactly one log record for each command it forwards on behalf of a client, written after the command's response is complete so that outcome and latency can be included. Commands that carry no SQL text SHALL still produce a record identifying the command type.

#### Scenario: Statement completes successfully

- **WHEN** a client executes a statement and the backend returns a response
- **THEN** the proxy emits one log record describing that statement and its outcome

#### Scenario: Statement fails

- **WHEN** the backend responds to a statement with an error
- **THEN** the proxy emits one log record recording the error code and SQL state

#### Scenario: Non-SQL command

- **WHEN** a client sends a command that carries no statement text, such as a ping or a database switch
- **THEN** the proxy emits a log record identifying the command type with no statement text

#### Scenario: Connection closes mid-statement

- **WHEN** a connection is torn down before a response completes
- **THEN** the proxy emits a log record marking the command as terminated without a response

### Requirement: Log records carry statement, outcome, timing, and connection identity

Each log record SHALL contain the time the command was received, the listener that accepted the connection, a proxy-assigned connection identifier stable for the life of the connection, the client's network address, the username and default database negotiated during the connection phase, the command type, the wall-clock duration from forwarding the command to completing its response, and the outcome of the command. For commands that carry SQL, the record SHALL contain the complete statement text as submitted by the client, unmodified and untruncated.

#### Scenario: Record identifies the connection

- **WHEN** a log record is emitted for any command
- **THEN** it contains the listener, proxy connection identifier, client address, username, and default database

#### Scenario: Record contains the full statement

- **WHEN** a client submits a statement containing literal values
- **THEN** the record contains the statement text exactly as submitted, including those literals

#### Scenario: Record reports rows for a result set

- **WHEN** a statement returns a result set
- **THEN** the record reports the number of rows returned

#### Scenario: Record reports affected rows

- **WHEN** a statement returns an OK response
- **THEN** the record reports the number of affected rows reported by the backend

#### Scenario: Two commands on one connection are distinguishable

- **WHEN** a client executes several commands on the same connection
- **THEN** each record shares the same connection identifier
- **AND** each record has its own timing and outcome

### Requirement: Statements are recorded with a normalized digest

For each command carrying SQL, the proxy SHALL derive and record a normalized digest of the statement together with a stable hash of that digest. Normalization SHALL replace literal values with a placeholder, collapse lists of literals to a single placeholder, remove comments, and normalize whitespace, such that statements differing only in their literal values share a digest.

#### Scenario: Statements differing only in literals

- **WHEN** two statements are identical apart from their literal values
- **THEN** both records carry the same digest and the same digest hash

#### Scenario: Statements differing in structure

- **WHEN** two statements differ in which columns or tables they reference
- **THEN** their digests differ

#### Scenario: Literal list is collapsed

- **WHEN** a statement contains a list of literal values of varying length
- **THEN** the digest represents the whole list as a single placeholder, so lists of different lengths share a digest

#### Scenario: Statement the digester cannot normalize

- **WHEN** normalization fails for a statement
- **THEN** the record still contains the full statement text and outcome
- **AND** the record marks the digest as unavailable rather than omitting the record

### Requirement: Records are written as JSON Lines to a configured file

The proxy SHALL write log records to the file path given in its configuration, one JSON object per line. The proxy SHALL create the file if it does not exist, with permissions restricting access to the owning user, and SHALL append to it if it does exist.

#### Scenario: Log file does not exist at startup

- **WHEN** the proxy starts and the configured log file is absent
- **THEN** the proxy creates it with owner-only read and write permissions

#### Scenario: Log file already exists

- **WHEN** the proxy starts and the configured log file already contains records
- **THEN** the proxy appends new records without truncating existing content

#### Scenario: Log file cannot be opened at startup

- **WHEN** the configured log file cannot be created or opened
- **THEN** the proxy fails to start and reports the reason
- **AND** does not begin accepting client connections

#### Scenario: Record is machine readable

- **WHEN** a record is written
- **THEN** it occupies exactly one line and parses as a single JSON object

### Requirement: Logging is lossy under pressure and never blocks query traffic

The logging path SHALL be bounded. When the proxy cannot write records as fast as they are produced, it SHALL discard records rather than delay, block, or fail the commands being proxied. The proxy SHALL count discarded records and SHALL make that count visible in the log file, so that a reader can tell the record stream is incomplete.

#### Scenario: Log writing falls behind

- **WHEN** records are produced faster than they can be written
- **THEN** excess records are discarded
- **AND** client commands continue to be forwarded and answered at unchanged latency

#### Scenario: Discards are reported

- **WHEN** records have been discarded
- **THEN** the proxy writes a record reporting the number discarded

#### Scenario: Write error occurs

- **WHEN** writing to the log file fails, including because the filesystem is full
- **THEN** the proxy continues proxying client connections
- **AND** subsequent records are discarded and counted until writing succeeds again

#### Scenario: Proxy shuts down

- **WHEN** the proxy shuts down cleanly
- **THEN** records already accepted into the logging path are written before exit

### Requirement: The log file can be rotated by an external tool

The proxy SHALL reopen its configured log file path on receipt of a hangup signal, so that an external rotation tool can rename the current file and have the proxy resume writing to a new one at the same path.

#### Scenario: External rotation

- **WHEN** the log file is renamed and the proxy receives a hangup signal
- **THEN** the proxy reopens the configured path
- **AND** subsequent records are written to the newly created file

#### Scenario: Reopen fails

- **WHEN** the proxy receives a hangup signal but cannot reopen the configured path
- **THEN** the proxy continues proxying client connections
- **AND** discards and counts records until the path can be reopened

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
