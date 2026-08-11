> ## CLOSED — 2026-08-11, at 70/75
>
> **Milestone 2 is closed and this change is closed with it.** It was not
> archived: `openspec/specs/` stays empty and nothing was folded into canonical
> form.
>
> The five unchecked items in §10 are **closed-unaddressed, not a backlog.**
> They are left unticked deliberately — they were not done, and a ticked box
> recording nothing is the exact false-signal shape this change kept finding.
> Milestones are independent; milestone 3 starts from its own scope and does not
> inherit these.
>
> Why it stopped here is the milestone's finding rather than a failure: §10 did
> not exist when the change was written. It was opened by the first real client
> and grew to eleven items while the first nine sections were closing, because
> fail-closed rewriting against a foreign SQL dialect has an open-ended
> compatibility surface. See `milestones/milestone-2.md` §5.

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
- [x] 7.8 Run `cargo mutants` over the rewriter and rejection logic; a surviving mutant in the fail-closed path is a defect to fix, not a metric to report. **Result against commit `598ccaf`, scoped to `rewrite.rs`/`analyze.rs`/`policy.rs`/`error.rs`: 155 caught, 11 missed, 7 timeout, 182 unviable — and every one of the 11 has a written reason.** Five real defects were found and fixed, not merely counted: the fail-closed re-check was a no-op in every test that went through `rewrite_statement`; five early exits in `is_guard` had no test at all; `is_restricted` was leaned on by a dozen assertions that all asserted the positive answer; `#` line comments and `/` in code position were unexercised in the scanner; and the integer arm of the emitter — a documented feature — had zero coverage because the fixture could not express it. The 7 timeouts are detections, not survivors: outside the `#` arm the scanner's state does not change, so a cursor that fails to advance re-reads the same byte forever. The 11 survivors are 1 stated-equivalent scanner mutant, 8 guards proven unreachable through `MySqlDialect` by grepping where each field is *assigned* in sqlparser 0.62, and 2 in `policy.rs` (a body that literally *is* `Self::default()`, and a serde `expecting` whose visitor handles every type TOML can produce). **One mutant was recorded as UNRESOLVED rather than equivalent when two people could not classify it by hand-tracing; it turned out to be both a fail-open bug and an infinite loop.** That is the argument for the standard: state why, or say unresolved

## 8. End-to-end verification

- [x] 8.1 Stand up an integration harness with a real Doris instance and a seeded policy table — `apache/doris:doris-all-in-one-2.1.0` (native arm64), FE on 9030, `sales.orders` seeded across APAC/EMEA/AMER, user `analyst` with a password. **This was reported as blocked for most of the change; it was never checked. Docker was available all along**
- [x] 8.2 End-to-end test: a restricted user's `SELECT` returns only permitted rows through the proxy — **verified**: `analyst` sees ids 1,2,4 (APAC/EMEA) and not 3,5 (AMER). **This also settles the project's largest unverified claim: a real Doris accepts the relayed auth scramble.** Every D1 test until now ran against a fake frontend that does no hashing, which proved relay-verbatim but could not prove the handshake works
- [x] 8.3 End-to-end test: the same query issued directly against the FE returns unfiltered rows, demonstrating that the network boundary — not the proxy — is what makes the control complete — **verified**: the same user, same credentials, straight to port 9030 returns all five rows including AMER. The README's first operator precondition is now demonstrated rather than argued
- [x] 8.4 End-to-end test: a known Doris-specific syntax that `sqlparser` cannot parse is rejected for a policy-bearing user — **verified** with `GROUP BY … WITH ROLLUP`: proxy returns `1235 (42000) proxy refused statement: statement could not be parsed`. **And the finding is smaller than recorded: Doris cannot parse it either** (`ParseException … no viable alternative at input 'WITH ROLLUP'`), so refusing costs nothing. It had been written into the README as this project's largest concrete compatibility cost
- [x] 8.5 Record every parser and dialect surprise found during integration, with provenance. **Verified against Doris 2.1.0: (a) Doris resolves `WITH orders AS (...) SELECT * FROM orders` to the CTE exactly as MySQL does — the sharpest known parser-differential instance is CLOSED, the assumption held; (b) Doris cannot parse `WITH ROLLUP` either, so that refusal costs nothing; (c) `lower_case_table_names=0` on this build, so the non-ASCII identifier refusal is conservative but safe. NEW AND SERIOUS: the proxy refuses `SET NAMES`, `SET autocommit`, `SET SESSION ...` and `SHOW TABLES` — see the blocker note at the foot of this file.** **Already found without a backend, pinned by tests:** `MySqlDialect` cannot parse `GROUP BY a WITH ROLLUP` at all — ordinary MySQL, common in reporting, so every policy-bearing user issuing one is refused; and `GROUP BY ALL` parses here but is rejected by MySQL/Doris as a reserved word, so it is refused deliberately. **Check first:** does Doris resolve `WITH orders AS (SELECT 1 AS region) SELECT * FROM orders` to the CTE, as MySQL does? The proxy forwards that verbatim and unconstrained on the strength of MySQL scoping — it is the only place it asserts an identifier is *not* a table
- [x] 8.6 Verified against a real backend under **both** default `sql_mode` and `NO_BACKSLASH_ESCAPES`: a permitted value `O'Brien` renders as `'O''Brien'` and matches exactly the intended row under both settings, with `Jones` excluded. The mode-dependence that drove the load-time rejection of backslash and `''` values is confirmed resolved for the values that *do* load. Original text: The subject is **`sqlparser`'s `Value` renderer**, which produces the emitted SQL — not `policy::PermittedValue::to_sql_literal()`, which is diagnostics-only. Measured locally: sqlparser doubles an isolated `'` but never escapes backslashes and leaves an already-doubled `''` alone, so `a\b` and `a''b` reach the backend as *different values* (a widening) and `a\` corrupts the statement. Those inputs are now rejected at load time, so this task verifies that the values which *do* load — plain text, integers, and values containing a single quote such as `O'Brien` — match exactly the rows intended and no others

## 9. Documentation and follow-up

- [x] 9.1 Write ADRs under `docs/adr/` for the non-spec-shaped decisions of design D7: task topology, buffer ownership, cancellation, error-enum shape, protocol crate choice
- [x] 9.2 Document the operator preconditions: FE reachable only from the proxy, and no view-creation rights on schemas holding policy tables
- [x] 9.3 Document the open bypass vectors — direct FE access, views, `information_schema`, parser differential, collation-dependent matching — where operators will read them, not only in `design.md`
- [x] 9.4 Replace the Commands section of the repository `CLAUDE.md` with the real crate layout and build steps

## 10. Found by integration, not yet addressed

- [x] 10.1 **BLOCKER for real clients: the proxy refuses ordinary session-management statements.** RESOLVED by analysing `SET`/`SHOW` through the same allowlist walk rather than special-casing them, per the new spec requirement. **Verified end-to-end against a real Doris with a genuine `mysql` client**: `SET NAMES utf8mb4; SET autocommit=1; SET SESSION sql_mode=''; SHOW TABLES FROM sales;` all forward, the client reaches a usable state, and the filter still holds in the same session (APAC 2, EMEA 1, no AMER). `SET @x = (SELECT total FROM sales.orders …)` is refused with a dedicated `RestrictedTableIntoSessionState` — the hazard that made blanket-forwarding `SET` a live bypass. Verified against Doris 2.1.0 with a real MySQL client: `SET NAMES utf8mb4`, `SET autocommit=1`, `SET SESSION sql_mode=''` and `SHOW TABLES FROM sales` all return `1235 (42000) statement uses a construct this proxy cannot analyse`. Nearly every MySQL connector — JDBC, ODBC, Python, Go — issues `SET NAMES` immediately on connect, so **the MVP cannot serve a normal client at all**; it worked here only because `mysql -e` sends none of them for a one-shot query. This is the fail-closed rule of D6 behaving exactly as specified, applied to statements that cannot reference a table and so cannot leak a row. Resolving it means deciding which statement kinds are analysable-by-construction rather than widening the allowlist ad hoc — a spec change, not a patch
- [x] 10.2 Decide whether `SHOW` statements should pass through — **yes, forwarded, and the decision was settled by measurement rather than argument.** `SELECT ... FROM information_schema.columns` through the proxy already returns `id`/`region`/`total` for `sales.orders`, because `information_schema` carries no policy. Refusing `SHOW COLUMNS` therefore withholds nothing while breaking clients that introspect. The spec now distinguishes **reads** (a policy table's rows into session state — refuse) from **names** (a policy table to report metadata — forward), which is a cleaner line than the one originally written
- [x] 10.3 **Same blocker class, second instance: transaction control.** Verified through the proxy against a real Doris as a restricted user: `BEGIN`, `COMMIT`, `ROLLBACK` and `START TRANSACTION` are all refused, and `BEGIN; SELECT ...; COMMIT;` fails. Any connector that manages transactions, or any client that turns autocommit off, is broken. **Worse than the SET/SHOW case in one respect:** `SET autocommit=0` now forwards while `COMMIT` refuses, so a client can enter a state it cannot leave — strictly worse than refusing both. These statements name no table and cannot return or write a row, so they fall under the requirement added for SET/SHOW, now generalised. Classification is in `analyze.rs`
- [x] 10.4 **Decide how the kind allowlist is meant to grow.** 10.1 and 10.3 are the same defect found twice, and the enumeration is open-ended — `CREATE TABLE`, `TRUNCATE`, `ALTER TABLE` and `COMMIT` all currently refuse via the D6 branch on `has_any_policy` alone, regardless of which tables they touch. Either state a principle that decides membership without a case-by-case ruling, or accept that each addition is a spec change and say so, so the next instance is expected rather than surprising
- [x] 10.5 **LIVE LEAK — metadata statements read policy tables unconstrained.** Confirmed against Doris 2.1.0 as a restricted user (5 rows exist, 3 permitted): `SHOW VARIABLES WHERE (SELECT COUNT(*) FROM sales.orders) = 5` **returns rows**, and `= 3` returns none — the subquery counted the unfiltered table. This generalises to a bit-at-a-time oracle over any predicate: `... WHERE (SELECT COUNT(*) FROM sales.orders WHERE total > 400) = 1`. **The strongest bypass found in this project, and introduced by the lead's own ruling an hour earlier.** Cause: `rewrite.rs`'s `StatementKind::Metadata` arm forwards unconditionally, and is reached only when the statement already touches a policy table — it cannot see *why* the table was enumerated. The spec's "names versus reads" distinction is real but unimplementable against a flat table list. Fix: `analyze.rs` must record provenance per reference — **name position** (the statement names the table to describe it) versus **expression position** (it evaluates SQL that reads its rows) — and the metadata arm forwards only if every policy-bearing reference is name-position. **Do not revert to a blanket `SHOW` refusal**: `information_schema` remains open so it would withhold nothing, reintroduce the client blocker, and leave the oracle reachable through other metadata forms
- [x] 10.6 Add a regression test asserting the oracle is closed — a metadata statement whose subquery counts a policy table must not observe rows the user cannot see. **This class had no test because every metadata test named a table rather than reading one**, which is the "a test that cannot fail in the direction the bug lies" shape, found for the fifth time
- [ ] 10.7 **`mysqldump` cannot run through the proxy at all — and this task was recorded at the wrong depth.** It was written as a `--single-transaction` problem, on the basis that `START TRANSACTION WITH CONSISTENT SNAPSHOT` fails to parse. That is true and **unreachable**: packet capture shows three of `mysqldump`'s first five statements are executable comments (`/*!40100 SET @@SQL_MODE='' */` and friends), so it dies on statement one, and plain `mysqldump` without `--single-transaction` is equally blocked. **A gap recorded at the wrong depth reads as understood and is not.** Resolving 10.6's executable-comment rule to refuse only when *rewriting* unblocks the first three; the parse gap below then becomes reachable and remains. Original text: `START TRANSACTION WITH CONSISTENT SNAPSHOT`, `COMMIT RELEASE` and `ROLLBACK RELEASE` all fail to **parse** under `MySqlDialect`, so a policy-bearing user cannot take a consistent dump. Unlike 10.1/10.3 this fails at parsing rather than classification, so no allowlist work fixes it — it is a `sqlparser` gap of exactly the kind D6 was written for. First instance where the compatibility cost is a named workflow rather than a syntax form
- [ ] 10.8 **Client-corpus test — the fix that actually prevents the next one.** Assert that the statements real clients and tools send are analysable: connector handshake sequences (JDBC, Python, Go, ODBC), the transaction lifecycle, `mysqldump --single-transaction`, BI introspection. Turns "found by a real client three hours later" into "found by `cargo test` in 200ms". **Ranked above 10.4's principle deliberately**, because the principle would not have prevented any of 10.1, 10.3 or 10.7 — all three were found by a client, and a better default would only have changed which ones
- [ ] 10.9 **10.4 resolution: change the default, not the list.** One branch serves three different failures — parse failure (refuse, correct: no AST, question unanswerable), classification failure (**the bug factory**: AST exists, question answerable, we decline to look), and unanalysable shape (refuse, correct). An unclassified kind should walk and refuse only if a policy-bearing reference turns up, rather than refusing without looking. Security property untouched: the walk stays an allowlist and an unknown shape *inside* a statement still refuses. Disposition when a reference IS policy-bearing is not derivable from the AST — `SELECT` constrains, `SET` refuses, metadata forwards — so each such kind stays a spec decision; only the default is derivable. Do not make the walk total over ~150 variants speculatively; let the corpus drive it
- [ ] 10.10 Log refusals with the statement-kind variant name, so compatibility loss is visible in a deployment rather than invisible until someone complains
- [ ] 10.11 DDL disposition: refuse if it names a policy table, forward otherwise, as with writes. **Record the consequence in the README rather than leaving it to be discovered:** a policy-bearing user can `CREATE TABLE`. That is correct — the proxy adds a row predicate and is not an authorization system, Doris grants decide — but it should be stated

### 10.5 closing note — the fix, and what the diagnosis turned out to be

Closed by recording provenance per reference and drawing the line at
**enumeration and lookup rather than disposition**: `decide_site` treats a
`Named` reference as unrestricted, so a naming statement never looks
policy-bearing and forwards by the ordinary path, and the `Metadata` arm is
reached *only* by a metadata statement that genuinely reads. Verified end to end
against Doris 2.1.0 on a binary built after the change: all three oracle forms
refuse, `SHOW COLUMNS` and `SHOW CREATE TABLE` forward, connector statements
work, the filter holds.

**The lead's diagnosis was wrong in an instructive way.** The spec sentence —
names forward, reads refuse — was judged "correct but unimplementable against a
flat table list". It was implementable; the lead was looking one layer too late.
The real cause, found by the analyser's owner: *"names are forwarded" had been
implemented by not enumerating the name at all* — a disposition decision taken
inside the analyser, breaking invariant 6 to express a policy the analyser has
no business holding. Losing provenance was the symptom; the decision sitting in
the wrong layer was the cause. **An analyser that quietly declines to report
something is indistinguishable from one that found nothing, and no care
downstream can recover the difference.**

The interim stopgap was correct only by an invisible invariant — metadata
statements happened to enumerate no names, so anything reaching that arm had to
be a read. Nothing expressed that, and re-adding name enumeration for any reason
would have silently reopened the hole.

Two design decisions did the work, both by someone guarding against someone
else: `visit_named` with **no default implementation**, so a silently-do-nothing
default could not recur in two other visitors — it became a compile error
forcing an explicit answer in each, and then caught its own author's test
visitor one layer further out. And the regression pair: neither test is
meaningful alone (one is satisfied by refusing everything, the other by
forwarding everything); only together do they pin the line.

