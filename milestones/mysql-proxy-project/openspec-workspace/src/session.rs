//! Client session handling, 1:1 backend mapping, and passthrough authentication
//! by relaying Doris's own handshake salt (design D1).
//!
//! Owned by the `connection-routing` capability. See
//! `openspec/changes/add-row-filter-proxy-mvp/specs/connection-routing/spec.md`.
//!
//! # Why the backend connection phase is hand-rolled
//!
//! D1 requires the proxy to present **Doris's** salt to the client and relay the
//! client's scramble to Doris verbatim, so no plaintext password ever exists
//! inside the proxy. The server half of that is supported by `opensrv-mysql`
//! ([`AsyncMysqlShim::salt`] is overridable, and [`AsyncMysqlShim::authenticate`]
//! hands over the client's raw auth bytes). The backend half is **not** supported
//! by `mysql_async` 0.37: its handshake nonce is a private field with no
//! accessor, and its handshake response is derived from a plaintext password held
//! in `Opts`. There is no constructor that takes an already-authenticated stream.
//!
//! So the backend connection phase is written here against a raw
//! [`tokio::net::TcpStream`]. This costs no dependency: the proxy never *computes*
//! a scramble — that is the whole point of D1 — so no hashing crate is needed.
//! `mysql_async` is consequently unused by this module.
//!
//! Only `mysql_native_password` is accepted from the backend. It is the one
//! plugin whose exchange is a single round trip over a server-chosen salt, which
//! is exactly what gets relayed; anything else would need the plaintext password
//! the proxy never holds. A frontend requiring another plugin is refused with
//! [`BackendRefusal::UnsupportedAuthPlugin`], which is reported to the client as
//! a proxy limitation rather than as a credential failure.
//!
//! # Capability flags (design D4)
//!
//! `opensrv-mysql` hardcodes the capability set it advertises to the client and
//! offers no hook to change it. That set happens to omit `CLIENT_MULTI_STATEMENTS`
//! and `CLIENT_MULTI_RESULTS`, so D4's observable outcome holds — by accident of
//! the crate, not by our control. [`advertised_client_capabilities`] and its test
//! pin that, so a dependency bump that reintroduces either flag breaks the build
//! rather than the control.
//!
//! Clearing the advertised bit is advisory in any case: nothing in `opensrv-mysql`
//! rejects a client that sets `CLIENT_MULTI_STATEMENTS` anyway and sends a
//! `;`-separated payload. Multi-statement rejection must therefore also happen at
//! the SQL level, in the statement gate. The backend connection is negotiated
//! without either flag as a second line of defence, so Doris itself would refuse
//! anything that got that far.
//!
//! # The statement gate
//!
//! This module owns connections and identity; it does not decide what SQL is
//! safe. That is [`StatementGate`]. [`PolicyGate`] is the production
//! implementation, joining the authenticated username and the session's current
//! database to `rewrite::rewrite_statement`. [`RefuseAllGate`] is the fail-closed
//! default for a build whose analysis stage is not connected: it refuses every
//! statement rather than forwarding unexamined SQL.

use std::sync::Arc;

use async_trait::async_trait;

use opensrv_mysql::{
    AsyncMysqlIntermediary, AsyncMysqlShim, CapabilityFlags, Column, ColumnFlags, ColumnType,
    ErrorKind, InitWriter, OkResponse, ParamParser, QueryResultWriter, StatementMetaWriter,
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, ToSocketAddrs};
use tokio::sync::Mutex;

use crate::error::{ProxyError, RefusalReason, Result};
use crate::policy::PolicySet;
use crate::rewrite::rewrite_statement;

/// Length of a `mysql_native_password` scramble.
const SCRAMBLE_LEN: usize = 20;

/// The only auth plugin passthrough can relay. See [`BackendConnection::connect`].
pub const MYSQL_NATIVE_PASSWORD: &str = "mysql_native_password";

/// Connection-phase packets are small. Refusing a larger one keeps a hostile or
/// broken backend from provoking an unbounded allocation (invariant 1).
const CONNECTION_PHASE_PACKET_LIMIT: usize = 64 * 1024;

/// The largest single packet body the MySQL framing can express.
const MAX_PACKET_BODY: usize = 0xff_ff_ff;

// ---------------------------------------------------------------------------
// Byte-level helpers
//
// Every accessor is bounds-checked and returns an error rather than panicking:
// these parse bytes the proxy did not produce (invariant 2).
// ---------------------------------------------------------------------------

struct Bytes<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Bytes<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Bytes { buf, pos: 0 }
    }

    fn short(what: &str) -> ProxyError {
        ProxyError::Protocol(format!("packet truncated while reading {what}"))
    }

    fn remaining(&self) -> usize {
        self.buf.len() - self.pos
    }

    fn is_empty(&self) -> bool {
        self.remaining() == 0
    }

    fn take(&mut self, n: usize, what: &str) -> Result<&'a [u8]> {
        if self.remaining() < n {
            return Err(Self::short(what));
        }
        let out = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(out)
    }

    fn u8(&mut self, what: &str) -> Result<u8> {
        Ok(self.take(1, what)?[0])
    }

    fn u16_le(&mut self, what: &str) -> Result<u16> {
        let b = self.take(2, what)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    fn u32_le(&mut self, what: &str) -> Result<u32> {
        let b = self.take(4, what)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn nul_terminated(&mut self, what: &str) -> Result<&'a [u8]> {
        match self.buf[self.pos..].iter().position(|&b| b == 0) {
            Some(end) => {
                let out = &self.buf[self.pos..self.pos + end];
                self.pos += end + 1;
                Ok(out)
            }
            None => Err(Self::short(what)),
        }
    }

    /// A length-encoded integer. `0xfb` (NULL) and `0xff` (ERR marker) are not
    /// valid lengths in the positions this is used and are rejected.
    fn lenenc_int(&mut self, what: &str) -> Result<u64> {
        let first = self.u8(what)?;
        match first {
            0x00..=0xfa => Ok(u64::from(first)),
            0xfc => Ok(u64::from(self.u16_le(what)?)),
            0xfd => {
                let b = self.take(3, what)?;
                Ok(u64::from(u32::from_le_bytes([b[0], b[1], b[2], 0])))
            }
            0xfe => {
                let b = self.take(8, what)?;
                Ok(u64::from_le_bytes([
                    b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
                ]))
            }
            _ => Err(ProxyError::Protocol(format!(
                "unexpected length-encoded integer prefix {first:#04x} while reading {what}"
            ))),
        }
    }

    /// A length-encoded string, or `None` for a SQL NULL field (`0xfb`).
    fn lenenc_bytes(&mut self, what: &str) -> Result<Option<&'a [u8]>> {
        if self.remaining() >= 1 && self.buf[self.pos] == 0xfb {
            self.pos += 1;
            return Ok(None);
        }
        let len = self.lenenc_int(what)?;
        let len = usize::try_from(len).map_err(|_| {
            ProxyError::Protocol(format!("{what} length exceeds addressable memory"))
        })?;
        Ok(Some(self.take(len, what)?))
    }

    fn skip(&mut self, n: usize, what: &str) -> Result<()> {
        self.take(n, what).map(|_| ())
    }
}

#[cfg(test)]
fn lenenc_int_bytes(value: usize, out: &mut Vec<u8>) {
    match value {
        0..=0xfa => out.push(value as u8),
        0xfb..=0xffff => {
            out.push(0xfc);
            out.extend_from_slice(&(value as u16).to_le_bytes());
        }
        0x1_0000..=0xff_ffff => {
            out.push(0xfd);
            out.extend_from_slice(&(value as u32).to_le_bytes()[..3]);
        }
        _ => {
            out.push(0xfe);
            out.extend_from_slice(&(value as u64).to_le_bytes());
        }
    }
}

async fn read_packet<R>(reader: &mut R, limit: usize) -> Result<(u8, Vec<u8>)>
where
    R: AsyncRead + Unpin,
{
    let mut header = [0u8; 4];
    reader.read_exact(&mut header).await?;
    let len = u32::from_le_bytes([header[0], header[1], header[2], 0]) as usize;
    let seq = header[3];
    if len > limit {
        return Err(ProxyError::Protocol(format!(
            "backend packet of {len} bytes exceeds the {limit}-byte limit the proxy accepts"
        )));
    }
    if len == MAX_PACKET_BODY {
        // A body of exactly 2^24-1 means the payload continues in the next
        // packet. Reassembling it would mean buffering without a bound, so the
        // proxy refuses instead of quietly violating invariant 1.
        return Err(ProxyError::Protocol(
            "backend sent a continued (multi-part) packet, which the proxy does not reassemble"
                .to_string(),
        ));
    }
    let mut body = vec![0u8; len];
    reader.read_exact(&mut body).await?;
    Ok((seq, body))
}

async fn write_packet<W>(writer: &mut W, seq: u8, body: &[u8]) -> Result<()>
where
    W: AsyncWrite + Unpin,
{
    if body.len() >= MAX_PACKET_BODY {
        return Err(ProxyError::Protocol(
            "refusing to write a packet that would need multi-part framing".to_string(),
        ));
    }
    let len = (body.len() as u32).to_le_bytes();
    writer.write_all(&[len[0], len[1], len[2], seq]).await?;
    writer.write_all(body).await?;
    writer.flush().await?;
    Ok(())
}

/// Render an ERR packet body as `code (sqlstate): message`.
///
/// The backend's own text is preserved: `connection-routing` requires backend
/// errors to reach the client rather than being replaced by a proxy message.
fn describe_err_packet(body: &[u8]) -> String {
    let mut r = Bytes::new(body);
    let mut out = String::new();
    match r.u8("error header").ok().zip(r.u16_le("error code").ok()) {
        Some((_, code)) => {
            out.push_str(&code.to_string());
            // Protocol 41 marks the SQL state with a '#' sigil.
            if r.remaining() >= 6 && r.buf[r.pos] == b'#' {
                if let Ok(state) = r.take(6, "sql state") {
                    out.push_str(&format!(" ({})", String::from_utf8_lossy(&state[1..])));
                }
            }
            out.push_str(": ");
            out.push_str(&String::from_utf8_lossy(&r.buf[r.pos..]));
        }
        None => out.push_str("malformed error packet from backend"),
    }
    out
}

/// Build an ERR packet body, used when the proxy must refuse a client before
/// `opensrv-mysql` has taken over the socket.
fn err_packet_body(kind: ErrorKind, message: &str) -> Vec<u8> {
    let mut body = Vec::with_capacity(message.len() + 9);
    body.push(0xff);
    body.extend_from_slice(&(kind as u16).to_le_bytes());
    body.push(b'#');
    body.extend_from_slice(kind.sqlstate());
    body.extend_from_slice(message.as_bytes());
    body
}

// ---------------------------------------------------------------------------
// Backend connection phase
// ---------------------------------------------------------------------------

/// What Doris told us in its Initial Handshake Packet.
///
/// [`BackendHandshake::salt`] is the value design D1 turns around and presents to
/// the client as the proxy's own.
#[derive(Debug, Clone)]
pub struct BackendHandshake {
    server_version: String,
    connection_id: u32,
    capabilities: CapabilityFlags,
    salt: [u8; SCRAMBLE_LEN],
    auth_plugin: String,
}

impl BackendHandshake {
    pub fn server_version(&self) -> &str {
        &self.server_version
    }

    pub fn connection_id(&self) -> u32 {
        self.connection_id
    }

    pub fn capabilities(&self) -> CapabilityFlags {
        self.capabilities
    }

    pub fn salt(&self) -> [u8; SCRAMBLE_LEN] {
        self.salt
    }

    pub fn auth_plugin(&self) -> &str {
        &self.auth_plugin
    }
}

/// Parse a `Protocol::HandshakeV10` body.
pub fn parse_initial_handshake(body: &[u8]) -> Result<BackendHandshake> {
    let mut r = Bytes::new(body);
    let protocol_version = r.u8("protocol version")?;
    if protocol_version == 0xff {
        return Err(ProxyError::Backend(format!(
            "Doris refused the connection: {}",
            describe_err_packet(body)
        )));
    }
    if protocol_version != 10 {
        return Err(ProxyError::Protocol(format!(
            "backend speaks handshake protocol {protocol_version}, expected 10"
        )));
    }

    let server_version = String::from_utf8_lossy(r.nul_terminated("server version")?).into_owned();
    let connection_id = r.u32_le("connection id")?;
    let scramble_1 = r.take(8, "auth-plugin-data-part-1")?.to_vec();
    r.skip(1, "filler after auth-plugin-data-part-1")?;
    let capabilities_low = r.u16_le("lower capability flags")?;

    if r.is_empty() {
        return Err(ProxyError::Protocol(
            "backend sent a pre-4.1 handshake, which passthrough authentication cannot use"
                .to_string(),
        ));
    }

    r.skip(1, "character set")?;
    r.skip(2, "status flags")?;
    let capabilities_high = r.u16_le("upper capability flags")?;
    let capabilities = CapabilityFlags::from_bits_truncate(
        u32::from(capabilities_low) | (u32::from(capabilities_high) << 16),
    );

    let auth_plugin_data_len = r.u8("auth-plugin-data length")? as usize;
    r.skip(10, "reserved bytes")?;

    // $len = MAX(13, auth_plugin_data_len - 8); the trailing byte is a NUL that
    // is not part of the scramble.
    let part_2_len = auth_plugin_data_len.saturating_sub(8).max(13);
    let scramble_2 = r.take(part_2_len.min(r.remaining()), "auth-plugin-data-part-2")?;
    let scramble_2 = scramble_2.split(|&b| b == 0).next().unwrap_or(&[]).to_vec();

    let auth_plugin = if capabilities.contains(CapabilityFlags::CLIENT_PLUGIN_AUTH) {
        String::from_utf8_lossy(r.nul_terminated("auth plugin name")?).into_owned()
    } else {
        MYSQL_NATIVE_PASSWORD.to_string()
    };

    let mut combined = scramble_1;
    combined.extend_from_slice(&scramble_2);
    let salt: [u8; SCRAMBLE_LEN] = combined.get(..SCRAMBLE_LEN).and_then(|s| s.try_into().ok()).ok_or_else(|| {
        ProxyError::Protocol(format!(
            "backend salt is {} bytes, but relaying a mysql_native_password scramble needs {SCRAMBLE_LEN}",
            combined.len()
        ))
    })?;

    Ok(BackendHandshake {
        server_version,
        connection_id,
        capabilities,
        salt,
        auth_plugin,
    })
}

/// The capability set the proxy asks for on the **backend** connection.
///
/// `CLIENT_MULTI_STATEMENTS` and `CLIENT_MULTI_RESULTS` are deliberately absent:
/// even if something smuggled a multi-statement payload past the SQL analysis,
/// Doris would refuse it (design D4).
fn desired_backend_capabilities() -> CapabilityFlags {
    CapabilityFlags::CLIENT_PROTOCOL_41
        | CapabilityFlags::CLIENT_SECURE_CONNECTION
        | CapabilityFlags::CLIENT_PLUGIN_AUTH
        | CapabilityFlags::CLIENT_LONG_PASSWORD
        | CapabilityFlags::CLIENT_LONG_FLAG
        | CapabilityFlags::CLIENT_TRANSACTIONS
        | CapabilityFlags::CLIENT_DEPRECATE_EOF
}

/// The capability set `opensrv-mysql` advertises to the **client**.
///
/// The crate hardcodes this with no hook to change it (`opensrv-mysql-0.7.0`,
/// `src/lib.rs:308-313`), so this function is a mirror, not a control. Its
/// purpose is to make D4 checkable: see `advertised_capabilities_clear_multi_flags`
/// and the on-the-wire test in `tests/session_passthrough.rs`.
pub fn advertised_client_capabilities() -> CapabilityFlags {
    CapabilityFlags::CLIENT_PROTOCOL_41
        | CapabilityFlags::CLIENT_SECURE_CONNECTION
        | CapabilityFlags::CLIENT_PLUGIN_AUTH
        | CapabilityFlags::CLIENT_PLUGIN_AUTH_LENENC_CLIENT_DATA
        | CapabilityFlags::CLIENT_CONNECT_WITH_DB
        | CapabilityFlags::CLIENT_DEPRECATE_EOF
}

/// Whatever came back after a `COM_QUERY`.
#[derive(Debug)]
enum QueryOutcome {
    /// No result set: an OK packet, carrying the counters the client expects.
    Completed(OkResponse),
    /// The backend refused the statement. Relayed to the client verbatim.
    Failed(String),
    /// A result set follows; read it with [`BackendConnection::next_row`].
    ResultSet(Vec<Column>),
}

/// One client session's exclusive connection to the Doris frontend.
///
/// Never pooled and never shared: dropping it closes the socket, which is how
/// "backend connection is released when the client disconnects" is implemented.
pub struct BackendConnection {
    stream: TcpStream,
    handshake: BackendHandshake,
    negotiated: CapabilityFlags,
}

/// Why a session could not be established.
///
/// Kept as distinct variants rather than one opaque failure so an operator
/// reading the client's error can tell a network problem from a Doris-side
/// refusal from a deployment that this proxy structurally cannot serve. In
/// particular, the auth-plugin case must never read as "your password is wrong":
/// no credential would work, and the fix is configuration, not a new password.
#[derive(Debug)]
pub enum BackendRefusal {
    /// The frontend could not be reached at all.
    Unreachable(std::io::Error),
    /// The frontend answered, but with an error instead of a handshake — too
    /// many connections, host blocked, and so on. Its own text is carried along.
    RefusedConnection(String),
    /// The frontend requires an auth plugin passthrough cannot relay.
    ///
    /// Only `mysql_native_password` is a single round trip over a server-chosen
    /// salt, which is what design D1 relays. Anything else would need the
    /// plaintext password the proxy deliberately never holds.
    UnsupportedAuthPlugin(String),
    /// The frontend's handshake was well-formed but unusable.
    IncompatibleHandshake(String),
}

impl BackendRefusal {
    /// The MySQL error code the client is told. Deliberately *not* an
    /// access-denied code for anything but a credential failure.
    pub fn error_kind(&self) -> ErrorKind {
        match self {
            BackendRefusal::Unreachable(_) | BackendRefusal::RefusedConnection(_) => {
                ErrorKind::ER_CONNECT_TO_MASTER
            }
            BackendRefusal::UnsupportedAuthPlugin(_) => ErrorKind::ER_NOT_SUPPORTED_AUTH_MODE,
            BackendRefusal::IncompatibleHandshake(_) => ErrorKind::ER_NOT_SUPPORTED_YET,
        }
    }

    pub fn client_message(&self) -> String {
        match self {
            BackendRefusal::Unreachable(error) => {
                format!("proxy could not reach the Doris frontend: {error}")
            }
            BackendRefusal::RefusedConnection(message) => {
                format!("the Doris frontend refused the proxy's connection: {message}")
            }
            BackendRefusal::UnsupportedAuthPlugin(plugin) => format!(
                "the Doris frontend requires the {plugin} authentication plugin; \
                 this proxy can only relay {MYSQL_NATIVE_PASSWORD}. \
                 This is a proxy limitation, not a credential problem"
            ),
            BackendRefusal::IncompatibleHandshake(message) => {
                format!("the Doris frontend's handshake is unusable by this proxy: {message}")
            }
        }
    }
}

impl std::fmt::Display for BackendRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.client_message())
    }
}

impl From<BackendRefusal> for ProxyError {
    fn from(refusal: BackendRefusal) -> Self {
        match refusal {
            BackendRefusal::Unreachable(error) => ProxyError::Io(error),
            other => ProxyError::Backend(other.client_message()),
        }
    }
}

impl BackendConnection {
    /// Dial Doris and read its handshake. Called **before** the client is asked
    /// to authenticate (design D1), which is also why an unreachable backend
    /// refuses the session outright.
    pub async fn connect<A: ToSocketAddrs>(addr: A) -> std::result::Result<Self, BackendRefusal> {
        let stream = TcpStream::connect(addr)
            .await
            .map_err(BackendRefusal::Unreachable)?;
        stream
            .set_nodelay(true)
            .map_err(BackendRefusal::Unreachable)?;
        Self::from_stream(stream).await
    }

    async fn from_stream(mut stream: TcpStream) -> std::result::Result<Self, BackendRefusal> {
        let (_, body) = read_packet(&mut stream, CONNECTION_PHASE_PACKET_LIMIT)
            .await
            .map_err(|e| BackendRefusal::IncompatibleHandshake(e.to_string()))?;

        let handshake = parse_initial_handshake(&body).map_err(|e| match e {
            // Doris sent an ERR packet where the handshake belongs.
            ProxyError::Backend(message) => BackendRefusal::RefusedConnection(message),
            other => BackendRefusal::IncompatibleHandshake(other.to_string()),
        })?;

        if handshake.auth_plugin != MYSQL_NATIVE_PASSWORD {
            // Relaying a scramble works only for a single-round-trip plugin.
            // caching_sha2_password's full-auth exchange needs the password the
            // proxy deliberately does not have, so refuse rather than half-relay.
            return Err(BackendRefusal::UnsupportedAuthPlugin(
                handshake.auth_plugin.clone(),
            ));
        }

        let negotiated = desired_backend_capabilities() & handshake.capabilities;
        for required in [
            CapabilityFlags::CLIENT_PROTOCOL_41,
            CapabilityFlags::CLIENT_SECURE_CONNECTION,
        ] {
            if !negotiated.contains(required) {
                return Err(BackendRefusal::IncompatibleHandshake(format!(
                    "the frontend does not offer {required:?}, which passthrough authentication requires"
                )));
            }
        }

        Ok(BackendConnection {
            stream,
            handshake,
            negotiated,
        })
    }

    pub fn handshake(&self) -> &BackendHandshake {
        &self.handshake
    }

    pub fn negotiated_capabilities(&self) -> CapabilityFlags {
        self.negotiated
    }

    /// Send the client's own scramble onward, unmodified.
    ///
    /// `auth_response` is the byte string the client computed over
    /// [`BackendHandshake::salt`]. The proxy neither inspects nor recomputes it.
    pub async fn authenticate(&mut self, username: &[u8], auth_response: &[u8]) -> Result<()> {
        let mut body = Vec::with_capacity(64 + username.len() + auth_response.len());
        body.extend_from_slice(&self.negotiated.bits().to_le_bytes());
        body.extend_from_slice(&(MAX_PACKET_BODY as u32).to_le_bytes());
        body.push(0x21); // utf8_general_ci, matching what opensrv advertises
        body.extend_from_slice(&[0u8; 23]);
        body.extend_from_slice(username);
        body.push(0x00);

        // CLIENT_CONNECT_WITH_DB is deliberately not negotiated: the client's
        // requested database arrives separately via `on_init`, so there is one
        // code path that sets the current database rather than two.
        let len = u8::try_from(auth_response.len()).map_err(|_| {
            ProxyError::Protocol("client auth response is too long to relay".to_string())
        })?;
        body.push(len);
        body.extend_from_slice(auth_response);

        if self
            .negotiated
            .contains(CapabilityFlags::CLIENT_PLUGIN_AUTH)
        {
            body.extend_from_slice(MYSQL_NATIVE_PASSWORD.as_bytes());
            body.push(0x00);
        }

        write_packet(&mut self.stream, 1, &body).await?;

        let (_, reply) = read_packet(&mut self.stream, CONNECTION_PHASE_PACKET_LIMIT).await?;
        match reply.first() {
            Some(0x00) => Ok(()),
            Some(0xff) => Err(ProxyError::Backend(describe_err_packet(&reply))),
            Some(0xfe) => Err(ProxyError::Protocol(
                "backend asked to switch auth plugin; the proxy holds no password to answer with"
                    .to_string(),
            )),
            Some(0x01) => Err(ProxyError::Protocol(
                "backend requested additional auth data, which passthrough cannot supply"
                    .to_string(),
            )),
            other => Err(ProxyError::Protocol(format!(
                "unexpected auth reply header {other:?} from backend"
            ))),
        }
    }

    async fn send_command(&mut self, command: u8, payload: &[u8]) -> Result<()> {
        let mut body = Vec::with_capacity(1 + payload.len());
        body.push(command);
        body.extend_from_slice(payload);
        write_packet(&mut self.stream, 0, &body).await
    }

    async fn next_packet(&mut self) -> Result<Vec<u8>> {
        let (_, body) = read_packet(&mut self.stream, MAX_PACKET_BODY).await?;
        Ok(body)
    }

    /// `COM_INIT_DB`. The session's current database is only updated when the
    /// backend accepts the change.
    pub async fn init_db(&mut self, database: &str) -> Result<()> {
        self.send_command(0x02, database.as_bytes()).await?;
        let reply = self.next_packet().await?;
        match reply.first() {
            Some(0x00) => Ok(()),
            Some(0xff) => Err(ProxyError::Backend(describe_err_packet(&reply))),
            other => Err(ProxyError::Protocol(format!(
                "unexpected COM_INIT_DB reply header {other:?} from backend"
            ))),
        }
    }

    async fn start_query(&mut self, sql: &str) -> Result<QueryOutcome> {
        self.send_command(0x03, sql.as_bytes()).await?;
        let first = self.next_packet().await?;
        match first.first() {
            Some(0x00) => Ok(QueryOutcome::Completed(parse_ok_packet(&first)?)),
            Some(0xff) => Ok(QueryOutcome::Failed(describe_err_packet(&first))),
            Some(0xfb) => Err(ProxyError::Protocol(
                "backend requested LOCAL INFILE, which the proxy does not relay".to_string(),
            )),
            Some(_) => {
                let mut r = Bytes::new(&first);
                let count = r.lenenc_int("result set column count")?;
                let count = usize::try_from(count).map_err(|_| {
                    ProxyError::Protocol("implausible result set column count".to_string())
                })?;
                let mut columns = Vec::with_capacity(count);
                for _ in 0..count {
                    let packet = self.next_packet().await?;
                    columns.push(parse_column_definition(&packet)?);
                }
                if !self
                    .negotiated
                    .contains(CapabilityFlags::CLIENT_DEPRECATE_EOF)
                {
                    let eof = self.next_packet().await?;
                    if eof.first() != Some(&0xfe) {
                        return Err(ProxyError::Protocol(
                            "expected EOF packet after column definitions".to_string(),
                        ));
                    }
                }
                Ok(QueryOutcome::ResultSet(columns))
            }
            None => Err(ProxyError::Protocol(
                "backend sent an empty packet in response to a query".to_string(),
            )),
        }
    }

    /// One row of the current result set, or `None` at its end.
    ///
    /// Fields are handed back as raw text-protocol bytes so they can be written
    /// on to the client unchanged — the proxy must not reinterpret values it is
    /// only relaying.
    #[allow(clippy::type_complexity)]
    async fn next_row(&mut self, column_count: usize) -> Result<Option<Vec<Option<Vec<u8>>>>> {
        let packet = self.next_packet().await?;
        match packet.first() {
            // With CLIENT_DEPRECATE_EOF the terminator is an OK packet that still
            // begins 0xfe; without it, an EOF packet. Both are short.
            Some(0xfe) if packet.len() < 9 => Ok(None),
            Some(0xff) => Err(ProxyError::Backend(describe_err_packet(&packet))),
            _ => {
                let mut r = Bytes::new(&packet);
                let mut row = Vec::with_capacity(column_count);
                for _ in 0..column_count {
                    row.push(r.lenenc_bytes("result row field")?.map(<[u8]>::to_vec));
                }
                Ok(Some(row))
            }
        }
    }

    /// The single scalar a one-row, one-column query returns.
    async fn query_scalar(&mut self, sql: &str) -> Result<Option<String>> {
        match self.start_query(sql).await? {
            QueryOutcome::Failed(message) => Err(ProxyError::Backend(message)),
            QueryOutcome::Completed(_) => Ok(None),
            QueryOutcome::ResultSet(columns) => {
                let mut first = None;
                while let Some(row) = self.next_row(columns.len()).await? {
                    if first.is_none() {
                        first = row
                            .into_iter()
                            .next()
                            .flatten()
                            .map(|v| String::from_utf8_lossy(&v).into_owned());
                    }
                }
                Ok(first)
            }
        }
    }
}

fn parse_ok_packet(body: &[u8]) -> Result<OkResponse> {
    let mut r = Bytes::new(body);
    r.skip(1, "ok header")?;
    let affected_rows = r.lenenc_int("affected rows")?;
    let last_insert_id = r.lenenc_int("last insert id")?;
    let status_flags = r.u16_le("status flags").unwrap_or(0);
    let warnings = r.u16_le("warnings").unwrap_or(0);
    Ok(OkResponse {
        header: 0x00,
        affected_rows,
        last_insert_id,
        status_flags: opensrv_mysql::StatusFlags::from_bits_truncate(status_flags),
        warnings,
        info: String::new(),
        session_state_info: String::new(),
    })
}

fn parse_column_definition(body: &[u8]) -> Result<Column> {
    let mut r = Bytes::new(body);
    r.lenenc_bytes("column catalog")?;
    r.lenenc_bytes("column schema")?;
    let table = r.lenenc_bytes("column table")?.unwrap_or(&[]).to_vec();
    r.lenenc_bytes("column org_table")?;
    let name = r.lenenc_bytes("column name")?.unwrap_or(&[]).to_vec();
    r.lenenc_bytes("column org_name")?;
    r.lenenc_int("column fixed-field length")?;
    r.skip(2, "column character set")?;
    r.skip(4, "column length")?;
    let coltype = r.u8("column type")?;
    let colflags = r.u16_le("column flags")?;

    Ok(Column {
        table: String::from_utf8_lossy(&table).into_owned(),
        column: String::from_utf8_lossy(&name).into_owned(),
        coltype: ColumnType::try_from(coltype).map_err(|_| {
            ProxyError::Protocol(format!("backend sent unknown column type {coltype:#04x}"))
        })?,
        colflags: ColumnFlags::from_bits_truncate(colflags),
    })
}

// ---------------------------------------------------------------------------
// Session identity and the statement gate
// ---------------------------------------------------------------------------

/// Who the session is, once Doris has said so.
///
/// This type cannot be constructed outside a successful backend authentication,
/// which is how `connection-routing`'s "Doris is the sole authority on identity"
/// is enforced by the type system rather than by discipline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionIdentity {
    authenticated_user: String,
    claimed_user: String,
}

impl SessionIdentity {
    /// The username **Doris** reports for this session, taken from
    /// `CURRENT_USER()` rather than from the handshake. Policy is keyed on this.
    ///
    /// The two can differ: MySQL-family servers match an account by user *and*
    /// host pattern, and may resolve a connection to a different account than the
    /// one named. Keying policy on the claimed name would let a client pick its
    /// own policy by choosing what to type.
    pub fn authenticated_user(&self) -> &str {
        &self.authenticated_user
    }

    /// What the client typed. Kept for diagnostics only — never for policy.
    pub fn claimed_user(&self) -> &str {
        &self.claimed_user
    }
}

/// What the analysis stage decided about one statement.
#[derive(Debug)]
pub enum Admission {
    /// Forward this SQL — which may differ from what the client sent, because
    /// the rewriter has constrained it.
    Forward(String),
    /// Refuse, with a reason the client may see.
    Refuse(RefusalReason),
}

/// The seam between session handling and statement analysis.
///
/// `session.rs` owns connections and identity; it does not decide what SQL is
/// safe. Implementing this is how `analyze`/`rewrite` gets wired in.
pub trait StatementGate: Send + Sync + 'static {
    fn admit(
        &self,
        identity: &SessionIdentity,
        current_database: Option<&str>,
        sql: &str,
    ) -> Admission;
}

/// The default gate: refuses everything.
///
/// A proxy whose analysis stage has not been connected must not forward
/// unexamined SQL. This makes "not wired up yet" a visible outage rather than a
/// silent hole.
pub struct RefuseAllGate;

impl StatementGate for RefuseAllGate {
    fn admit(&self, _: &SessionIdentity, _: Option<&str>, _: &str) -> Admission {
        Admission::Refuse(RefusalReason::UnsupportedShape {
            construct: "statement analysis is not wired into this build".to_string(),
        })
    }
}

/// The gate a running proxy uses: analyse each statement against the policy set
/// and constrain it, or refuse.
///
/// This is the join between the three halves of the proxy. The session layer
/// supplies **who** — the username Doris authenticated, never the one the client
/// typed — and **where** — the current database, `None` when genuinely unset.
/// Both are inputs to a security decision: the user selects the policy, and the
/// database is what an unqualified table name resolves against. A `None`
/// database with a policy-bearing user makes an unqualified reference
/// unresolvable, which the rewriter refuses rather than guesses at.
pub struct PolicyGate {
    policies: PolicySet,
}

impl PolicyGate {
    pub fn new(policies: PolicySet) -> Self {
        PolicyGate { policies }
    }
}

impl StatementGate for PolicyGate {
    fn admit(
        &self,
        identity: &SessionIdentity,
        current_database: Option<&str>,
        sql: &str,
    ) -> Admission {
        match rewrite_statement(
            sql,
            identity.authenticated_user(),
            current_database,
            &self.policies,
        ) {
            Ok(rewritten) => Admission::Forward(rewritten),
            Err(reason) => Admission::Refuse(reason),
        }
    }
}

/// Forwards every statement unexamined.
///
/// **Applies no row filtering whatsoever.** It exists so the connection and
/// identity behaviour can be tested independently of the rewriter. Never deploy
/// a proxy configured with it.
pub struct UnfilteredGate;

impl StatementGate for UnfilteredGate {
    fn admit(&self, _: &SessionIdentity, _: Option<&str>, sql: &str) -> Admission {
        Admission::Forward(sql.to_string())
    }
}

// ---------------------------------------------------------------------------
// The session shim
// ---------------------------------------------------------------------------

/// One client session, holding its one backend connection.
pub struct ProxySession {
    salt: [u8; SCRAMBLE_LEN],
    auth_plugin: String,
    server_version: String,
    connection_id: u32,
    backend: Mutex<Option<BackendConnection>>,
    identity: Mutex<Option<SessionIdentity>>,
    current_database: Mutex<Option<String>>,
    gate: Arc<dyn StatementGate>,
}

impl ProxySession {
    /// Wrap an already-connected backend. Taking the connection by value is what
    /// makes the 1:1 mapping structural: a session cannot exist without exactly
    /// one backend connection, and cannot acquire a second.
    pub fn new(backend: BackendConnection, gate: Arc<dyn StatementGate>) -> Self {
        let handshake = backend.handshake().clone();
        ProxySession {
            salt: handshake.salt(),
            auth_plugin: handshake.auth_plugin().to_string(),
            server_version: handshake.server_version().to_string(),
            connection_id: handshake.connection_id(),
            backend: Mutex::new(Some(backend)),
            identity: Mutex::new(None),
            current_database: Mutex::new(None),
            gate,
        }
    }

    /// The session identity, or `None` until Doris has authenticated it.
    pub async fn identity(&self) -> Option<SessionIdentity> {
        self.identity.lock().await.clone()
    }

    /// The database `USE` last selected, tracked so unqualified table names can
    /// be resolved (`policy-config`'s unqualified-name rule).
    pub async fn current_database(&self) -> Option<String> {
        self.current_database.lock().await.clone()
    }

    /// Relay the client's scramble to Doris, and on success take the identity
    /// from Doris rather than from the handshake.
    async fn authenticate_against_backend(&self, username: &[u8], auth_response: &[u8]) -> bool {
        let claimed = String::from_utf8_lossy(username).into_owned();
        let mut guard = self.backend.lock().await;
        let Some(backend) = guard.as_mut() else {
            tracing::warn!("authentication attempted with no backend connection");
            return false;
        };

        if let Err(error) = backend.authenticate(username, auth_response).await {
            tracing::info!(%claimed, %error, "Doris rejected the session's credentials");
            // The backend connection is unusable and must not outlive the failed
            // attempt: dropping it closes the socket.
            *guard = None;
            return false;
        }

        // Ask Doris who it thinks this is. A client that claims `admin` but is
        // resolved by the server to `analyst` must get `analyst`'s policy.
        let authenticated_user = match backend.query_scalar("SELECT CURRENT_USER()").await {
            Ok(Some(value)) => strip_host_part(&value),
            Ok(None) => {
                tracing::error!("Doris returned no row for CURRENT_USER(); refusing the session");
                *guard = None;
                return false;
            }
            Err(error) => {
                tracing::error!(%error, "could not resolve CURRENT_USER(); refusing the session");
                *guard = None;
                return false;
            }
        };

        if authenticated_user != claimed {
            tracing::info!(
                %claimed,
                %authenticated_user,
                "backend resolved the session to a different account than the client claimed"
            );
        }

        *self.identity.lock().await = Some(SessionIdentity {
            authenticated_user,
            claimed_user: claimed,
        });
        true
    }
}

/// `CURRENT_USER()` returns `user@host`. Policy is keyed on the account name.
fn strip_host_part(current_user: &str) -> String {
    match current_user.rsplit_once('@') {
        Some((user, _host)) => user.trim_matches('\'').to_string(),
        None => current_user.trim_matches('\'').to_string(),
    }
}

#[async_trait]
impl<W> AsyncMysqlShim<W> for ProxySession
where
    W: AsyncWrite + Send + Unpin,
{
    type Error = ProxyError;

    /// Report Doris's own version, so clients apply the same compatibility
    /// quirks through the proxy as they would against the frontend directly.
    fn version(&self) -> String {
        self.server_version.clone()
    }

    /// Report the backend's connection id, so `KILL <id>` names something real.
    fn connect_id(&self) -> u32 {
        self.connection_id
    }

    fn default_auth_plugin(&self) -> &str {
        &self.auth_plugin
    }

    /// Design D1: the client is challenged with **Doris's** salt, so the response
    /// it computes is directly usable against Doris.
    fn salt(&self) -> [u8; SCRAMBLE_LEN] {
        self.salt
    }

    async fn auth_plugin_for_username(&self, _user: &[u8]) -> &str {
        &self.auth_plugin
    }

    async fn authenticate(
        &self,
        auth_plugin: &str,
        username: &[u8],
        salt: &[u8],
        auth_data: &[u8],
    ) -> bool {
        if auth_plugin != MYSQL_NATIVE_PASSWORD {
            tracing::warn!(%auth_plugin, "refusing a plugin the proxy cannot relay");
            return false;
        }
        // Defensive: the whole passthrough rests on the client having been
        // challenged with the backend's salt. If the crate ever hands over a
        // different one, the relay would be silently useless.
        if salt != self.salt {
            tracing::error!("handshake salt did not survive to the authenticate callback");
            return false;
        }
        self.authenticate_against_backend(username, auth_data).await
    }

    /// Design D4 / task 6.9: the text seen at prepare time is not what Doris
    /// executes at bind time, so the rewriter cannot vouch for it.
    async fn on_prepare<'a>(
        &'a mut self,
        _query: &'a str,
        info: StatementMetaWriter<'a, W>,
    ) -> std::result::Result<(), Self::Error> {
        let reason = RefusalReason::PreparedStatement;
        info.error(
            ErrorKind::ER_NOT_SUPPORTED_YET,
            reason.client_message().as_bytes(),
        )
        .await?;
        Ok(())
    }

    /// Unreachable in practice, since no statement is ever prepared — but a
    /// client may still send `COM_STMT_EXECUTE`, and the answer is the same.
    async fn on_execute<'a>(
        &'a mut self,
        _id: u32,
        _params: ParamParser<'a>,
        results: QueryResultWriter<'a, W>,
    ) -> std::result::Result<(), Self::Error> {
        let reason = RefusalReason::PreparedStatement;
        results
            .error(
                ErrorKind::ER_NOT_SUPPORTED_YET,
                reason.client_message().as_bytes(),
            )
            .await?;
        Ok(())
    }

    async fn on_close<'a>(&'a mut self, _stmt: u32)
    where
        W: 'async_trait,
    {
    }

    async fn on_query<'a>(
        &'a mut self,
        query: &'a str,
        results: QueryResultWriter<'a, W>,
    ) -> std::result::Result<(), Self::Error> {
        let Some(identity) = self.identity.lock().await.clone() else {
            // `opensrv-mysql` does not dispatch commands before authentication,
            // so this is a belt-and-braces check on that guarantee rather than
            // the only thing enforcing it.
            results
                .error(
                    ErrorKind::ER_ACCESS_DENIED_ERROR,
                    b"proxy refused statement: session is not authenticated",
                )
                .await?;
            return Ok(());
        };

        let current_database = self.current_database.lock().await.clone();
        let sql = match self
            .gate
            .admit(&identity, current_database.as_deref(), query)
        {
            Admission::Forward(sql) => sql,
            Admission::Refuse(reason) => {
                // `Debug`, not `Display`. Display is the client-facing wording,
                // which is deliberately identical across refusals that must not
                // be distinguishable from outside — an unsupported shape and an
                // unresolvable reference read the same to a client (design D5).
                // Only the variant separates them, and the operator needs that
                // separation: one is a compatibility gap, the other is a client
                // connecting without a default schema.
                tracing::info!(
                    user = %identity.authenticated_user(),
                    detail = ?reason,
                    "refused statement"
                );
                results
                    .error(
                        ErrorKind::ER_NOT_SUPPORTED_YET,
                        reason.client_message().as_bytes(),
                    )
                    .await?;
                return Ok(());
            }
        };

        let mut guard = self.backend.lock().await;
        let Some(backend) = guard.as_mut() else {
            results
                .error(
                    ErrorKind::ER_SERVER_SHUTDOWN,
                    b"proxy refused statement: the backend connection is gone",
                )
                .await?;
            return Ok(());
        };

        match backend.start_query(&sql).await? {
            QueryOutcome::Completed(ok) => results.completed(ok).await?,
            QueryOutcome::Failed(message) => {
                // `connection-routing`: the client sees Doris's error, not a
                // generic proxy failure.
                results
                    .error(ErrorKind::ER_UNKNOWN_ERROR, message.as_bytes())
                    .await?
            }
            QueryOutcome::ResultSet(columns) => {
                let count = columns.len();
                let mut writer = results.start(&columns).await?;
                // Streamed one row at a time: memory stays constant with respect
                // to result size (invariant 1).
                while let Some(row) = backend.next_row(count).await? {
                    writer.write_row(row).await?;
                }
                writer.finish().await?;
            }
        }
        Ok(())
    }

    async fn on_init<'a>(
        &'a mut self,
        database: &'a str,
        writer: InitWriter<'a, W>,
    ) -> std::result::Result<(), Self::Error> {
        let mut guard = self.backend.lock().await;
        let Some(backend) = guard.as_mut() else {
            writer
                .error(
                    ErrorKind::ER_SERVER_SHUTDOWN,
                    b"proxy refused statement: the backend connection is gone",
                )
                .await?;
            return Ok(());
        };

        match backend.init_db(database).await {
            Ok(()) => {
                // Only now, once Doris agreed, does the proxy's idea of the
                // current database change. A database the backend refused must
                // leave it untouched: `policy-config` resolves unqualified table
                // names against this, so a wrong value picks the wrong policy.
                *self.current_database.lock().await = Some(database.to_string());
                writer.ok().await?;
            }
            Err(ProxyError::Backend(message)) => {
                writer
                    .error(ErrorKind::ER_BAD_DB_ERROR, message.as_bytes())
                    .await?;
            }
            Err(other) => return Err(other),
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Listener
// ---------------------------------------------------------------------------

/// Accepts client connections and gives each one its own backend connection.
pub struct ProxyServer {
    backend_addr: String,
    gate: Arc<dyn StatementGate>,
}

impl ProxyServer {
    pub fn new(backend_addr: impl Into<String>, gate: Arc<dyn StatementGate>) -> Self {
        ProxyServer {
            backend_addr: backend_addr.into(),
            gate,
        }
    }

    /// Accept forever. Each session runs on its own task and owns its own
    /// backend connection; nothing is shared between them.
    pub async fn run(self: Arc<Self>, listener: TcpListener) -> Result<()> {
        loop {
            let (client, peer) = listener.accept().await?;
            let server = Arc::clone(&self);
            tokio::spawn(async move {
                if let Err(error) = server.handle_client(client).await {
                    tracing::info!(%peer, %error, "session ended");
                }
            });
        }
    }

    /// Drive one client session from accept to close.
    pub async fn handle_client(&self, mut client: TcpStream) -> Result<()> {
        client.set_nodelay(true)?;

        // Design D1: the backend comes first, because its salt is what the
        // client must be challenged with.
        let backend = match BackendConnection::connect(&self.backend_addr).await {
            Ok(backend) => backend,
            Err(refusal) => {
                // No backend means no session. Refusing here — in place of the
                // handshake — is what stops the proxy from accepting statements
                // it could never forward. The error code and text vary by cause
                // so an operator is not left guessing whether it is the network,
                // Doris, or this proxy's own limits.
                tracing::warn!(%refusal, "refusing client: no backend connection");
                let body = err_packet_body(refusal.error_kind(), &refusal.client_message());
                let _ = write_packet(&mut client, 0, &body).await;
                return Err(refusal.into());
            }
        };

        let session = ProxySession::new(backend, Arc::clone(&self.gate));
        let (reader, writer) = client.into_split();
        let outcome = AsyncMysqlIntermediary::run_on(session, reader, writer).await;
        // `session` was moved into `run_on` and is dropped as it returns, taking
        // the backend `TcpStream` with it. That is what closes the backend
        // connection when the client disconnects or the link is lost.
        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Design D4, as far as it can be checked without a socket. The wire-level
    /// version of this assertion is in `tests/session_passthrough.rs`.
    #[test]
    fn advertised_capabilities_clear_multi_flags() {
        let advertised = advertised_client_capabilities();
        assert!(!advertised.contains(CapabilityFlags::CLIENT_MULTI_STATEMENTS));
        assert!(!advertised.contains(CapabilityFlags::CLIENT_MULTI_RESULTS));
    }

    #[test]
    fn backend_connection_never_negotiates_multi_statements() {
        let desired = desired_backend_capabilities();
        assert!(!desired.contains(CapabilityFlags::CLIENT_MULTI_STATEMENTS));
        assert!(!desired.contains(CapabilityFlags::CLIENT_MULTI_RESULTS));
    }

    fn handshake_bytes(salt: &[u8; SCRAMBLE_LEN], plugin: &str) -> Vec<u8> {
        let caps = CapabilityFlags::CLIENT_PROTOCOL_41
            | CapabilityFlags::CLIENT_SECURE_CONNECTION
            | CapabilityFlags::CLIENT_PLUGIN_AUTH
            | CapabilityFlags::CLIENT_DEPRECATE_EOF;
        let bits = caps.bits().to_le_bytes();
        let mut body = vec![10];
        body.extend_from_slice(b"8.0.33-Doris\0");
        body.extend_from_slice(&77u32.to_le_bytes());
        body.extend_from_slice(&salt[..8]);
        body.push(0x00);
        body.extend_from_slice(&bits[..2]);
        body.push(0x21);
        body.extend_from_slice(&[0x00, 0x00]);
        body.extend_from_slice(&bits[2..]);
        body.push(21);
        body.extend_from_slice(&[0u8; 10]);
        body.extend_from_slice(&salt[8..]);
        body.push(0x00);
        body.extend_from_slice(plugin.as_bytes());
        body.push(0x00);
        body
    }

    #[test]
    fn handshake_salt_is_recovered_from_both_parts() {
        let salt: [u8; SCRAMBLE_LEN] = *b"DORIS-SALT-012345678";
        let parsed =
            parse_initial_handshake(&handshake_bytes(&salt, MYSQL_NATIVE_PASSWORD)).unwrap();
        assert_eq!(parsed.salt(), salt);
        assert_eq!(parsed.server_version(), "8.0.33-Doris");
        assert_eq!(parsed.connection_id(), 77);
        assert_eq!(parsed.auth_plugin(), MYSQL_NATIVE_PASSWORD);
    }

    #[test]
    fn truncated_handshake_is_an_error_not_a_panic() {
        let salt: [u8; SCRAMBLE_LEN] = *b"DORIS-SALT-012345678";
        let full = handshake_bytes(&salt, MYSQL_NATIVE_PASSWORD);
        for cut in 0..full.len() {
            // Invariant 2: no panic on bytes the proxy did not produce.
            let _ = parse_initial_handshake(&full[..cut]);
        }
    }

    #[test]
    fn error_packet_in_place_of_handshake_is_reported_as_a_backend_error() {
        let body = err_packet_body(ErrorKind::ER_CON_COUNT_ERROR, "Too many connections");
        let error = parse_initial_handshake(&body).unwrap_err();
        assert!(matches!(error, ProxyError::Backend(_)));
        assert!(error.to_string().contains("Too many connections"));
    }

    #[test]
    fn current_user_host_part_is_stripped() {
        assert_eq!(strip_host_part("analyst@%"), "analyst");
        assert_eq!(strip_host_part("'analyst'@'10.0.0.1'"), "analyst");
        assert_eq!(strip_host_part("analyst"), "analyst");
    }

    #[test]
    fn unwired_proxy_refuses_every_statement() {
        let identity = SessionIdentity {
            authenticated_user: "analyst".to_string(),
            claimed_user: "analyst".to_string(),
        };
        let admission = RefuseAllGate.admit(&identity, None, "SELECT 1");
        assert!(matches!(admission, Admission::Refuse(_)));
    }

    #[test]
    fn lenenc_int_round_trips() {
        for value in [0usize, 0xfa, 0xfb, 0xffff, 0x1_0000, 0xff_fffe] {
            let mut buf = Vec::new();
            lenenc_int_bytes(value, &mut buf);
            let decoded = Bytes::new(&buf).lenenc_int("test").unwrap();
            assert_eq!(decoded, value as u64);
        }
    }

    #[test]
    fn lenenc_bytes_reads_null_as_none() {
        let mut r = Bytes::new(&[0xfb, 0x02, b'h', b'i']);
        assert_eq!(r.lenenc_bytes("field").unwrap(), None);
        assert_eq!(r.lenenc_bytes("field").unwrap(), Some(&b"hi"[..]));
    }
}
