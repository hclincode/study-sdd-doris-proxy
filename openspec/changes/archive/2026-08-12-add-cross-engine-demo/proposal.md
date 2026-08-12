## Why

The proxy's row filtering has been verified against MySQL 8 and nothing else, and there is no way to *show* it working to anyone — the existing verification script prints `PASS`/`FAIL`, which proves correctness but demonstrates nothing.

A live demo is planned that runs the same query four ways: directly against MySQL, through the proxy to MySQL, directly against Apache Doris, and through the proxy to Doris. A spike on 2026-08-12 confirmed all four cases already work with the proxy binary unchanged.

The spike also showed the demo proves more than "the filter works". Doris is an independent implementation of the MySQL wire protocol, not a MySQL derivative:

| | MySQL 8.0.46 | Apache Doris 2.1.0 |
| --- | --- | --- |
| reports itself as | `8.0.46` | `5.7.99` |
| auth plugin | `caching_sha2_password` | `mysql_native_password` |
| capability flags | `0xdfffffff` (all 28) | `0x0008828c` (6) |
| `CLIENT_DEPRECATE_EOF` | advertised | **not advertised** |
| result-set terminator | OK packet | explicit EOF packets |

Because Doris does not negotiate `CLIENT_DEPRECATE_EOF`, proxying it runs the *other* branch of the response state machine — the one that until the spike had only hand-built unit-test packets behind it. The demo is therefore also the first evidence that the proxy handles a second protocol dialect.

## What Changes

- **New `demo/` directory** containing everything needed to run the presentation. No changes to the proxy, its configuration format, or any existing script.
- **Three scripts**, split along the demo's real timeline:
  - `01-setup.sh` — run well before the presentation. Verifies both engines are reachable, seeds identical data into each, starts the proxy, then **smoke-tests all four cases** and prints a readiness table.
  - `02-direct.sh` — cases 1-1 and 2-1: the query straight to each engine, returning every row.
  - `03-proxied.sh` — cases 1-2 and 2-2: the same query through the proxy, returning only the filtered rows, alongside the statement the proxy actually forwarded.
- **One shared `query.sql`**, read by both `02` and `03`, so the claim that all four cases run identical SQL is verifiable rather than asserted.
- **Per-engine seed files**, because the DDL genuinely differs: Doris requires `DISTRIBUTED BY` and `replication_num`. The demo states this rather than implying the schemas match.
- **A proxy config with two listeners**, one per backend, both filtering `orders` by `region`, demonstrating that a single process handles both engines.
- **A README** covering how to run it, what each case shows, and the timing constraint.

**Non-goals:**

- Any change to the proxy. The spike established none is needed, and fixing Doris-related problems in Rust is explicitly out of scope for this work.
- Adding these cases to CI or the existing verification script. This is a one-time presentation aid; `scripts/verify-with-mysql.sh` remains the regression suite.
- Container orchestration. The scripts detect an environment that is already running rather than creating one.
- Filtering anything but reads, or demonstrating skip behaviour. The demo shows the working path; the best-effort limitations are documented in the README but not staged.

## Capabilities

No capability is added or modified. This change adds demonstration scripts that exercise the proxy's existing, already-specified behaviour; the binary, its configuration schema, and every requirement under `openspec/specs/mysql-proxy/` are untouched. The change declares `skip_specs: true` for that reason.

## Impact

- **New files only**, all under `demo/`. Nothing existing is edited.
- **Depends on an environment the operator provides**: a MySQL 8 reachable on `127.0.0.1:13306` (the repo's `docker-compose.yml` already provides this) and an Apache Doris reachable on `127.0.0.1:9030`. Doris takes several minutes to become ready and its image is ~10 GB, which is why the scripts detect rather than start it.
- **Credentials differ per engine** — MySQL uses `app`/`apppw`, Doris uses `root` with no password. The query is identical across all four cases; the connection parameters necessarily are not.
- **Live-demo risk is concentrated in setup.** Because no proxy changes are permitted, a failure discovered during the presentation cannot be fixed. The readiness check in `01-setup.sh` exists specifically to surface any failure while there is still time to adjust the presentation.
