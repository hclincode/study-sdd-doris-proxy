## Context

The repository is empty; this change introduces the first code. See `proposal.md` — Why for motivation, and the delta specs under `specs/mysql-proxy/` for the behavior contract.

Two constraints shape every decision below:

1. **A follow-on capability is already known.** Phase 2 will inject a configured row-filter predicate into read-only statements before forwarding them, selected per listener from a table-to-predicate map. This change ships no rewriting, but its structure must make rewriting an added stage rather than a restructuring.
2. **The proxy sits on the latency path of every query.** Anything the proxy does synchronously is added to every statement's response time, and anything it gets wrong about the wire protocol desynchronizes a connection in ways that are hard to diagnose from either end.

The scope decisions that make this tractable — sticky connections, relayed authentication, no TLS, no compression — were settled before this change was written and are recorded as non-goals in the proposal.

## Goals / Non-Goals

**Goals:**

- A connection handler that stays synchronized with the MySQL protocol for the command set real clients use, and that fails loudly rather than silently desynchronizing when it meets something it cannot track.
- A request path shaped as a stage pipeline over whole command packets, where phase 2's rewriter is a new stage and nothing else changes.
- Logging that is bounded and lossy by construction, so that the log sink can never become a source of database unavailability.
- Byte-level pass-through as the default path, with decoding confined to what the specs actually require.

**Non-Goals:**

- Performance tuning beyond avoiding obvious per-row work. Correctness and protocol fidelity come first; this is not a design for a high-throughput proxy.
- A metrics/telemetry export surface. Counters are surfaced through the log file (see Decisions) rather than a separate endpoint.
- Abstracting over database protocols. The design is MySQL-specific throughout.

## Decisions

### One task per connection, sticky 1:1 to the backend

Each accepted client connection gets a task owning both sockets. No connection pool, no shared state between connections beyond the log channel and configuration.

The alternative — pooling and multiplexing backend connections — is what makes proxies valuable at scale, but it requires classifying every command by whether it mutates session state (current database, session and user variables, temporary tables, table locks, open transactions, prepared statements) and pinning or replaying accordingly. That is the largest single source of complexity in production MySQL proxies, it is not required by anything in this change, and phase 2 does not need it either. Sticky connections make the whole class of bug impossible.

The cost is one backend connection per client connection, so the proxy multiplies rather than reduces connection pressure on MySQL. Acceptable given the deployment posture; revisit only if pooling becomes a requirement in its own right.

### Relay authentication rather than terminate it

The proxy forwards connection-phase packets between client and backend, observing them for the fields it needs (negotiated capabilities, username, default database, backend connection id) but never participating in the authentication algorithm.

This is possible only because the phase-2 predicate is per-listener and static: the proxy never needs to know *who* is connecting in order to decide what to do. Had the predicate been identity-derived, the proxy would have to verify clients itself, which means holding credentials or a verifier, implementing `caching_sha2_password` including its RSA path, and becoming a security-relevant credential store. Relaying avoids all of that and works with any authentication plugin the backend supports, including future ones.

The consequence to keep in view: the proxy is transparent to authentication, so the backend's `GRANT`s remain the only access control, and the proxy cannot attribute a statement to anyone the backend would not have attributed it to anyway.

### Capability masking is the mechanism for every "unsupported" feature

Rather than detecting and rejecting TLS, compression, and local-file loading after the fact, the proxy clears their capability bits in the handshake it forwards. Well-behaved clients then never attempt them.

The masked set and its reasons:

| Bit | Why it is cleared |
| --- | --- |
| `CLIENT_SSL` | TLS is out of scope. Without masking, a client that upgrades makes all subsequent traffic opaque to the proxy, silently defeating logging. |
| `CLIENT_COMPRESS` | Compressed packets wrap the normal framing in a second layer; supporting it means implementing a second framing format for no benefit here. |
| `CLIENT_LOCAL_FILES` | Lets the server instruct the client to read a local file. A proxy is the natural place to remove that capability, and nothing in scope needs it. |

Masking is not by itself a control, because a hostile client can set a bit that was not advertised. The specs therefore also require closing any connection whose handshake response asserts a masked bit — masking handles the honest case, the check handles the dishonest one.

The proxy must record the flags *actually* negotiated, not the ones it prefers, because response framing depends on them — `CLIENT_DEPRECATE_EOF` in particular changes the packet structure of every result set. Misreading it is the most likely cause of a desynchronization bug in this design.

### Whole command packets are buffered; responses stream

Client-to-backend command packets are buffered in full before forwarding. This is required anyway to log statement text, and it is what makes phase 2 possible: a rewriter cannot operate on a stream.

Backend-to-client traffic is not buffered as a unit — a large result set must not be accumulated in memory. The response path reads packet by packet, forwards immediately, and maintains only the state machine position plus counters.

### The request path is a pipeline of stages over `Cow<'_, [u8]>`

A stage receives a decoded view of a command and returns either the original bytes borrowed, or replacement bytes owned:

```
  buffered packet ─▶ classify ─▶ [ stage₁ … stageₙ ] ─▶ emit ─▶ backend
                                        │
              phase 1: one stage, observe-only, always returns Borrowed
              phase 2: adds the predicate injector, returns Owned when it rewrites
```

The alternative — a stage signature taking `&Command` and returning `()` — is the natural shape for a logger and is a refactor of every stage the moment one needs to modify the payload. Choosing the `Cow` signature now costs nothing at runtime on the borrowed path and is the single most important structural decision in this change.

### Rows are counted, not decoded

The response state machine must know where a result set ends. It does not need column values to do so, and the specs only require a row *count*.

So in the row phase the proxy inspects each packet's header to decide "terminator or row" and forwards the bytes untouched, never decoding column values. Column definition packets are likewise counted, not parsed. This keeps per-row work to a header check and a forward, and it means result sets containing types or encodings the proxy has never heard of pass through correctly.

The alternative — decoding rows into typed values, as a server-side protocol library would — would make every proxied result set pay a decode-and-re-encode cost for information nothing needs.

### Commands with untrackable responses are refused, not forwarded

Some commands put a connection into a mode the proxy cannot follow — replication streams being the clear example. Forwarding one means the proxy's state machine is wrong from that point on, and it has no way to recover or even to detect it.

Refusing with an ERR packet keeps the failure legible and local: the client gets a clear error, the connection stays usable, and the proxy stays synchronized. The alternative of "forward it and stop tracking" trades a loud, immediate failure for a silent, permanent one.

### Digests come from a purpose-built tokenizer, not a SQL parser

Digest normalization needs to walk a statement's lexical structure — strings, comments, identifiers, numbers — and nothing more. It does not need a grammar, and it must degrade gracefully on statements it cannot fully make sense of.

A full SQL parser (`sqlparser-rs` being the obvious candidate) brings a grammar's worth of dialect surface, and its failure mode on unsupported syntax is "no result" for a statement that MySQL itself accepts. Worse for phase 2: the byte-splice approach needs a reliable *byte offset* into the original statement, which is a property of the tokenizer's spans rather than of an AST, and re-rendering from an AST would discard comments, hints, and formatting the client sent deliberately.

So: a hand-written tokenizer that yields tokens with byte spans, shared by phase 1 (digest normalization) and phase 2 (locating the clause boundary to splice at). The risk is that MySQL's lexical rules have real corners — see Risks.

### Logging is a bounded channel to a dedicated writer task

Connection tasks construct a record and offer it to a bounded channel using a non-blocking send. If the channel is full the record is dropped and a counter incremented; the connection task never awaits the logging path. A single writer task owns the file handle, serializes records as JSON Lines, and uses buffered writes without forcing a sync.

Drop-on-full rather than block-on-full is the decision that makes logging incapable of causing an outage: a slow or full disk degrades observability instead of stalling queries. It also means the log is explicitly not an audit trail, which is recorded in the proposal so that nobody later relies on completeness. If an audit-grade record is ever required, this decision — not the format or the sink — is the one that has to change.

Records are emitted after the response completes so that outcome, row count, and latency are known. A command that never completes because the connection dropped is recorded with that outcome rather than lost.

### Drop counts are surfaced in the log file itself

The specs require discarded records to be visible. Rather than introduce a metrics endpoint — a whole separate surface, its own port, and a dependency — the writer task emits a distinct record type reporting the running discard count. A reader of the file can therefore always tell whether the stream in front of them is complete.

This is a deliberate minimum. If a real metrics surface is wanted later it can be added without changing anything else, and the log-file counter can remain as a durable record.

### Rotation is delegated via reopen-on-hangup

The writer task reopens the configured path on `SIGHUP`. External tooling renames the file and signals; the proxy resumes at the same path.

Building size- or time-based rotation internally means owning naming, retention, and compression — all of which existing tooling does better. Reopen-on-hangup is the conventional contract for exactly this and is a few lines of work.

### Configuration is a TOML file supplied at startup

TOML is idiomatic in Rust, maps cleanly onto `serde`, and the configuration is a small static document. Listeners are the top-level unit, each carrying its own backend target and log destination, which is the shape phase 2 needs when each listener also carries a table-to-predicate map.

Configuration is read once at startup. Reload without restart is not implemented; a restart drops connections, which is acceptable for this stage and avoids designing for consistency between a live connection and a changed configuration.

### Structure for phase 2, without building it

Three things in this change exist because of phase 2, and are cheap now:

- The `Cow` stage signature described above.
- Statement classification and table extraction from the tokenizer, which phase 1 needs for digests and phase 2 needs to select a predicate.
- Preserving packet count as an invariant. Phase 2's guard will be to skip any rewrite that would change the number of packets a command occupies, which keeps sequence identifiers untouched and means sequence renumbering never has to be implemented. Phase 1 simply must not build in an assumption that conflicts with this.

Two rules that belong to phase 2 but are recorded here because they are correctness-critical and easy to get wrong: an injected predicate must parenthesize both the original `WHERE` contents and the injected predicate, or operator precedence silently widens the result set; and the predicate text from configuration must be validated at load as a single complete boolean expression, since it is spliced into queries verbatim.

### Testing is against real client implementations, not only the CLI

The characteristic failure of a protocol proxy is passing tests against one client and breaking against another, because clients negotiate different capabilities and use different command sets — notably, whether they use text queries or prepared statements by default.

The test strategy is therefore a matrix of real clients driven against a containerized MySQL through the proxy, covering at minimum a text-protocol client and a prepared-statement client, plus unit tests over the tokenizer and the response state machine using recorded packet sequences. Docker is available in this environment; no local MySQL client is installed.

## Risks / Trade-offs

**MySQL's lexical rules have sharp corners that a hand-written tokenizer can get wrong** → Backslash escapes in strings, doubled quotes, backtick identifiers with embedded doubled backticks, `#` comments, `-- ` comments requiring trailing whitespace, and version-gated `/*! … */` comments whose contents are executable SQL rather than commentary. A mistake here corrupts digests in phase 1 and, in phase 2, could place a splice inside a string literal. Mitigation: a table-driven corpus of lexical edge cases as unit tests, written before the tokenizer is used by anything; and in phase 2, a rule that the splice offset must land at a token boundary the tokenizer positively identified, never at a computed position.

**Misreading negotiated capability flags desynchronizes result-set framing** → `CLIENT_DEPRECATE_EOF` changes result-set packet structure, and the proxy must use the value actually negotiated rather than a default. Mitigation: capture flags during the connection phase into per-connection state that the response state machine reads explicitly; integration tests against clients that differ in what they negotiate.

**Sticky connections multiply backend connection count** → Every client connection consumes a backend connection for its lifetime, so `max_connections` pressure on MySQL roughly doubles relative to clients connecting directly. Mitigation: document it; treat pooling as a separate future change rather than something to bolt on.

**Full statement text on disk is a data-sensitivity surface** → The log contains literal values, which may be personal or sensitive. Mitigation: create the file with owner-only permissions, and treat rotation and retention as a data-retention policy. Digest-only logging would remove the exposure but was explicitly not wanted.

**The proxy is transparent to authentication and carries no TLS, so the network is the boundary** → Credentials cross the client-to-proxy leg unencrypted, and with a per-listener predicate in phase 2, reachability of a port determines which filter applies. Mitigation: state the trusted-segment requirement in operational documentation. Note that with an unencrypted client leg, a `caching_sha2_password` cache miss takes the RSA public-key path, so passwords are not exposed in the clear — but that relay path must be implemented correctly for authentication to work at all against a default MySQL 8 server.

**Refusing untrackable commands may break a client that legitimately needs one** → A tool that expects to open a replication stream through the proxy will fail. Mitigation: the error is explicit and names the command, and such tools can connect to the backend directly.

## Migration Plan

There is no existing system to migrate. Deployment is: run the proxy with a configuration file, then point application clients at the listener port instead of the database. Rollback is pointing them back at the database directly, with no data or schema change involved in either direction.

The one operational precondition is that clients must tolerate an unencrypted connection; a client configured to require TLS will fail to connect and must be reconfigured before cutover.

## Open Questions

- Which hash function to use for the digest hash. Any stable, non-cryptographic hash satisfies the specs; the choice does not affect structure and can be made when the digester is written.
- Whether the connection-level record should also carry the backend's connection id for correlation with the backend's own logs. Useful, cheap, and additive to the record format.
- Whether to add a metrics export surface later. The design keeps counters in the log file precisely so that this can be deferred without rework.
