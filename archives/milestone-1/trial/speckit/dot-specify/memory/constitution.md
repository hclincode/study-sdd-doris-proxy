<!--
Sync Impact Report
Version change: (template, unversioned) → 1.0.0
Bump rationale: MAJOR/initial ratification. All five principle slots defined for the
first time; no prior version existed, so this is an initial adoption rather than an
amendment.

Modified principles:
  [PRINCIPLE_1_NAME] → I. Semantic Preservation (NON-NEGOTIABLE)
  [PRINCIPLE_2_NAME] → II. Fail Open, Never Fail Wrong
  [PRINCIPLE_3_NAME] → III. Every Rewrite Is Observable
  [PRINCIPLE_4_NAME] → IV. Rewriter Is Test-First
  [PRINCIPLE_5_NAME] → V. One Job Only

Added sections:
  [SECTION_2_NAME]  → Technical Constraints
  [SECTION_3_NAME]  → Development Workflow & Quality Gates

Removed sections: none

Follow-up TODOs: none. RATIFICATION_DATE is the date of this initial adoption.
-->

# Doris LIMIT Proxy Constitution

## Core Principles

### I. Semantic Preservation (NON-NEGOTIABLE)

A query that this proxy rewrites MUST return the same rows, in the same order, with the
same computed values, as the original query would have returned — except that the final
result set MAY be truncated to the first N rows.

Concretely: a rewrite MUST NOT change the value of an aggregate, MUST NOT change which
rows feed a join, a filter, a window function, or a set operation, and MUST NOT change
the contents of any intermediate relation whose rows are consumed by another operator
rather than returned to the client.

Rationale: this proxy sits invisibly between the client and Doris. A user who gets fewer
rows than they asked for can see that and re-ask. A user who gets a `COUNT(*)` of 200 when
the true count is 4.2 million cannot see that, and will make decisions on a wrong number.
Silent wrongness is a worse failure than an uncapped scan. When this principle conflicts
with reducing cluster load, this principle wins.

### II. Fail Open, Never Fail Wrong

When the proxy cannot confidently determine a safe rewrite — the statement does not parse,
uses syntax the parser does not model, or contains a construct whose capping safety is not
established — it MUST forward the statement unchanged and MUST record why.

The proxy MUST NOT reject a statement it merely failed to understand, and MUST NOT guess
at a rewrite it cannot justify. An un-capped query that reaches Doris is a resource risk,
bounded by Doris's own query timeouts and resource groups. A wrongly-capped query is a
correctness defect with no backstop.

Rationale: this proxy is a load guardrail, not a security boundary. It has no data-leak
threat model to defend, so the cost of missing a cap is strictly operational and strictly
recoverable. Availability of the query path outranks coverage of the cap.

### III. Every Rewrite Is Observable

For every statement it handles, the proxy MUST emit a structured record containing: the
statement fingerprint, whether a rewrite was applied, the reason no rewrite was applied
when none was, the rewritten SQL text when one was, and the parse outcome.

Any behavior a user might notice — a truncated result, a skipped cap, a parse failure —
MUST be answerable from these records alone, without reproducing the query.

Rationale: a proxy that silently mutates SQL is undebuggable from either side. The client
sees results it did not ask for; the DBA sees SQL the client never wrote. The log is the
only place those two views can be reconciled.

### IV. Rewriter Is Test-First

The rewrite logic MUST be developed against tests written before the implementation. Each
rewrite rule MUST have, before that rule is implemented: at least one case proving it fires
where it should, and at least one case proving it does not fire where it must not.

The negative cases (Principle I violations that must not occur) are mandatory, not optional.
A rule with only positive tests MUST NOT be merged.

Rationale: the failure mode this project most needs to prevent — a cap applied in a position
where it corrupts a result — is invisible in a happy-path test suite. It is only caught by
tests written to assert the absence of a rewrite.

### V. One Job Only

The proxy parses SQL, decides on a cap, forwards, and relays the response. It MUST NOT
pool, multiplex, or share backend connections; MUST NOT cache results; MUST NOT interpret,
translate, or store credentials; and MUST NOT add routing, sharding, or failover behavior.

Any new capability that is not "decide whether to cap this statement" MUST be justified in
the plan's Complexity Tracking table or rejected.

Rationale: every stateful behavior added to a transparent proxy is a new way for the proxy
to disagree with the database about what is true. Statelessness per connection is what makes
the proxy's failure modes enumerable.

## Technical Constraints

- **Connection model**: exactly one backend connection to Doris per accepted client
  connection, opened on client connect and closed on client disconnect. No pooling.
- **Authentication**: passthrough only. The proxy relays the client's authentication exchange
  to Doris and forms no opinion about it. It never holds a credential of its own.
- **SQL parsing**: the `sqlparser` crate with `MySqlDialect`. There is no Doris dialect. Doris
  accepts syntax `MySqlDialect` rejects; those statements are Principle II pass-throughs, not
  bugs to be fixed by forking the parser.
- **Rewrite output**: the forwarded statement MUST be produced by serializing the modified AST,
  not by string-splicing the original text.
- **Statement scope**: rewrite decisions are made per statement. The proxy holds no state
  between statements on a connection beyond what the wire session requires.

## Development Workflow & Quality Gates

- Every feature begins with a spec that passes its own quality checklist before planning
  starts. Unresolved `[NEEDS CLARIFICATION]` markers block `/speckit-plan`.
- Every plan includes a Constitution Check evaluated principle by principle. A principle
  marked FAIL blocks the plan until the design changes or the deviation is recorded in
  Complexity Tracking with a rejected simpler alternative.
- Changes to rewrite rules require a differential test: original SQL and rewritten SQL are
  both executed and their results compared, for every rule that claims to be result-preserving.
- Code review MUST verify Principle I for every new or modified rewrite rule. "Looks right"
  is not a review outcome; the reviewer names the intermediate relations the rule touches.

## Governance

This constitution supersedes team convention, individual preference, and prior practice for
anything it addresses. Where it is silent, normal engineering judgment applies.

Amendments require: a written statement of the principle being changed and why, a version
bump under the policy below, and a migration note for any spec, plan, or task list already
written against the old text.

Versioning policy — MAJOR: a principle is removed or redefined such that previously compliant
work becomes non-compliant. MINOR: a principle or section is added, or existing guidance is
materially expanded. PATCH: wording, clarification, or typo fixes that do not change what is
permitted.

Compliance review: the Constitution Check gate in each plan is the enforcement point. Plans
that record a Complexity Tracking deviation carry it forward into review; the deviation is
re-examined when the feature is complete, and either removed or promoted to an amendment.

**Version**: 1.0.0 | **Ratified**: 2026-08-08 | **Last Amended**: 2026-08-08
