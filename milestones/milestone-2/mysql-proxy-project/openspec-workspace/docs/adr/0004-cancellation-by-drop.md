# ADR 0004 — Cancellation is `Drop`; there is no cancellation machinery

- **Status:** Accepted
- **Date:** 2026-08-09
- **Decides:** design D7, "cancellation behaviour when one side drops"
- **Written after the fact.**

## Context

`connection-routing` requires that when a client closes its connection, or the
connection is lost, the proxy closes the corresponding backend connection. A
proxy that leaks a backend connection per abandoned client will exhaust the
frontend's connection limit, and it will do so silently.

The usual toolkit is a `CancellationToken`, a `select!` over both directions, or
a shutdown channel between reader and writer tasks.

## Decision

**None of them. Cancellation is ownership.**

There is no `select!`, no `CancellationToken`, and no timeout anywhere in the
crate. The chain is:

- The session task ends — `run_on` returns, whether from a clean `COM_QUIT`, a
  read error, or a write error.
- `ProxySession` was moved into `run_on`, so it is dropped as that returns.
- `ProxySession` owns `BackendConnection`, which owns the backend `TcpStream`.
- Dropping the stream closes the socket.

Backend loss travels the same path in the other direction: a read from the
backend fails, the error propagates out of the shim method, `run_on` returns, and
the client's socket is dropped.

## Rationale

**This works only because ADR 0002 chose one task.** With a reader and a writer
task, "the session ended" would be an event needing propagation between them, and
`Drop` would not be enough. The two decisions are a pair; changing the topology
reopens this one.

**And only because the command phase is half-duplex.** There is never a point at
which the proxy is waiting on both sockets, so there is nothing for a `select!`
to arbitrate. Adding one would be adding a state machine to express a choice that
is never available.

**The failure this avoids is the silent one.** A leaked backend connection does
not produce an error anywhere; it shows up much later as a frontend that will not
accept new connections. Making the lifetime structural — the socket is a field of
the thing that dies — means there is no path where the session ends and the
connection survives, rather than a path that is currently correct.

Authentication failure is handled the same way but earlier: when Doris rejects
the credentials, the code takes the connection out of its `Option` and drops it
there, rather than waiting for end of session.

## Consequences

- **Not graceful.** A client that disconnects mid-result-set causes the backend
  connection to be dropped with rows still unread, rather than drained. For a
  proxy this is right — the rows have nowhere to go — but it means Doris sees an
  aborted connection rather than a clean `COM_QUIT`, which will appear in its
  logs.
- **No timeouts.** A client that connects and sends nothing holds a task and a
  backend connection indefinitely. A backend that accepts the connection and
  never answers the handshake does the same. Both need an operational
  connection cap and idle timeout; neither is implemented. This is the largest
  gap in this ADR and it is a real one.
- **No server-initiated shutdown.** There is no way to drain sessions for a
  restart; the process exits and every session dies with it.
- Verified rather than assumed: `client_disconnect_releases_the_backend_connection`
  in `tests/session_passthrough.rs` drops the client and waits for the fake
  frontend's connection handler to observe EOF. The assertion is made at the
  backend, not at the proxy — the proxy believing it has closed a socket is not
  the claim worth testing.

## Alternatives considered

| Option | Rejected because |
|---|---|
| `CancellationToken` shared by session tasks | Nothing to cancel: one task, and the only awaits are on sockets that error when the peer goes away |
| `select!` over client and backend readability | The command phase is half-duplex; there is no moment where both are live |
| An explicit `close()` on `BackendConnection` | Adds a path where the session ends without it being called. `Drop` has no such path |
| Idle and connection-phase timeouts | Should exist, and are named above as a gap. Deferred as operational configuration rather than implemented ad hoc — but deferred is not the same as handled |
