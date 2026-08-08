# Survey: Domain Fit — SDD for a Rust MySQL-Wire Proxy in Front of Apache Doris

> **Template version:** 1.0 · **Surveyed:** 2026-08-08 · **Author:** agent (domain-fit researcher)
> **Scope:** Characterizes the *problem domain* this repo is aimed at, and asks which parts of it
> are spec-shaped enough to justify an SDD tool. Deliberately excludes head-to-head tool comparison —
> that lives in the sibling tool surveys; this document supplies the criteria they should be judged against.
>
> **Revised 2026-08-08** against the concrete scope decided in `discussions/milestone-1/02-proxy-scope.md`
> (tenant-isolation SQL rewriting, 1:1 backend connections, passthrough auth). §1, §3a, §3c and §7
> were rewritten; the research in §5, §6, §8 and §10 is unchanged and was not scope-dependent.

## 1. Snapshot

This survey's subject is a domain, not a product, so the Snapshot characterizes the work.

> **Scope note.** The original survey assumed a general-purpose Doris proxy with pooling,
> load balancing and read/write splitting. The project has since narrowed to a **1:1,
> non-pooling L7 proxy whose purpose is SQL rewriting for tenant isolation and schema
> remapping**. That change *removes* most of what was hardest to specify (multiplexing safety,
> failover, session-state leakage) and *adds* the single most spec-shaped surface in the
> project (the rewrite-rule catalogue). Net effect: the spec-shaped fraction goes **up**.

| Field | Value |
|---|---|
| Domain | Network proxy in Rust, terminating the **MySQL client/server wire protocol** and forwarding to Apache Doris FE nodes (default `query_port` 9030) |
| External contracts consumed | MySQL protocol (client side + backend side), Doris FE behavior (MySQL-compatible SQL dialect + Doris extensions), optionally PROXY protocol v1, optionally Doris HTTP/Stream-Load and Arrow Flight SQL ports |
| Contract stability | **High but under-documented.** MySQL's protocol is ~25 years old and back-compat-bound; the authoritative reference is Doxygen-generated server internals docs, with real behavior sometimes only visible in MySQL's C source (see §6, salt-byte example) |
| Contract ownership | **External.** Neither the protocol nor Doris is under this project's control — a proxy is a conformance exercise, not a design exercise, on its boundaries |
| Purpose of the proxy | **SQL rewriting for tenant/row isolation and table/schema remapping.** Not load balancing, not sharding, not read/write splitting |
| Concurrency model | Async Rust / tokio: one bidirectional relay per client connection, **1:1 to the backend — no pool**, cancellation-safe select loops, backpressure between two sockets that can each stall |
| Correctness bar | **Two bars, and the second is higher.** (1) Byte-exact on the wire, or clients break in ways that look like data corruption. (2) **A rewrite rule that silently fails to apply is a data leak** — isolation makes this a security property, not a quality one |
| Dominant failure modes | Protocol desync (sequence-id / packet-boundary drift), **a statement reaching the backend without its isolation predicate**, placeholder-position drift after rewriting `COM_STMT_PREPARE`, auth-plugin negotiation dead ends, resultset buffering blowups on large scans |
| Testing strategy that actually works | Real-client conformance matrix (JDBC, Go, Python, Node, Rust drivers) + property/round-trip tests on the codec + **negative tests proving no allowlisted statement can reach the backend un-predicated** + fuzzing the decoder + `cargo-mutants` on rewrite and allowlist predicates. Unit tests alone are near-worthless for protocol conformance |
| Prior art density | **High for the pattern, near-zero for the target.** ProxySQL, VTGate, ShardingSphere-Proxy are mature; a serious *Doris-specific* proxy does not appear to exist (§6) |
| Rust ecosystem maturity | **Client-side strong, server-side thin.** `mysql_async` 0.37.0 (2026-05-25), `sqlparser` 0.62.0 (2026-05-07), `tokio` 1.53.1 — all actively released. `opensrv-mysql` 0.7.0 last published **2024-02-21**; `msql-srv` 0.11.0 last published **2023-12-17** |
| Estimated spec-shaped fraction | **Revised up: ~70–75% by surface area, ~45–55% by effort.** Dropping pooling/routing removed the least specifiable work; the rewrite-rule catalogue and allowlist added the most specifiable |
| Greenfield vs brownfield | Greenfield — zero code, zero legacy constraints, and therefore zero existing spec to drift from |
| Project posture | Name contains "study": likely **learning/exploration**, which changes the value calculus (§7, §9) |

## 2. One-paragraph thesis

Building a Doris proxy means writing a program whose entire externally-visible behavior is dictated by
contracts someone else already wrote down: the MySQL wire protocol on both sides, and Doris's FE
semantics behind it. That is unusually favorable ground for spec-driven development, because the
hardest question SDD normally struggles with — *"what should this do?"* — has an authoritative answer
that is discoverable, enumerable, and testable. The catch is that the protocol's real specification is
*partly folklore*: the published documentation is incomplete, and implementers routinely discover
constraints only from MySQL's source or from packet captures. So the bet is not "an SDD tool will tell
us what to build"; it is **"an SDD tool gives us a durable place to write down what we learn about a
contract we do not own, in a form that maps 1:1 onto tests."** Against that, roughly half the project —
the async ownership design, the pooling/backpressure model, the error-handling ergonomics, the
borrow-checker-driven refactors — is genuinely exploratory work where a spec written in advance is a
prediction, not a requirement, and predictions in this area age badly.

## 3. Workflow / artifact model

### 3a. Surface inventory — the crux

The decision hinges on this table more than on any tool feature comparison.

| # | Work surface | Spec-shaped? | Why |
|---|---|---|---|
| 1 | Packet codec (framing, int/string types, OK/ERR/EOF, column defs) | **Strongly** | Field-level tables exist upstream; every field is a testable assertion |
| 2 | Connection phase / handshake state machine | **Strongly** | Finite states, enumerable transitions, capability-flag negotiation is a truth table |
| 3 | Auth: `mysql_native_password`, `caching_sha2_password`, auth-switch, passthrough | **Strongly** (behavior) / **partly** (crypto detail) | Message sequences are specifiable; what a proxy *can* do is constrained by not holding cleartext passwords |
| 4 | Command-phase coverage matrix (`COM_QUERY`, `COM_STMT_*`, `COM_PING`, `COM_INIT_DB`, `COM_CHANGE_USER`, `LOCAL INFILE`, multi-resultset) | **Strongly** | This is a compatibility matrix — the canonical spec artifact |
| 5 | **Rewrite rules — isolation-predicate injection, table/schema remapping** | **Strongly — the most spec-shaped surface in the project** | Each rule *is* `GIVEN a query matching X, WHEN forwarded, THEN the backend receives Y`. Simultaneously a requirement, an acceptance test, and a `proptest` case, with near-zero translation loss |
| 6 | **Statement allowlist — which statement shapes may be forwarded at all** | **Strongly** | A matrix, and it is the security boundary. Grows one delta at a time; the canonical living document |
| 7 | Config schema + admin interface, **including the tenant↔Doris-user mapping** | **Strongly** | Ordinary external contract; versioned, user-visible |
| 8 | Error mapping (internal error → MySQL ERR packet, code + SQLSTATE) | **Strongly** | A table, and clients depend on it. Now also carries the rejection errors for non-allowlisted SQL |
| 9 | Prepared-statement handling under rewriting (`COM_STMT_PREPARE` rewrite, placeholder stability) | **Strongly** (the invariant) / **Medium** (the mechanism) | 1:1 connections delete the cross-backend ID-remapping problem entirely. What remains is the invariant *rewrites must not change placeholder count, order, or result column shape* — specifiable, and a prime `cargo-mutants` target |
| 10 | Parse-failure policy | **Strongly** | Isolation makes fail-closed a security requirement. Must be stated explicitly, never left implied |

*Removed as out of scope (2026-08-08): FE selection / read-write splitting / sticky sessions, and
session-state multiplexing safety. Both were rated strongly spec-shaped, but pooling and multi-backend
routing are no longer in the project. If pooling is ever added, restore rows for both — ProxySQL's
multiplexing-disabler list is effectively a published spec of that exact problem (§10 #7).*
| 11 | Async ownership design (task topology, channels vs shared state, cancellation safety) | **No** | Discovered against the borrow checker and tokio's semantics; a pre-written spec here is fiction |
| 12 | Backpressure / zero-copy relay / buffer strategy | **No** | Determined by measurement, not by requirement |
| 13 | Performance tuning, allocation avoidance, syscall batching | **No** | Only budgets/SLOs are specifiable, never the method |
| 14 | Error-handling ergonomics (`thiserror` shape, error enum granularity) | **No** | Refactored continuously; specifying it creates drift liability |

Rule of thumb this yields: **spec what an outsider can observe or depend on; do not spec what only the
compiler and the profiler can adjudicate.** Rows 1–8 are where an SDD tool earns its keep. Rows 11–14 are
where SDD ceremony converts directly into wasted tokens and stale documents.

### 3b. What a protocol spec must actually contain

Generic SDD templates (user story → acceptance criteria) under-serve this domain. A usable protocol spec needs:

1. **Packet field tables** — name, type (`int<1>`, `int<lenenc>`, `string<NUL>`, `string<lenenc>`), presence condition (capability flag), and value constraints.
2. **State machines** — connection phase, command phase, prepared-statement lifecycle. Ideally as a diagram *plus* an explicit transition table, because the table is what becomes tests.
3. **Capability/compatibility matrices** — per protocol feature: supported / passthrough / rejected-with-error / unsupported, plus which client drivers exercise it.
4. **Invariant statements** — e.g. "sequence IDs are contiguous within a command exchange", "a backend connection is never returned to the pool while session state is dirty". These are the mutation-testing and property-testing targets.
5. **Negative/adversarial cases** — malformed length prefixes, truncated packets, oversized payloads. A proxy parses untrusted client bytes; the spec must say what happens, not leave it to `unwrap()`.
6. **A "known-unknowns" register** — for constraints discovered empirically from packet captures or MySQL source. This is domain-specific and, notably, **no SDD tool surveyed ships a slot for it**; it has to be a hand-rolled section.

### 3c. Proposed artifact tree (tool-agnostic shape)

```
specs/                          # living, long-lived — the contract layer
  protocol/
    framing.md                  # packet header, seq id, 16MB splitting, compression envelope
    connection-phase.md         # handshake v10, capability negotiation, TLS upgrade, auth switch
    command-phase.md            # per-COM_* coverage matrix + passthrough rules
    prepared-statements.md      # stmt-id remap invariants, lifecycle, multiplexing constraints
    error-mapping.md            # internal error -> (errno, SQLSTATE, message) table
    compatibility-matrix.md     # feature x {supported|passthrough|rejected|unsupported} x driver
  rewrite/
    allowlist.md                # statement shapes forwardable at all; the security boundary
    isolation-predicates.md     # how a tenant predicate is derived and injected, per shape
    schema-remapping.md         # logical -> physical table/database name rules
    parse-failure-policy.md     # fail closed; what error the client sees
  config-schema.md              # incl. tenant <-> Doris-user mapping
  admin-api.md
  invariants.md                 # cross-cutting; the property-test and mutants target list
  unknowns.md                   # empirically-discovered constraints + provenance (pcap/source line)
                                # incl. every Doris construct sqlparser-rs cannot parse
changes/                        # short-lived proposals, if the chosen tool has a delta model
docs/adr/                       # design decisions for the EXPLORATORY half (rows 11-14 above)
```

Note the split: `specs/` for external contracts, `docs/adr/` for the async/ownership decisions. Trying
to put row-11 material into a spec format is the single most likely way this goes wrong.

**Command surface:** the actual dev loop is Rust's, and SDD sits *above* it, not inside it —

```sh
cargo test                 # unit + integration
cargo test --test conform  # real-driver conformance matrix (needs a live Doris or a mock FE)
cargo fuzz run codec       # decoder against untrusted bytes
cargo mutants --in-diff    # test-quality gate on changed routing/codec logic
cargo clippy && cargo fmt
```

**Artifact lifecycle:** protocol specs are *append-and-refine*, not *write-once-and-archive*. The
compatibility matrix in particular is a permanent, growing document — every newly-supported `COM_*`
or capability flag is an edit to a living table. This favors tools with a **delta model over a living
spec store** (proposal → delta → merge back into `specs/`) over tools whose unit of work is a
self-contained feature folder that is never revisited. Version control story is trivially fine for all
candidates: everything is markdown in git.

## 4. What it enforces vs. what it suggests

Read as: *in this domain*, which correctness concerns can be mechanically enforced, and which can only
be asserted in prose by a spec layer?

| Concern | Enforced (tooling blocks you) | Suggested (prompt/prose only) |
|---|---|---|
| Spec exists before code | Nothing in the Rust toolchain enforces this. Only the SDD tool can, and most do so at prompt level | Entirely a discipline question; the tool's real contribution |
| Spec ↔ code traceability | Partially mechanizable: test names / `#[test]` doc comments citing spec IDs, checked by a lint or CI grep — **must be built, no tool ships it for Rust** | Otherwise prose-level cross references that rot silently |
| Test/acceptance criteria | `cargo test` enforces that written tests pass; `cargo-mutants` enforces that tests are *meaningful* (kills surviving mutants) — this is unusually strong for a spec layer to lean on | Whether the tests correspond to the spec's scenarios |
| Review gate | CI can gate on tests/clippy/mutants; cannot gate on "the spec is right about MySQL" | Human/agent review against upstream protocol docs |
| Drift detection | Compiler catches internal drift (types, signatures). **Nothing catches spec-vs-wire drift** except the conformance test matrix | Spec text claiming behavior the code no longer has |
| Structural validity (which states are representable) | **Rust's type system, strongly** — typestate for the handshake, non-exhaustive enums for `COM_*`, newtypes for stmt IDs | A prose state machine that duplicates this adds nothing |
| Byte-level encoding correctness | Property tests (`encode(decode(b)) == b`) + fuzzing + real-driver tests | Field tables in a spec — necessary as the *source*, powerless as a *check* |
| Cross-cutting invariants ("never buffer a full resultset", "no panic on client input") | `#![deny(clippy::unwrap_used)]`, `#![forbid(unsafe_code)]`, memory-ceiling tests — partially | A "constitution"-style artifact is the right home for the rest |

The honest reading: **Rust's toolchain enforces a lot, and a spec layer enforces nothing.** The spec
layer's value is as a *source of truth for what tests to write*, and as institutional memory about an
under-documented external contract. Any tool sold on "the spec keeps the code honest" is overselling
in this domain; the tests keep the code honest, and the spec keeps the tests honest.

## 5. Strengths

Where this domain is unusually *good* ground for SDD:

- **The requirements are discoverable, not invented.** Most SDD pain comes from specs that are guesses
  about product behavior. Here, ~half the spec content can be transcribed from upstream references and
  prior art. An agent writing "the handshake response packet contains a 4-byte capability flags field"
  is not hallucinating a requirement; it is doing research that is checkable.
- **Scenarios map onto tests with near-zero translation loss.** Given/When/Then over packet exchanges is
  literally an integration test: *given a client that advertises `CLIENT_DEPRECATE_EOF`, when a resultset
  completes, then an OK packet (not EOF) is sent.* This is the tightest spec→test correspondence any
  domain offers, and it is where SDD's acceptance-criteria formats actually pay.
- **Compatibility matrices are the natural artifact of a proxy, and markdown tables are a perfect
  container.** The living-spec model matches the real work rhythm: incremental coverage expansion.
- **Prior art is documented in spec-like form.** ProxySQL publishes its multiplexing disabler rules and
  prepared-statement remapping architecture as prose specs; Vitess documents its MySQL-compatibility
  gaps explicitly. These can be lifted as starting specs rather than rediscovered.
- **Explicitly bounded scope is a survival requirement here, and specs enforce bounds well.** The
  MySQL protocol is effectively unbounded; the practical advice from someone who shipped an
  implementation is to implement only what you need. A written "unsupported → return ERR 1295" matrix
  is how you keep that boundary from eroding.
- **Mutation testing gives the spec layer teeth it normally lacks.** `cargo-mutants` on routing
  predicates and codec branches converts "we wrote a scenario" into "the scenario actually discriminates".
  The repo's `.gitignore` already anticipating `mutants.out` suggests this is a deliberate intent.

## 6. Weaknesses / friction

- **The upstream spec is incomplete, and the gap is exactly where bugs live.** A documented case: the
  handshake salt bytes must avoid ASCII 36 and stay in `[1,35] ∪ [37,127]` — a constraint absent from
  the published documentation and visible only in MySQL's source. A spec-driven workflow that treats
  the published docs as ground truth will produce a confidently wrong spec. **Packet captures against a
  real client/server are non-negotiable, and no SDD tool has a workflow for "spec derived from pcap".**
- **Half the project resists specification.** Async ownership, pooling lifetimes, cancellation safety,
  and backpressure are discovered by fighting the compiler and the runtime. Writing plans for these in
  advance produces documents that are wrong by the second refactor. This is the single largest source of
  SDD ceremony waste in this domain.
- **Rust's server-side MySQL ecosystem is stale, which pushes work *toward* the unspecifiable half.**
  `opensrv-mysql` (0.7.0, published 2024-02-21) and `msql-srv` (0.11.0, published 2023-12-17) are both
  well over two years without a release as of this survey. `msql-srv` notably lacked authentication
  support, forcing at least one team to implement auth from scratch. If the codec must be hand-written,
  more of the project is low-level Rust and less of it is orchestration that a spec can direct.
- **Near-zero Doris-specific prior art.** A GitHub search for MySQL proxies in Rust returns mostly
  small or abandoned projects: `AgilData/mysql-proxy-rs` (195 stars, last push **2016**),
  `turbine-dev/turbine-proxy` (33 stars), several sub-15-star hobby projects. The largest active Rust
  codebase touching the MySQL protocol is `warp-tech/warpgate` (7.5k stars), and it is a bastion, not a
  query router. No `doris-proxy` of substance was found. **There is no reference implementation to
  spec against** — Doris's own docs point users at ProxySQL/HAProxy/Nginx instead.
- **Doris reduces the need for some routing logic, which shrinks the spec-shaped payoff.** Doris FE
  already forwards master-only operations internally, and Connector/J supports `jdbc:mysql:loadbalance://`
  natively. Some of the obvious "routing rules" a spec would describe are solving a problem the stack
  already solves — worth confirming before specifying.
- **Conformance testing is expensive and slow, which weakens the mutation-testing story.** `cargo-mutants`
  wants fast deterministic tests; a driver matrix against a live Doris is neither. The likely outcome is
  mutants run only on pure codec/routing modules — which is fine, but narrower than it first appears.
  (`cargo-mutants`' published cautions page covers test side effects and `--in-place` risk; it does not
  document async/timeout limitations, so treat those as unverified.)
- **A spec layer that restates Rust types is pure drift liability.** The temptation in this domain is
  strong, because packet structs *look* like spec content. `struct HandshakeResponse41 { ... }` plus
  rustdoc is a better artifact than a markdown copy of the same field list. The spec should carry
  *encoding rules, conditions, and behavior*, not structure.

## 7. Fit signals

**Strong fit when** the work item is:

- Codec, handshake, auth negotiation, command coverage, error mapping — surfaces 1–8 in §3a.
- Building or extending the **compatibility matrix** or the **statement allowlist**; each increment
  is a delta against a living spec.
- **Rewrite rules.** The strongest case in the project: an isolation predicate or a name-remapping
  rule is a Given/When/Then that converts directly into an integration test and a `proptest`
  property. If SDD does not pay here, it pays nowhere.
- Config schema and admin surface — anything a user of the proxy depends on.
- Onboarding-shaped work: because this is a "study" project, the act of writing the state machine down
  *is* a substantial part of the learning goal. Here the spec's value is pedagogical and does not need to
  pay for itself in delivery speed.
- Work delegated to an agent that will otherwise invent protocol details. A spec is the cheapest
  anti-hallucination device available for byte-level work.

**Poor fit when** the work item is:

- Task topology, channel vs shared-state decisions, cancellation-safe `select!` loops, buffer ownership.
- Anything downstream of a profiler. Specify a budget ("p99 added latency < 1 ms", "constant memory per
  connection regardless of resultset size"); never specify the technique.
- Error-enum shape, module boundaries, trait design — these will be refactored repeatedly and are better
  served by ADRs written *after* the fact.
- Exploratory spikes ("can `opensrv-mysql` be made to do auth-switch, or do we fork?"). The answer is
  found by writing throwaway code, and a spec written first would just be a question in disguise.
- Small protocol fixes where the spec edit costs more than the code edit.

**Implied tool requirements** (for the sibling surveys to score against — expressiveness claims below are
inferred from documentation, not hands-on, and should be cross-checked):

| Requirement | Why this domain needs it |
|---|---|
| Living spec store + delta model | Compatibility matrix **and the statement allowlist and rewrite-rule catalogue** grow forever; feature-folder-only models handle this poorly. OpenSpec's `specs/` + `changes/` with ADDED/MODIFIED/REMOVED deltas is the closest published match |
| A place to mark a requirement **security-relevant** | Tenant isolation means some requirements are load-bearing for correctness in a way others are not — a reviewer must be able to tell at a glance which rules cannot be relaxed. **No surveyed tool ships this**; a house convention (tag, or a dedicated `isolation` spec domain) is required regardless of tool |
| Cross-cutting invariant artifact | "No panic on client input", "no full-resultset buffering" apply to every change. Spec Kit's `constitution.md` is the closest published match |
| Trigger→response requirement syntax | Protocol state machines are literally *WHEN `<packet>` THE SYSTEM SHALL `<response>`*. Kiro's EARS notation is the closest published match |
| Free-form markdown tables + diagrams inside spec bodies | Packet field tables and state diagrams. All markdown-based tools clear this bar; it is not a differentiator |
| A place for empirically-derived, source-of-truth-unknown facts | **No surveyed tool ships this.** Must be hand-rolled (`specs/unknowns.md`) regardless of tool choice |
| Low ceremony for the exploratory half | The tool must be *skippable* for surfaces 11–14 without breaking its own model. Tools that gate implementation on spec presence will be fought or bypassed |

## 8. Rust / systems-software notes

**Does a spec layer add value on top of Rust's type system, or duplicate it?** Both, depending on where
it is pointed. The split is clean enough to state as a rule:

- **The type system already is an executable spec for structure.** Typestate (`Conn<Handshake>` →
  `Conn<Authenticating>` → `Conn<Command>`) makes illegal protocol states unrepresentable, which is
  strictly stronger than a prose state machine — it is checked on every build. Newtypes for
  `ClientStmtId` vs `BackendStmtId` mechanically prevent the exact class of bug that prepared-statement
  remapping invites. Exhaustive `match` on a `Command` enum turns "did we handle every `COM_*`?" into a
  compile error. **A markdown state machine that duplicates typestate is dead weight**; a markdown state
  machine that *precedes* and *justifies* the typestate is design work worth keeping.
- **The type system cannot express the wire format.** Nothing in Rust says the capability flags field is
  4 bytes little-endian, or that `CLIENT_DEPRECATE_EOF` changes which terminator packet is legal. That
  is irreducibly spec content, and it is the majority of the protocol work.
- **The type system cannot express policy or compatibility.** "Route `SELECT` to Observers unless the
  session is in a transaction" is a decision, not a type.

**How the testing stack changes the calculus** — this is the domain's distinguishing feature:

- **Property testing** (`proptest`, 1.11.0, very widely used) makes codec specs directly executable:
  round-trip `encode(decode(bytes)) == bytes` and `decode(encode(pkt)) == pkt` are the canonical
  properties, and shrinking turns failures into minimal reproducers. A spec that lists field constraints
  translates into `Strategy` definitions almost mechanically.
- **Fuzzing** (`cargo-fuzz`) is not optional for a proxy: the decoder consumes attacker-controlled bytes
  from any client that can reach the port. The spec's *negative* cases (§3b item 5) are what tell you
  whether a fuzz crash is a bug or expected rejection.
- **Mutation testing** (`cargo-mutants` 27.1.0, released 2026-06-02) is the piece that gives specs
  accountability: it answers "does this scenario's test actually discriminate?" Best applied to pure
  functions — codec, routing predicates, multiplexing-safety checks. Expect it to be impractical against
  live-Doris integration tests.
- **The conformance matrix is the real oracle.** Documented experience implementing the MySQL server
  protocol reports that testing against Java, Node, Python, Ruby, and Rust clients *consistently* revealed
  bugs that the documentation did not, and that Wireshark comparison against a live server was the
  effective debugging method. Any spec-driven plan for this project that does not budget for a
  multi-driver test harness has mis-scoped the work.

**Evidence of SDD applied to Rust systems software: essentially none found.** This is worth stating
plainly rather than dressing up.

- The one substantive Rust + Spec Kit writeup located (Mar 2026) builds an **Axum/Tokio BMI-calculator
  REST API with an embedded web UI** — explicitly a workflow demo, not systems work. Its most
  interesting reported result (the constitution catching a stateless-principle violation when shared
  mutable state was introduced) is genuinely relevant to invariant enforcement, but the project shares
  no domain characteristics with a wire-protocol proxy.
- No experience report was found for SDD on a protocol implementation, a proxy, a database component,
  or any Rust codebase where the borrow checker was a design driver. **Treat all claims about SDD
  helping this domain as untested.**
- The adjacent evidence that *does* exist is academic and points in a supportive direction: several
  2025–2026 papers build LLM agents that check protocol implementations against RFC specifications
  (RFCAudit), generate conformance tests from specs (iPanda), and do hierarchical protocol testing
  (NeTestLLM). These validate the *premise* — that a natural-language protocol spec is a usable oracle
  for an agent — without validating any particular SDD tool or workflow.

**Web/CRUD bias risk:** every tool in the sibling surveys was designed and demonstrated on
feature-shaped product work (user stories, endpoints, screens). None was demonstrated on packet-level
work. The specific mismatches to check for: (a) requirement templates that assume a user actor when the
actor is a MySQL client driver; (b) "user story" framings that don't decompose to packet exchanges;
(c) task generators that assume a model→service→endpoint dependency order irrelevant here;
(d) plan phases that assume a database and an API layer exist.

## 9. Cost & lock-in

- **Money:** the domain imposes no cost; tool costs belong to the tool surveys. The domain-specific
  spend is **test infrastructure**: a running Doris instance (or a mock FE) plus a multi-language driver
  matrix in CI. That is the real budget line, and it is required with or without SDD.
- **Token cost:** protocol specs are token-heavy in an unusual way — field tables and compatibility
  matrices are large, low-entropy documents that an agent must re-read to stay consistent. Expect
  per-feature overhead well above the web-app baseline for surfaces 1–8, and expect the compatibility
  matrix to become a context-window problem as it grows. Mitigation: split specs per command family
  rather than one monolith (reflected in the §3c tree).
- **Lock-in:** low for any markdown-based tool. The protocol specs are the durable asset and survive a
  tool change intact; the tool-specific parts (proposal/plan/task scaffolding) are disposable. This
  argues for keeping `specs/` structured around the *protocol*, not around the tool's workflow vocabulary.
- **Exit path:** the genuine lock-in risk is not the tool — it is **writing specs that only describe
  what was already built**, at which point they carry no information and deleting them costs nothing but
  keeping them costs review attention forever. The exit test to apply at each milestone: *if we deleted
  `specs/`, would we lose knowledge that is not in the code or the tests?* For surfaces 1–8 the answer
  should be yes (encoding rules, upstream provenance, unsupported-feature decisions). For 11–14 the
  answer will be no — which is the signal not to have specced them.

## 10. Evidence & sources

| # | Source | Type | Date | Notes |
|---|---|---|---|---|
| 1 | https://doris.apache.org/docs/3.0/gettingStarted/what-is-apache-doris/ | docs | current | FE/BE split: FE handles request parsing, planning, metadata, node management; BE handles storage and execution |
| 2 | https://doris.apache.org/docs/3.x/db-connect/database-connect/ | docs | current | Clients connect over MySQL protocol on FE `query_port` (default 9030) |
| 3 | https://doris.apache.org/docs/3.x/admin-manual/cluster-management/load-balancing/ | docs | current | Four documented LB options (Connector/J `loadbalance://`, Nginx TCP, HAProxy, ProxySQL); PROXY protocol v1 support since 2.1.1 via `enable_proxy_protocol` for client-IP passthrough (whitelists, audit) |
| 4 | https://doris.apache.org/community/design/metadata-design/ | design doc | — | FE Master/Follower/Observer roles; metadata fully in memory, replicated via BDB JE; Observers add query concurrency and don't vote; master-only ops forwarded automatically |
| 5 | https://dev.mysql.com/doc/dev/mysql-server/latest/PAGE_PROTOCOL.html | primary spec | current | Doxygen-generated: protocol basics, connection lifecycle, connection phase, command phase; TLS and compression as transparent layers. Implementer-oriented, not tutorial |
| 6 | https://ochagavia.nl/blog/implementing-the-mysql-server-protocol-for-fun-and-profit/ | hands-on report | — | **Key evidence.** Docs ambiguous; salt bytes constrained to `[1,35] ∪ [37,127]` — undocumented, found in MySQL source; `msql-srv` had no auth, so auth was written from scratch; scoped to MySQL 5.x default auth only; multi-language client testing and Wireshark comparison were what found bugs |
| 7 | https://proxysql.com/documentation/multiplexing/ | docs | current | Multiplexing disablers: open transaction, `CREATE TEMPORARY TABLE`, `PREPARE`, `SQL_CALC_FOUND_ROWS` (last two disable it permanently on that connection) |
| 8 | https://proxysql.com/documentation/mysql-prepared-statements-architecture | docs | current | Statement IDs are per-backend-connection and server-assigned; proxy maintains a global-ID ↔ per-backend-ID mapping and dedupes by statement hash |
| 9 | https://www.percona.com/blog/caching_sha2_password-support-for-proxysql-is-finally-available/ | blog | — | Auth-switch cases a proxy cannot satisfy without cleartext passwords; how long `caching_sha2_password` took to land in a mature proxy |
| 10 | https://proxysql.com/documentation/architecture/ | docs | current | Thread-type separation (worker / admin / monitor / cluster) as a proven proxy topology |
| 11 | https://vitess.io/docs/24.0/overview/whatisvitess/ | docs | current | VTGate: stateless MySQL-protocol proxy; connection pooling/multiplexing, query de-duping, rewriting, blocking, killing; single-shard ACID, multi-shard best-effort by default |
| 12 | https://shardingsphere.apache.org/blog/en/material/proxy/ | project blog | — | Frontend (NIO, protocol codec) / core (parse-rewrite-route-merge) / backend (pooled) three-layer split — a directly reusable module decomposition |
| 13 | https://github.com/databendlabs/opensrv · https://crates.io/crates/opensrv-mysql | repo/registry | v0.7.0, 2024-02-21 | Async (tokio) MySQL server bindings; used by Databend, GreptimeDB, CeresDB; self-described alpha. ~475k total / ~42k recent downloads. **No release in ~2.5 years** |
| 14 | https://github.com/jonhoo/msql-srv · https://crates.io/crates/msql-srv | repo/registry | v0.11.0, 2023-12-17 | Sync MySQL server emulation via a `MysqlShim` trait. **No release in ~2.7 years**; no built-in auth (see #6) |
| 15 | https://crates.io/crates/mysql_async | registry | v0.37.0, 2026-05-25 | Active client-side crate; ~1.08M recent downloads — contrast with #13/#14 |
| 16 | https://github.com/apache/datafusion-sqlparser-rs | repo | v0.62.0, 2026-05-07 | Syntax-only parser, deliberately no semantics; accepts queries real engines reject. Used by CipherStash Proxy, JumpWire, GreptimeDB and others |
| 17 | https://www.databend.com/blog/category-engineering/2025-09-10-query-parser/ | engineering blog | 2025-09-10 | Why a database team wrote its own SQL parser rather than adapting an existing one — relevant to §3a row 10 |
| 18 | https://mutants.rs/ · https://mutants.rs/cautions.html | docs | v27.1.0, 2026-06-02 | Mutation testing for Rust; cautions cover test side effects and `--in-place` risk. **Async/timeout limitations not documented there — unverified** |
| 19 | https://crates.io/crates/proptest | registry | v1.11.0, 2026-03-24 | Property testing with shrinking; ~43M recent downloads. Round-trip parser properties are the standard idiom |
| 20 | https://arxiv.org/abs/2506.00714 (RFCAudit) | paper | 2025 | LLM agent checking protocol implementations against RFC specs — supports "NL spec as oracle" |
| 21 | https://arxiv.org/pdf/2507.00378 (iPanda) | paper | 2025 | LLM agent generating protocol conformance tests by simulating how developers write them |
| 22 | https://arxiv.org/html/2510.13248 (NeTestLLM) | paper | 2025 | Hierarchical feedback for automated protocol testing with LLM agents |
| 23 | https://medium.com/@philippe.baucour/stop-vibe-coding-start-spec-driven-development-with-rust-and-github-spec-kit-1dcd4e06a9cf | blog | 2026-03 | **The only Rust + SDD writeup found.** Axum/Tokio/Serde BMI REST API + web UI. Constitution caught a stateless-principle violation. Author notes discipline cost, model dependence, overkill for small work. No systems-level content |
| 24 | https://martinfowler.com/articles/exploring-gen-ai/sdd-3-tools.html | article | — | Comparative treatment of Kiro / spec-kit / Tessl artifact models |
| 25 | https://github.com/Fission-AI/OpenSpec/blob/main/docs/concepts.md | docs | current | `specs/` (living truth) vs `changes/` (proposals); ADDED/MODIFIED/REMOVED deltas; Given/When/Then scenarios; RFC-2119 keywords |
| 26 | GitHub API search (`mysql proxy language:rust`, `doris proxy mysql`), queried 2026-08-08 | primary data | 2026-08-08 | `AgilData/mysql-proxy-rs` 195★ last push **2016-10-11**; `turbine-dev/turbine-proxy` 33★; `wbtlb/mini-proxy` 14★; `pzhenzhou/haentgl` 6★; largest active Rust MySQL-protocol codebase is `warp-tech/warpgate` 7.5k★ (bastion, not router). No substantive Doris proxy found |
| 27 | https://github.com/apache/doris | repo | pushed 2026-08-07 | 15.7k★, actively developed, Java FE — confirms the target is a live moving contract |

## 11. Confidence & gaps

- **Confidence: medium-high on the domain characterization, low on the SDD-benefit claim.**
  The protocol's hard parts, the prior-art landscape, and the Rust crate maturity are all sourced from
  primary references and live registry/API data queried on the survey date. The claim that SDD helps
  *this* domain is a reasoned argument from surface structure, **not an empirical finding** — no
  experience report of SDD on Rust systems software was found, and that absence is the single biggest
  gap in this survey.

- **Unverified claims:**
  - The ~50–60% / ~25–35% spec-shaped estimates in §1 are the author's judgment from the §3a inventory,
    not measured. Treat as an ordering, not a number.
  - Whether Doris's FE requires proxy-side routing at all beyond FE selection. Doris forwards master-only
    operations internally and Connector/J does client-side LB; the marginal value of protocol-aware
    routing here is **assumed, not confirmed**.
  - Whether `opensrv-mysql` is usable as-is for a proxy (it emulates a *server*; a proxy needs
    server-side termination *and* client-side origination, and the two halves must agree on capability
    flags). Its 2.5-year release gap suggests a fork may be needed. Not verified by reading its source.
  - `cargo-mutants` behavior on async/tokio code and on tests with network timeouts — its cautions page
    does not address this.
  - Exact protocol-feature coverage of `opensrv-mysql` / `msql-srv` (auth plugins, compression, TLS,
    binary protocol completeness). Registry metadata was verified; feature matrices were not.
  - Tool-capability mappings in §7 (OpenSpec delta model, Spec Kit constitution, Kiro EARS) are inferred
    from documentation, not hands-on. **Cross-check against the sibling tool surveys before relying on them.**

- **Open questions a decision-maker should settle hands-on:**
  1. **Codec build-vs-adopt.** Spend a day trying to make `opensrv-mysql` terminate a real Connector/J
     connection with `caching_sha2_password`. The answer determines how much of this project is
     specifiable orchestration versus unspecifiable byte-wrangling — and therefore how much any SDD tool
     can help. *This should be the first spike, before the tool is chosen.*
  2. **Does the spec survive contact with a packet capture?** Write one spec (the connection phase),
     implement it, then diff against a Wireshark trace of a real client↔Doris session. Count the
     corrections. That number is the honest measure of SDD's value here.
  3. **Can the compatibility matrix stay in context?** Draft it at realistic size and check whether an
     agent can hold it while implementing. If not, the tool needs per-file spec granularity, which
     eliminates monolithic-spec models.
  4. **What is the actual goal?** If "study" means learning the protocol and Rust async, the spec's
     pedagogical value stands on its own and low-ceremony tooling wins. If a usable proxy is the goal,
     scope must be cut to a documented feature subset first — and the cut list *is* the first spec.
  5. **Does the test harness exist before the spec?** Given §10 #6, a multi-driver conformance harness is
     the load-bearing artifact. A spec with nothing to execute it against is a wish list.
