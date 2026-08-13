#!/usr/bin/env python3
"""Reads a MySQL-protocol server's initial handshake and reports what it is.

Used by 01-setup.sh to show that the two engines behind the demo are genuinely
different implementations of the same wire protocol, rather than asking the
audience to take it on faith.

The capability flags matter for more than colour: a server that does not
advertise CLIENT_DEPRECATE_EOF terminates its result sets with explicit EOF
packets instead of an OK packet, so the proxy runs a different branch of its
response state machine for it.

Nothing here is the proxy. This connects to a server, reads the first packet it
sends, prints what it says, and hangs up without authenticating — a handshake is
offered before any credentials are exchanged, which is why this can report on an
engine it has no password for. The proxy's own parsing lives in
`src/protocol/connection_phase.rs` and is far more careful; treat this as a
demonstration instrument, not a second implementation.

Usage: inspect-server.py <host> <port> [label]
"""
import socket
import sys

# The flags worth showing an audience, out of the 32 a server can advertise.
# The first three are the ones the proxy strips from the backend's handshake
# before passing it on, so a client can never negotiate them; the last four are
# ones whose presence or absence changes how the proxy must read the responses
# that follow. CLIENT_DEPRECATE_EOF is the load-bearing one — see the module
# docstring, and note that Doris does not advertise it.
INTERESTING = [
    (0x00000800, "CLIENT_SSL"),
    (0x00000020, "CLIENT_COMPRESS"),
    (0x00000080, "CLIENT_LOCAL_FILES"),
    (0x00002000, "CLIENT_TRANSACTIONS"),
    (0x00020000, "CLIENT_MULTI_RESULTS"),
    (0x01000000, "CLIENT_DEPRECATE_EOF"),
    (0x08000000, "CLIENT_QUERY_ATTRIBUTES"),
]


def read_handshake(host, port):
    """Returns the payload of the server's first packet, header stripped.

    Every MySQL packet is a 4-byte header followed by that many bytes of
    payload: three bytes of length, little-endian, then a sequence id this
    script ignores because it never sends anything to increment it.
    """
    sock = socket.create_connection((host, port), timeout=10)
    try:
        header = sock.recv(4)
        if len(header) < 4:
            # A server that hangs up here is usually one that rejected the
            # source address before saying anything — a blocked host, or
            # Doris still starting up.
            raise RuntimeError("server closed before sending a handshake")
        length = int.from_bytes(header[:3], "little")
        payload = b""
        # recv returns what has arrived, not what was asked for. The loop is
        # not defensive padding: a handshake carrying a long plugin name can
        # cross a TCP segment boundary, and reading once would truncate it.
        while len(payload) < length:
            chunk = sock.recv(length - len(payload))
            if not chunk:
                raise RuntimeError("server closed mid-handshake")
            payload += chunk
        return payload
    finally:
        sock.close()


def parse(payload):
    """Pulls the server version, capability flags and auth plugin out of a
    HandshakeV10 packet.

    The layout is fixed-width apart from two NUL-terminated strings, so this
    walks it by offset. Everything skipped is skipped deliberately:

        1   protocol version (0x0a)
        ..  server version           NUL-terminated  <- read
        4   connection id
        8   auth-plugin-data part 1  (the salt)
        1   filler, always 0x00
        2   capability flags, LOWER half             <- read
        1   character set
        2   status flags
        2   capability flags, UPPER half             <- read
        1   length of the full auth-plugin-data      <- read
        10  reserved, all zero
        ..  auth-plugin-data part 2
        ..  auth plugin name         NUL-terminated  <- read

    Offsets are written as the sums of those field widths rather than as
    constants, so each one can be checked against the table above instead of
    trusted.
    """
    # An ERR packet instead of a handshake: the server refused the connection
    # before offering one at all — host not allowed, or out of connections.
    # Nine bytes in is past the 0xff marker, the 2-byte error code, the '#'
    # and the 5-character SQL state, leaving the human-readable message.
    if payload[0] == 0xFF:
        raise RuntimeError(payload[9:].decode(errors="replace"))

    pos = 1
    end = payload.index(b"\0", pos)
    version = payload[pos:end].decode(errors="replace")
    pos = end + 1 + 4 + 8 + 1                      # conn id, salt, filler

    # The capability bitmap is split across two non-adjacent 2-byte fields with
    # the character set and status flags sitting between them. That is a
    # historical accident — the field was extended from 16 bits to 32 without
    # moving what came after it — and it is why this cannot be read in one go.
    lower = int.from_bytes(payload[pos:pos + 2], "little")
    pos += 2 + 1 + 2                               # charset, status
    upper = int.from_bytes(payload[pos:pos + 2], "little")
    caps = lower | (upper << 16)
    pos += 2

    auth_len = payload[pos]
    pos += 1 + 10                                  # the length byte, reserved

    plugin = "(none advertised)"
    if caps & 0x00080000:                          # CLIENT_PLUGIN_AUTH
        # Part 2 of the auth data is auth_len - 8 bytes (part 1 took the first
        # 8), but never fewer than 13. The floor is the compatibility rule that
        # matters here: MySQL declares 21 and writes 13, while Doris declares
        # 21 as well — take the arithmetic literally and the plugin name is
        # read from the wrong offset, which is how "mysql_native_password"
        # comes back as mojibake.
        tail = payload[pos + max(13, auth_len - 8):]
        plugin = tail.split(b"\0")[0].decode(errors="replace")

    return version, caps, plugin


def main():
    host, port = sys.argv[1], int(sys.argv[2])
    label = sys.argv[3] if len(sys.argv) > 3 else f"{host}:{port}"
    try:
        version, caps, plugin = parse(read_handshake(host, port))
    except Exception as exc:                       # noqa: BLE001 - report, don't raise
        # Every failure prints one line and exits non-zero rather than raising:
        # this runs inside 01-setup.sh, where a stack trace in the middle of a
        # readiness report would obscure the three checks that did pass.
        print(f"  {label:<12} unreachable: {exc}")
        return 1

    # The headline number in the demo: MySQL advertises 31 of the 32 possible
    # flags, Doris 6. It is a crude measure of surface area, not of quality —
    # its only job is to make "these are different implementations" visible at
    # a glance, before the named flags below give the reason it matters.
    advertised = sum(1 for bit in range(32) if caps & (1 << bit))
    print(f"  {label:<12} version {version:<10} auth {plugin:<24} "
          f"capabilities 0x{caps:08x} ({advertised} flags)")
    for bit, name in INTERESTING:
        mark = "yes" if caps & bit else "no"
        print(f"                 {name:<26} {mark}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
