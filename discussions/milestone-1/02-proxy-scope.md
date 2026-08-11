# Discussion: Proxy scope

**Date:** 2026-08-08 · **Status:** ⏹ **Closed without decision, 2026-08-09**
**Resolves:** D4 in `01-sdd-tool-selection.md` (partially)

> **⏹ Closed without a decision.** This discussion is over and nothing further
> will be decided in it. The questions it left open — T1 (whether the proxy is a
> security boundary), B2 (one FE or several), and the fail-closed-versus-
> pass-through choice in §1 — were **never resolved here**. They were overtaken
> when milestone 2 restated the proxy's scope from scratch rather than
> continuing this thread.
>
> Nothing below should be read as a current requirement, including the parts
> marked "resolved". For current scope see `CLAUDE.md` — Proxy scope, and the
> change under
> `milestones/mysql-proxy-project/openspec-workspace/openspec/changes/`.
>
> **Correction to the 2026-08-08 banner below.** That banner states that the
> tenant-isolation analysis "no longer applies" because the rewrite is resource
> protection rather than a security control. That is no longer true. Milestone 2
> adopted **row-level filtering by authenticated user**, which is security-
> relevant, so the isolation reasoning is live again — the `LIMIT 200` scope that
> displaced it is itself now superseded. Kept as written, because a decision
> record is history; read the two banners in order.
>
> Read for provenance, not direction. Its value is that C1, C2, C3 and T1 were
> reached independently and largely match what milestone 2 arrived at — see
> "Where this landed" at the foot of the file.

## Stated scope

> An **L7 MySQL proxy for Doris**. Future direction: **modify SQL before
> forwarding to the Doris backend.**

> **⚠️ Superseded in part, 2026-08-08.** The rewrite purpose was later stated
> directly as: **parse SQL and append `LIMIT 200` to every query and sub-query
> before forwarding**, to protect the Doris cluster from unbounded scans. It is
> **resource protection, not a security control.**
>
> That inverts several conclusions below. Sections analysing tenant isolation
> (C1's fail-closed requirement, C3's tenant identity derivation, T1's security
> boundary) **no longer apply** — a missed `LIMIT` is a heavy query, not a data
> leak, so pass-through on parse failure is defensible rather than forbidden.
> Read those sections as a record of how the analysis would go *if* the rewrite
> were security-relevant; they are not current requirements.
>
> What survives unchanged: the L7-not-L4 rationale, 1:1 backend connections,
> passthrough auth, the `sqlparser` Doris-dialect gap, and the prepared-statement
> placeholder-stability invariant. The new central question — which sub-query
> contexts are safe to cap — is tracked in `surveys/sql-limit-injection.md` and
> in `CLAUDE.md`.
>
> Sections below that dwell on handshake and packet mechanics are background,
> not the point. Keep worked examples on **parse → rewrite → forward**.

Two commitments follow directly, and both matter more than they look.

### L7, not L4

The proxy parses the MySQL wire protocol rather than relaying bytes. That is a
deliberate choice of the expensive option, and it is the right one for the
stated endgame — you cannot rewrite SQL you have not decoded. But it means the
following are no longer optional:

- Packet framing: 3-byte length + sequence ID, 16 MB packet splitting
- Connection phase: handshake v10, capability negotiation, auth
- Command phase dispatch: at minimum `COM_QUERY`, `COM_PING`, `COM_INIT_DB`,
  `COM_QUIT`, `COM_STMT_*`
- Resultset framing on the way back, including the `CLIENT_DEPRECATE_EOF`
  variation

An L4 proxy would need none of this. This is the cost of admission for SQL
rewriting, and it should be understood as such rather than discovered later.

### SQL rewriting as the endgame

This is the part that changes the SDD calculus, and it changes it *favorably* —
see §"Effect on the tool decision" below.

## Consequences worth deciding early

### 1. The parser gap is real

`sqlparser-rs` (`datafusion-sqlparser-rs`) ships `MySqlDialect` and
`GenericDialect`. **It has no Doris or StarRocks dialect.** Those exist in
`sqlglot` (Python) and `polyglot-sql`, neither of which is a drop-in for a Rust
proxy. Doris additionally accepts SQL beyond MySQL syntax, and since 2.1
supports several foreign dialects (Presto, Trino, Hive, Spark, ClickHouse) via
its own converter.

The proxy will therefore encounter SQL it cannot parse. **What happens then is
a specification decision, not an implementation detail.** The two viable
answers:

- **Fail closed** — reject unparseable SQL with an ERR packet. Safe, but breaks
  any client using a Doris extension the parser does not know.
- **Pass through unchanged** — forward verbatim when parsing fails, applying no
  rewrite. Preserves compatibility; means rewrite rules are best-effort and can
  silently not apply.

Pass-through is almost certainly right for a proxy, but it has a sharp
consequence: **a rewrite rule that silently does not fire is invisible.** If
rewriting ever becomes security-relevant (tenant isolation, row filtering),
pass-through-on-parse-failure is a bypass. That tension belongs in the spec
before either path is coded.

A third option worth weighing for a study project: **prefix sniffing** — do not
full-parse at all; classify statements by leading keyword and rewrite textually.
Far less machinery, handles unknown dialects gracefully, and is honest about
being approximate. Full parsing can come later if rules demand it.

### 2. Prepared statements are the hard part

If SQL is rewritten, it must be rewritten at `COM_STMT_PREPARE` time, because
that is where the SQL text lives. `COM_STMT_EXECUTE` carries only a statement ID
and parameters. Consequences:

- **Placeholder positions can shift.** A rewrite that adds, removes, or reorders
  `?` placeholders invalidates the parameter binding the client computed against
  the *original* statement. Rules must either preserve placeholder count and
  order, or the proxy must remap parameters on every execute.
- **Statement IDs are per-connection.** Client, proxy, and each backend
  connection assign independent numeric IDs. ProxySQL solves this with a
  two-tier global↔backend mapping plus content-hash deduplication.
- **Prepare returns metadata** — column definitions and parameter counts. If the
  rewrite changes the projection, the metadata the client receives must
  correspond to what `COM_STMT_EXECUTE` will actually return.

The cheapest correct posture for this exercise is an explicit invariant:
**rewrite rules MUST NOT change placeholder count, order, or result column
shape.** That reduces prepared-statement handling to ID mapping and makes the
whole feature tractable. It is exactly the kind of cross-cutting rule that
belongs in the constitution/invariants artifact.

### 3. Other rewriting traps

- `CLIENT_MULTI_STATEMENTS` means one `COM_QUERY` may carry several
  semicolon-separated statements. Rewrite must handle each, or the capability
  must be refused.
- `LOAD DATA LOCAL INFILE` inverts the data flow — the server asks the *client*
  for a file. Simplest correct answer is to reject it.
- Text protocol (`COM_QUERY`) and binary protocol (`COM_STMT_EXECUTE`) return
  resultsets in different encodings.

## Effect on the tool decision (D2)

This scope makes the case for SDD **stronger**, and specifically strengthens the
case for a living-spec/delta model.

- **Rewrite rules are the most spec-shaped artifact in the entire project.**
  "GIVEN a query matching `<pattern>`, WHEN forwarded, THEN the backend receives
  `<transformed>`" is simultaneously a requirement, an acceptance test, and a
  `proptest` case. There is near-zero translation loss.
- **The rule catalog grows forever**, exactly like the compatibility matrix.
  Append-and-refine against a living spec store, not a sequence of self-contained
  feature folders. This is the delta model's home turf and the thing Spec Kit
  has no answer for.
- **It shrinks the unspecifiable half.** Dropping multi-backend routing, sharding
  and read/write splitting removes much of the pooling-topology and failover work
  that resisted specification. What remains — the relay loop's cancellation
  safety and buffer ownership — is real but narrower. §3a of
  `surveys/domain-fit-rust-proxy.md` shifts correspondingly toward spec-shaped.
- **`unknowns.md` becomes more important, not less.** Every Doris SQL construct
  `sqlparser-rs` chokes on is an empirical discovery with provenance worth
  recording.

## Resolved boundaries

| # | Question | Answer (2026-08-08) |
|---|---|---|
| B1 | What is the rewriting for? | **Tenant/row isolation + table/schema remapping** |
| B3 | Auth model | **Passthrough** — client credentials relayed to Doris |
| B5 | Backend connection model | **1:1 per client connection** — no pooling, no multiplexing |
| B2 | One FE or several? | Deferred; single FE assumed until revisited |
| B4 | Full parse or prefix sniffing? | **Determined by B1 — see C1 below. Full parse, allowlisted.** |

## Derived consequences

These are not new decisions; they follow necessarily from B1/B3/B5.

### C1 — Rewriting is security-relevant, so it must fail closed

Tenant isolation means a rule that silently fails to apply is a **data leak**,
not a missed optimization. Pass-through-on-parse-failure is therefore
unavailable, and prefix sniffing is unavailable — you cannot assert isolation
over a statement you have not fully understood.

This collides with the parser gap in §1: `sqlparser-rs` has no Doris dialect, so
**the proxy will reject SQL that Doris would happily accept.** That is a real
compatibility cost and it is unavoidable given B1. Accept it explicitly.

The correct posture is an **allowlist**: enumerate the statement shapes the
proxy can fully parse *and* prove isolation over; reject everything else with a
clear error. Note this is excellent news for the exercise — an allowlist is more
spec-shaped than a rewriter, and the supported-statement matrix becomes the
project's central living document, growing one delta at a time.

### C2 — Passthrough auth forces backend-first connection ordering

With `mysql_native_password`, the client's response is computed against the
**salt the proxy issued**. The proxy cannot forward that response to Doris,
because Doris issued a *different* salt in its own handshake.

The standard resolution is for the proxy to **dial the backend first, take the
backend's salt, and present it as its own** in the Initial Handshake Packet.
The client's response is then valid against Doris and can be relayed verbatim.

This is a hard ordering constraint on the accept path — and note it is exactly
what the first trial's protocol spec already required for capability
intersection. Two independent requirements converge on the same ordering, which
is a good sign the design is coherent. (That file no longer exists: it belonged
to the hand-written first trial attempt, which was deleted as malformed — see
`milestones/milestone-1.md` §2.3. The surviving trials under `archives/milestone-1/trial/`
target the rewrite path, not the wire protocol.)

`caching_sha2_password` is harder: full authentication requires either TLS or an
RSA key exchange, and the proxy cannot transparently relay it the same way.
Expect to require `mysql_native_password` initially and record that in the
compatibility matrix as a known limitation.

### C3 — Tenant identity must derive from the connection, not from the proxy

Because the proxy holds no separate identity for the client (B3), tenant
identity must be derived from something the handshake already carries:
the **username**, the **default database**, or **connection attributes**
(`CLIENT_CONNECT_ATTRS`). Username is the most robust of the three.

Implication: the tenant↔Doris-user mapping is part of the config schema and is
itself spec-shaped. It also means one Doris user cannot serve two tenants.

## Unresolved — needs an explicit decision

### T1 — The proxy is not a security boundary unless the network makes it one

This is the most important open item and it is not a code question.

With passthrough auth (B3), clients hold **real Doris credentials**. Nothing
stops a client from connecting directly to the Doris FE on port 9030 and
bypassing every isolation rule the proxy enforces. Tenant isolation implemented
in the proxy is therefore only meaningful if direct FE access is blocked at the
network layer.

Three honest postures:

1. **Accept it — this is a study project.** Isolation is a modelling exercise,
   the threat model is "cooperative clients," and the bypass is written down as
   a known, accepted limitation. Entirely legitimate; must be explicit.
2. **Assume network isolation.** State as an environmental precondition that
   only the proxy can reach the FE. Shifts the boundary out of the code, and the
   spec must say so rather than implying the proxy enforces it alone.
3. **Move to proxy-held credentials.** Reverses B3, makes the proxy a real
   security boundary, and introduces a secret store. Also solves C2 (the proxy
   issues its own salt) at the cost of managing Doris service credentials.

**Whichever is chosen belongs in the constitution/invariants artifact**, because
it governs every rewrite rule ever written. Getting this wrong is the difference
between a spec that describes a security control and one that describes a
convention.

---

## Where this landed

Recorded 2026-08-09 when this discussion was closed. **This section decides
nothing.** It is a forwarding address, so a reader who arrives here does not
mistake open questions for abandoned ones. Every item below was settled
elsewhere, in milestone 2, and that other artifact is authoritative.

| Item here | Where it was actually settled |
|---|---|
| §1 fail closed vs pass through | Fail closed. Milestone 2's scope is security-relevant, so the tension flagged here resolved the way this file warned it would |
| §1 prefix sniffing as a cheaper option | Not taken — full parse, as C1 predicted would be forced |
| §2 placeholder-stability invariant | Adopted verbatim as a cross-cutting invariant |
| §3 `CLIENT_MULTI_STATEMENTS` | Capability negotiated away rather than handled |
| B1 what the rewriting is for | Restated, not inherited — row-level filtering by authenticated user |
| B2 one FE or several | Still deferred; single FE assumed |
| B3 passthrough auth · B5 1:1 connections | Carried forward unchanged |
| C1 allowlist over statement shapes | Adopted, including the compatibility cost this file said to accept explicitly |
| C2 dial backend first, relay its salt | Adopted as-is |
| C3 identity from the username | Adopted as-is |
| T1 proxy is not a security boundary | Closed as posture 2 — network isolation is an environmental precondition, stated as such rather than implied |

The reason to keep this file: **C1, C2, C3 and T1 were derived here from first
principles and milestone 2 reached the same answers independently.** That is
some evidence the reasoning was sound rather than lucky — and it is also a
reminder that the analysis was available a milestone before it was used, which
is the more useful lesson.
