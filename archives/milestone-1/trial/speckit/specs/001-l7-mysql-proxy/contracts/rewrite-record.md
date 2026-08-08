# Contract: Rewrite Record

**Feature**: 001-l7-mysql-proxy | **Date**: 2026-08-08
**Implements**: FR-010, SC-006 | **Enforces**: Constitution Principle III

The proxy's only outward-facing data interface. It is consumed by the platform log pipeline and, through
it, by whoever answers "why did my query return 200 rows?".

## Emission rule

Exactly one record per statement, emitted after the decision and before the statement is forwarded.
No exceptions, no sampling, no suppression of pass-throughs. A statement with no record is
indistinguishable from a statement that never arrived, which would make SC-006 unachievable.

## Field schema

| Field | Type | Always present | Notes |
|-------|------|----------------|-------|
| `fingerprint` | 32-char hex | yes | Statement identity with literals normalized out. The field an operator searches on. |
| `connection_id` | integer | yes | Proxy-assigned. |
| `sequence` | integer | yes | Index within a multi-statement submission. |
| `parse_outcome` | `"parsed"` \| `"failed"` | yes | — |
| `parse_error` | string | only when `parse_outcome` is `"failed"` | The parser's message, truncated to 512 bytes. |
| `decision` | `"capped"` \| `"already_bounded"` \| `"forwarded_unchanged"` | yes | — |
| `reason` | string | only when `decision` is `"forwarded_unchanged"` | One of the closed reason set in [rewrite-decision.md](./rewrite-decision.md). |
| `applied_limit` | integer | only when `decision` is `"capped"` | — |
| `existing_limit` | integer | when the statement carried one | Present for `capped` and `already_bounded`. |
| `forwarded_sql` | string | only when `decision` is `"capped"` | The rewritten text. |
| `elapsed_micros` | integer | yes | Parse through verify. Feeds SC-004. |

## Privacy rule

`forwarded_sql` is recorded only for `capped` decisions. Original user SQL is never written to the record —
not for pass-throughs, not for parse failures. A parse failure records the parser's message and the
fingerprint, which is enough to find and reproduce the shape without putting user data in logs.

This is the one place where Principle III (observe everything) and ordinary data hygiene pull against each
other. The resolution: the proxy logs what *it* did, never what the user wrote.

## Answering the three operator questions

The schema exists to make exactly these answerable from records alone, which is the SC-006 test:

1. *"Why did my query return exactly 200 rows?"* — find by fingerprint, read `decision`. If `capped`,
   `forwarded_sql` shows the SQL Doris ran.
2. *"Why wasn't my query capped?"* — `decision` is `forwarded_unchanged`, and `reason` distinguishes an
   unparseable statement from an unsafe shape from a write, without guesswork.
3. *"Is the proxy adding latency?"* — `elapsed_micros` across all records, no correlation with any other
   system required.

## Aggregate signals

Derived from the same records, no additional instrumentation:

- Share of records with `reason = "parse_failed"` — the SC-003 measurement.
- Any nonzero rate of `reason = "verification_failed"` — a defect signal, not normal operation. Expected
  count in healthy operation is zero.
- Ratio of `capped` to total — the coverage the feature actually delivers, which after the Principle I
  narrowing is expected to be well below what the original request implied.
