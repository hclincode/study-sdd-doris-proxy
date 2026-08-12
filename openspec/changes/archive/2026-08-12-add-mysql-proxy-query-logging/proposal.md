## Why

SQL traffic reaching MySQL is currently opaque: there is no vantage point between application clients and the database where every statement can be observed, attributed, and measured without modifying application code or enabling per-server logging on the database itself.

A MySQL wire-protocol proxy provides that vantage point. This change delivers the observability half — a proxy that transparently relays client connections to a backend MySQL server and records every statement that passes through. It is deliberately scoped to observation only, but establishes the structural seams required by the planned follow-on capability: injecting a configured row-filter predicate into read-only statements before forwarding them.

## What Changes

- **New Rust service** (`tokio`-based) that listens on one or more configured TCP ports and proxies the MySQL client/server protocol to a configured backend MySQL server.
- **Connection model**: sticky 1:1 — each accepted client connection opens and owns exactly one backend connection for its lifetime. No pooling, no multiplexing, no session-state tracking.
- **Authentication relay**: the proxy does not authenticate clients and holds no credentials. It observes and forwards handshake packets so authentication remains end-to-end between client and backend, including the `caching_sha2_password` RSA public-key exchange.
- **Capability masking**: the proxy strips `CLIENT_SSL`, `CLIENT_COMPRESS`, and `CLIENT_LOCAL_FILES` from the backend's advertised capabilities before forwarding the handshake, so every connection is plaintext, uncompressed, and cannot be used to trigger a client-side file read.
- **Statement logging**: every command is decoded far enough to record the statement text, a normalized digest, the responding outcome (OK / error / result set), row count, and wall-clock latency. Records are written as JSON Lines to a configured file.
- **Bounded, lossy logging**: log records pass through a bounded channel to a dedicated writer task. When the channel is full, records are dropped and counted rather than blocking query traffic. The log file is an observability record, explicitly **not** an audit trail.
- **Rewrite seam (structure only, no behavior)**: the request path is a pipeline of stages over whole buffered command packets, where a stage may return either the original bytes or replacement bytes. Phase 1 ships exactly one stage, which observes and returns the input unchanged.

**Non-goals for this change** — each is a deliberate exclusion, not an oversight:

- TLS in any form. Clients requiring TLS will fail to connect, by design.
- SQL rewriting or predicate injection (the planned follow-on).
- Connection pooling, multiplexing, read/write splitting, failover, or sharding.
- Authenticating or authorizing clients; the backend's own `GRANT`s remain the sole access control.
- Compressed protocol, `LOAD DATA LOCAL INFILE`, and multi-statement support.

## Capabilities

### New Capabilities

- `mysql-proxy/protocol-relay`: Accepting client connections, relaying and masking the MySQL handshake, framing packets in both directions, tracking the command/response state machine, and tearing down connection pairs cleanly.
- `mysql-proxy/query-logging`: Deciding what is recorded for each statement, the record format and destination file, digest normalization, and the drop-on-full behavior of the logging path.

### Modified Capabilities

None — this is the first capability in the project.

## Impact

- **New codebase.** The repository currently contains no source code. This change introduces the initial Rust crate, its module layout, and its dependency set.
- **Expected dependencies**: `tokio` (async runtime, TCP), `bytes` (packet buffers), `tokio-util` (codec framing), `serde`/`serde_json` (log records), plus a configuration format crate. A SQL tokenizer is needed for digest normalization; whether that is a dependency or hand-written is a design decision.
- **No existing systems affected.** Deployment requires routing clients at the proxy's listener port instead of the database directly.
- **Operational posture**: because TLS is excluded and clients are not authenticated by the proxy, the proxy and its clients must sit on a trusted network segment. Network reachability of a listener port is the effective boundary, and the follow-on row-filter capability will inherit that property.
- **Data sensitivity**: the log file contains full statement text, including literal values that may be personal or sensitive. Its permissions, rotation, and retention are a data-retention concern rather than only disk hygiene.
