## Context

See `proposal.md` — Why for motivation, and the delta specs under `specs/mysql-proxy/` for the behavior contract. The architecture this builds on is described in `openspec/changes/archive/2026-08-12-add-mysql-proxy-query-logging/design.md`; three of its decisions exist specifically to make this change small:

- The pipeline stage signature returns `Cow<'_, [u8]>`, so a stage that replaces a command's payload already has somewhere to put it.
- The tokenizer emits tokens carrying byte spans into the original statement, which is what locating a splice point requires.
- A command's packet count is a preserved invariant, so a rewrite that does not change it needs no sequence-identifier renumbering.

One property dominates every decision below: **the failure mode of this feature is silent.** A rewrite that lands in the wrong place, or omits a pair of parentheses, produces a statement that runs successfully and returns more rows than it should. Nothing errors, nothing is logged as wrong, and the client cannot tell. Everything here is arranged so that uncertainty resolves to "do not rewrite" rather than "rewrite approximately".

The change is also, by explicit choice, best-effort: a statement the proxy cannot rewrite is forwarded unfiltered rather than rejected. That decision is recorded in the proposal along with its consequences and is not revisited here.

## Goals / Non-Goals

**Goals:**

- Rewrite the single-table read case correctly, and refuse to guess about anything else.
- Keep the rewrite decision derivable from the token stream alone, so a construct the analyzer has never seen is skipped rather than mishandled.
- Make the skipped cases countable and attributable, so the decision about what to support next comes from real traffic instead of speculation.
- Preserve everything the client wrote that is not the inserted text.

**Non-Goals:**

- A SQL grammar. The analyzer recognizes a narrow statement shape and gives up outside it.
- Deciding, for a multi-table statement, which table a predicate belongs to.
- Any behavior that depends on knowing the database schema. The proxy never inspects the backend's catalog.

## Decisions

### A second pipeline stage, after the observer

Injection is a `RowFilterStage` registered after the existing `ObserveStage`. It is the first stage to return `StageOutput::Replaced`.

Ordering matters and is deliberate: the observer computes the digest from the statement the client submitted, so running it first means the digest is unaffected by whether a rule happened to apply. Two clients issuing the same query against differently-configured listeners group together, which is what makes the digest useful.

The stage writes its outcome — rewritten with which rule, or skipped with which reason — into the existing `StageContext`, and the connection loop copies it onto the record once the response completes. This is the mechanism the context was introduced for.

### Analysis is a shape scanner over tokens, not a parser

Eligibility and the insertion point are decided by one pass over the token stream, tracking parenthesis depth and looking only at tokens at depth zero.

A statement is eligible only if all of the following hold:

```
  first significant token is SELECT      ← a leading '(' means a parenthesized
                                           UNION branch; skip
  no `( SELECT` sequence anywhere        ← any subquery; skip
  no top-level UNION / INTERSECT / EXCEPT
  exactly one top-level FROM
  the table reference is one name, optionally schema-qualified,
    optionally followed by an alias
  the token after the table reference is a clause keyword or end of input
    — a comma or any JOIN keyword means more than one table; skip
  no statement separator at depth zero   ← several statements in one command
```

The alternative would be a real parser. It was rejected in phase one for digests and is rejected again here for a sharper reason: a parser's failure mode on unsupported syntax is to produce *something*, and the analyzer's job is to distinguish "I recognize this exact shape" from everything else. A scanner that only matches an explicit whitelist of shapes is the honest expression of that. It also means new MySQL syntax cannot cause a wrong rewrite — only a skip.

The insertion point is the byte offset of the first depth-zero clause keyword after the table reference, drawn from a fixed set: `WHERE`, `GROUP`, `HAVING`, `WINDOW`, `ORDER`, `LIMIT`, `OFFSET`, `PROCEDURE`, `INTO`, `FOR`, `LOCK`. If none is present, the insertion point is the end of the statement.

**The offset must come from a token's recorded span, never from arithmetic on one.** That rule is what guarantees a splice cannot land inside a string literal or a comment: the tokenizer already decided where those are, and a keyword-looking sequence inside one is not a token of that kind.

### Two splice shapes, both built in a single pass

```
  no existing WHERE:
    SELECT * FROM orders ORDER BY id
                        ↑ insert
    SELECT * FROM orders WHERE (tenant_id = 7) ORDER BY id

  existing WHERE:
    SELECT * FROM orders WHERE a = 1 OR b = 2 ORDER BY id
                               ↑ open              ↑ close
    SELECT * FROM orders WHERE (a = 1 OR b = 2) AND (tenant_id = 7) ORDER BY id
```

The second case needs two offsets — just after the `WHERE` keyword, and at the end of the `WHERE` clause, which is the next depth-zero clause keyword or the end of the statement. The output is assembled once from the original bytes and the inserted fragments; the original is never mutated in place.

**Both sides are parenthesized, unconditionally.** Without the parentheses around the original condition, `WHERE a = 1 OR b = 2 AND tenant_id = 7` binds as `a = 1 OR (b = 2 AND tenant_id = 7)` and returns every tenant's rows where `a = 1`. This is the single most likely way for this change to fail silently, it is invisible in any test whose predicate is a lone `AND`, and it is why an `OR` case is called out as a required test rather than left to judgement. The predicate is parenthesized too, so that a configured `a = 1 OR b = 2` cannot do the same thing from the other direction.

### Table names are matched case-insensitively, and qualification must agree

Rule keys and table references are normalized by stripping backticks and lowercasing before comparison.

Case-insensitive matching is the safer error: MySQL's own table-name case sensitivity depends on `lower_case_table_names` and on the host filesystem, and a rule that fails to match returns unfiltered rows — the failure this change exists to prevent. Matching too eagerly, by contrast, produces a loud error if the column does not exist on the other table.

Qualification is matched strictly, because guessing here would silently apply one schema's policy to another:

| Reference in statement | Rule key | Matches |
| --- | --- | --- |
| `orders` | `orders` | yes |
| `shop.orders` | `shop.orders` | yes |
| `shop.orders` | `orders` | yes — bare rule is the fallback for any schema |
| `orders` | `shop.orders` | no — the reference names no schema, so the proxy cannot know |

A qualified rule is preferred over a bare one when both could match.

### Predicates are validated structurally at load, and comments are banned outright

At startup each predicate is tokenized and rejected unless it satisfies:

```
  tokenizes successfully              (balanced quotes, terminated literals)
  parenthesis depth returns to zero and never goes negative
  contains no ';'                     (would end the statement it is spliced into)
  contains no '?'                     (would change a prepared statement's param count)
  contains no comment token of any form
  is not empty
```

The comment ban deserves its reason: the predicate is wrapped in parentheses, so a trailing `-- note` inside it comments out the closing parenthesis and everything after it on that line, including the rest of the client's statement. A block comment is harmless, but distinguishing the two is a rule nobody will remember, and no legitimate configured predicate needs a comment.

What this cannot check is whether the predicate is semantically a boolean expression, or whether the column it names exists on the table. There is no schema access at startup, and inventing one would be a much larger change. A predicate that is structurally valid but wrong produces a loud query-time error on exactly the tables that carry the rule, which is an acceptable and diagnosable failure.

### The packet-count guard, which is what makes sequence numbering a non-problem

Before emitting, the stage compares the packet count of the rewritten payload with the original and skips the rewrite if they differ.

The alternative is to renumber sequence identifiers for the rest of the exchange, because a command that grows from one packet to two shifts every subsequent sequence number by one and the client will reject the response. That is per-connection state threaded through the whole relay. The guard replaces it with one comparison, at the cost of not rewriting statements within a few tens of bytes of a 16 MB packet boundary — which is a case that does not occur in practice, and which is counted if it ever does.

### Skip reasons are a closed enum, and "no rule" is not one of them

Skips carry one of: multiple tables, unsupported structure, not a select, multiple statements, tokenize failed, no insertion point, packet count changed.

A statement that reads a table with no configured rule is **not** a skip and records no reason. This distinction is what makes the counters usable: a skip means "a filter was wanted here and did not happen", so the total is a direct measure of how much traffic this feature is failing to cover, and the breakdown says which construct to support next. If unfiltered tables counted as skips, the number would be dominated by traffic nobody intended to filter.

### Testing asserts row visibility, not rewritten SQL text

Unit tests over the analyzer and splicer compare strings, because that is the level at which insertion points and parentheses are decided.

The tests that matter, though, run against real MySQL with two tenants' rows in one table and assert **which rows come back**. String equality cannot catch the precedence bug — `WHERE a = 1 OR b = 2 AND tenant_id = 7` is a perfectly reasonable-looking string — and row counts can. The required cases are: a plain select, a select with `WHERE`, a select whose `WHERE` uses `OR`, a select with `ORDER BY`/`LIMIT` after the injection point, and the same query issued as a prepared statement.

The skip paths need the complementary test: a join over a filtered table must return the *unfiltered* row count, because that is the specified behavior, and a test that asserts filtering there would be asserting something the proxy does not do.

## Risks / Trade-offs

**A wrong rewrite widens a result set silently** → The defining hazard: no error, no log line, just extra rows. Mitigations are layered — unconditional parenthesization of both operands; splice offsets taken only from recorded token spans; an eligibility check that whitelists one statement shape; and row-visibility tests against real MySQL including the `OR` case that string comparison cannot catch.

**Skipped rewrites return unfiltered rows** → Inherent to the best-effort choice, not a defect. Mitigation: every skip is counted and attributed, the proposal states plainly that this is not a security boundary, and the README must say so where an operator will read it. If the counters later show meaningful traffic being skipped on filtered tables, the answer is either to support the construct or to revisit the fail-open decision — both are new changes, and this design does not preclude either.

**Table-name matching can miss** → A quoted, aliased, or oddly-cased reference that fails to match its rule returns unfiltered rows. Mitigation: normalize by stripping backticks and lowercasing; strict qualification rules so a miss is predictable rather than arbitrary; and a test corpus of reference spellings.

**A structurally valid but semantically wrong predicate breaks every query on that table** → For example naming a column the table does not have. Mitigation: it fails loudly with a clear MySQL error and is scoped to the tables carrying that rule; startup validation catches the structural cases that would fail confusingly instead.

**The analyzer and the digest tokenizer share code and could drift** → Both consume the same token stream, and a change made for one could alter the other. Mitigation: the tokenizer is not modified by this change; clause-boundary location is added alongside it as a separate function over its output, and the existing tokenizer test corpus stays as the guard.

**Configuration becomes security-relevant input** → Predicate text is spliced into statements verbatim, so the config file now has the properties of a code path. Mitigation: startup validation with an explicit reject list, failure to boot rather than failure at query time, and documentation that the file should be managed accordingly.

## Migration Plan

Additive and backward compatible. The rule map is optional, so an existing configuration file remains valid and produces identical behavior; a listener gains filtering only when rules are added to it.

Deployment is a restart, which drops connections. Rollback is removing the rules and restarting — no data or schema change is involved in either direction.

Rolling out a rule safely is worth stating, since the failure mode is silent: add the rule to a non-production listener first, issue the application's real queries through it, and read the log. The records show the forwarded statement for rewrites and a reason for skips, which is enough to tell whether the rule is doing what was intended before any client depends on it.

## Open Questions

- Whether multi-table support is worth building. Deliberately deferred: the skip counters answer it with evidence, and any answer changes only what the analyzer accepts.
- Whether to offer a per-listener mode that rejects statements it cannot rewrite instead of forwarding them. The fail-open behavior was chosen for this change; adding an opt-in strict mode later would not disturb anything here.
- Whether rules should be expressible per user as well as per listener. That would require the proxy to terminate authentication rather than relay it, so it is a much larger change and not a variation on this one.
