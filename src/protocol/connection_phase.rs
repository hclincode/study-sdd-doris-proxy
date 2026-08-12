//! The MySQL connection phase.
//!
//! The proxy does not authenticate anyone. It reads the backend's handshake to
//! learn what was negotiated, clears the capability bits it cannot support, and
//! then relays connection-phase packets in both directions until the backend
//! answers with OK or ERR. Authentication therefore stays end-to-end between
//! client and backend, and any authentication plugin works — including
//! `caching_sha2_password`, whose public-key exchange the proxy forwards
//! without being able to read it.

use std::io;

use tokio::io::{AsyncRead, AsyncWrite};

use super::capabilities::*;
use super::framing::{Cursor, PacketReader, PacketWriter};

/// Everything the command phase needs to know about a connection.
#[derive(Debug, Clone)]
pub struct Session {
    /// Capabilities actually negotiated. Response framing depends on these.
    pub capabilities: u32,
    pub username: String,
    pub database: Option<String>,
    pub backend_connection_id: u32,
}

impl Session {
    pub fn has(&self, flag: u32) -> bool {
        self.capabilities & flag != 0
    }

    pub fn deprecate_eof(&self) -> bool {
        self.has(CLIENT_DEPRECATE_EOF)
    }
}

#[derive(Debug)]
pub enum HandshakeError {
    /// Transport failure on either leg.
    Io(io::Error),
    /// The peer sent something that is not valid for this point in the protocol.
    Protocol(String),
    /// The client asserted a capability the proxy masked off.
    ForbiddenCapability(Vec<&'static str>),
    /// The backend refused the connection; its ERR was relayed to the client.
    Rejected,
}

impl From<io::Error> for HandshakeError {
    fn from(e: io::Error) -> Self {
        HandshakeError::Io(e)
    }
}

impl std::fmt::Display for HandshakeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HandshakeError::Io(e) => write!(f, "io error during handshake: {e}"),
            HandshakeError::Protocol(m) => write!(f, "protocol error during handshake: {m}"),
            HandshakeError::ForbiddenCapability(names) => write!(
                f,
                "client asserted unadvertised capabilities: {}",
                names.join(", ")
            ),
            HandshakeError::Rejected => write!(f, "backend rejected the connection"),
        }
    }
}

/// Builds an ERR packet payload the proxy can send to a client on its own behalf.
pub fn err_payload(code: u16, sqlstate: &str, message: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(9 + message.len());
    out.push(0xFF);
    out.extend_from_slice(&code.to_le_bytes());
    out.push(b'#');
    let mut state = sqlstate.as_bytes().to_vec();
    state.resize(5, b'0');
    out.extend_from_slice(&state);
    out.extend_from_slice(message.as_bytes());
    out
}

/// What the proxy learned from the backend's initial handshake, plus the
/// rewritten packet to forward to the client.
struct ServerHandshake {
    masked_payload: Vec<u8>,
    capabilities: u32,
    connection_id: u32,
}

/// Parses the backend handshake and clears the masked capability bits in place.
///
/// The capability field is split across two locations in the packet, and both
/// sit at offsets that depend on the length of the server version string, so
/// the packet is parsed to find them and then patched rather than rebuilt.
fn mask_server_handshake(payload: &[u8]) -> Result<ServerHandshake, HandshakeError> {
    let mut c = Cursor::new(payload);
    let protocol_version = c
        .u8()
        .ok_or_else(|| HandshakeError::Protocol("empty handshake".into()))?;
    if protocol_version != 10 {
        return Err(HandshakeError::Protocol(format!(
            "unsupported protocol version {protocol_version}"
        )));
    }

    c.nul_bytes()
        .ok_or_else(|| HandshakeError::Protocol("truncated server version".into()))?;
    let connection_id = c
        .u32_le()
        .ok_or_else(|| HandshakeError::Protocol("truncated connection id".into()))?;
    c.bytes(8)
        .ok_or_else(|| HandshakeError::Protocol("truncated auth data".into()))?;
    c.u8()
        .ok_or_else(|| HandshakeError::Protocol("truncated filler".into()))?;

    let lower_at = c.position();
    let lower = c
        .u16_le()
        .ok_or_else(|| HandshakeError::Protocol("truncated capabilities".into()))?;

    let mut capabilities = lower as u32;
    let mut upper_at = None;
    if c.remaining() > 0 {
        c.u8()
            .ok_or_else(|| HandshakeError::Protocol("truncated charset".into()))?;
        c.u16_le()
            .ok_or_else(|| HandshakeError::Protocol("truncated status flags".into()))?;
        let at = c.position();
        let upper = c
            .u16_le()
            .ok_or_else(|| HandshakeError::Protocol("truncated capabilities".into()))?;
        capabilities |= (upper as u32) << 16;
        upper_at = Some(at);
    }

    let masked = capabilities & !MASKED_CAPABILITIES;
    let mut out = payload.to_vec();
    out[lower_at..lower_at + 2].copy_from_slice(&(masked as u16).to_le_bytes());
    if let Some(at) = upper_at {
        out[at..at + 2].copy_from_slice(&((masked >> 16) as u16).to_le_bytes());
    }

    Ok(ServerHandshake {
        masked_payload: out,
        capabilities: masked,
        connection_id,
    })
}

/// Fields the proxy needs from the client's handshake response.
struct ClientHandshake {
    capabilities: u32,
    username: String,
    database: Option<String>,
}

fn parse_client_handshake(payload: &[u8]) -> Result<ClientHandshake, HandshakeError> {
    let mut c = Cursor::new(payload);
    let capabilities = c
        .u32_le()
        .ok_or_else(|| HandshakeError::Protocol("truncated client capabilities".into()))?;

    // An SSLRequest packet is a 32-byte prefix of the full response; the caller
    // rejects it via the forbidden-capability check before we get this far.
    if payload.len() <= 32 {
        return Ok(ClientHandshake {
            capabilities,
            username: String::new(),
            database: None,
        });
    }

    c.skip(4)
        .and_then(|_| c.u8())
        .and_then(|_| c.skip(23))
        .ok_or_else(|| HandshakeError::Protocol("truncated handshake response".into()))?;

    let username = c
        .nul_bytes()
        .map(|b| String::from_utf8_lossy(b).into_owned())
        .ok_or_else(|| HandshakeError::Protocol("truncated username".into()))?;

    // Skip the auth response, whose framing depends on negotiated capabilities.
    if capabilities & CLIENT_PLUGIN_AUTH_LENENC_CLIENT_DATA != 0 {
        c.lenenc_bytes()
            .ok_or_else(|| HandshakeError::Protocol("truncated auth response".into()))?;
    } else if capabilities & CLIENT_SECURE_CONNECTION != 0 {
        let len = c
            .u8()
            .ok_or_else(|| HandshakeError::Protocol("truncated auth response".into()))?;
        c.bytes(len as usize)
            .ok_or_else(|| HandshakeError::Protocol("truncated auth response".into()))?;
    } else {
        c.nul_bytes()
            .ok_or_else(|| HandshakeError::Protocol("truncated auth response".into()))?;
    }

    let database = if capabilities & CLIENT_CONNECT_WITH_DB != 0 {
        c.nul_bytes()
            .map(|b| String::from_utf8_lossy(b).into_owned())
            .filter(|s| !s.is_empty())
    } else {
        None
    };

    Ok(ClientHandshake {
        capabilities,
        username,
        database,
    })
}

/// Runs the connection phase to completion, relaying packets in both directions.
pub async fn run<CR, CW, BR, BW>(
    client_rx: &mut PacketReader<CR>,
    client_tx: &mut PacketWriter<CW>,
    backend_rx: &mut PacketReader<BR>,
    backend_tx: &mut PacketWriter<BW>,
) -> Result<Session, HandshakeError>
where
    CR: AsyncRead + Unpin,
    CW: AsyncWrite + Unpin,
    BR: AsyncRead + Unpin,
    BW: AsyncWrite + Unpin,
{
    // 1. Backend handshake -> mask -> client.
    let greeting = backend_rx
        .next_packet()
        .await?
        .ok_or_else(|| HandshakeError::Protocol("backend closed before handshake".into()))?;

    // A backend that refuses the connection outright answers with ERR instead
    // of a handshake; relay it so the client sees the real reason.
    if greeting.payload.first() == Some(&0xFF) {
        client_tx.write_packet(greeting.seq, &greeting.payload).await?;
        return Err(HandshakeError::Rejected);
    }

    let server = mask_server_handshake(&greeting.payload)?;
    client_tx
        .write_packet(greeting.seq, &server.masked_payload)
        .await?;

    // 2. Client response -> backend, after checking it respects the mask.
    let response = client_rx
        .next_packet()
        .await?
        .ok_or_else(|| HandshakeError::Protocol("client closed during handshake".into()))?;
    let client = parse_client_handshake(&response.payload)?;

    // Real clients advertise what they are capable of, not what the server
    // offered — `CLIENT_LOCAL_FILES` and `CLIENT_QUERY_ATTRIBUTES` are set
    // unconditionally by the official client. Capabilities are an intersection,
    // so the right response is to clear the masked bits from what the backend
    // sees, not to refuse the connection.
    //
    // `CLIENT_SSL` is the exception: a client that sets it sends a truncated
    // response and immediately begins a TLS handshake, so there is no plaintext
    // session left to continue and the connection must be refused.
    if client.capabilities & CLIENT_SSL != 0 {
        return Err(HandshakeError::ForbiddenCapability(masked_names(CLIENT_SSL)));
    }

    let client_capabilities = client.capabilities & !MASKED_CAPABILITIES;
    let mut forwarded = response.payload.to_vec();
    forwarded[..4].copy_from_slice(&client_capabilities.to_le_bytes());
    backend_tx.write_packet(response.seq, &forwarded).await?;

    // Only bits both sides set are actually in force.
    let negotiated = client_capabilities & server.capabilities;
    let session = Session {
        capabilities: negotiated,
        username: client.username,
        database: client.database,
        backend_connection_id: server.connection_id,
    };

    // 3. Relay until the backend concludes the phase.
    //
    // Whether the client answers a given backend packet is determined by the
    // packet itself, so the exchange stays a strict alternation and needs no
    // concurrent read of both legs.
    loop {
        let from_backend = backend_rx.next_packet().await?.ok_or_else(|| {
            HandshakeError::Protocol("backend closed during authentication".into())
        })?;
        client_tx
            .write_packet(from_backend.seq, &from_backend.payload)
            .await?;

        match from_backend.payload.first() {
            Some(0x00) => return Ok(session),
            Some(0xFF) => return Err(HandshakeError::Rejected),
            // `caching_sha2_password` reports fast-auth success as AuthMoreData
            // 0x03 and then sends OK itself; the client says nothing in between.
            Some(0x01) if from_backend.payload.get(1) == Some(&0x03) => continue,
            None => {
                return Err(HandshakeError::Protocol(
                    "empty packet during authentication".into(),
                ))
            }
            _ => {}
        }

        let from_client = client_rx.next_packet().await?.ok_or_else(|| {
            HandshakeError::Protocol("client closed during authentication".into())
        })?;
        backend_tx
            .write_packet(from_client.seq, &from_client.payload)
            .await?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_handshake(caps: u32) -> Vec<u8> {
        let mut p = Vec::new();
        p.push(10);
        p.extend_from_slice(b"8.0.36\0");
        p.extend_from_slice(&42u32.to_le_bytes());
        p.extend_from_slice(b"12345678");
        p.push(0);
        p.extend_from_slice(&(caps as u16).to_le_bytes());
        p.push(0x21);
        p.extend_from_slice(&2u16.to_le_bytes());
        p.extend_from_slice(&((caps >> 16) as u16).to_le_bytes());
        p.push(21);
        p.extend_from_slice(&[0u8; 10]);
        p.extend_from_slice(b"abcdefghijkl\0");
        p.extend_from_slice(b"caching_sha2_password\0");
        p
    }

    #[test]
    fn masks_unsupported_capabilities() {
        let caps = CLIENT_PROTOCOL_41
            | CLIENT_SSL
            | CLIENT_COMPRESS
            | CLIENT_LOCAL_FILES
            | CLIENT_ZSTD_COMPRESSION_ALGORITHM
            | CLIENT_QUERY_ATTRIBUTES
            | CLIENT_DEPRECATE_EOF
            | CLIENT_PLUGIN_AUTH;
        let out = mask_server_handshake(&sample_handshake(caps)).unwrap();

        assert_eq!(out.capabilities & MASKED_CAPABILITIES, 0);
        assert!(out.capabilities & CLIENT_PROTOCOL_41 != 0);
        assert!(out.capabilities & CLIENT_DEPRECATE_EOF != 0);
        assert_eq!(out.connection_id, 42);

        // Re-parsing the rewritten packet must agree, and the packet must not
        // have changed length.
        let reparsed = mask_server_handshake(&out.masked_payload).unwrap();
        assert_eq!(reparsed.capabilities, out.capabilities);
        assert_eq!(out.masked_payload.len(), sample_handshake(caps).len());
    }

    #[test]
    fn leaves_supported_capabilities_alone() {
        let caps = CLIENT_PROTOCOL_41 | CLIENT_SECURE_CONNECTION | CLIENT_MULTI_RESULTS;
        let out = mask_server_handshake(&sample_handshake(caps)).unwrap();
        assert_eq!(out.capabilities, caps);
    }

    #[test]
    fn rejects_a_non_v10_handshake() {
        let mut p = sample_handshake(CLIENT_PROTOCOL_41);
        p[0] = 9;
        assert!(mask_server_handshake(&p).is_err());
    }

    fn sample_client_response(caps: u32, user: &str, db: Option<&str>) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&caps.to_le_bytes());
        p.extend_from_slice(&(16u32 * 1024 * 1024).to_le_bytes());
        p.push(0x21);
        p.extend_from_slice(&[0u8; 23]);
        p.extend_from_slice(user.as_bytes());
        p.push(0);
        p.push(4);
        p.extend_from_slice(b"auth");
        if let Some(db) = db {
            p.extend_from_slice(db.as_bytes());
            p.push(0);
        }
        p
    }

    #[test]
    fn parses_username_and_database() {
        let caps = CLIENT_PROTOCOL_41 | CLIENT_SECURE_CONNECTION | CLIENT_CONNECT_WITH_DB;
        let parsed =
            parse_client_handshake(&sample_client_response(caps, "app_user", Some("shop")))
                .unwrap();
        assert_eq!(parsed.username, "app_user");
        assert_eq!(parsed.database.as_deref(), Some("shop"));
    }

    #[test]
    fn parses_response_without_database() {
        let caps = CLIENT_PROTOCOL_41 | CLIENT_SECURE_CONNECTION;
        let parsed = parse_client_handshake(&sample_client_response(caps, "u", None)).unwrap();
        assert_eq!(parsed.username, "u");
        assert!(parsed.database.is_none());
    }

    #[test]
    fn detects_a_tls_upgrade_attempt_in_the_client_response() {
        let caps = CLIENT_PROTOCOL_41 | CLIENT_SECURE_CONNECTION | CLIENT_SSL;
        let parsed = parse_client_handshake(&sample_client_response(caps, "u", None)).unwrap();
        assert_ne!(parsed.capabilities & CLIENT_SSL, 0);
        assert_eq!(masked_names(CLIENT_SSL), vec!["CLIENT_SSL"]);
    }

    #[test]
    fn clearing_masked_bits_leaves_the_response_length_unchanged() {
        // Real clients set these regardless of what the server advertised, so
        // the proxy clears them in place rather than refusing the connection.
        let caps = CLIENT_PROTOCOL_41
            | CLIENT_SECURE_CONNECTION
            | CLIENT_LOCAL_FILES
            | CLIENT_QUERY_ATTRIBUTES
            | CLIENT_CONNECT_WITH_DB;
        let original = sample_client_response(caps, "app_user", Some("shop"));

        let cleared = caps & !MASKED_CAPABILITIES;
        let mut forwarded = original.clone();
        forwarded[..4].copy_from_slice(&cleared.to_le_bytes());

        assert_eq!(forwarded.len(), original.len());
        let reparsed = parse_client_handshake(&forwarded).unwrap();
        assert_eq!(reparsed.capabilities & MASKED_CAPABILITIES, 0);
        assert_ne!(reparsed.capabilities & CLIENT_PROTOCOL_41, 0);
        // Fields after the capability word must still parse identically.
        assert_eq!(reparsed.username, "app_user");
        assert_eq!(reparsed.database.as_deref(), Some("shop"));
    }

    #[test]
    fn err_payload_is_well_formed() {
        let p = err_payload(1235, "0A000", "nope");
        assert_eq!(p[0], 0xFF);
        assert_eq!(u16::from_le_bytes([p[1], p[2]]), 1235);
        assert_eq!(p[3], b'#');
        assert_eq!(&p[4..9], b"0A000");
        assert_eq!(&p[9..], b"nope");
    }
}
