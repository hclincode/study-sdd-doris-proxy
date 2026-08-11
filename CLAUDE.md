# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Status

This repository is a practice vehicle for **spec-driven development (SDD)**. The exercise is an **L7 MySQL proxy for [Apache Doris](https://doris.apache.org/)**.

The primary goal is learning SDD; the proxy is the exercise, not the deliverable. Do not optimize the spec ceremony away for the sake of shipping code faster.

| Milestone | State |
|---|---|
| **1** — choosing an SDD tool | ✅ **Closed** 2026-08-09. No tool chosen, deliberately; the decision moved to hands-on use. See `milestones/milestone-1.md` |
| **2** — building the proxy with OpenSpec | ✅ **Closed** 2026-08-11 with its change at 70/75 tasks and five deliberately unaddressed. A working, heavily verified MVP exists. See `milestones/milestone-2.md` |
| **3** | ▶ **Starting. Scope not yet defined.** Nothing in this file describes live work until it is |

> ⚠️ **Milestones are independent.** Each one sets its own goal, its own scope and its own workspace. **Do not resume milestone 2's change, its open tasks, or its backlog** — `add-row-filter-proxy-mvp` is closed and frozen where it stands, and its five open items in `tasks.md` §10 are closed-unaddressed, not a to-do list. Everything below about the proxy and about OpenSpec is **milestone-2 history kept for reference**; treat it as evidence about what worked, not as instructions for what to do next.

Milestone 2's own account of why it closed there: the change did not stall on difficulty but on cost. Fail-closed rewriting against a foreign SQL dialect has an open-ended compatibility surface, and every real client discovered another piece of it. The author's diagnosis was that the first task was simply too big.

## Proxy scope — as settled in milestone 2 (closed)

> **This section is a record, not a live requirement.** It is what milestone 2 built and verified against a real Doris. Milestone 3 defines its own scope from scratch and is free to narrow, widen or replace all of it. Read this to avoid re-deriving hard-won domain knowledge — not to infer what to build.

Terminate the MySQL wire protocol (parse, don't just relay bytes), forward to a Doris FE, and **rewrite SQL before it reaches the backend**. L7 rather than L4 is deliberate — you cannot rewrite SQL you have not decoded.

> ⚠️ **Keep worked examples on the rewriting path, not the wire protocol.** Handshake, capability flags, packet framing and sequence IDs are incidental plumbing. An earlier pass used them as the illustrative example and spent most of the effort on material that was beside the point. Anchor specs and examples on **parse → rewrite → forward**.

| Decision | Choice |
|---|---|
| Purpose of rewriting | **Row-level filtering by authenticated user** — a config maps `(user, table)` to a column and a set of permitted values; the proxy constrains that user's `SELECT`s to matching rows |
| Backend connections | **1:1 per client** — no pooling, no multiplexing |
| Authentication | **Passthrough** — client credentials relayed to Doris, which remains the sole authority on identity |
| Parse strategy | **Full parse** with `sqlparser` (MySqlDialect) |
| SQL surface | **`SELECT`, including joins and subqueries.** Writes touching a policy table are rejected, not constrained |
| Default rule | A table with **no policy for the requesting user is unrestricted** — absence of a policy means no restriction, not no access |

Out of scope unless revisited: sharding, read/write splitting, multi-backend routing, connection pooling.

> ⚠️ **Superseded scope — do not re-derive it.** Through milestone 1 the rewriting purpose was *append `LIMIT 200` to every query and sub-query*. That was abandoned in milestone 2 in favour of row filtering. Milestone 1's finding still stands and is worth not repeating: `LIMIT` caps rows *returned*, not rows *read*, so it never achieved its stated goal, and Doris's `sql_select_limit` already did the useful part. `surveys/sql-limit-injection.md` and `milestones/milestone-1.md` are history, not current intent.

Consequences that follow from the above:

1. **This IS a security control — fail closed.** This inverts the earlier scope's central assumption. A missed filter is a **data leak**, not a slow query. A statement that does not parse, or that parses into a shape the rewriter cannot prove it has fully constrained, **must be rejected, never forwarded**. `sqlparser` has no Doris dialect, so some valid Doris SQL will be refused; reduced compatibility is the accepted price. There is no "forward it and see" branch anywhere in this design.
2. **A single injection mechanism, proven once.** The predicate is applied by wrapping each policy-bearing relation in a derived table — `FROM (SELECT * FROM t WHERE col IN (…)) AS t` — rather than appending `AND …` to the user's `WHERE`. Appending is wrong for two independent reasons: `WHERE a OR b AND policy` binds as `a OR (b AND policy)`, leaving `a` unconstrained, and injecting into the enclosing `WHERE` converts `LEFT JOIN` to inner-join semantics. Wrapping never touches the user's predicate, so neither applies.
3. **Enumeration must be closed by construction.** The rewriter walks the AST with an **allowlist** of node kinds and refuses anything unrecognised. A denylist would turn every future `sqlparser` release into a silent bypass.
4. **A native mechanism is genuinely stronger.** Doris has shipped [`CREATE ROW POLICY`](https://doris.apache.org/docs/dev/sql-manual/sql-statements/data-governance/CREATE-ROW-POLICY/) since 1.2, enforced in the FE and immune to parser gaps. The proxy is justified only by *where policy lives* — versioned outside Doris, centrally applied across clusters — never by claiming better enforcement. **Say so in any proposal rather than routing around it.**
5. **Known-open bypass vectors, stated not hidden.** Direct connections to the FE (a deployment precondition the proxy cannot enforce), views over policy tables, `information_schema` and `SHOW`, collation-dependent value matching, and — deepest — a **parser differential** where `sqlparser` and Doris read the same SQL differently. Rejection cannot help with the last one, because nothing appears to have failed.

The repo was renamed from `study-openspec-doris-proxy` to `study-sdd-doris-proxy` because **the SDD tool is not yet chosen** — the original name presumed [OpenSpec](https://github.com/Fission-AI/OpenSpec). Milestone 2 used OpenSpec and produced a clear picture of it (see `milestones/milestone-2.md` §3.5), but did not settle the choice; Spec Kit and the no-framework baseline remain untried in anger.

## Repository layout

| Path | Contents |
|---|---|
| `surveys/` | Research surveys on SDD and candidate tools, all following `_TEMPLATE.md` |
| `discussions/` | Decision records for the project's own process choices, filed per milestone (`discussions/milestone-1/`, `discussions/milestone-2/`) |
| `milestones/` | One re-orientation document per milestone — read `milestones/milestone-1.md` and `milestones/milestone-2.md` instead of re-reading the research and the change artifacts |
| `milestones/milestone-2/mysql-proxy-project/openspec-workspace/` | **Closed and frozen — reference only, not a working directory.** Milestone 2's OpenSpec workspace and the `doris-row-filter-proxy` crate. See below |
| `archives/milestone-1/` | **Archived, not a live reference.** Holds `trial/`, the milestone-1 side-by-side comparison: one real spec written in two competing tool formats. Kept for history; do not work from it |
| `study-note.md` | **Human-only.** The author's own journal, carrying a constitution header. Never edited by an agent, and not a source of direction |

Milestone 3 gets its own directory under `milestones/milestone-3/` when its scope is set. Do not add to milestone 2's.

### The milestone-2 OpenSpec workspace (closed)

Milestone 2 developed the proxy using OpenSpec (v1.8.0, installed globally via npm). The workspace is `milestones/milestone-2/mysql-proxy-project/openspec-workspace/`, and it holds both the spec artifacts and the crate:

```
milestones/milestone-2/mysql-proxy-project/openspec-workspace/
├── openspec/
│   ├── config.yaml     # project context + per-artifact rules, fed to the AI on every artifact
│   ├── specs/          # EMPTY — the change was never archived, so nothing was ever folded in
│   └── changes/        # add-row-filter-proxy-mvp, closed at 70/75 tasks
├── Cargo.toml          # doris-row-filter-proxy
├── src/                # 5,885 lines
├── tests/              # 6,054 lines — 245 tests across 18 targets, all passing
├── docs/adr/           # 0001-0006, written after the fact
├── examples/policy.toml
├── notes/adr-material.md
└── README.md           # operator-facing, incl. what this does NOT protect against
```

> ⚠️ **The `openspec` CLI resolves its root by searching *upward* from the current directory, never downward.** Running it from the repo root fails with `No OpenSpec root found from the current directory`. Every `openspec` invocation — and every `/opsx:*` command or `openspec-*` skill, which shell out to it — must run with the working directory set to the workspace. The skills and commands themselves live in the repo-root `.claude/`, because that is the only place Claude Code loads them from; init was therefore run with `--tools none` to avoid creating a nested `.claude/` that would be ignored.

`openspec/config.yaml` is not boilerplate — it carries the project context, the cross-cutting invariants, the fail-closed rule, the known bypass vectors, and the requirement to justify the proxy against Doris `CREATE ROW POLICY`. Verified that its `context` and `rules` blocks are injected into `openspec instructions <artifact> --change <name>`. It is what the tool shows the model when generating every proposal, design, spec and task list, so a new workspace needs its own — copying milestone 2's is the fast start, but its contents are milestone-2 scope.

> ⚠️ **`config.yaml` fails silently.** A YAML error makes OpenSpec print `could not parse … ignoring it` and continue with **no context and no rules at all**; an unknown `rules:` key is dropped with a one-line warning. Both have already happened here — a plain list item containing `": "` parses as an implicit map key (use an em dash or quote it), and `rules.spec` was silently discarded because the schema's artifact id is `specs`. `openspec validate --strict` catches neither, since it checks change artifacts rather than config. After editing, run `openspec instructions proposal --change <name> --json` and check for `Warning`/`Unknown` and that `context` and `rules` are actually populated.

**Closed change: `add-row-filter-proxy-mvp`** — the MVP proxy, three capabilities (`connection-routing`, `policy-config`, `row-filter-rewrite`), all four planning artifacts complete and validating `--strict`, implemented and verified against a real Doris 2.1.0. Closed at 70/75; the five open items in `tasks.md` §10 are **closed-unaddressed and are not a backlog**. Its `design.md` is the best artifact in the repo to read — D1 (relay Doris's own auth salt) and D2 (wrap relations in derived tables) are load-bearing, and D4 is preserved *with its wrong original text* alongside the correction, which is the clearest demonstration of what a design document is for.

`surveys/domain-fit-rust-proxy.md` is the most important of these — its §3a table classifies which parts of the proxy are spec-shaped (codec, handshake, auth negotiation, command coverage, rewrite rules, error mapping) versus which resist specification (async task topology, cancellation safety, buffer ownership, error-enum shape). **Specify what an outsider can observe; do not specify what only the compiler and the profiler can adjudicate.**

Process decisions that survived milestone 2 and are worth reusing:

- Design decisions for the non-spec-shaped half are recorded as **ADRs written after the fact**, not as specs — ADRs are immutable history and therefore cannot drift. The rule was honoured the first time it bit, by the agent that wrote it, against its own document.
- **Name scenarios for the behaviour, not the mechanism.** "Multi-statement request is rejected" stayed true when the mechanism behind it was disproved; a task named for the mechanism had to be rewritten.
- The SDD tool choice is **still open**. Milestone 1 deferred it to hands-on use; milestone 2 gave OpenSpec that use. See `discussions/milestone-1/01-sdd-tool-selection.md` and `discussions/milestone-2/01-what-actually-caught-defects.md`.

Milestone 2's cross-cutting invariants, carried in that workspace's `openspec/config.yaml` so the tool restated them on every artifact. Recorded here because they are what a fail-closed rewriter actually needs, not because they bind milestone 3:

- No unbounded buffering — memory per connection is constant with respect to result size.
- No panic on client input — the decoder consumes attacker-controlled bytes.
- Rewrite rules **must not** change `?` placeholder count, order, or result column shape. This keeps `COM_STMT_PREPARE` tractable; without it, parameter binding computed by the client against the original statement becomes wrong. (Permitted values emitted as literals satisfy this; a placeholder would not.)
- A rewrite **must not change the result set except by removing rows the user is not permitted to see**. It must not alter column shape or order, aggregate semantics over the permitted rows, or ordering among them.
- Unsupported is explicit — reject with a clear error rather than half-applying a rule. **Half-applied filtering is the worst outcome available: it looks like it worked.**
- Enforcement must be **provable statement by statement**. If the rewriter cannot enumerate every table reference and show each policy-bearing one was constrained, the statement does not run.

## Toolchain

The `.gitignore` targets a Rust/Cargo workspace and includes [`cargo-mutants`](https://mutants.rs/) for mutation testing (`**/mutants.out*/` is ignored).

| Tool | Status |
|---|---|
| Node.js | v26.0.0 ✅ |
| OpenSpec | v1.8.0 ✅ — global npm install, `openspec` on PATH |
| Rust / Cargo | v1.97.1 ✅ — stable-aarch64-apple-darwin, via rustup 1.29.0 |
| rustfmt / clippy | 1.9.0 / 0.1.97 ✅ — installed with the `default` rustup profile |
| cargo-mutants | v27.1.0 ✅ — installed and used in milestone 2, where it found five real defects nothing else caught |
| Docker | ✅ — `apache/doris:doris-all-in-one-2.1.0` runs native arm64, FE on 9030. Milestone 2 recorded this as blocked for most of the change without ever checking |

Rust was installed 2026-08-09 with the official installer (`https://sh.rustup.rs`, `--profile default`), not Homebrew — rustup is the toolchain manager the Rust project ships, and layering `brew` on top adds a second manager for no gain. It wired `. "$HOME/.cargo/env"` into `~/.zshenv` and `~/.profile`. Verified end to end on a throwaway crate: `cargo new` → `cargo run` → `cargo test` → `cargo clippy` all succeed, so the linker and Xcode Command Line Tools (`/Library/Developer/CommandLineTools`) are in place.

## Commands

These are verified working on this machine. **Milestone 3 will need its own workspace**; until it has one, the only directory these run in is milestone 2's closed workspace, and running them there is for reading, not for changing it.

**Every command below runs from an OpenSpec workspace directory, not the repo root** — for milestone 2 that is `milestones/milestone-2/mysql-proxy-project/openspec-workspace/`.

OpenSpec:

```sh
cd milestones/milestone-2/mysql-proxy-project/openspec-workspace   # required — the CLI only searches upward

openspec list                                  # active changes
openspec list --specs                          # canonical specs
openspec new change <name>                     # start a change
openspec instructions <artifact> --change <n>  # authoritative format spec — read BEFORE authoring
openspec status --change <name>                # artifact completion
openspec validate --all --strict               # structural validation
openspec archive <name> --yes                  # archive and fold into canonical specs
openspec doctor                                # confirm the resolved root
```

`instructions` is the ground truth for artifact format — more reliable than the docs. Guessing the formats produced eight distinct errors during milestone 1.

Shell output on this machine is polluted by harmless `_encode`/`_decode` zsh errors; pipe through `grep -v "_encode\|_decode"` to read it.

Rust — the crate `doris-row-filter-proxy` lives in the same workspace directory, alongside `openspec/`.

```sh
export PATH="$HOME/.cargo/bin:$PATH"   # cargo is NOT on PATH in non-interactive shells here

cargo build
cargo test                             # all targets
cargo test --test rewrite_positions    # one target
cargo test <name>                      # tests matching a substring
cargo test -- --nocapture              # show stdout/stderr from tests
cargo fmt --check                      # verify formatting
cargo clippy --all-targets -- -D warnings
cargo mutants                          # pin the commit before running; results describe a snapshot

cargo run -- --policy examples/policy.toml --listen 127.0.0.1:3307 --backend 127.0.0.1:9030
```

> ⚠️ **`cargo` is absent from `PATH` in non-interactive shells**, even though `~/.zshenv` sources `~/.cargo/env` on its first line — something in zsh startup (likely whatever emits the `_encode`/`_decode` errors) aborts before it takes effect. Export it explicitly in every command.

### Crate layout

| Path | Contents |
|---|---|
| `src/policy.rs` | Policy model, config loading and validation, `(user, table)` lookup. Returns **three** outcomes — `Unrestricted` / `Restricted` / `Unresolvable` — and the third is a refusal |
| `src/analyze.rs` | Parse, then the allowlist AST walk enumerating every table reference. No `_ => {}` arm continues the walk |
| `src/rewrite.rs` | Derived-table wrapping, plus an independent re-check of the *rendered* output before forwarding |
| `src/session.rs` | Client sessions, 1:1 backend mapping, passthrough auth by relaying Doris's own salt. Backend connection phase is hand-rolled on a raw `TcpStream` |
| `src/error.rs` | Refusal reasons. Deliberately has no "could not analyse, forwarded anyway" variant |
| `src/main.rs` | Startup ordering enforced by types: `serve` takes `PolicySet` by value, so the bind cannot precede validation |
| `examples/policy.toml` | Worked policy file, loaded by a test so it cannot rot |
| `notes/adr-material.md` | The raw findings `docs/adr/` was written from, including evidence that did not become a decision |
| `README.md` | Operator-facing: preconditions, and the table of what this does **not** protect against |

**If milestone 3 ever touches this crate, read `README.md` and the change's `design.md` first.** The decision records explain why each mechanism is shaped as it is, and several were arrived at the hard way — reversing one without reading the reasoning reintroduces a bypass.

## The one finding to carry into any milestone

Milestone 1 predicted it and milestone 2 demonstrated it with a live disclosure hole underneath a green suite:

> A spec catches nothing on its own. It makes disagreement legible, which is only useful if someone holds an implementation against it. **Not one defect in milestone 2 was found by writing a spec, by `openspec validate`, or by any tool gate** — `validate --strict` passed continuously, including while a live bypass was in the code. Every defect was found by a party other than the one who wrote it, at a seam between two individually-correct things.
>
> The tests keep the code honest; the spec keeps the tests honest; and mutation testing keeps the tests honest about being honest.

Two habits from milestone 2 that cost real time to learn:

- **Check what a fixture cannot express, not only what it does express.** A mock simpler than the real collaborator hides exactly the class of defect the real one has.
- **For anything security-relevant, rebuild in the same command that measures.** A stale artifact makes you confidently wrong with evidence in hand — it happened four times, and once it corrupted the verification of a security fix.
