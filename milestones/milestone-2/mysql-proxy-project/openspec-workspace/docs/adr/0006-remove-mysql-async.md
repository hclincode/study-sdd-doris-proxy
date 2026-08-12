# ADR 0006 — `mysql_async` removed from the dependency graph

- **Status:** Accepted
- **Date:** 2026-08-09
- **Supersedes:** the "Consequences" note in [ADR 0001](0001-wire-protocol-crates.md) that `mysql_async` is "retained in `Cargo.toml` pending a decision on whether anything still needs a full-featured backend client". That decision is now made. **0001 is otherwise unchanged and remains the record of the crate-selection decision.**

## Context

ADR 0001 established that `mysql_async` 0.37 cannot serve design D1: its handshake
nonce is private with no accessor, its handshake response is derived from a
plaintext password in `Opts`, and its only constructors run the whole connection
phase internally. The backend connection phase was hand-rolled instead.

The dependency was left in place at that point because it was not yet settled
whether some later piece — result-set relay, in particular — would want a
full-featured client. That question has since been answered by the code: the
result-set relay is hand-rolled too, on the same `BackendConnection` that the
connection phase produced. `COM_QUERY`, `COM_INIT_DB`, column-definition parsing
and row streaming are all in `src/session.rs`.

Nothing in the crate referenced `mysql_async`.

## Decision

Remove `mysql_async` from `Cargo.toml`.

## Rationale

The dependency graph is part of what the specification says the project needs. An
unused entry in it is a small untruth: a reader — or a future maintainer deciding
what may be changed — would reasonably infer that the backend path depends on a
client library, which is exactly the inference ADR 0001 exists to correct.

There is a second, sharper reason. `mysql_async`'s continued presence made the
hand-rolled connection phase look like a workaround alongside a real client,
rather than what it is: the only way to relay an auth response verbatim, because
**relaying credentials unchanged is precisely the thing a client library is built
not to let you do.** Removing it makes the code say that.

The removal costs nothing to reverse. If a future change wants a full-featured
backend client for something D1 does not constrain, adding the dependency back is
one line — and it will be a deliberate act with its own justification rather than
an inheritance.

## Consequences

- One fewer dependency, and a smaller transitive tree. `mysql_common` 0.37 leaves
  with it; `mysql_common` 0.32 remains as `opensrv-mysql`'s own dependency.
- `cargo test` is unchanged at 179 passing, 0 failures — which is the evidence
  that nothing used it. Had anything depended on it, the build would have failed
  rather than the tests.
- `Cargo.toml` now carries a comment recording that the crate was tried and
  removed, so the next person to reach for a backend client finds the answer
  before repeating the investigation.
- ADR 0001's analysis of *why* `mysql_async` cannot serve D1 remains the
  authoritative record and is unaffected. Only its disposition changed.

## A note on process

This is the first time 0001's own immutability rule bit its author, and it is
recorded here rather than fixed in place for that reason. The rule — *a decision
that changes gets a successor; the old one stays* — is worth nothing if the first
inconvenient application is quietly an edit instead. The cost of honouring it is
this file. The benefit is that ADR 0001 still says what was believed on the day
the crate was chosen, including the part that later turned out to be provisional,
which is the whole reason after-the-fact ADRs were preferred to specifications
for these decisions (design D7).
