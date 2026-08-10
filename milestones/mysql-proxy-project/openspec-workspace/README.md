# doris-row-filter-proxy

An L7 MySQL proxy for Apache Doris that constrains each authenticated user's `SELECT` statements to the rows a policy file permits.

> **⚠️ Read "What this does not protect against" before deploying this.** It is a resource-governance and policy-distribution tool with real, documented gaps — not a complete access control. Apache Doris ships `CREATE ROW POLICY`, which is enforced inside the frontend and is strictly stronger. Choose this only if you need policy to live outside Doris, and only with the preconditions below in place.

## What it does

A configuration maps a user and a table to a column and a set of permitted values. For each `SELECT` that user issues, every reference to a policy-bearing table is rewritten so only permitted rows can be returned:

```sql
-- the client sends
SELECT * FROM sales.orders AS o WHERE o.total > 100

-- Doris receives
SELECT * FROM (SELECT * FROM sales.orders WHERE region IN ('APAC','EMEA')) AS o WHERE o.total > 100
```

The predicate wraps the *relation* rather than joining the user's `WHERE`. That is deliberate: `WHERE a OR b AND policy` binds as `a OR (b AND policy)` and would leave `a` unconstrained, and injecting into the enclosing `WHERE` would turn a `LEFT JOIN` into inner-join semantics and change the result set.

## Running it

```sh
doris-row-filter-proxy --policy policy.toml --listen 127.0.0.1:3307 --backend 127.0.0.1:9030
```

Proxy settings are command-line arguments, not entries in the policy file. The policy file rejects unknown keys and unknown sections so that a misspelled `[[policy]]` cannot silently yield zero policies and disable all filtering — which means it cannot also carry a `[proxy]` section.

The policy file is validated **before** anything binds. Invalid configuration exits non-zero and nothing ever listens.

### Policy file

```toml
[[policy]]
user = "analyst"
database = "sales"
table = "orders"
column = "region"
permitted_values = ["APAC", "EMEA"]

[[policy]]
user = "analyst"
database = "sales"
table = "invoices"
column = "tenant_id"
permitted_values = [17, 23]
```

- A table with **no policy for the requesting user is unrestricted** — absence means no restriction, not no access.
- Database and table names match case-insensitively, so a policy cannot be evaded by respelling. Usernames match case-sensitively, matching MySQL account semantics.
- Permitted values are text or 64-bit signed integers. A Doris `LARGEINT` or `BIGINT UNSIGNED` key outside that range **cannot be expressed today** and is rejected at load rather than truncated.
- Values containing a backslash, a doubled quote, or a NUL are rejected at load, because the proxy cannot guarantee they reach the backend unaltered.

## Operator preconditions

**These are not recommendations. Without them the control is decorative.**

1. **Clients must not be able to reach the Doris frontend directly.** Authentication is passthrough, so clients hold real Doris credentials and can simply connect to port 9030 and bypass every rule this proxy enforces. Enforce this at the network layer. The proxy cannot enforce its own non-circumvention, and nothing in it will warn you if traffic is going around it.

2. **Do not grant view-creation rights on schemas holding policy tables.** The proxy sees a view name, not its definition, and cannot distinguish a view from a table without consulting the catalogue. A view defined over a policy table returns unfiltered rows.

3. **Only `mysql_native_password` works.** Relaying a scramble works only for a single-round-trip auth plugin. Anything else — `caching_sha2_password` full auth, for instance — would require data derived from a password the proxy deliberately does not hold, so the session is refused. This is a proxy limitation, and the error says so rather than reporting an access-denied state that would send you hunting for a credential problem.

4. **Rejections are expected and are not bugs.** `sqlparser` has no Doris dialect. Valid Doris SQL that it cannot parse is refused for any user who has a policy, because "does this touch a policy table?" cannot be answered for SQL that did not parse. Reduced compatibility is the deliberate price of the security property.

## What this does not protect against

Listing only what is closed would read as a claim that the rest is handled.

| Gap | Status |
|---|---|
| **Direct connection to the Doris FE** | Entirely outside the proxy's reach. The largest gap, and the main reason native `CREATE ROW POLICY` is the stronger control. |
| **Views over policy tables** | Not mitigated. The proxy sees a name, not a definition. |
| **`information_schema` and `SHOW`** | Not filtered. Table existence, column names and row-count statistics remain visible. |
| **Parser differential** | The deepest risk. SQL that `sqlparser` parses into one meaning and Doris executes as another. The proxy would constrain the reference it believes exists while Doris reads something else. Rejection cannot help, because nothing appears to have failed. |
| **Collation-dependent matching** | `region IN ('APAC')` matches per Doris's collation. Under a case-insensitive collation `apac` matches too, which may be wider than intended. |
| **Write statements** | Not constrained. `INSERT`/`UPDATE`/`DELETE`/`REPLACE` touching a policy table are refused outright. |
| **Prepared statements** | `COM_STMT_PREPARE` is refused. The text seen at prepare time is not what executes at bind time. |
| **Generated column labels** | A client reading column metadata sees a **different column name** than it would connecting to Doris directly, whenever an *unaliased* projection expression contains a policy table. MySQL derives such a label from the expression as written, and the injected guard is written into it: `(SELECT MAX(total) FROM sales.orders)` comes back labelled with the guard inside it. Column count, order and values are unaffected. The remedy is in your hands — an explicit alias survives the rewrite exactly, so `... AS peak` is labelled `peak`. |
| **`db.table.column` references** | A column qualified by **both** database and table — `sales.orders.total` — stops resolving once the relation is wrapped and aliased to `orders`, and Doris returns an unknown-column error. The statement fails rather than returning unfiltered rows, so this costs compatibility and not confidentiality. Qualify by table or alias alone. |
| **`sql_mode` dependence of literal rendering** | Values are emitted through `sqlparser`'s renderer. That its output means the same thing to Doris under both default `sql_mode` and `NO_BACKSLASH_ESCAPES` is **assumed, not verified** — no local test can settle it. Hazardous inputs are rejected at load to narrow the exposure. |
| **Result rows larger than 16 MB** | Not relayed. A row whose packet reaches the MySQL framing limit of 2²⁴−1 bytes continues into a further packet, and the proxy refuses rather than reassembling — reassembly means buffering without a bound, which the memory invariant forbids. A large `BLOB` column will therefore fail the session rather than returning truncated. Connecting to the frontend directly would have worked. See ADR 0003. |
| **No timeouts anywhere** | A client that connects and then sends nothing holds one task and one backend connection **indefinitely**. So does a backend that accepts the connection and never completes the handshake. There is no idle timeout, no connection-phase timeout and no connection cap, so an unauthenticated client can consume Doris frontend connections one per attempt — the backend connection is opened *before* the client authenticates, which passthrough authentication requires. Bound this outside the proxy, at the network layer or with a frontend connection limit. See ADR 0004. |

## Development

```sh
cargo build
cargo test                          # all targets
cargo test --test rewrite_positions # one target
cargo fmt --check
cargo clippy --all-targets -- -D warnings
```

`cargo` may not be on `PATH` in non-interactive shells; `export PATH="$HOME/.cargo/bin:$PATH"` first if so.

| Path | Contents |
|---|---|
| `src/policy.rs` | Policy model, config loading, validation, `(user, table)` lookup |
| `src/analyze.rs` | Parse, then the allowlist AST walk enumerating every table reference |
| `src/rewrite.rs` | Derived-table wrapping, and the independent re-check before forwarding |
| `src/session.rs` | Client sessions, 1:1 backend mapping, passthrough auth by relaying Doris's salt |
| `src/error.rs` | Refusal reasons. Deliberately has no "could not analyse, forwarded anyway" variant |
| `src/main.rs` | Startup ordering: `serve` takes `PolicySet` by value so the bind cannot precede validation |
| `docs/adr/` | Decision records for the parts that are not spec-shaped: task topology, buffer ownership, cancellation, error-enum shape, protocol crate choice |
| `notes/adr-material.md` | The raw findings the ADRs were written from, including evidence that did not become a decision |

The specification lives in `openspec/`. `openspec/changes/add-row-filter-proxy-mvp/` holds the proposal, design and per-capability specs; the design document's decision records explain *why* each mechanism is shaped the way it is, and its bypass tables are the authoritative version of the gaps listed above.
