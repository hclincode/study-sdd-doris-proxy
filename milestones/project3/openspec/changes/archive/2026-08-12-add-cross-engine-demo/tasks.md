## 1. Demo assets

- [x] 1.1 Create the `demo/` directory
- [x] 1.2 Write `demo/query.sql` containing the single query both live scripts run, so the identical-SQL claim is verifiable
- [x] 1.3 Write `demo/seed-mysql.sql`: `orders` with `id`, `region`, `amount` and 8 rows across `EU`, `US`, `APAC` — 3 of them `EU`
- [x] 1.4 Write `demo/seed-doris.sql` with the same 8 rows and Doris DDL (`DISTRIBUTED BY HASH(id) BUCKETS 1`, `replication_num = 1`)
- [x] 1.5 Write `demo/proxy.toml`: one proxy, a listener to MySQL and a listener to Doris, both filtering `orders` on `region = 'EU'`
- [x] 1.6 Keep engine endpoints and credentials in one place the three scripts share, since they differ per engine

## 2. Setup script

- [x] 2.1 Detect that MySQL is reachable on its configured port, failing with the command to start it rather than starting it
- [x] 2.2 Detect that Doris is reachable on its configured port, failing with a clear message that it takes several minutes to become ready
- [x] 2.3 Apply the per-engine seed files, making the script safe to re-run
- [x] 2.4 Start the proxy in the background against `demo/proxy.toml` and record its PID for teardown
- [x] 2.5 Report each engine's advertised protocol version, auth plugin, and capability flags, so the environment visibly contains two different implementations
- [x] 2.6 Smoke-test all four cases and print a readiness table naming any that failed
- [x] 2.7 Exit non-zero when any case fails, so a broken environment cannot be mistaken for a ready one
- [x] 2.8 Provide a way to stop the proxy and clean up, either a flag on this script or a separate teardown script

## 3. Direct-connection script

- [x] 3.1 Run `demo/query.sql` against MySQL directly and show the rows (case 1-1)
- [x] 3.2 Run the same file against Doris directly and show the rows (case 2-1)
- [x] 3.3 Print the query text before the results so the audience sees what was sent
- [x] 3.4 Label each case and report its row count against the table total
- [x] 3.5 Fail with a readable message, not a stack trace, when an engine is unreachable

## 4. Proxied-connection script

- [x] 4.1 Run `demo/query.sql` through the proxy to MySQL and show the filtered rows (case 1-2)
- [x] 4.2 Run the same file through the proxy to Doris and show the filtered rows (case 2-2)
- [x] 4.3 Read `forwarded_statement` from each listener's log and show it beside the client's query, so the injected predicate is visible rather than asserted
- [x] 4.4 Report filtered row count against the total, making the contrast with the direct script explicit
- [x] 4.5 Fail with a readable message when the proxy is not running, pointing back at the setup script

## 5. Documentation

- [x] 5.1 Write `demo/README.md` with prerequisites, the three commands in order, and what each of the four cases demonstrates
- [x] 5.2 State the timing constraint prominently: run setup well before the presentation, never during it
- [x] 5.3 Record why the demo proves more than the filter working — Doris does not negotiate `CLIENT_DEPRECATE_EOF`, so the two proxied cases exercise different response-framing paths
- [x] 5.4 Note honestly that the query is identical across cases while schemas and credentials differ per engine
- [x] 5.5 Point at the README's row-filter section for the best-effort limitations, which the demo does not stage

## 6. Rehearsal

- [x] 6.1 Run the three scripts in order end to end and confirm the four cases produce 8, 3, 8, 3 rows
- [x] 6.2 Confirm the setup script's readiness table catches a genuine failure, for example with the proxy stopped
- [x] 6.3 Confirm the live scripts complete fast enough to run in front of an audience
