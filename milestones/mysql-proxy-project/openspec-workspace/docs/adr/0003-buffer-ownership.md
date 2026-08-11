# ADR 0003 — Packet-at-a-time buffers, row-at-a-time relay, and hard size limits

- **Status:** Accepted
- **Date:** 2026-08-09
- **Decides:** design D7, "buffer ownership across the client/backend copy"
- **Written after the fact.**

## Context

Two cross-cutting invariants govern this area, and they pull in different
directions from the convenient implementation:

1. *No unbounded buffering.* Memory per connection is constant with respect to
   result size.
2. *No panic on client input.* The decoder consumes attacker-controlled bytes.

Because ADR 0001 puts MySQL packet framing inside this crate, both invariants are
this module's problem rather than a dependency's.

## Decision

**Own one packet at a time, and never a whole result set.**

- `read_packet` allocates a `Vec<u8>` sized to the packet body, after checking
  the length against a caller-supplied limit. The buffer is owned by the caller
  and dropped when the packet is handled.
- Result sets are relayed **row at a time**: `BackendConnection::next_row` reads
  one row packet and `RowWriter::write_row` writes it on to the client before the
  next is read. No `Vec<Row>` is ever built.
- Two limits, chosen for what the phase actually needs rather than one global
  number: `CONNECTION_PHASE_PACKET_LIMIT` of 64 KiB for handshake and auth
  packets, and `MAX_PACKET_BODY` (2²⁴−1, the largest the framing can express) for
  the command phase.
- **Continued packets are refused, not reassembled.** A body of exactly 2²⁴−1
  means the payload continues in the next packet. Reassembling would mean
  buffering without a bound, so `read_packet` returns a protocol error instead.

Every byte-level accessor lives on a bounds-checked `Bytes` reader that returns
`Result` and never indexes directly.

## Rationale

**The 64 KiB connection-phase limit is the interesting one.** Handshake packets
are a few hundred bytes. Accepting a 16 MB one because the framing permits it
would let a hostile or broken backend provoke a 16 MB allocation per connection
attempt, before any authentication has happened. The tighter limit costs nothing
real and removes that.

**Refusing continued packets is a deliberate compatibility loss.** A single row
larger than 16 MB — a large `BLOB`, say — will fail rather than being relayed.
That is the correct direction for a control whose invariant is bounded memory:
the alternative is an unbounded `Vec` fed by the backend. It is a real
limitation and belongs in operator documentation, not only here.

**Bounds-checked accessors are the whole of invariant 2.** `Bytes::take`,
`u8`, `u16_le`, `u32_le`, `nul_terminated`, `lenenc_int` and `lenenc_bytes` each
check remaining length and return `ProxyError::Protocol` rather than slicing.
`truncated_handshake_is_an_error_not_a_panic` in `src/session.rs` feeds the
parser every prefix of a valid handshake, from zero bytes to full length, and
asserts none of them panics. That test exists because the failure it guards
against is one a reviewer cannot see by reading — an off-by-one in one accessor
looks exactly like a correct one.

**Row values are relayed as raw bytes.** `next_row` yields
`Vec<Option<Vec<u8>>>` — the text-protocol field bytes, or `None` for SQL NULL —
and hands them straight to the writer. The proxy does not parse a value into a
typed representation and re-render it. It has no business reinterpreting data it
is only carrying, and any round trip through a typed form is an opportunity to
change it.

## Consequences

- Memory per session is bounded by the largest single packet, not by the result
  set. A million-row result streams in constant space.
- One allocation per packet. Not free, and a buffer pool would remove it, but
  nothing has been measured and `design.md` is explicit that performance work is
  a non-goal for the MVP. Measure before optimising.
- A result row larger than 16 MB fails the session.
- The 64 KiB and 16 MB limits are compile-time constants, not configuration. If
  either needs to move, it should move with a stated reason rather than becoming
  a knob.
- Client-side framing remains `opensrv-mysql`'s, so its buffering behaviour is
  not covered by this ADR and has not been audited.

## Alternatives considered

| Option | Rejected because |
|---|---|
| Collect the result set, then write it | Violates invariant 1 outright; memory would scale with the query's output |
| Reassemble continued packets | Requires buffering without a bound. Refusing is the fail-closed direction |
| One global packet limit | The connection phase has no legitimate use for a 16 MB packet, and the tighter bound is free |
| A reusable buffer pool per session | An optimisation for a cost nobody has measured; adds ownership complexity to the code most in need of being obviously correct |
| Decode row values into typed Rust values | Invites changing data the proxy is only relaying, and buys nothing — the bytes are already in the wire form the client expects |
