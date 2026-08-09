# Architecture decision records

Design decisions that are **not spec-shaped** are recorded here rather than as
requirements. `surveys/domain-fit-rust-proxy.md` §3a draws the line: specify what
an outsider can observe — which statements are accepted, which are rejected, what
rows come back, what error the client sees — and do not try to specify what only
the compiler and the profiler can adjudicate.

Async task topology, buffer ownership, cancellation behaviour and the shape of
the error enum are all in the second category. Written as requirements they would
drift, because nothing outside the process can tell whether they still hold.
Written as ADRs they cannot drift: an ADR is a record of a decision at a point in
time, and history does not change.

**These are written after the fact.** Each records what was actually built and
the evidence that settled it, not what was planned. Where the evidence is a line
of a dependency's source, the file and line are cited so a reader can check
rather than trust. Where something is a known gap, it says so — an ADR that lists
only what was solved reads as a claim that the rest was handled.

**They are immutable.** A decision that changes gets a new ADR that supersedes
the old one; the old one stays. If one of these turns out to be wrong, that fact
belongs in its successor, not in an edit.

| ADR | Decision |
|---|---|
| [0001](0001-wire-protocol-crates.md) | `opensrv-mysql` on the client side; the backend connection phase hand-rolled, because `mysql_async` cannot relay a precomputed auth response |
| [0002](0002-one-task-per-session.md) | One task per session; the backend connection behind a mutex forced by an upstream `&self` signature |
| [0003](0003-buffer-ownership.md) | One packet in memory at a time, result sets relayed row at a time, hard size limits, continued packets refused |
| [0004](0004-cancellation-by-drop.md) | No cancellation machinery — the backend socket is a field of the session, so ending the session closes it |
| [0005](0005-error-enum-shape.md) | Three error types split by audience: client, operator, and the code that must not be able to say "forwarded anyway" |

`../../notes/adr-material.md` holds the raw notes these were written from,
including evidence that did not make it into a decision. It is a scratch record
with provenance, not authoritative.

## Reading order

0001 first: it is the only one that constrains the others. 0002 and 0004 are a
pair — cancellation by `Drop` works *because* there is one task per session, so
changing the topology reopens the cancellation question. 0003 and 0005 stand
alone.
