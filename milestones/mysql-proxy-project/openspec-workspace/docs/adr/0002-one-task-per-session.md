# ADR 0002 — One task per session, and the backend connection behind a mutex

- **Status:** Accepted
- **Date:** 2026-08-09
- **Decides:** design D7, "async task topology per session"
- **Written after the fact.**

## Context

Each client session owns exactly one backend connection (`connection-routing`:
no pooling, no multiplexing). Something has to decide how many async tasks that
involves and who owns the two sockets.

The obvious options in a proxy are shaped by whether the protocol is
full-duplex. MySQL's command phase is not: the client sends one command and
waits, the server answers, and only then may the client send again. There is no
point in a session at which both directions carry traffic the proxy must
interleave.

## Decision

**One `tokio` task per client session.** `ProxyServer::run` accepts in a loop and
spawns a single task per connection (`src/session.rs`, the only `tokio::spawn` in
the crate). That task calls `AsyncMysqlIntermediary::run_on(session, reader,
writer)` and stays there until the session ends.

The backend connection lives inside `ProxySession` as
`Mutex<Option<BackendConnection>>`, alongside `Mutex<Option<SessionIdentity>>`
and `Mutex<Option<String>>` for the current database.

## Rationale

**No second task, because there is no concurrency to express.** A reader task and
a writer task joined by a channel is the standard shape for a full-duplex proxy.
Here it would add a channel, a shutdown protocol between the two halves, and a
state machine, in exchange for interleaving that the protocol forbids. It would
also make ADR 0004's cancellation story materially harder: with one task, "the
session ended" and "the task returned" are the same event.

**The mutexes are not for concurrency.** Nothing contends for them — a session
runs on one task and the protocol is serialized. They exist because
`AsyncMysqlShim::authenticate` takes `&self`, not `&mut self`
(`opensrv-mysql-0.7.0/src/lib.rs:150`), and authentication is precisely where the
backend connection must be used and the identity recorded. Interior mutability is
forced by the trait signature, not chosen for parallelism. `tokio::sync::Mutex`
rather than `std::sync::Mutex` because the guard is held across `.await` points
while talking to Doris.

**`Option<BackendConnection>` rather than `BackendConnection`.** Taking the
connection out and dropping it is how a failed authentication releases the socket
immediately rather than at end of session. The `None` state means "this session
had a backend and no longer does", which the command paths check and answer with
an error rather than a panic.

**Ownership makes the 1:1 rule structural.** `ProxySession::new` takes a
`BackendConnection` **by value**. A session cannot be constructed without exactly
one backend connection, and has no method that acquires a second. The rule is not
enforced by review.

## Consequences

- A slow Doris response blocks only its own session. No shared runtime state
  exists between sessions beyond the `Arc<dyn StatementGate>`, which is
  immutable.
- Backpressure is implicit: the session task cannot read the next client command
  until it has finished writing the previous response.
- An unauthenticated client can cause a backend connection to be opened, because
  D1 dials the backend before the client authenticates. This is a
  denial-of-service consideration noted in `design.md`'s Risks; mitigating it
  needs an accept-rate limit and a connection cap, which are operational settings
  and are **not implemented**.
- There is no per-session timeout. A client that connects and never sends holds
  one task and one backend connection indefinitely. Also unmitigated, and the
  same operational setting would cover it.

## Alternatives considered

| Option | Rejected because |
|---|---|
| Reader task + writer task joined by a channel | Buys interleaving the protocol forbids; costs a shutdown protocol and complicates cancellation |
| A task per backend connection, shared via channels | Would make pooling expressible, which the spec forbids — the topology would permit a violation the design rules out |
| `std::sync::Mutex` | The guard is held across `.await` while talking to Doris |
| No mutex; restructure to `&mut self` | Not available: the trait method is `&self` and is upstream |
