## Context

See `proposal.md` — Why, for motivation. See `specs/` for the behaviour this design has to deliver.

The constraints that actually shape the approach:

- The proxy must know the **authenticated** username before it can choose a policy, but it terminates the MySQL protocol, so it cannot simply forward the client's handshake bytes onward.
- `sqlparser` is a third-party MySQL parser, not Doris's. It has no Doris dialect. Two consequences: some valid Doris SQL will not parse, and — more dangerous — some SQL may parse into an AST whose meaning differs from what Doris will execute.
- The rewrite is a security control. Every design choice below resolves toward "refuse" when correctness cannot be demonstrated.
- Nothing exists yet: no crate, no Cargo.toml, and no Rust toolchain installed on the development machine.

## Goals / Non-Goals

**Goals:**

- A single injection mechanism that is correct in *every* syntactic position a table reference can occupy, rather than a different rule per position.
- An enumeration strategy that is closed by construction: unknown AST shapes refuse rather than fall through.
- Authentication passthrough that never requires the proxy to hold or see a plaintext password.
- A written, explicit list of which bypass vectors this design closes and which it does not.

**Non-Goals:**

- Parser parity with Doris. Not achievable with an off-the-shelf parser; the design absorbs the gap by rejecting.
- Performance work. Wrapping relations in derived tables may cost Doris some optimisation opportunity; the MVP accepts that.
- Protecting against a client that can reach the Doris FE directly. Out of the proxy's reach by construction.

## Decisions

### D1: Connect to the backend first and relay Doris's own auth challenge

**Decision.** On accepting a client, the proxy immediately opens its backend connection, reads Doris's handshake, and presents **Doris's own salt** to the client as its own. The client's scrambled auth response is then relayed verbatim to Doris, and Doris's OK/ERR determines whether the session proceeds.

**Why.** Passthrough authentication with `mysql_native_password` is a challenge-response over a server-chosen salt. If the proxy generated its own salt, the client's response would be unusable against Doris, and the proxy would have to obtain the plaintext password to re-authenticate.

**Alternatives considered.**

| Option | Rejected because |
|---|---|
| Generate own salt, require `mysql_clear_password` | The proxy would hold every user's plaintext password, and it would only be safe under mandatory TLS. A much larger security surface than the feature justifies. |
| Proxy keeps its own credential store | Duplicates identity, and the proxy's view of who a user is could drift from Doris's. Contradicts `connection-routing`'s requirement that Doris is the sole authority. |

**Consequence.** The backend connection is established *before* the client authenticates, which is why `connection-routing` specifies that a session is refused when the backend is unreachable. It also means an unauthenticated client can cause a backend connection to be opened — a denial-of-service consideration noted under Risks.

### D2: Constrain by wrapping each policy-bearing relation in a derived table

**Decision.** Every reference to a policy-bearing table is replaced by a parenthesised derived table carrying the predicate, aliased to the reference's *effective name*:

```sql
-- before
FROM sales.orders
-- after
FROM (SELECT * FROM sales.orders WHERE region IN ('APAC','EMEA')) AS orders

-- before
FROM sales.orders AS o
-- after
FROM (SELECT * FROM sales.orders WHERE region IN ('APAC','EMEA')) AS o
```

The effective name is the user's alias when present, otherwise the bare table name — so existing qualified column references such as `o.total` or `orders.total` continue to resolve.

**Why this placement, and why it is correct in each position.** The predicate is applied to the relation *before* it participates in anything else, so the surrounding syntax cannot weaken it:

| Position | Why wrapping is correct |
|---|---|
| Simple `FROM` | Equivalent to `WHERE` injection, but with no dependence on the user's predicate structure. |
| User `WHERE` containing `OR` | **The vector that kills naive injection.** `WHERE a OR b AND policy` binds as `a OR (b AND policy)`, leaving `a` unconstrained. Wrapping never touches the user's `WHERE`, so operator precedence is irrelevant and no parenthesisation of the user's predicate is required. |
| Inner join operand | Filtering before the join cannot change which permitted rows match. |
| **Outer join operand** | Injecting into the enclosing `WHERE` would convert `LEFT JOIN` to inner-join semantics and drop preserved-side rows — a result-set change forbidden by invariant 4. Filtering inside the operand keeps the join type's semantics over the permitted rows. |
| Subquery / derived table | Each relation is wrapped where it appears, so nesting depth is irrelevant. |
| CTE body | The CTE's own `FROM` is rewritten; every use of the CTE then reads constrained rows. |
| Set-operation branch | Each branch's `FROM` is rewritten independently. |
| Correlated `EXISTS` / `IN` subquery | Wrapped like any other relation, so existence tests cannot probe restricted rows. |

The single mechanism is the point: one rule proven once, rather than a per-position rule set where the position nobody thought about is the leak.

**Alternative considered — append `AND <policy>` to each `SELECT`'s `WHERE`.** Simpler to implement and produces more idiomatic SQL, but it requires parenthesising the user's predicate correctly in every case, and it is outright wrong for outer joins. Rejected.

**Known cost.** A column reference qualified by database *and* table (`sales.orders.total`) no longer resolves once the relation is aliased to `orders`. Doris returns an unknown-column error. This is a visible compatibility loss, not a leak — the query fails rather than returning unfiltered rows.

### D3: Enumerate table references with an allowlist walk, and refuse on anything unrecognised

**Decision.** The rewriter walks the parsed AST with an explicit allowlist of node kinds it understands. Encountering any construct not on that list aborts the statement with a rejection. There is no default-continue branch.

**Why.** `specs/row-filter-rewrite` requires that a forwarded statement have *every* policy-bearing reference constrained. That is only provable if unhandled shapes cannot silently pass. A denylist inverts the failure mode: every future `sqlparser` version that adds a node type would quietly become a bypass.

**Consequence.** Upgrading `sqlparser` may turn previously-working queries into rejections. That is the intended direction of failure and should be treated as a routine upgrade cost, not a regression.

### D4: Refuse the capabilities the design cannot police

**Decision.** The proxy advertises a capability set with `CLIENT_MULTI_STATEMENTS` and `CLIENT_MULTI_RESULTS` cleared, refuses `COM_STMT_PREPARE` with an error, and — **independently** — refuses any request that parses into more than one statement.

> **⚠️ Corrected 2026-08-09, during implementation.** This decision originally read *"Negotiate away the capabilities the design cannot police"*, and argued that protocol-level refusal was **stronger** than a SQL-level check: "a client that never negotiates multi-statement support cannot send one, so the multi-statement requirement is enforced by the protocol rather than by a semicolon scan that comment-smuggling could defeat."
>
> **That was wrong, and it was load-bearing.** Verified against `opensrv-mysql` 0.7 sources:
> - Clearing the server's advertised bit is **advisory only**. The crate's `run()` loop hands the entire `COM_QUERY` payload to the query handler as a single string regardless of embedded semicolons. A client that sets `CLIENT_MULTI_STATEMENTS` in its own handshake response anyway and sends `SELECT 1; SELECT 2` reaches the handler intact.
> - The advertised flag set is hardcoded in the crate with no hook to change it. `MULTI_STATEMENTS` and `MULTI_RESULTS` are absent, so the intended outcome holds — **by accident of a third-party crate's hardcoding, not by anything this design controls.** A dependency bump could silently reintroduce them.
>
> Multi-statement rejection is therefore enforced **at the SQL level**: if parsing yields more than one statement, refuse. The capability clearing is retained as defence in depth and pinned by a test that reads the handshake bytes off the wire and asserts the bits are clear, so a dependency bump breaks the build rather than the control.
>
> The general lesson is worth keeping: **a capability flag is a request to the client, not a constraint on it.** Any control resting on "the client did not negotiate X" is resting on the client's goodwill.

**Why the rest still holds.** Prepared statements are excluded from the MVP because the text the proxy sees at prepare time is not what Doris executes at bind time.

### D5: Rejections are MySQL error packets, and say nothing about policy

**Decision.** A refusal is returned as a standard error packet with SQLSTATE `42000` and a message stating that the proxy refused the statement and why in general terms — unparseable, unsupported shape, write to a restricted table, or multi-statement. The message never names another user, another table's policy, or any permitted value.

**Why.** Clients and drivers already handle error packets. Detailed policy content in an error message is itself a disclosure channel: an attacker who can trigger errors could enumerate the policy set.

### D6: Parse failures are a category, not an exception

Expected `sqlparser` MySqlDialect gaps against Doris include Doris-specific hints, `LATERAL VIEW` / `explode` forms, array and struct literals and accessors, table-valued functions, `SELECT ... INTO OUTFILE S3`, partition-selection clauses, and materialized-view and index DDL. Each produces the same outcome for a policy-bearing user: a `42000` error saying the statement could not be analysed.

The unqualified rule from `specs/row-filter-rewrite` — reject when the user has **any** configured policy, forward when the user has **none** — is deliberate. It is decidable without parsing, which matters because the alternative ("reject only if it touches a policy table") requires understanding SQL the proxy has just failed to understand.

### D7: Decisions deferred to ADRs rather than specified

Async task topology per session, buffer ownership across the client/backend copy, cancellation behaviour when one side drops, and the shape of the error enum are all implementation-internal. They are not observable to a client and would drift if written as requirements. They will be recorded as after-the-fact ADRs under `docs/adr/` once the code exists, per the project's standing process.

Crate selection is likewise not spec-shaped and is deliberately left to implementation: the design constrains only that the server side must expose the raw handshake salt and capability flags (required by D1 and D4), and that the parser be `sqlparser` with `MySqlDialect`.

## Bypass vectors

Listing only what is closed would read as a claim that the rest is handled. Both columns are part of the design.

**Closed by this design:**

| Vector | Closed by |
|---|---|
| `OR` in the user's `WHERE` widening the row set | D2 — predicate never enters the user's `WHERE` |
| Subquery / derived table reading the table unconstrained | D2 — every relation wrapped at its own site |
| CTE body, and every branch of a `UNION` | D2 |
| Outer join leaking rows or their existence via NULL-extension | D2 — filter inside the operand, join type preserved |
| Correlated `EXISTS` / `IN` probing restricted rows | D2 |
| Alias or unqualified spelling evading a policy match | Qualified-name resolution against the session database, per `specs/policy-config` |
| Multi-statement smuggling, including via comments | **SQL-level statement-count check** — refuse when parsing yields more than one statement. D4's capability clearing is defence in depth only; see the correction under D4, which explains why it is not enforcement |
| Prepare/bind text divergence | D4 — `COM_STMT_PREPARE` refused |
| Unparseable SQL forwarded uncapped | D6 — rejected for any policy-bearing user |
| A new `sqlparser` node type silently passing through | D3 — allowlist walk |

**Left open, and why:**

| Vector | Status |
|---|---|
| **Direct connection to the Doris FE** | Out of the proxy's reach entirely. A deployment precondition, stated in the proposal. This is the largest gap and the reason native `CREATE ROW POLICY` remains the stronger control. |
| **Views over policy tables** | The proxy sees a view name, not its definition, and cannot distinguish a view from a table without consulting the catalogue. A view defined over `sales.orders` returns unfiltered rows. Not mitigated in the MVP. |
| **`information_schema` and `SHOW`** | Not filtered. Table existence, column names and row-count statistics remain visible. |
| **Parser differential** | The deepest risk: SQL that `sqlparser` parses into one meaning and Doris executes as another. The proxy would constrain the reference it believes exists while Doris reads something else. Rejection cannot help here, because nothing appears to have failed. Not mitigated; see Risks. |
| **Collation-dependent value matching** | `region IN ('APAC')` matches according to Doris's collation. Under a case-insensitive collation, `apac` matches too, which may be wider than the policy author intended. |
| **Functions and expressions with side effects or catalogue access** | Not analysed beyond table-reference enumeration. |

## Risks / Trade-offs

- **Parser differential between `sqlparser` and Doris** → No mitigation exists within this design. Reduce exposure by keeping the accepted-shape allowlist (D3) narrow, and by treating every allowlist widening as a security change requiring its own test that the constrained statement returns what Doris actually executes. This risk is the strongest argument for Doris-native `CREATE ROW POLICY`, and it should be restated whenever this proxy is proposed for production use.
- **Views silently bypass the control** → Not closed in the MVP. A future change could read the catalogue at startup and reject statements referencing views that transitively read a policy table. Until then, the deployment must not grant view-creation rights on schemas containing policy tables, and the limitation belongs in operator documentation, not only here.
- **Derived-table wrapping loses `db.table.column` references** → Accepted. Fails as an error, not as a disclosure.
- **Wrapping may degrade query plans** → Accepted for the MVP; measure before optimising, and never by relaxing D2.
- **Backend connection opened before client authentication (D1)** → An unauthenticated client can consume a backend connection per attempt. Mitigate with an accept-rate limit and a connection cap; both are operational settings, not spec requirements.
- **Allowlist rejections look like proxy bugs to users** → The error message must distinguish "unsupported shape" from "you are not permitted", so operators can tell a compatibility gap from a policy denial.
- **Startup fails on invalid config, by design** → A bad policy deploy takes the proxy down rather than silently under-enforcing. That is the correct trade for a security control, but it makes config validation a release-gating concern.

## Migration Plan

Greenfield: there is nothing to migrate. Deployment sequence:

1. Install a Rust toolchain — **currently absent on the development machine**, and a hard blocker for implementation.
2. Deploy the proxy alongside the Doris FE with a validated policy file.
3. Move clients to the proxy endpoint.
4. Restrict network access to the FE so it is reachable only from the proxy. **Until this step, the control is decorative.**

Rollback is repointing clients at the FE directly, which removes all row filtering — so rollback is a policy decision, not merely an operational one.

## Open Questions

These can be answered during implementation without changing the specs, the approach, or the task breakdown:

- Which server-side MySQL protocol crate exposes the raw handshake salt and capability negotiation that D1 and D4 require, versus needing that layer hand-rolled.
- The concrete configuration file format (TOML, YAML or JSON). `specs/policy-config` constrains the semantics and the validation behaviour but deliberately not the syntax.
- Whether the permitted-value set should be typed (numeric versus string) in configuration, or always emitted as quoted literals and left to Doris to coerce.
