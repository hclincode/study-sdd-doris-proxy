## 1. Configuration and predicate validation

- [x] 1.1 Add an optional table-to-predicate map to `ListenerConfig`, defaulting to empty so existing configuration files stay valid
- [x] 1.2 Normalize rule keys at load: strip backticks, lowercase, and split an optional schema qualifier from the table name
- [x] 1.3 Implement predicate validation over the tokenizer: reject unbalanced parentheses, a statement separator, a parameter placeholder, any comment token, an empty predicate, and anything that fails to tokenize
- [x] 1.4 Fail startup on an invalid predicate, naming the listener, the table, and the reason, before any listener binds
- [x] 1.5 Unit-test validation: accept ordinary comparisons and compound expressions; reject each banned construct individually, including a trailing line comment
- [x] 1.6 Unit-test rule-key normalization and the qualified-versus-bare matching table from the design

## 2. Statement analysis

- [x] 2.1 Add clause-boundary location over the existing token stream, tracking parenthesis depth and considering only depth-zero tokens; do not modify the tokenizer itself
- [x] 2.2 Implement the eligibility check: first significant token is `SELECT`, no `( SELECT` sequence anywhere, no top-level set operator, exactly one top-level `FROM`, no depth-zero statement separator
- [x] 2.3 Extract the single table reference with its optional schema qualifier and optional alias, rejecting a following comma or any `JOIN` keyword as multiple tables
- [x] 2.4 Locate the insertion point: the byte offset of the first depth-zero clause keyword after the table reference, or the end of the statement
- [x] 2.5 Locate the extent of an existing `WHERE` clause, returning the offsets just after the keyword and at the clause's end
- [x] 2.6 Return a typed skip reason rather than a boolean whenever analysis declines, using the closed set from the design
- [x] 2.7 Assert that every offset returned comes from a recorded token span, never from arithmetic, so a splice cannot land inside a string or comment
- [x] 2.8 Unit-test eligibility: accept plain selects with and without a `WHERE`; reject joins, comma-joins, subqueries, `UNION`, CTEs, a leading parenthesis, multi-statement payloads, and non-`SELECT` statements
- [x] 2.9 Unit-test insertion points against statements ending in `GROUP BY`, `HAVING`, `ORDER BY`, `LIMIT`, a locking clause, and nothing at all
- [x] 2.10 Unit-test that a clause keyword appearing inside a string literal, a quoted identifier, or a comment is not treated as a clause boundary

## 3. Splice construction

- [x] 3.1 Build the no-`WHERE` splice, inserting a `WHERE` clause with the predicate parenthesized
- [x] 3.2 Build the existing-`WHERE` splice, parenthesizing the original condition and the predicate and joining them with `AND`
- [x] 3.3 Assemble output from the original bytes in one pass, leaving everything outside the inserted fragments byte-identical
- [x] 3.4 Implement the packet-count guard: skip the rewrite when the rewritten payload would occupy more packets than the client sent, and report it as a skip reason
- [x] 3.5 Unit-test that both splice shapes produce the expected statement text, including with comments, hints, and unusual whitespace preserved
- [x] 3.6 Unit-test the precedence case explicitly: an original condition using `OR` must end up parenthesized
- [x] 3.7 Unit-test that a predicate containing `OR` is itself parenthesized
- [x] 3.8 Unit-test the packet-count guard at the boundary where a rewrite would add a packet

## 4. The row-filter stage

- [x] 4.1 Implement `RowFilterStage` holding the listener's compiled rules, returning replaced bytes on a rewrite and unchanged otherwise
- [x] 4.2 Apply the stage to `COM_QUERY` and `COM_STMT_PREPARE`, and to no other command
- [x] 4.3 Register the stage after the observe stage so the digest continues to describe the client's statement
- [x] 4.4 Record the outcome in `StageContext`: rewritten with the rule applied and the forwarded statement, or not rewritten with a skip reason, or not rewritten with no reason when no rule applied
- [x] 4.5 Construct the stage only when the listener has rules, so an unconfigured listener does no analysis work
- [x] 4.6 Unit-test that a rewrite returns owned bytes and a pass-through returns borrowed bytes
- [x] 4.7 Unit-test that commands other than the two SQL-carrying ones are never analyzed

## 5. Log records

- [x] 5.1 Add fields to `CommandRecord`: whether the statement was rewritten, the forwarded statement, the rule applied, and the skip reason
- [x] 5.2 Populate them from `StageContext` in the connection loop, leaving `statement` as the client's original text
- [x] 5.3 Omit the new fields when they do not apply, so records for unfiltered traffic are unchanged in shape
- [x] 5.4 Unit-test serialization for a rewritten statement, a skipped rewrite, and a statement no rule applied to

## 6. Integration tests

- [x] 6.1 Extend the test harness so a `RunningProxy` can be started with row-filter rules
- [x] 6.2 Verify a rewritten statement reaches the mock backend with the predicate applied, and that the client's original text is what the record reports as `statement`
- [x] 6.3 Verify a statement with an existing `WHERE` arrives with both conditions parenthesized
- [x] 6.4 Verify a prepared statement is rewritten at prepare time
- [x] 6.5 Verify each skip reason: a join, a subquery, a `UNION`, a multi-statement payload, an unparsable statement, and a non-`SELECT` — each forwarded unchanged and recorded with its reason
- [x] 6.6 Verify a table with no rule is forwarded unchanged and recorded with no skip reason
- [x] 6.7 Verify a listener with no rules behaves exactly as before this change

## 7. Verification against real MySQL

- [x] 7.1 Extend `scripts/verify-with-mysql.sh` with a table holding rows for two tenants and a listener filtered to one of them
- [x] 7.2 Assert row visibility, not SQL text: a plain select returns only the filtered tenant's rows
- [x] 7.3 Assert the same for a select with an existing `WHERE`
- [x] 7.4 Assert the precedence case: a select whose `WHERE` uses `OR` returns only the filtered tenant's rows
- [x] 7.5 Assert a select with `ORDER BY` and `LIMIT` after the injection point stays valid and filtered
- [x] 7.6 Assert a prepared statement returns only the filtered tenant's rows, with parameter and column counts unchanged
- [x] 7.7 Assert a join over the filtered table returns the unfiltered row count, which is the specified best-effort behavior
- [x] 7.8 Assert an unfiltered listener against the same table still sees every row

## 8. Documentation

- [x] 8.1 Document the rule configuration format in the README with a worked example
- [x] 8.2 State plainly in the README that a statement the proxy cannot rewrite is forwarded unfiltered, and that this is a convenience layer rather than a security control
- [x] 8.3 Document which constructs are supported and which are skipped, and how to read the skip reasons in the log
- [x] 8.4 Document the safe rollout procedure from the design: apply a rule to a non-production listener, run the application's real queries, and read the log before any client depends on it
