# mysql-proxy

A MySQL wire-protocol proxy that records every statement passing through it.

Clients connect to the proxy instead of the database. The proxy relays the
connection to a backend MySQL server, forwards traffic in both directions, and
appends a JSON record for every command to a log file.

```
   client ──▶ proxy ──▶ mysqld
                │
                └──▶ statements.jsonl
```

## Quick start

```bash
cargo build --release

cat > proxy.toml <<'EOF'
[[listener]]
name = "primary"
bind = "127.0.0.1:3307"
backend = "127.0.0.1:3306"
log_file = "/var/log/mysql-proxy/primary.jsonl"
EOF

./target/release/mysql-proxy proxy.toml
```

Then point a client at the proxy instead of the database:

```bash
mysql -h 127.0.0.1 -P 3307 -u app -p --ssl-mode=DISABLED --get-server-public-key shop
```

`--ssl-mode=DISABLED` is required because the proxy does not support TLS (see
[Operational posture](#operational-posture)). `--get-server-public-key` is what
lets `caching_sha2_password` — the MySQL 8 default — complete over an
unencrypted connection: the client fetches the server's RSA key and encrypts its
password with it. The proxy relays those packets without being able to read them.

## Configuration

One TOML file, given as the only argument. Each `[[listener]]` is an independent
front door with its own backend and its own log file.

```toml
[[listener]]
name = "primary"                  # appears in every record from this listener
bind = "127.0.0.1:3307"           # where clients connect
backend = "db.internal:3306"      # where their traffic goes
log_file = "/var/log/mysql-proxy/primary.jsonl"
log_channel_capacity = 8192       # optional; queued records before discarding

# Optional row filters: table -> predicate. See "Row filters" below.
row_filters = { orders = "tenant_id = 7", "shop.invoices" = "org_id = 7" }
```

Configuration is read once at startup. There is no reload — restart to apply
changes, which drops connections.

Startup is fail-loud: the configuration is validated, every log file is opened,
and every port is bound before a single connection is served. Any failure exits
non-zero with a reason.

## Log records

One JSON object per line, appended to `log_file`. Records with `"type":
"command"` describe a statement:

```json
{
  "type": "command",
  "ts": "2026-08-11T14:22:31.481Z",
  "ts_unix_ms": 1786458151481,
  "listener": "primary",
  "connection_id": 42,
  "backend_connection_id": 918,
  "client_addr": "10.0.3.17:52233",
  "username": "app",
  "database": "shop",
  "command": "COM_QUERY",
  "statement": "SELECT * FROM orders WHERE id = 1234",
  "digest": "SELECT * FROM orders WHERE id = ?",
  "digest_hash": "8f2b1c9d4a7e6053",
  "duration_us": 1832,
  "outcome": "result_set",
  "returned_rows": 1,
  "result_sets": 1
}
```

| Field | Meaning |
| --- | --- |
| `connection_id` | Assigned by the proxy, stable for the client connection |
| `backend_connection_id` | The backend's own id, for correlating with its logs |
| `statement` | Complete text as submitted, untruncated, literals included |
| `digest` | Statement with literals replaced, comments stripped, whitespace normalized |
| `digest_hash` | Stable hash of the digest — the field to group by |
| `duration_us` | From forwarding the command to completing its response |
| `outcome` | `ok`, `error`, `result_set`, `prepared`, `statistics`, `no_response`, `refused`, `terminated` |
| `affected_rows` / `returned_rows` | Present according to the outcome |
| `error_code`, `sql_state`, `error_message` | Present when the backend returned an error |
| `digest_unavailable` | Present and `true` when the statement could not be normalized |
| `rewritten` | Present and `true` when a row filter was injected |
| `forwarded_statement` | What was actually sent to the backend, on a rewrite |
| `filter_table` | The table whose rule was applied |
| `filter_skipped` | Why a wanted rewrite did not happen (see [Row filters](#row-filters)) |

The last four are omitted entirely for traffic no rule touched, so records from
an unfiltered listener have exactly the shape they had before row filtering
existed. `statement` always holds what the client submitted, even on a rewrite.

Records with `"type": "dropped"` report discards:

```json
{"type":"dropped","ts":"...","listener":"primary","dropped_total":128,"dropped_since_last":128}
```

### The log is observability, not an audit trail

Records pass through a bounded queue to the writer. When the queue is full —
because the disk is slow, full, or the proxy is busier than the writer — records
are **discarded and counted**, never queued indefinitely and never allowed to
delay a query. A full disk degrades logging; it does not degrade the database.

The consequence is that the file is deliberately incomplete under pressure. The
`dropped` records tell you when and how much, so you can always tell whether the
stream in front of you is whole. Do not treat it as a compliance record.

### Rotation and retention

The proxy reopens its log file on `SIGHUP`, which is the contract `logrotate`
expects:

```
/var/log/mysql-proxy/*.jsonl {
    daily
    rotate 14
    compress
    missingok
    postrotate
        kill -HUP $(cat /run/mysql-proxy.pid)
    endscript
}
```

**Records contain full statement text, including literal values.** Those values
may be personal or sensitive, so the log file is a data-sensitivity surface, not
just a disk-space one. The proxy creates it with owner-only permissions (`0600`);
retention and access are yours to set, and should be driven by a data-retention
policy rather than by how much disk you have.

## Row filters

A listener can append a predicate to reads of a given table, so a client
connecting through it sees only the matching rows without the application
needing to write the filter itself.

```toml
[[listener]]
name = "tenant-7"
bind = "127.0.0.1:3307"
backend = "db.internal:3306"
log_file = "/var/log/mysql-proxy/tenant-7.jsonl"
row_filters = { orders = "tenant_id = 7", invoices = "tenant_id = 7" }
```

```
   client sends:   SELECT * FROM orders WHERE a = 1 OR b = 2
   backend gets:   SELECT * FROM orders WHERE (a = 1 OR b = 2) AND (tenant_id = 7)
                                              ▲              ▲
                     both sides parenthesized, always — without this, AND binds
                     tighter than OR and the filter would widen the result set
```

Rules are matched case-insensitively with backticks stripped. A qualified rule
(`shop.orders`) matches only a qualified reference; a bare rule (`orders`)
matches either, and a qualified rule wins when both could apply. A table with no
rule is never filtered.

Predicates are validated when the proxy starts and must be a single complete
boolean expression. A predicate containing `;`, `?`, a comment, or unbalanced
parentheses is a **boot failure**, not a query-time surprise. Treat the config
file as you would code: its contents are spliced into statements verbatim.

### What gets rewritten

Only a `SELECT` reading exactly one table that has a rule, submitted either
directly or for preparation. Everything else is **forwarded unchanged** and
recorded with a reason:

| `filter_skipped` | Meaning |
| --- | --- |
| `multiple_tables` | A join, or a comma-separated table list |
| `unsupported_structure` | A subquery, `UNION`, CTE, derived table, or index hint |
| `not_select` | A write, DDL, `SHOW`, `CALL`, or anything else |
| `multiple_statements` | More than one statement in a single command |
| `tokenize_failed` | The statement could not be lexed |
| `no_insertion_point` | The clause boundary could not be located |
| `packet_count_changed` | The rewrite would cross a 16 MB packet boundary |

A skip is recorded only when a ruled table is mentioned, so the count measures
filters that were wanted and did not happen — not traffic nobody meant to
filter. Grouping records by `filter_skipped` tells you which construct your real
traffic uses, which is the input to deciding whether to support it.

### This is not a security control

**A statement the proxy cannot rewrite returns unfiltered rows.** That is the
designed behaviour, not a defect: the filter never breaks a query, and it
therefore cannot contain one. Three consequences follow:

- Any skip above — including a statement that merely failed to lex — means the
  client received rows the predicate would have excluded.
- Views, stored procedures, `PREPARE … FROM @variable`, and `HANDLER` hide the
  table name, so no rule can match them.
- Whoever can reach a listener's port gets that listener's filter, and nothing
  else enforces it.

Use it to prevent accidental cross-tenant reads in ordinary application traffic.
**The backend's `GRANT`s remain the only access control**, and should be set as
though the proxy were not there.

### Rolling out a rule safely

The failure mode is silent, so verify before anything depends on it:

1. Add the rule to a non-production listener pointing at the same backend.
2. Run the application's real queries through it.
3. Read the log. `forwarded_statement` shows what each rewrite became, and
   `filter_skipped` shows what the rule failed to cover.
4. Compare row counts against an unfiltered listener for the queries that matter.

## Operational posture

**The proxy must run on a trusted network segment.** Three properties combine to
make that a requirement rather than a recommendation:

- **No TLS.** The proxy strips `CLIENT_SSL` from the handshake, so every
  connection is plaintext. A client configured with `--ssl-mode=REQUIRED` will
  fail to connect, by design; one set to `PREFERRED` (the default) falls back
  silently. Credentials themselves are not exposed — an unencrypted leg pushes
  `caching_sha2_password` onto the RSA public-key path — but query text and
  results cross the wire in the clear.
- **No client authentication.** The proxy holds no credentials and verifies
  nobody. Authentication is relayed end-to-end between client and backend, so
  **the backend's own `GRANT`s remain the only access control.** The proxy adds
  visibility, not authorization.
- **Row filters are best-effort.** See [Row filters](#row-filters): a statement
  the proxy cannot rewrite is forwarded unfiltered.
- **One backend connection per client connection.** Connections are sticky and
  never shared, so session state cannot leak between clients — but connection
  pressure on MySQL roughly doubles compared to clients connecting directly.
  Size `max_connections` accordingly.

### Unsupported by design

| Capability | Behaviour |
| --- | --- |
| TLS | Masked off; `REQUIRED` clients are refused |
| Compression (`CLIENT_COMPRESS`, zstd) | Masked off |
| `LOAD DATA LOCAL INFILE` | Masked off; a backend that requests one anyway ends the connection |
| Query attributes | Masked off, so `COM_QUERY` stays a plain statement |
| Replication commands, `COM_CHANGE_USER` | Refused with an error; the connection stays usable |

Replication commands are refused rather than forwarded because their responses
are streams the proxy cannot follow. Forwarding one would desynchronize the
connection silently; refusing fails loudly and locally. Point such tools at the
database directly.

## Development

```bash
cargo test                        # unit and integration tests, no database needed
cargo clippy --all-targets
./scripts/verify-with-mysql.sh    # against real MySQL 8 in Docker
```

`cargo test` drives the proxy against a scriptable backend in `tests/support`,
which keeps behavioural tests deterministic. That proves the proxy's own logic
but says nothing about how real MySQL and real drivers negotiate, so
`scripts/verify-with-mysql.sh` closes the gap: it starts MySQL 8 via
`docker-compose.yml`, runs the proxy in front of it, and drives traffic through
with the official client — the `caching_sha2_password` RSA path, prepared
statements, multi-result stored procedures, TLS refusal, session isolation, and
log rotation.

### Layout

```
src/
  protocol/     framing, capabilities, connection phase, command classification,
                response state machine
  sql/          tokenizer, digest normalization, statement shape analysis
  pipeline.rs   the request-side stage pipeline
  row_filter.rs predicate validation, rule matching, splicing, the filter stage
  logging/      record types and the writer task
  proxy.rs      accept loop and command loop
  config.rs     TOML configuration
```

The design notes, including why each of the above is shaped the way it is, are in
`openspec/changes/archive/`.
