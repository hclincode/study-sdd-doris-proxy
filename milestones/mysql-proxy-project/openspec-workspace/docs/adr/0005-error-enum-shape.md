# ADR 0005 — Three error types, split by who reads them

- **Status:** Accepted
- **Date:** 2026-08-09
- **Decides:** design D7, "the shape of the error enum"
- **Written after the fact.**

## Context

A single `ProxyError` covering everything is the default shape and would have
worked. What made it wrong here is that this proxy's errors have three different
audiences with three different needs, and one of those needs is a security
property.

The audiences: a **client**, which must learn that its statement was refused and
in what general terms, and must learn nothing about the policy set; an
**operator**, who must be able to tell a compatibility gap from a policy denial
from a network fault, because the response differs in each case; and the
**code**, which must be unable to express "could not analyse, forwarded anyway".

## Decision

Three types.

**`RefusalReason`** — why a statement was refused. `Unparseable`,
`UnsupportedShape { construct }`, `UnresolvableTableReference`,
`WriteToRestrictedTable`, `MultiStatement`, `PreparedStatement`. Every variant is
a refusal; there is deliberately no variant meaning "forwarded anyway". Its
`Display` is the client-facing wording; `client_message()` prefixes it.

**`BackendRefusal`** — why a session could not be established. `Unreachable`,
`RefusedConnection`, `UnsupportedAuthPlugin`, `IncompatibleHandshake`. Each
carries its own `ErrorKind`, so the MySQL error code the client sees varies by
cause.

**`ProxyError`** — the transport and lifecycle error the code propagates.
`Refused(RefusalReason)`, `Config`, `Backend`, `Protocol`, `Io`.

## Rationale

**Refusals are a closed set because the spec says enforcement must be provable
statement by statement.** `RefusalReason` is the enumeration of every way a
statement can fail to be forwarded. Keeping it separate from transport errors
means the fail-closed property is inspectable in one place: a reader can check
that no variant means "gave up and forwarded", which is not checkable in an enum
that also carries `Io`.

**`UnresolvableTableReference` has the same `Display` as `UnsupportedShape`, on
purpose.** The two must be indistinguishable *to the client*: this refusal fires
for a table that might be policy-bearing, so wording of its own would tell the
client which references the proxy could resolve, and an unresolvable one is
precisely a candidate policy table (design D5). The distinction is for the
operator, and it is a real one — a spike of unresolvable references means clients
connecting without a default schema, plausibly a probing pattern, not a parser
gap. `rewrite_rejections.rs` asserts the two messages are byte-identical, so
anyone giving one its own wording has to confront what they are disclosing.

**Consequence, and it is easy to get wrong:** because `Display` is deliberately
ambiguous, refusals are logged with `Debug`, not `Display`. Logging `%reason`
would print the client-facing text and erase exactly the distinction the variant
exists to carry. Both the variant and the log site say so; a tidy-up back to
`%reason` would silently undo the change.

**`UnsupportedShape` carries `construct` but never shows it.** The field names the
AST node kind that could not be analysed. It is derived from the variant name,
never from the user's SQL, so it cannot echo query text back. It goes to the
operator's log and not into the error packet — the one piece of information
needed to tell a compatibility gap from a denial, previously computed and
discarded.

**`BackendRefusal` exists because "cannot connect" is three different operator
problems.** A network fault, a frontend refusing the connection, and a frontend
requiring an auth plugin the proxy cannot relay all used to produce one generic
message. The third is the one that matters: it is not a credential problem, no
password would work, and the fix is configuration. It gets
`ER_NOT_SUPPORTED_AUTH_MODE` and says "This is a proxy limitation, not a
credential problem". A test asserts its SQLSTATE is **not** `28000`, because an
access-denied state would send an operator hunting for a password that does not
exist.

## Consequences

- Every refusal reaches the client as SQLSTATE `42000`, asserted on the wire
  across nine categories in `tests/session_refusals.rs`. Drivers see one class.
- Two error types must be kept in step by hand: `RefusalReason`'s variants and
  the `ErrorKind` chosen in `on_query`. Nothing enforces that a new variant gets
  a sensible code — a gap a future change should close if the set grows.
- `ProxyError::Backend(String)` flattens the backend's error into text at the
  boundary. Structure is lost, but the requirement is that the backend's own
  message reaches the client rather than a proxy substitute, and the string is
  what carries it.
- `thiserror` throughout; no `anyhow`. The error set is closed and small enough
  to enumerate, which is the point.

## Alternatives considered

| Option | Rejected because |
|---|---|
| One `ProxyError` for everything | Mixes refusals with transport faults, so "no variant means forwarded anyway" stops being checkable in one place |
| `anyhow::Error` | Erases the closed set that makes the fail-closed property inspectable |
| Give `UnresolvableTableReference` its own client wording | Tells the client which references the proxy could resolve; an unresolvable one is a candidate policy table (D5) |
| Fold `BackendRefusal` into `ProxyError` | Loses the per-cause `ErrorKind`, and with it the distinction between a network fault and a credential problem |
| Put `construct` in the client message | Turns every refusal into an oracle for the policy set |
