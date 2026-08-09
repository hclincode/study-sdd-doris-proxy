## 1. Toolchain and scaffold

- [x] 1.1 Install a Rust toolchain — `cargo` and `rustc` are absent from PATH, which blocks every task below
- [x] 1.2 Create the crate in the workspace directory with `Cargo.toml` and `src/`, and confirm `cargo build`, `cargo fmt`, `cargo clippy` and `cargo test` all run clean on the empty crate
- [x] 1.3 Add dependencies: async runtime, server-side MySQL protocol crate exposing raw handshake salt and capability flags (design D1/D4), backend MySQL client, and `sqlparser` with `MySqlDialect`
- [x] 1.4 Record the chosen protocol crate and the reason in a note for the later ADR, per design D7

## 2. Policy configuration

- [x] 2.1 Define the policy model: qualified table, column, permitted value set, keyed by username
- [x] 2.2 Implement config loading from file at startup, fixed for process lifetime
- [x] 2.3 Implement validation rejecting malformed files, missing user/table/column, and empty permitted sets
- [x] 2.4 Make invalid config abort startup before any listener binds, with a diagnostic naming the offending policy
- [x] 2.5 Implement policy lookup for `(user, qualified table)`, returning "no policy" distinctly from "no permitted values"
- [x] 2.6 Implement unqualified-name resolution against the session's current database
- [x] 2.7 Tests: valid config loads; each invalid form aborts startup; `sales.orders` policy does not match `staging.orders`; unqualified `orders` matches only under the right current database; aliased reference still matches

## 3. Session establishment and passthrough authentication

- [x] 3.1 Accept client connections and open exactly one backend connection per session
- [x] 3.2 Implement design D1: read Doris's handshake first, relay its salt to the client, relay the client's auth response verbatim to Doris
- [x] 3.3 Advertise capabilities with `CLIENT_MULTI_STATEMENTS` and `CLIENT_MULTI_RESULTS` cleared (design D4)
- [x] 3.4 Record the backend-authenticated username as the session identity, and reject statements issued before authentication completes
- [x] 3.5 Refuse the client session when the backend connection cannot be established
- [x] 3.6 Close the backend connection when the client disconnects or the connection is lost
- [x] 3.7 Track the session's current database and keep it updated as it changes
- [x] 3.8 Tests: two concurrent sessions hold two distinct backend connections; credentials rejected by Doris yield no usable session; a client claiming `admin` but authenticated as `analyst` gets `analyst`'s policy; backend-down refuses the session; client disconnect releases the backend connection

## 4. Statement analysis

- [x] 4.1 Parse each incoming statement with `sqlparser` / `MySqlDialect`
- [x] 4.2 Implement the allowlist AST walk of design D3 — enumerate every table reference, with no default-continue branch
- [x] 4.3 Cover reference sites in the walk: `FROM`, every join operand, derived tables, subqueries in any clause, CTE bodies, and every set-operation branch
- [x] 4.4 Return a distinct "cannot analyse" outcome for any construct not on the allowlist, carrying which construct caused it
- [x] 4.5 Tests: the walk finds every table reference in nested, joined, CTE and UNION statements; a synthetic unlisted node kind produces "cannot analyse" rather than an empty reference list
- [x] 4.6 Property test: for generated statements, every table name appearing in the statement text is either enumerated by the walk or the statement is refused — never silently missed

## 5. Predicate injection

- [x] 5.1 Implement derived-table wrapping (design D2), aliasing to the reference's effective name — the user's alias if present, else the bare table name
- [x] 5.2 Apply wrapping at every enumerated policy-bearing reference site, leaving references without a policy untouched
- [x] 5.3 Verify no policy-bearing reference remains unwrapped before forwarding, and refuse the statement if any does
- [x] 5.4 Emit permitted values as literals, never as parameter placeholders, preserving placeholder count and order
- [x] 5.5 Tests, one per position from the design D2 table: simple `FROM`; user `WHERE` containing `OR`; inner join operand; outer join operand; subquery; CTE body; set-operation branch; correlated `EXISTS`/`IN`; self-join under two aliases
- [x] 5.6 Test: a table with no policy for the user is forwarded unwrapped
- [x] 5.7 Test: `SELECT id, region, total` returns exactly those columns in that order after rewriting
- [x] 5.8 Test: when every row's value is within the permitted set, the rewritten result equals the unrewritten result

## 6. Rejection paths — one task per unsupported shape

- [x] 6.1 Reject unparseable statements when the user has at least one configured policy
- [x] 6.2 Forward unparseable statements unmodified when the user has no configured policy at all
- [x] 6.3 Reject parseable statements whose shape the allowlist walk cannot analyse
- [x] 6.4 Reject `INSERT` referencing a policy-bearing table, including `INSERT ... SELECT`
- [x] 6.5 Reject `UPDATE` referencing a policy-bearing table
- [x] 6.6 Reject `DELETE` referencing a policy-bearing table
- [x] 6.7 Reject `REPLACE` referencing a policy-bearing table
- [x] 6.8 Forward write statements against tables with no policy for the user
- [x] 6.9 Refuse `COM_STMT_PREPARE` with an error (design D4)
- [x] 6.10 Reject any request that parses into more than one statement, at the SQL level. Capability negotiation does **not** prevent multi-statement requests — clearing the advertised bit is advisory and `opensrv-mysql` never checks whether the client honoured it, so the advertised flags are defence in depth only (see design D4's correction). A trailing semicolon must not be mistaken for a second statement
- [x] 6.11 Implement the error packet of design D5: SQLSTATE `42000`, distinguishing unsupported-shape from policy-denial, disclosing no policy contents
- [x] 6.12 Tests: one per rejection above, each asserting that nothing reached the backend — not merely that the client saw an error
- [x] 6.13 Test: rejection messages contain no table name, username or permitted value from another user's policy

## 7. Bypass resistance

- [x] 7.1 Test: `WHERE region = 'AMER' OR 1=1` returns no row outside the permitted values
- [x] 7.2 Test: `SELECT COUNT(*) FROM (SELECT * FROM sales.orders) t` counts only permitted rows
- [x] 7.3 Test: an aggregate with no row projection is computed only over permitted rows
- [x] 7.4 Test: `LEFT JOIN` with the policy table as operand discloses no restricted row, including through NULL-extended output
- [x] 7.5 Test: a correlated `EXISTS` subquery cannot be used to infer the existence of a restricted row
- [x] 7.6 Test: comment-smuggled second statement never reaches the backend
- [x] 7.7 Property test: across generated `SELECT` statements against a policy table, every statement is either refused or returns only permitted rows — no third outcome
- [ ] 7.8 Run `cargo mutants` over the rewriter and rejection logic; a surviving mutant in the fail-closed path is a defect to fix, not a metric to report

## 8. End-to-end verification

- [ ] 8.1 Stand up an integration harness with a real Doris instance and a seeded policy table
- [ ] 8.2 End-to-end test: a restricted user's `SELECT` returns only permitted rows through the proxy
- [ ] 8.3 End-to-end test: the same query issued directly against the FE returns unfiltered rows, demonstrating that the network boundary — not the proxy — is what makes the control complete
- [ ] 8.4 End-to-end test: a known Doris-specific syntax that `sqlparser` cannot parse is rejected for a policy-bearing user
- [ ] 8.5 Record every parser and dialect surprise found during integration, with provenance
- [ ] 8.6 Verify literal rendering against a real backend, under **both** default `sql_mode` and `NO_BACKSLASH_ESCAPES`. The subject is **`sqlparser`'s `Value` renderer**, which produces the emitted SQL — not `policy::PermittedValue::to_sql_literal()`, which is diagnostics-only. Measured locally: sqlparser doubles an isolated `'` but never escapes backslashes and leaves an already-doubled `''` alone, so `a\b` and `a''b` reach the backend as *different values* (a widening) and `a\` corrupts the statement. Those inputs are now rejected at load time, so this task verifies that the values which *do* load — plain text, integers, and values containing a single quote such as `O'Brien` — match exactly the rows intended and no others

## 9. Documentation and follow-up

- [x] 9.1 Write ADRs under `docs/adr/` for the non-spec-shaped decisions of design D7: task topology, buffer ownership, cancellation, error-enum shape, protocol crate choice
- [x] 9.2 Document the operator preconditions: FE reachable only from the proxy, and no view-creation rights on schemas holding policy tables
- [x] 9.3 Document the open bypass vectors — direct FE access, views, `information_schema`, parser differential, collation-dependent matching — where operators will read them, not only in `design.md`
- [x] 9.4 Replace the Commands section of the repository `CLAUDE.md` with the real crate layout and build steps
