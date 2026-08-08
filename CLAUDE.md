# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Status

This is a **greenfield Rust project** with no source code, `Cargo.toml`, or README yet. It is an **L7 MySQL proxy for [Apache Doris](https://doris.apache.org/)**, built as a practice vehicle for **spec-driven development (SDD)**.

The primary goal is learning SDD; the proxy is the exercise, not the deliverable. Do not optimize the spec ceremony away for the sake of shipping code faster.

## Proxy scope

Terminate the MySQL wire protocol (parse, don't just relay bytes), forward to a Doris FE, and **rewrite SQL before it reaches the backend**. L7 rather than L4 is deliberate — you cannot rewrite SQL you have not decoded.

> ⚠️ **Keep worked examples on the rewriting path, not the wire protocol.** Handshake, capability flags, packet framing and sequence IDs are incidental plumbing. An earlier pass used them as the illustrative example and spent most of the effort on material that was beside the point. Anchor specs and examples on **parse → rewrite → forward**.

| Decision | Choice |
|---|---|
| Purpose of rewriting | **Append `LIMIT 200` to every query and sub-query** — protect the Doris cluster from unbounded scans |
| Backend connections | **1:1 per client** — no pooling, no multiplexing |
| Authentication | **Passthrough** — client credentials relayed to Doris |
| Parse strategy | **Full parse** with `sqlparser` (MySqlDialect) |

Out of scope unless revisited: sharding, read/write splitting, multi-backend routing, connection pooling.

Consequences that follow from the above:

1. **This is resource protection, not a security control.** A missed `LIMIT` is a heavy query, not a data leak. That makes **pass-through on parse failure defensible** — the opposite of what a security-relevant rewrite would require. `sqlparser` has no Doris dialect, so some valid Doris SQL will not parse; forwarding it uncapped is a legitimate choice, but it must be an explicit, stated one.
2. **Capping sub-queries changes results.** `SELECT COUNT(*) FROM (SELECT … FROM big) t` returns a wrong count if the inner query is capped; the same applies to `ORDER BY` inside a sub-query, `IN (SELECT …)`, UNION branches, CTEs, and `INSERT … SELECT`. Capping the outer query is safe; capping every sub-query is not. **Which contexts are safe to cap is the project's central spec artifact** — a table, growing one entry at a time.
3. **An existing `LIMIT` needs a stated rule.** Replace with 200, take the minimum, or leave alone; and `LIMIT n OFFSET m` interacts with the cap. Pick one and write it down.
4. **A native mechanism may make this unnecessary.** MySQL has a `sql_select_limit` session variable, and Doris has workload/resource groups and query limits. If either covers the goal, rewriting SQL is the wrong tool. Being confirmed in `surveys/sql-limit-injection.md`.

The repo was renamed from `study-openspec-doris-proxy` to `study-sdd-doris-proxy` because **the SDD tool is not yet chosen** — the original name presumed [OpenSpec](https://github.com/Fission-AI/OpenSpec). Treat all of the above as intent, not established architecture, and update this file as real structure lands.

## Repository layout (pre-code)

| Path | Contents |
|---|---|
| `surveys/` | Research surveys on SDD and candidate tools, all following `_TEMPLATE.md` |
| `discussions/` | Decision records for the project's own process choices, filed per milestone (`discussions/milestone-1/`) |
| `milestones/` | One re-orientation document per milestone — read `milestones/milestone-1.md` instead of re-reading the research |
| `archives/milestone-1/` | **Archived, not a live reference.** Holds `trial/`, the milestone-1 side-by-side comparison: one real spec written in two competing tool formats. Kept for history; do not work from it |

`surveys/domain-fit-rust-proxy.md` is the most important of these — its §3a table classifies which parts of the proxy are spec-shaped (codec, handshake, auth negotiation, command coverage, rewrite rules, error mapping) versus which resist specification (async task topology, cancellation safety, buffer ownership, error-enum shape). **Specify what an outsider can observe; do not specify what only the compiler and the profiler can adjudicate.**

Process decisions made so far:

- Design decisions for the non-spec-shaped half are recorded as **ADRs written after the fact** in `docs/adr/`, not as specs — ADRs are immutable history and therefore cannot drift.
- The SDD tool choice itself is still open. See `discussions/milestone-1/01-sdd-tool-selection.md`.

Cross-cutting invariants that will govern every change, once an invariants artifact exists:

- No unbounded buffering — memory per connection is constant with respect to result size.
- No panic on client input — the decoder consumes attacker-controlled bytes.
- Rewrite rules **must not** change `?` placeholder count, order, or result column shape. This keeps `COM_STMT_PREPARE` tractable; without it, parameter binding computed by the client against the original statement becomes wrong. (A `LIMIT` appended as a literal satisfies this; a `LIMIT ?` would not.)
- A rewrite **must not change the result set** of a query it caps, beyond truncating the outer row count. Any context where appending `LIMIT` alters values — counts, aggregates, ordering — is out of bounds for capping.
- Unsupported is explicit — reject with a clear error rather than half-applying a rule.

## Toolchain

The `.gitignore` targets a Rust/Cargo workspace and includes [`cargo-mutants`](https://mutants.rs/) for mutation testing (`**/mutants.out*/` is ignored).

## Commands

Once a `Cargo.toml` exists, the standard workflow applies:

```sh
cargo build                 # compile
cargo run                   # run the binary
cargo test                  # run all tests
cargo test <name>           # run tests matching a substring (single test)
cargo test -- --nocapture   # show stdout/stderr from tests
cargo fmt                   # format (rustfmt)
cargo clippy                # lint
cargo mutants               # mutation testing (see cargo-mutants tool above)
```

When the project is scaffolded, replace this section with the actual crate/workspace layout, binary names, and any non-standard build or run steps.
