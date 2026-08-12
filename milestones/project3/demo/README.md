# Cross-engine row-filter demo

Four cases, one query, two database engines:

```
                     ┌──────────── demo/query.sql ────────────┐
                     │      SELECT * FROM orders ORDER BY id  │
                     └────────────────────────────────────────┘
                                       │
        ┌──────────────────┬───────────┴───────────┬──────────────────┐
        ▼                  ▼                       ▼                  ▼
   case 1-1           case 1-2                case 2-1           case 2-2
   MySQL direct    proxy :13307 → MySQL     Doris direct    proxy :13308 → Doris
        │                  │                       │                  │
     8 rows          3 rows (EU)               8 rows          3 rows (EU)
```

The client is unchanged and unaware. Neither engine is configured for any of
this. One proxy process holds both listeners and one rule: reads of `orders`
are narrowed to `region = 'EU'`.

## Running it

```bash
./demo/01-setup.sh      # BEFORE the presentation — see timing below
./demo/02-direct.sh     # cases 1-1 and 2-1
./demo/03-proxied.sh    # cases 1-2 and 2-2
./demo/01-setup.sh --stop   # afterwards; leaves the engines running
```

### Timing matters

**Run `01-setup.sh` well before you present — never during.** It is the only
slow step, and it is the one that can fail. It does not start the database
engines and does not wait for them: Doris needs several minutes after launch
before it accepts connections, and that is not a wait you want in front of an
audience.

The script ends with a readiness table covering all four cases and exits
non-zero if any of them is wrong. That check exists because **the proxy is not
modified for this demo** — a failure discovered while presenting cannot be
fixed, so the whole point is to discover it early enough to drop a case or
adjust the story.

`02` and `03` are fast, read-only, and safe to re-run as often as you like.

## Prerequisites

| | where | credentials |
| --- | --- | --- |
| MySQL 8 | `127.0.0.1:13306` | `app` / `apppw` |
| Apache Doris | `127.0.0.1:9030` | `root`, no password |
| Docker | for the `mysql` command-line client, which the host does not have |

MySQL comes from the repository's `docker-compose.yml` (`docker compose up -d`).
Doris you supply yourself; the scripts detect it rather than starting it.

Endpoints and credentials live in `demo/env.sh` — change them in one place.

## What each case shows

**`02-direct.sh` — the baseline.** Both engines hold the same eight orders
across `EU`, `US`, and `APAC`. Both return all eight. Nothing is filtering
anything.

**`03-proxied.sh` — the same query, through the proxy.** Three rows from each.
The script prints the statement the engine *actually received*, read back from
the proxy's own log:

```
  you typed:
    SELECT * FROM orders ORDER BY id;

  the engine actually received:
    SELECT * FROM orders WHERE (region = 'EU') ORDER BY id
      — injected by the proxy, never typed by the client
```

Note where the predicate lands: **before** `ORDER BY`, not appended at the end.
Finding that insertion point without misreading a keyword inside a string or a
comment is the part that was actually hard.

## Why two engines is the point

Doris is not a MySQL fork — it is an independent implementation of the same
wire protocol. `01-setup.sh` shows both handshakes side by side:

| | MySQL 8.0.46 | Apache Doris 2.1.0 |
| --- | --- | --- |
| reports itself as | `8.0.46` | `5.7.99` |
| auth plugin | `caching_sha2_password` | `mysql_native_password` |
| capability flags | `0xdfffffff` (31) | `0x0008828c` (6) |
| `CLIENT_DEPRECATE_EOF` | yes | **no** |

That last row has teeth. A server that does not negotiate
`CLIENT_DEPRECATE_EOF` terminates its result sets with explicit EOF packets
instead of an OK packet, so **case 2-2 runs a different branch of the proxy's
response state machine than case 1-2 does**. The demo is not the same trick
twice; it is the same rule applied across two protocol dialects.

## Honest notes

Things worth saying out loud if someone asks:

- **The query is identical; the schema and credentials are not.** Doris needs
  `DISTRIBUTED BY` and a replication setting, and uses different credentials.
  See `seed-mysql.sql` and `seed-doris.sql` — the rows are the same, the DDL is
  not, and pretending otherwise would be dishonest.
- **The filter is best-effort, not a security boundary.** The demo stages the
  working path only. A statement the proxy cannot rewrite — a join, a subquery,
  a `UNION` — is forwarded **unfiltered** and counted, never rejected. Writes
  are not filtered at all. See the "Row filters" section of the repository
  README for the full picture; the backend's `GRANT`s remain the access control.
- **This is a presentation aid, not a test.** `scripts/verify-with-mysql.sh` is
  the regression suite and runs a much broader set of checks.

## Files

```
  query.sql            the one query, read by both live scripts
  seed-mysql.sql       8 rows, MySQL DDL
  seed-doris.sql       the same 8 rows, Doris DDL
  proxy.toml           one proxy, two listeners, one rule
  env.sh               endpoints, credentials, shared helpers
  inspect-server.py    reads a server's handshake and reports what it is
  01-setup.sh          detect, seed, start, verify all four cases
  02-direct.sh         cases 1-1 and 2-1
  03-proxied.sh        cases 1-2 and 2-2
  logs/                proxy output and the JSONL that 03 reads back
```
