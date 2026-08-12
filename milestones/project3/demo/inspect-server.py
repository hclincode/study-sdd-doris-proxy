#!/usr/bin/env python3
"""Reads a MySQL-protocol server's initial handshake and reports what it is.

Used by 01-setup.sh to show that the two engines behind the demo are genuinely
different implementations of the same wire protocol, rather than asking the
audience to take it on faith.

The capability flags matter for more than colour: a server that does not
advertise CLIENT_DEPRECATE_EOF terminates its result sets with explicit EOF
packets instead of an OK packet, so the proxy runs a different branch of its
response state machine for it.

Usage: inspect-server.py <host> <port> [label]
"""
import socket
import sys

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
    sock = socket.create_connection((host, port), timeout=10)
    try:
        header = sock.recv(4)
        if len(header) < 4:
            raise RuntimeError("server closed before sending a handshake")
        length = int.from_bytes(header[:3], "little")
        payload = b""
        while len(payload) < length:
            chunk = sock.recv(length - len(payload))
            if not chunk:
                raise RuntimeError("server closed mid-handshake")
            payload += chunk
        return payload
    finally:
        sock.close()


def parse(payload):
    if payload[0] == 0xFF:
        raise RuntimeError(payload[9:].decode(errors="replace"))

    pos = 1
    end = payload.index(b"\0", pos)
    version = payload[pos:end].decode(errors="replace")
    pos = end + 1 + 4 + 8 + 1                      # conn id, salt, filler
    lower = int.from_bytes(payload[pos:pos + 2], "little")
    pos += 2 + 1 + 2                               # charset, status
    upper = int.from_bytes(payload[pos:pos + 2], "little")
    caps = lower | (upper << 16)
    pos += 2
    auth_len = payload[pos]
    pos += 1 + 10

    plugin = "(none advertised)"
    if caps & 0x00080000:                          # CLIENT_PLUGIN_AUTH
        tail = payload[pos + max(13, auth_len - 8):]
        plugin = tail.split(b"\0")[0].decode(errors="replace")

    return version, caps, plugin


def main():
    host, port = sys.argv[1], int(sys.argv[2])
    label = sys.argv[3] if len(sys.argv) > 3 else f"{host}:{port}"
    try:
        version, caps, plugin = parse(read_handshake(host, port))
    except Exception as exc:                       # noqa: BLE001 - report, don't raise
        print(f"  {label:<12} unreachable: {exc}")
        return 1

    advertised = sum(1 for bit in range(32) if caps & (1 << bit))
    print(f"  {label:<12} version {version:<10} auth {plugin:<24} "
          f"capabilities 0x{caps:08x} ({advertised} flags)")
    for bit, name in INTERESTING:
        mark = "yes" if caps & bit else "no"
        print(f"                 {name:<26} {mark}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
