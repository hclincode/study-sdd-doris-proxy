//! Capability flags and server status flags.

#![allow(dead_code)]

pub const CLIENT_LONG_PASSWORD: u32 = 0x0000_0001;
pub const CLIENT_FOUND_ROWS: u32 = 0x0000_0002;
pub const CLIENT_LONG_FLAG: u32 = 0x0000_0004;
pub const CLIENT_CONNECT_WITH_DB: u32 = 0x0000_0008;
pub const CLIENT_NO_SCHEMA: u32 = 0x0000_0010;
pub const CLIENT_COMPRESS: u32 = 0x0000_0020;
pub const CLIENT_ODBC: u32 = 0x0000_0040;
pub const CLIENT_LOCAL_FILES: u32 = 0x0000_0080;
pub const CLIENT_IGNORE_SPACE: u32 = 0x0000_0100;
pub const CLIENT_PROTOCOL_41: u32 = 0x0000_0200;
pub const CLIENT_INTERACTIVE: u32 = 0x0000_0400;
pub const CLIENT_SSL: u32 = 0x0000_0800;
pub const CLIENT_IGNORE_SIGPIPE: u32 = 0x0000_1000;
pub const CLIENT_TRANSACTIONS: u32 = 0x0000_2000;
pub const CLIENT_RESERVED: u32 = 0x0000_4000;
pub const CLIENT_SECURE_CONNECTION: u32 = 0x0000_8000;
pub const CLIENT_MULTI_STATEMENTS: u32 = 0x0001_0000;
pub const CLIENT_MULTI_RESULTS: u32 = 0x0002_0000;
pub const CLIENT_PS_MULTI_RESULTS: u32 = 0x0004_0000;
pub const CLIENT_PLUGIN_AUTH: u32 = 0x0008_0000;
pub const CLIENT_CONNECT_ATTRS: u32 = 0x0010_0000;
pub const CLIENT_PLUGIN_AUTH_LENENC_CLIENT_DATA: u32 = 0x0020_0000;
pub const CLIENT_CAN_HANDLE_EXPIRED_PASSWORDS: u32 = 0x0040_0000;
pub const CLIENT_SESSION_TRACK: u32 = 0x0080_0000;
pub const CLIENT_DEPRECATE_EOF: u32 = 0x0100_0000;
pub const CLIENT_OPTIONAL_RESULTSET_METADATA: u32 = 0x0200_0000;
pub const CLIENT_ZSTD_COMPRESSION_ALGORITHM: u32 = 0x0400_0000;
pub const CLIENT_QUERY_ATTRIBUTES: u32 = 0x0800_0000;

pub const SERVER_MORE_RESULTS_EXISTS: u16 = 0x0008;

/// Capability bits the proxy clears from the backend's advertised set before
/// forwarding the handshake to the client.
///
/// The first three are the exclusions named in the specs. The remaining two are
/// cleared for the same reason — each would otherwise put bytes on the wire the
/// proxy cannot interpret:
///
/// * `CLIENT_ZSTD_COMPRESSION_ALGORITHM` is a second compression negotiation
///   independent of `CLIENT_COMPRESS`; leaving it set would let a connection
///   compress despite compression being out of scope.
/// * `CLIENT_QUERY_ATTRIBUTES` prefixes `COM_QUERY` with a binary-encoded
///   parameter block, so extracting statement text would require decoding
///   binary protocol values. Clearing it keeps `COM_QUERY` a plain string.
pub const MASKED_CAPABILITIES: u32 = CLIENT_SSL
    | CLIENT_COMPRESS
    | CLIENT_LOCAL_FILES
    | CLIENT_ZSTD_COMPRESSION_ALGORITHM
    | CLIENT_QUERY_ATTRIBUTES;

/// Human-readable names for the masked bits, for error messages.
pub fn masked_names(flags: u32) -> Vec<&'static str> {
    let mut out = Vec::new();
    for (bit, name) in [
        (CLIENT_SSL, "CLIENT_SSL"),
        (CLIENT_COMPRESS, "CLIENT_COMPRESS"),
        (CLIENT_LOCAL_FILES, "CLIENT_LOCAL_FILES"),
        (
            CLIENT_ZSTD_COMPRESSION_ALGORITHM,
            "CLIENT_ZSTD_COMPRESSION_ALGORITHM",
        ),
        (CLIENT_QUERY_ATTRIBUTES, "CLIENT_QUERY_ATTRIBUTES"),
    ] {
        if flags & bit != 0 {
            out.push(name);
        }
    }
    out
}
