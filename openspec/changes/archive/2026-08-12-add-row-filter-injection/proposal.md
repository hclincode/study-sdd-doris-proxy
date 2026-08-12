## Why

The proxy can see every statement passing through it but cannot yet act on one. The reason it exists in front of the database rather than beside it is to constrain what a connection is able to read: a client reaching the database through a given listener should see only the rows that listener is allowed to see, without every application needing to remember to write the filter itself.

This change adds that: a configured predicate is appended to the `WHERE` clause of read-only statements before they reach the backend, chosen per listener and per table from static configuration.

## What Changes

- **Row-filter rules in listener configuration.** Each listener gains an optional map from table name to a predicate expression. A table with no entry is not filtered. Predicates are validated at startup as complete boolean expressions and are otherwise spliced verbatim, so an invalid one is a boot failure rather than broken SQL at query time.
- **Predicate injection for single-table reads.** For `COM_QUERY` and `COM_STMT_PREPARE` carrying a `SELECT` over exactly one table that has a rule, the proxy inserts `WHERE (<predicate>)` — or rewrites an existing `WHERE x` to `WHERE (x) AND (<predicate>)` — at the correct position in the statement, before any `GROUP BY`, `HAVING`, `WINDOW`, `ORDER BY`, `LIMIT`, or locking clause.
- **Byte splicing, not re-rendering.** The insertion point is a byte offset located by the existing tokenizer. Everything the client wrote outside the inserted text — comments, hints, formatting, dialect corners — is forwarded exactly as submitted.
- **Best-effort by construction.** Anything the proxy cannot rewrite with confidence is forwarded **unchanged**: joins, subqueries, `UNION`, CTEs, multi-statement payloads, statements that fail to tokenize, and rewrites that would change the packet count. Each is counted and recorded with a reason.
- **A second pipeline stage.** Injection is registered after the existing observe-only stage and is the first stage to return owned bytes. No change to the pipeline itself.
- **Log records describe the rewrite.** Records gain the forwarded statement, a flag for whether injection happened, the rule applied, and the reason when it did not. The `statement` field continues to carry what the client submitted.
- **BREAKING** (internal, no released consumers): a forwarded `COM_QUERY` payload is no longer guaranteed byte-identical to what the client sent. The existing byte-faithfulness requirement is narrowed to responses and to commands the proxy chose not to rewrite.

**Non-goals for this change:**

- Filtering writes. `UPDATE`, `DELETE`, `INSERT` and DDL are forwarded untouched, so a client can still modify rows it cannot see.
- Multi-table statements. Joins, subqueries, `UNION` and CTEs are recognized only well enough to be skipped safely; resolving which table a predicate belongs to across them is deferred until the skip counters show it is worth doing.
- Predicates containing placeholders. A `?` in a predicate would change the parameter count a prepared statement promised its client, which is the expensive rewrite class this whole approach exists to avoid.
- Identity-derived predicates. Rules stay per listener and static, which is what keeps authentication a pure relay.
- Rejecting anything. See below.

### This is not a security boundary

The proxy applies the filter when it recognizes the statement and forwards the statement unfiltered when it does not. That is a deliberate choice: it cannot break an application, and it cannot be relied on to contain one.

Three consequences follow, and all of them are load-bearing:

- A statement the proxy cannot rewrite — including one that merely fails to tokenize — **returns unfiltered rows**. The skip is counted and logged, but the client still gets the data.
- Views, stored procedures, dynamic SQL via `PREPARE … FROM @variable`, and `HANDLER` statements hide the table name entirely, so no rule can match them.
- Whoever can reach a listener's port gets that listener's filter, and nothing else enforces it.

**The backend's own `GRANT`s remain the only access control.** This change reduces accidental cross-tenant reads in ordinary application traffic; it does not resist a client trying to evade it.

## Capabilities

### New Capabilities

- `mysql-proxy/row-filter`: Selecting a predicate for a statement from listener configuration, deciding whether a statement can be rewritten, where the predicate is inserted, and what happens when it cannot be applied.

### Modified Capabilities

- `mysql-proxy/protocol-relay`: Byte-faithfulness of forwarded traffic is narrowed. Command payloads may now be rewritten; responses, and commands the proxy did not rewrite, remain byte-identical.
- `mysql-proxy/query-logging`: Records gain fields describing whether a statement was rewritten, what it was rewritten to, and why a rewrite was skipped.

## Impact

- **Code**: a new `row_filter` module (rule lookup, statement analysis, splice construction) and a new pipeline stage. `config` gains the rule map and its startup validation. `sql/tokenizer` gains clause-boundary location built on the byte spans it already emits; the tokenizer itself does not change. `logging/record` gains fields.
- **No new dependencies.** The tokenizer, the stage pipeline, and the packet-count invariant were all built for this.
- **Configuration compatibility**: the rule map is optional, so existing configuration files remain valid and behave identically.
- **Operational**: the log becomes the feedback loop for scope. Skip reasons say empirically which constructs real traffic uses, and that is the input to deciding whether multi-table support is worth building.
- **Risk concentrates in one place**: a wrong insertion point or a missing pair of parentheses silently widens a result set rather than failing loudly. This is the change's defining hazard and is addressed in the design.
