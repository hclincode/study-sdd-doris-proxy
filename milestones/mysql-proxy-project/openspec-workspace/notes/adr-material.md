# ADR material

Raw notes captured during implementation. **The ADRs have now been written** —
see `docs/adr/` (task 9.1, design D7). This file remains as the scratch record
they were written from, including evidence that did not make it into a decision.
Not a spec, not authoritative; where the two disagree, the ADR is the record and
this is the working-out.

| Decision | ADR |
|---|---|
| Protocol crate choice, D1/D4 feasibility | `docs/adr/0001-wire-protocol-crates.md` |
| Async task topology | `docs/adr/0002-one-task-per-session.md` |
| Buffer ownership | `docs/adr/0003-buffer-ownership.md` |
| Cancellation | `docs/adr/0004-cancellation-by-drop.md` |
| Error enum shape | `docs/adr/0005-error-enum-shape.md` |

## Dependency selection (task 1.3 / 1.4)

Versions verified against the crates.io index on 2026-08-09 with `cargo search`,
not assumed:

| Crate | Version | Role | Status |
|---|---|---|---|
| `tokio` | 1.53 | async runtime | settled |
| `sqlparser` | 0.62 | SQL parse, `MySqlDialect` | settled — mandated by the design |
| `serde` + `toml` | 1.0 / 1.1 | policy config | provisional; format was an open question in `design.md` |
| `thiserror` | 2.0 | error enum | settled |
| `tracing` / `tracing-subscriber` | 0.1 / 0.3 | diagnostics | settled |
| `opensrv-mysql` | 0.7 | server side of the wire protocol | **UNVERIFIED — see below** |
| `mysql_async` | 0.37 | backend client | provisional |
| `proptest` | 1 | property tests | settled |

Alternative considered for the server side: `msql-srv` 0.11. `opensrv-mysql` is
the Databend fork of it and is the more actively maintained line, which is the
only reason it was tried first.

### RESOLVED: does `opensrv-mysql` support design D1 and D4?

D1 requires presenting **Doris's own handshake salt** to the client rather than a
locally generated one, so the client's `mysql_native_password` scramble can be
relayed verbatim. D4 requires **clearing** `CLIENT_MULTI_STATEMENTS` and
`CLIENT_MULTI_RESULTS` in the advertised capability flags.

Answered on 2026-08-09 by reading the vendored crate sources under
`~/.cargo/registry/src/`, not docs.rs. Line references are to those sources.

**D1, server half — supported.**

| Need | Where | Verdict |
|---|---|---|
| Supply our own salt | `opensrv-mysql-0.7.0/src/lib.rs:137` — `AsyncMysqlShim::salt(&self) -> [u8; 20]` | Overridable. Used at `lib.rs:324` (written into the Initial Handshake Packet) and `lib.rs:455` (passed to `authenticate`). |
| Get the client's raw auth bytes | `lib.rs:150` — `authenticate(&self, plugin, username, salt, auth_data)` | `auth_data` is `handshake.auth_response` verbatim (`lib.rs:504-512`, field at `commands.rs:27`). |

Two consequences for the implementation. `salt()` is synchronous and takes
`&self`, so the shim must already hold the backend's salt when it is
constructed — which forces exactly the "dial the backend first" ordering D1
prescribes. And `authenticate` takes `&self`, not `&mut self`, so relaying the
response and recording the identity needs interior mutability.

Worth knowing about the crate: its *default* `salt()` returns a hardcoded
constant (`";X,po_k}>o6^Wz!/kM}N"`, `lib.rs:138`) — the same scramble on every
connection. Harmless here because we always override it, but it is a fair
indication of how much the crate concerns itself with this.

**D1, backend half — NOT supported by `mysql_async` 0.37.** Three independent
blocks, any one of which is sufficient:

1. `mysql_async-0.37.0/src/conn/mod.rs:118` — `nonce: Vec<u8>` is a private field
   of `ConnInner`. `grep -rn "fn nonce" src/` returns nothing: there is no
   accessor. Doris's salt is unreachable.
2. `src/conn/mod.rs:604-605` — the handshake response is
   `auth_plugin.gen_data(self.inner.opts.pass(), &self.inner.nonce)`. The scramble
   is derived from a **plaintext password held in `Opts`**. No API accepts a
   precomputed auth response. (`AuthPlugin::Other(_)` is not an escape hatch:
   `mysql_common-0.37.3/src/packets/mod.rs:1415` returns `None` for it.)
3. `src/conn/mod.rs:1024` (`Conn::new`) and `:1059` (`from_url`) are the only
   public constructors, and both run `handle_handshake()` +
   `do_handshake_response()` internally (`:1043-1045`). There is no
   `Conn::from_authenticated_stream`; the connection phase cannot be split.

**Resolution taken:** the backend connection phase is hand-rolled over a raw
`tokio::net::TcpStream` in `src/session.rs`. D1 is preserved exactly as designed
— no cleartext password path, no proxy-held credentials. This needed **no new
dependency**, because the proxy never *computes* a scramble; that is the whole
point of D1, so no hashing crate is required. `mysql_async` is consequently
unused by the connection phase and is now dead weight unless the result-set relay
keeps it.

**D4 — not controllable, but incidentally satisfied.**
`opensrv-mysql-0.7.0/src/lib.rs:308-313` hardcodes `server_capabilities` as a
local `let` inside `init_before_ssl`. There is no shim hook and no field for it in
`IntermediaryOptions` (`lib.rs:210-216`, which carries only
`process_use_statement_on_query` and `reject_connection_on_dbname_absence`). The
advertised set cannot be changed.

The hardcoded set is `CLIENT_PROTOCOL_41 | CLIENT_SECURE_CONNECTION |
CLIENT_PLUGIN_AUTH | CLIENT_PLUGIN_AUTH_LENENC_CLIENT_DATA |
CLIENT_CONNECT_WITH_DB | CLIENT_DEPRECATE_EOF`. Neither multi-statement flag is
in it, so D4's observable outcome holds — by accident of the crate, not by our
control. A dependency bump could reintroduce either flag silently, so
`tests/session_passthrough.rs` reads the handshake off the wire and asserts both
bits are clear.

### D4's stated rationale is wrong, and the gap is real

`design.md:93` argues that "a client that never negotiates multi-statement
support cannot send one", and `design.md:127` lists multi-statement smuggling as
**closed** by D4. Neither holds with this crate:

- Clearing the server's advertised bit is advisory. Nothing in `opensrv-mysql`
  checks whether the client honoured it — `run()` at `lib.rs:562-628` hands the
  whole `COM_QUERY` payload to `on_query` as one string regardless of embedded
  `;`. A client that sets `CLIENT_MULTI_STATEMENTS` in its own handshake response
  anyway and sends `SELECT 1; SELECT 2` reaches `on_query` intact.
- The proxy cannot even inspect what the client claimed: `client_capabilities` is
  `pub(crate)` (`lib.rs:230`).

So multi-statement rejection has to be enforced at the SQL level. It is:
`analyze::parse_single` refuses anything that parses to more than one statement
with `RefusalReason::MultiStatement`. The design document should be corrected to
say the control lives there, with D4 as defence in depth rather than the
mechanism. Recorded here rather than edited into `design.md`, which this task
does not own.

As a second line of defence the **backend** connection is negotiated without
either flag (`desired_backend_capabilities` in `src/session.rs`), so Doris itself
would refuse a multi-statement payload that somehow got past analysis.

### Consequential narrowing: only `mysql_native_password` is accepted

Relaying a scramble works only for a single-round-trip plugin. If the backend
names anything else — `caching_sha2_password`'s full-auth exchange, say — the
proxy would have to answer with data derived from the password it deliberately
does not hold. `BackendConnection::connect` therefore refuses the session rather
than half-relaying. Fail-closed, consistent with the project's posture, but it is
a compatibility limit that belongs in operator documentation.

### `async-trait` was a missing direct dependency — now added

`AsyncMysqlShim` is declared with `#[async_trait]` (`lib.rs:107`), but
`async-trait` was only a transitive dependency, so `use async_trait::async_trait`
did not resolve. The impl in `src/session.rs` was first written in the desugared
form the macro emits — `Pin<Box<dyn Future + Send + 'async_trait>>` with explicit
`'life0..'lifeN` bounds on every method. That compiles and is behaviourally
identical, but it is unreadable, and the authentication path is the code most in
need of review.

`async-trait = "0.1"` was therefore added to `Cargo.toml` and the impl collapsed
back to ordinary `async fn`s. Worth recording as a general point: a crate whose
public trait is declared with `#[async_trait]` forces that macro on every
implementor, so `async-trait` is effectively part of `opensrv-mysql`'s public API
without appearing in its documentation.

### `mysql_async` is now unused, and its future is unresolved

The connection phase does not use `mysql_async` at all — it is hand-rolled over
`tokio::net::TcpStream` (see above), and `src/session.rs` also does its own
`COM_QUERY` / `COM_INIT_DB` and text-protocol result-set relay on top of that.
Nothing in the crate currently depends on `mysql_async`.

It is deliberately **left in `Cargo.toml` rather than removed.** Whether anything
still needs a full-featured backend client is not yet decided, and removing a
dependency is a call for whoever ends up owning the forwarding path, not for the
session work that happened to stop needing it. Revisit before the change is
archived: either a use is found, or it comes out.

### Startup ordering is enforced by types, not by comment

`policy-config` requires that invalid configuration prevent startup *before any
listener binds*. In `src/main.rs` this is not left to the order statements happen
to appear in: `serve` takes a `PolicySet` **by value**, and the only way to obtain
one is a successful `PolicySet::load_from_path`. Moving the bind above the load is
not a refactor that merely reads badly — there would be no `PolicySet` to pass, so
it does not compile.

`tests/session_startup_ordering.rs` runs the real binary as a subprocess and
checks the ordering behaviourally, because an exit status alone cannot
distinguish "never bound" from "bound, then failed": each negative case also
asserts that nothing ever accepted a connection on the listen address, and a
positive control asserts that a valid file *does* bind, so the negatives are not
passing for an unrelated reason.

One consequence worth stating: **proxy settings cannot live in the policy file.**
That file rejects unknown keys and unknown sections by design, so that a
misspelled `[[policy]]` cannot silently yield zero policies and disable all
filtering. Adding a `[proxy]` section to it would fail to load. Listen and backend
addresses are command-line arguments instead.

### `QueryResultWriter::start` and dynamically decoded columns

`opensrv-mysql`'s `QueryResultWriter::start` takes `&'a [Column]` where `'a` is
the writer's own lifetime parameter (`src/resultset.rs:160`), which reads as
though it forbids passing columns decoded from the backend inside `on_query`. It
does not: `QueryResultWriter<'a, W>` is covariant in `'a` (it holds only
`&'a mut PacketWriter<W>`), so the writer is reborrowed at the local lifetime and
`start(&local_columns)` compiles. Recorded because it looks like a blocker and
cost time to establish that it is not.
