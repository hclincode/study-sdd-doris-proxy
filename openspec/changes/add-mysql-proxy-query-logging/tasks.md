## 1. Project setup

- [ ] 1.1 Initialize the Rust binary crate with edition, MSRV, and lint configuration
- [ ] 1.2 Add runtime dependencies: `tokio` (net, io, sync, signal, macros, rt-multi-thread), `bytes`, `serde`, `serde_json`, `toml`
- [ ] 1.3 Create the module skeleton: `config`, `protocol` (framing, connection phase, command phase), `sql` (tokenizer, digest), `logging`, `proxy`
- [ ] 1.4 Add a `docker-compose` or equivalent definition bringing up a MySQL 8 server for local and integration testing

## 2. Configuration

- [ ] 2.1 Define the TOML configuration model: a list of listeners, each with bind address, backend address, and log file path
- [ ] 2.2 Implement loading and deserialization with a clear error for a malformed or missing file
- [ ] 2.3 Validate configuration at startup: unique bind addresses, resolvable backend addresses, non-empty log path
- [ ] 2.4 Wire configuration into a startup path that fails loudly and exits non-zero before any listener binds

## 3. Packet framing

- [ ] 3.1 Implement the packet codec: decode the length and sequence header, expose payload and sequence id, encode outbound packets
- [ ] 3.2 Handle payloads that span multiple packets because they exceed the maximum packet size, preserving packet count and sequence ids on forward
- [ ] 3.3 Implement length-encoded integer and length-encoded string readers needed by the connection and command phases
- [ ] 3.4 Unit-test the codec against recorded byte sequences, including a maximum-size payload boundary and a split payload

## 4. Connection phase

- [ ] 4.1 Parse the backend's initial handshake, capturing server capabilities, salt, backend connection id, and default auth plugin
- [ ] 4.2 Clear `CLIENT_SSL`, `CLIENT_COMPRESS`, and `CLIENT_LOCAL_FILES` from the capabilities before re-encoding the handshake for the client
- [ ] 4.3 Parse the client's handshake response, capturing negotiated capabilities, username, and default database
- [ ] 4.4 Close the connection when the client's response asserts a capability the proxy did not advertise
- [ ] 4.5 Relay auth-switch requests, auth-more-data packets, and the public-key exchange without interpreting plugin payloads
- [ ] 4.6 Store negotiated per-connection state, including `CLIENT_DEPRECATE_EOF`, for use by the response state machine
- [ ] 4.7 Forward the backend's terminal OK or ERR to the client and transition to the command phase, or tear down on ERR

## 5. Command and response handling

- [ ] 5.1 Buffer each client command packet in full and classify it by command byte
- [ ] 5.2 Extract statement text for the command types that carry SQL
- [ ] 5.3 Refuse commands whose response framing cannot be tracked by returning an ERR packet without forwarding, keeping the connection usable
- [ ] 5.4 Implement the response state machine: OK, ERR, and result set (column count, column definitions, rows, terminator), honoring `CLIENT_DEPRECATE_EOF`
- [ ] 5.5 Continue reading result sets while the terminator signals more results remain
- [ ] 5.6 Count rows by inspecting packet headers only, forwarding row bytes without decoding column values
- [ ] 5.7 Capture per-command outcome: affected rows, returned rows, error code and SQL state
- [ ] 5.8 Terminate both connections if the backend sends a local-file request
- [ ] 5.9 Unit-test the state machine against recorded response sequences for each outcome, with `CLIENT_DEPRECATE_EOF` both set and clear

## 6. Stage pipeline

- [ ] 6.1 Define the command representation passed to stages and the stage trait returning `Cow<'_, [u8]>`
- [ ] 6.2 Implement pipeline execution: run stages in order, emit borrowed bytes unchanged, re-encode only when a stage returns owned bytes
- [ ] 6.3 Implement the observe-only logging stage as the single registered stage
- [ ] 6.4 Unit-test that a borrowed return forwards the original bytes verbatim, and that an owned return re-encodes correctly

## 7. SQL tokenizer and digest

- [ ] 7.1 Implement the tokenizer over MySQL lexical rules: single- and double-quoted strings with backslash and doubled-quote escaping, backtick identifiers with doubled backticks, numeric, hex, and bit literals
- [ ] 7.2 Handle all comment forms, including `#`, `-- ` requiring trailing whitespace, `/* */`, and version-gated `/*! */` whose contents are executable SQL
- [ ] 7.3 Emit tokens carrying byte spans into the original statement
- [ ] 7.4 Build the normalizer: replace literals with a placeholder, collapse literal lists to one placeholder, strip comments, normalize whitespace
- [ ] 7.5 Compute a stable hash over the normalized digest
- [ ] 7.6 Return an explicit "not normalizable" outcome rather than failing, so the record is still emitted
- [ ] 7.7 Write a table-driven corpus of lexical edge cases covering every rule in 7.1 and 7.2
- [ ] 7.8 Test that statements differing only in literals share a digest, that structurally different statements do not, and that literal lists of differing length collapse alike

## 8. Logging pipeline

- [ ] 8.1 Define the log record type and its JSON serialization: timestamp, listener, connection id, client address, username, default database, command type, statement text, digest and hash, latency, outcome, row counts, error code and SQL state
- [ ] 8.2 Create the bounded channel and implement non-blocking send from connection tasks, incrementing a discard counter when full
- [ ] 8.3 Implement the writer task: own the file handle, append JSON Lines, use buffered writes without forcing a sync
- [ ] 8.4 Open or create the log file at startup with owner-only permissions, appending to an existing file, failing startup if it cannot be opened
- [ ] 8.5 Emit a discard-count record so a reader can detect an incomplete stream
- [ ] 8.6 Keep proxying and continue discarding when a write fails, including on a full filesystem, resuming when writes succeed
- [ ] 8.7 Reopen the configured path on `SIGHUP`, continuing to proxy and discarding records if the reopen fails
- [ ] 8.8 Flush records already accepted into the channel on clean shutdown
- [ ] 8.9 Emit a record for a command terminated by connection teardown before its response completed

## 9. Connection lifecycle

- [ ] 9.1 Bind each configured listener and spawn a task per accepted connection
- [ ] 9.2 Dial the backend on accept, returning an ERR packet to the client and closing cleanly when the backend is unreachable
- [ ] 9.3 Assign a proxy connection id stable for the connection's lifetime
- [ ] 9.4 Tear down both sockets together on client quit, on either side closing, and on any protocol error
- [ ] 9.5 Implement clean shutdown: stop accepting, close connections, flush the logging path

## 10. Integration testing

- [ ] 10.1 Build a harness that starts the proxy against a containerized MySQL and runs client sessions through it
- [ ] 10.2 Verify authentication succeeds end-to-end against a `caching_sha2_password` user, exercising the public-key path on an unencrypted connection
- [ ] 10.3 Verify a text-protocol client: queries, errors, empty result sets, large result sets, multi-megabyte statements
- [ ] 10.4 Verify a prepared-statement client, confirming the binary protocol passes through and is logged
- [ ] 10.5 Verify a client that prefers TLS connects unencrypted, and that a client requiring TLS fails without hanging the proxy
- [ ] 10.6 Verify session isolation: two concurrent clients do not observe each other's database, session variables, or temporary tables
- [ ] 10.7 Verify log output: one record per command, correct row counts and error codes, full statement text, well-formed JSON Lines
- [ ] 10.8 Verify lossiness under load: saturate the channel and confirm queries are unaffected and discards are reported
- [ ] 10.9 Verify rotation: rename the file, send `SIGHUP`, confirm writing resumes at the configured path

## 11. Documentation

- [ ] 11.1 Write a README covering the configuration format, how to run the proxy, and a worked example
- [ ] 11.2 Document the operational posture: trusted network segment required, no TLS, backend `GRANT`s remain the only access control
- [ ] 11.3 Document the log record schema and state plainly that the log is lossy observability, not an audit trail
- [ ] 11.4 Document the log file's data-sensitivity implications and the expectation that rotation and retention are configured externally
