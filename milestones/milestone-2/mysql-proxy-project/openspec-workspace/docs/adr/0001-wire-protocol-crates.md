# ADR 0001 — `opensrv-mysql` on the client side, a hand-rolled connection phase on the backend side

- **Status:** Accepted
- **Date:** 2026-08-09
- **Decides:** design D7, "which server-side MySQL protocol crate exposes the raw handshake salt and capability negotiation that D1 and D4 require, versus needing that layer hand-rolled"
- **Written after the fact**, from evidence gathered during implementation. Raw notes in `notes/adr-material.md`.

## Context

Design D1 requires passthrough authentication in which the proxy never sees a
plaintext password: it dials Doris first, presents **Doris's own** handshake salt
to the client as its own, and relays the client's `mysql_native_password`
scramble onward verbatim. Design D4 requires the advertised capability flags to
have `CLIENT_MULTI_STATEMENTS` and `CLIENT_MULTI_RESULTS` cleared.

Both need the wire-protocol libraries to expose handshake construction rather
than own it. `design.md` listed this as an open question. `Cargo.toml` provisionally
carried `opensrv-mysql` 0.7 for the server side and `mysql_async` 0.37 for the
backend side, both unverified.

The question was settled by reading the vendored crate sources under
`~/.cargo/registry/src/`, not the documentation. Line numbers below refer to
those sources.

## Decision

Use `opensrv-mysql` 0.7 for the client-facing half. **Hand-roll the backend
connection phase over a raw `tokio::net::TcpStream`** in `src/session.rs`.

## Rationale

### The client half is supported

| Requirement | Where | Verdict |
|---|---|---|
| Supply our own salt | `opensrv-mysql-0.7.0/src/lib.rs:137` — `AsyncMysqlShim::salt(&self) -> [u8; 20]` | Overridable trait method, used at `lib.rs:324` when writing the Initial Handshake Packet and again at `lib.rs:455` when verifying |
| Obtain the client's raw auth bytes | `lib.rs:150` — `authenticate(&self, plugin, username, salt, auth_data) -> bool` | `auth_data` is the client's `auth_response` verbatim (`lib.rs:504-512`; field at `commands.rs:27`) |

Two shapes in the code follow directly from those signatures. `salt()` is
synchronous and takes `&self`, so the shim must already hold the backend's salt
when it is constructed — which forces exactly the "dial the backend first"
ordering D1 prescribes, rather than merely permitting it. And `authenticate`
takes `&self`, not `&mut self`, so relaying the response and recording the
resulting identity needs interior mutability; hence the `Mutex` fields on
`ProxySession` (see ADR 0002).

Worth recording about this crate: its *default* `salt()` returns a hardcoded
constant string, identical on every connection (`lib.rs:138`). Harmless here
because the implementation always overrides it, but it indicates how much
attention the crate gives this area, and it is a reason to keep reading its
source rather than trusting its behaviour.

### The backend half is not supported by `mysql_async` 0.37

Three independent blocks, any one of which is sufficient:

1. `mysql_async-0.37.0/src/conn/mod.rs:118` — `nonce: Vec<u8>` is a private field
   of `ConnInner`. `grep -rn "fn nonce" src/` returns nothing: there is no
   accessor, public or otherwise. Doris's salt is unreachable.
2. `src/conn/mod.rs:604-605` — the handshake response is built as
   `auth_plugin.gen_data(self.inner.opts.pass(), &self.inner.nonce)`. The
   scramble is derived from a **plaintext password held in `Opts`**. No API
   accepts a precomputed auth response. `AuthPlugin::Other(_)` is not an escape
   hatch: `mysql_common-0.37.3/src/packets/mod.rs:1415` returns `None` for it.
3. `src/conn/mod.rs:1024` (`Conn::new`) and `:1059` (`from_url`) are the only
   public constructors, and both run `handle_handshake()` and
   `do_handshake_response()` internally (`:1043-1045`). There is no
   `Conn::from_authenticated_stream`. The connection phase cannot be split.

Using `mysql_async` would therefore have required the proxy to hold every user's
plaintext password — the alternative D1 explicitly rejects, and a much larger
security surface than the feature justifies.

### Hand-rolling costs no dependency

This is the part that made the decision easy, and it is worth stating plainly
because it is counter-intuitive: **the proxy never computes a scramble.** That is
the whole point of D1. So the hand-rolled connection phase needs no hashing or
crypto crate — only `tokio` for the socket, and `CapabilityFlags`, which
`opensrv-mysql` already re-exports (`lib.rs:38`). What had looked like the
expensive option turned out to add nothing to the dependency graph.

## Consequences

- **`mysql_async` is now unused.** It is retained in `Cargo.toml` pending a
  decision on whether anything still needs a full-featured backend client. An
  unused dependency carried into the canonical spec is a small lie about what the
  project needs; it should be removed or justified before archive.
- **`async-trait` became a direct dependency.** `AsyncMysqlShim` is declared with
  `#[async_trait]` (`lib.rs:107`), so every implementor must use that macro. The
  impl was first written in the desugared form — `Pin<Box<dyn Future + Send + 'async_trait>>`
  with explicit `'life0..'lifeN` bounds on each method — which compiles and is
  behaviourally identical but is unreadable, and this is the code most likely to
  be read closely by someone checking the security property. Note for anyone
  repeating the desugaring: `on_close` is declared `where W: 'async_trait`, and an
  implementor must repeat that bound; it is the one mention the macro cannot
  remove. Generalising: a crate whose public trait is declared with
  `#[async_trait]` makes that macro part of its public API without saying so in
  its documentation.
- **Only `mysql_native_password` is accepted from the backend.** It is the one
  plugin whose exchange is a single round trip over a server-chosen salt, which
  is exactly what gets relayed. `caching_sha2_password`'s full-auth exchange would
  need the password the proxy deliberately never holds, so
  `BackendConnection::connect` refuses the session rather than half-relaying.
  Milestone 1 reached the same conclusion independently
  (`discussions/milestone-1/02-proxy-scope.md` §C2). This is a stated
  compatibility limit, not a bug, and belongs in operator documentation.
- **The proxy owns MySQL packet framing.** Roughly 200 lines of bounds-checked
  parsing that would otherwise have been someone else's problem, and which
  consumes bytes the proxy did not produce — see invariant 2 and ADR 0003.

### D4 is satisfied, but not by us

`opensrv-mysql` hardcodes the advertised capability set as a local `let` inside
`init_before_ssl` (`lib.rs:308-313`). There is no shim hook and no field for it in
`IntermediaryOptions` (`lib.rs:210-216`). **The advertised flags cannot be set.**

The hardcoded set omits both multi-statement flags, so D4's observable outcome
holds — by accident of this crate, not by any control of ours. A version bump
could reintroduce either flag silently. `tests/session_passthrough.rs`'s
`advertised_capabilities_exclude_multi_statement` reads the flags back off the
handshake bytes so that such a bump breaks the build rather than the control.

This also disproved D4's original rationale. Clearing an advertised bit is
advisory: `opensrv-mysql` never checks whether the client honoured it —
`run()` at `lib.rs:562-628` hands the whole `COM_QUERY` payload to `on_query`
regardless of embedded `;` — and the proxy cannot even inspect what the client
claimed, since `client_capabilities` is `pub(crate)` (`lib.rs:230`).
Multi-statement rejection therefore has to happen at the SQL level, and does
(`analyze::parse_single`). `design.md`'s D4 has been corrected accordingly. The
backend connection is additionally negotiated without either flag, so Doris
itself would refuse anything that got that far.

## Alternatives considered

| Option | Rejected because |
|---|---|
| `mysql_async` for the backend connection | Cannot relay a precomputed auth response; would require the proxy to hold plaintext passwords |
| `msql-srv` 0.11 for the client side | `opensrv-mysql` is the Databend fork of it and the more actively maintained line. Not separately evaluated, since `opensrv-mysql` met the requirements |
| Fork `opensrv-mysql` to expose capability flags | Disproportionate: the flags we need are already absent, and a fork is a maintenance liability for a property a test can pin |
| Revisit D1 — generate our own salt and require `mysql_clear_password` | A design change requiring sign-off, not an implementation workaround. It would put every user's plaintext password inside the proxy and would be safe only under mandatory TLS |
