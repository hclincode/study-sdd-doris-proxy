## Why

Row-level access rules for our Doris tables currently have nowhere to live that we control. Doris can enforce them natively with `CREATE ROW POLICY`, but that puts the policy inside each cluster as mutable DDL state: it is not reviewable in a pull request, not diffable, not deployable as an artifact, and it must be duplicated and kept in sync across every cluster we run.

This change introduces an MVP L7 MySQL proxy that sits in front of a Doris FE, reads a policy file we own in version control, and constrains each authenticated user's `SELECT` statements to the rows that file permits — one policy set, enforced identically across every backend it fronts.

**The honest comparison.** Doris `CREATE ROW POLICY` (available since 1.2) is the *stronger* enforcement point and we are not claiming otherwise. It runs inside the FE, applies to every statement the FE can execute, and cannot be defeated by SQL a third-party parser fails to understand. This proxy is chosen for where the policy *lives* and how it is *governed*, not because it enforces better. Anyone who can open a connection to the Doris FE directly bypasses this proxy entirely — the control is only as good as the network boundary that forces traffic through it. That boundary is a deployment precondition, not something this change delivers.

## What Changes

- **New Rust binary**: an L7 MySQL proxy. It terminates the MySQL wire protocol and parses statements rather than relaying bytes, because it cannot rewrite SQL it has not decoded.
- **1:1 connection mapping**: each accepted client session opens and owns exactly one backend connection to the configured Doris FE. No pooling, no multiplexing, no routing decisions.
- **Passthrough authentication**: credentials are relayed to Doris, which remains the sole authority on identity. The proxy applies no policy until Doris has authenticated the user; a claimed username is not an identity.
- **Policy configuration loaded from a file**: maps `(user, table)` to a column and a set of permitted values. Invalid or unparseable config is a startup failure, not a runtime surprise.
- **Row-filter rewriting**: for each `SELECT` a user issues, every reference to a table with a policy for that user is constrained so only rows whose column value is in the permitted set can be returned. Tables with no policy for that user are untouched and pass through.
- **Fail-closed rejection** of anything the rewriter cannot prove it has constrained (detailed below).

### Statements that become REJECTED

This is a user-visible compatibility cost and the direct consequence of treating the feature as a security control. Against a table that has a policy for the requesting user, the proxy rejects with a clear MySQL error rather than forwarding:

- Any statement that **fails to parse**. `sqlparser` has no Doris dialect, so some valid Doris SQL will be refused. This is accepted, not a defect to be worked around by widening pass-through.
- Any **write statement** — `INSERT`, `UPDATE`, `DELETE`, `REPLACE` — touching a policy table. Constraining writes is out of scope for the MVP, and forwarding them unconstrained would let a user modify rows outside their slice.
- Any **query shape the rewriter cannot enumerate completely**: if it cannot list every table reference in the statement and show that each policy-bearing one was constrained, the statement does not run.
- **Multi-statement packets**, where a second statement could be smuggled past a check applied to the first.

A statement that touches no policy table for that user is forwarded unchanged, including statements that fail to parse — there is nothing to protect, so there is nothing to reject. Determining "touches no policy table" for unparseable SQL is itself impossible, so in practice unparseable SQL is rejected whenever the user has any policy at all; the design phase must settle this rule precisely.

## Non-goals

- **Not a replacement for Doris authorization.** Table-, column- and database-level grants stay in Doris. This adds a row predicate; it does not manage privileges.
- **Not a network boundary.** The proxy assumes something else prevents direct FE access. It cannot enforce its own bypass prevention.
- **No write-path filtering.** `INSERT`/`UPDATE`/`DELETE` against policy tables are rejected, not constrained.
- **No prepared-statement support** (`COM_STMT_PREPARE`) in the MVP.
- **No view awareness.** A view whose definition reads a policy table is not expanded by the proxy; the proxy sees only the view name.
- **No metadata protection.** `information_schema` and `SHOW` statements are not filtered and may disclose the existence and shape of restricted data.
- **No sharding, read/write splitting, multi-backend routing, or connection pooling.**
- **Not a performance feature.** Parsing and rewriting add latency; the MVP does not optimize it.

## Capabilities

### New Capabilities

- `connection-routing`: accepting client connections, the 1:1 backend mapping to a Doris FE, passthrough authentication, and the session lifecycle that establishes *which authenticated user* a statement belongs to. Owns the rule that policy is applied only after the backend authenticates.
- `policy-config`: the on-disk policy format, its validation, and load-time behavior. Owns what a well-formed policy is, what makes config invalid, and the requirement that invalid config prevents startup rather than degrading enforcement.
- `row-filter-rewrite`: the parse → rewrite → forward path. Owns which statements are accepted, which are rejected, where the injected predicate lands for each supported shape, and the guarantee that a forwarded statement has every policy-bearing table reference constrained.

### Modified Capabilities

None — this is the project's first change and `openspec/specs/` is empty.

## Impact

- **New crate** in `milestones/milestone-2/mysql-proxy-project/openspec-workspace/`: `Cargo.toml`, `src/`. None of this exists yet.
- **New dependencies**: an async runtime, a MySQL wire-protocol implementation, and `sqlparser` with `MySqlDialect`. Specific crate selection belongs in `design.md`.
- **Toolchain blocker**: Rust is not currently installed on this machine (`cargo` and `rustc` are absent from PATH). Planning is unaffected, but implementation cannot begin until a toolchain is present.
- **Operational precondition**: clients must be prevented from reaching the Doris FE directly, or the control is decorative.
- **Deployment surface**: a policy file that must be distributed with the binary, and whose invalidity takes the proxy down by design.
