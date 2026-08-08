## 1. Rewrite scaffolding

- [ ] 1.1 Add the `sqlparser` dependency and pin the dialect to `MySqlDialect`
- [ ] 1.2 Add a `cap` configuration key defaulting to 200, rejecting non-positive values at startup
- [ ] 1.3 Insert a rewrite stage between statement read and forward, defaulting to identity so it can ship dark
- [ ] 1.4 Keep the original statement bytes alongside the parsed AST so unmodified statements forward verbatim rather than re-serialised

## 2. Cap placement

- [ ] 2.1 Apply the cap to the top-level query of a `SELECT`
- [ ] 2.2 Apply the cap to derived tables in `FROM`
- [ ] 2.3 Apply the cap to CTE bodies
- [ ] 2.4 Apply the cap to each branch of `UNION` / `INTERSECT` / `EXCEPT`
- [ ] 2.5 Reduce an existing `LIMIT` to `min(existing, cap)` and preserve `OFFSET`

## 3. Exemptions

- [ ] 3.1 Skip sub-queries in predicate position: `IN`, `NOT IN`, `EXISTS`, `NOT EXISTS`, `ANY`, `ALL`
- [ ] 3.2 Skip scalar sub-queries in `SELECT` list, `WHERE`, `HAVING`, and `ON`
- [ ] 3.3 Skip the source query of `INSERT INTO ... SELECT`, `CREATE TABLE ... AS SELECT`, and `INSERT OVERWRITE`, emitting a warning record
- [ ] 3.4 Forward non-query statements (DDL, DML, `SET`, `USE`, `SHOW`, `DESCRIBE`, admin) as original bytes

## 4. Parse fallback

- [ ] 4.1 On parse error, forward the original bytes unchanged
- [ ] 4.2 Increment a parse-failure counter and emit a record with statement text and parser error
- [ ] 4.3 Add the fail-closed configuration option, defaulting to off

## 5. Observability

- [ ] 5.1 Emit a structured rewrite record with original SQL, forwarded SQL, and the list of capped nodes
- [ ] 5.2 Mark ordered sub-query caps distinctly in the record so they are separately countable
- [ ] 5.3 Suppress the record for statements forwarded unchanged
- [ ] 5.4 Add counters for rewrites, write-statement exemptions, and parse failures

## 6. Verification

- [ ] 6.1 Table-driven rewrite tests, one case per scenario in `specs/query-limit-injection/spec.md`
- [ ] 6.2 Table-driven tests for each scenario in `specs/sql-parse-fallback/spec.md`
- [ ] 6.3 Assert `SELECT COUNT(*) FROM (SELECT id FROM big) t` returns the capped count, pinning the known hazard as tested behaviour rather than a bug
- [ ] 6.4 Assert unmodified statements forward byte-for-byte, including odd whitespace and quoting
- [ ] 6.5 Replay a captured production query log through the rewriter and report the parse-failure rate before enabling the stage
