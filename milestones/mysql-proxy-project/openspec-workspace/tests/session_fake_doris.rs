//! A Doris frontend that exists only far enough to test the proxy's connection
//! phase, plus a raw MySQL client that speaks to the proxy the same way.
//!
//! There is no Doris instance available to this project, so every assertion
//! about passthrough authentication has to be made against something that
//! reproduces the wire protocol. Both halves here are deliberately dumb: the fake
//! frontend does no cryptography at all, which is what lets a test assert that a
//! scramble was relayed **byte for byte** rather than merely that authentication
//! succeeded. A fake that recomputed the hash could not tell the two apart.
//!
//! This file is a support module, included by the integration tests with
//! `mod session_fake_doris;`. Compiled as a test target of its own it contains no
//! tests, which is why every item is `#![allow(dead_code)]`-tolerant.

#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

pub const CLIENT_PROTOCOL_41: u32 = 0x0000_0200;
pub const CLIENT_CONNECT_WITH_DB: u32 = 0x0000_0008;
pub const CLIENT_SECURE_CONNECTION: u32 = 0x0000_8000;
pub const CLIENT_MULTI_STATEMENTS: u32 = 0x0001_0000;
pub const CLIENT_MULTI_RESULTS: u32 = 0x0002_0000;
pub const CLIENT_PLUGIN_AUTH: u32 = 0x0008_0000;
pub const CLIENT_PLUGIN_AUTH_LENENC_CLIENT_DATA: u32 = 0x0020_0000;
pub const CLIENT_DEPRECATE_EOF: u32 = 0x0100_0000;

/// The salt the fake frontend issues. Chosen to be recognisable in a failure
/// message and impossible to produce by accident.
pub const DORIS_SALT: &[u8; 20] = b"DORIS-SALT-012345678";

// ---------------------------------------------------------------------------
// Packet plumbing
// ---------------------------------------------------------------------------

pub fn lenenc_int(value: usize, out: &mut Vec<u8>) {
    match value {
        0..=0xfa => out.push(value as u8),
        0xfb..=0xffff => {
            out.push(0xfc);
            out.extend_from_slice(&(value as u16).to_le_bytes());
        }
        _ => {
            out.push(0xfd);
            out.extend_from_slice(&(value as u32).to_le_bytes()[..3]);
        }
    }
}

pub fn lenenc_bytes(value: &[u8], out: &mut Vec<u8>) {
    lenenc_int(value.len(), out);
    out.extend_from_slice(value);
}

pub async fn write_packet(stream: &mut TcpStream, seq: u8, body: &[u8]) -> std::io::Result<()> {
    let len = (body.len() as u32).to_le_bytes();
    stream.write_all(&[len[0], len[1], len[2], seq]).await?;
    stream.write_all(body).await?;
    stream.flush().await
}

pub async fn read_packet(stream: &mut TcpStream) -> std::io::Result<(u8, Vec<u8>)> {
    let mut header = [0u8; 4];
    stream.read_exact(&mut header).await?;
    let len = u32::from_le_bytes([header[0], header[1], header[2], 0]) as usize;
    let mut body = vec![0u8; len];
    stream.read_exact(&mut body).await?;
    Ok((header[3], body))
}

/// Bounds-checked reader. Tests must fail with a readable message, not a panic
/// in the middle of an async task.
pub struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Reader { buf, pos: 0 }
    }

    pub fn take(&mut self, n: usize) -> &'a [u8] {
        assert!(
            self.pos + n <= self.buf.len(),
            "packet truncated: wanted {n} bytes at offset {} of {}",
            self.pos,
            self.buf.len()
        );
        let out = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        out
    }

    pub fn u8(&mut self) -> u8 {
        self.take(1)[0]
    }

    pub fn u16_le(&mut self) -> u16 {
        let b = self.take(2);
        u16::from_le_bytes([b[0], b[1]])
    }

    pub fn u32_le(&mut self) -> u32 {
        let b = self.take(4);
        u32::from_le_bytes([b[0], b[1], b[2], b[3]])
    }

    pub fn nul_terminated(&mut self) -> &'a [u8] {
        let end = self.buf[self.pos..]
            .iter()
            .position(|&b| b == 0)
            .expect("unterminated string in packet");
        let out = &self.buf[self.pos..self.pos + end];
        self.pos += end + 1;
        out
    }

    pub fn lenenc_int(&mut self) -> u64 {
        match self.u8() {
            n @ 0..=0xfa => u64::from(n),
            0xfc => u64::from(self.u16_le()),
            0xfd => {
                let b = self.take(3);
                u64::from(u32::from_le_bytes([b[0], b[1], b[2], 0]))
            }
            0xfe => {
                let b = self.take(8);
                u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
            }
            other => panic!("unexpected length-encoded prefix {other:#04x}"),
        }
    }

    pub fn lenenc_bytes(&mut self) -> &'a [u8] {
        let len = self.lenenc_int() as usize;
        self.take(len)
    }

    pub fn rest(&self) -> &'a [u8] {
        &self.buf[self.pos..]
    }
}

pub fn ok_packet() -> Vec<u8> {
    vec![0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00]
}

/// The terminator of a result set when `CLIENT_DEPRECATE_EOF` is negotiated: an
/// OK packet wearing the EOF header.
pub fn eof_ok_packet() -> Vec<u8> {
    vec![0xfe, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00]
}

pub fn err_packet(code: u16, sqlstate: &str, message: &str) -> Vec<u8> {
    let mut body = vec![0xff];
    body.extend_from_slice(&code.to_le_bytes());
    body.push(b'#');
    body.extend_from_slice(sqlstate.as_bytes());
    body.extend_from_slice(message.as_bytes());
    body
}

/// A one-column, one-row text result set.
pub fn single_value_result_set(column: &str, value: &str) -> Vec<Vec<u8>> {
    let mut column_count = Vec::new();
    lenenc_int(1, &mut column_count);

    let mut def = Vec::new();
    lenenc_bytes(b"def", &mut def);
    lenenc_bytes(b"", &mut def); // schema
    lenenc_bytes(b"", &mut def); // table
    lenenc_bytes(b"", &mut def); // org_table
    lenenc_bytes(column.as_bytes(), &mut def);
    lenenc_bytes(b"", &mut def); // org_name
    lenenc_int(0x0c, &mut def);
    def.extend_from_slice(&33u16.to_le_bytes()); // charset
    def.extend_from_slice(&255u32.to_le_bytes()); // column length
    def.push(0xfd); // MYSQL_TYPE_VAR_STRING
    def.extend_from_slice(&0u16.to_le_bytes()); // flags
    def.push(0); // decimals
    def.extend_from_slice(&[0, 0]); // filler

    let mut row = Vec::new();
    lenenc_bytes(value.as_bytes(), &mut row);

    vec![column_count, def, row, eof_ok_packet()]
}

// ---------------------------------------------------------------------------
// The fake frontend
// ---------------------------------------------------------------------------

/// What one backend connection did, as observed by the fake frontend.
#[derive(Debug, Clone, Default)]
pub struct BackendSession {
    pub username: String,
    /// The client's auth response exactly as it arrived. The whole point: if the
    /// proxy had re-scrambled it, this would not match what the client sent.
    pub auth_response: Vec<u8>,
    pub capabilities: u32,
    pub authenticated: bool,
    pub queries: Vec<String>,
    pub init_dbs: Vec<String>,
    pub commands: Vec<u8>,
    /// Set when this connection reached EOF or errored — that is, when the proxy
    /// closed it.
    pub closed: bool,
}

#[derive(Debug, Default)]
pub struct Recorded {
    pub sessions: Vec<BackendSession>,
}

impl Recorded {
    fn ensure(&mut self, index: usize) -> &mut BackendSession {
        while self.sessions.len() <= index {
            self.sessions.push(BackendSession::default());
        }
        &mut self.sessions[index]
    }
}

/// Configuration for how the fake frontend behaves.
#[derive(Clone)]
pub struct FakeDorisConfig {
    /// Usernames the frontend accepts. Anything else gets an access-denied ERR.
    pub accepted_users: Vec<String>,
    /// What `SELECT CURRENT_USER()` reports, keyed by the username presented.
    /// A missing entry reports `<user>@%`.
    pub current_user_overrides: HashMap<String, String>,
    /// Databases `COM_INIT_DB` accepts. `None` accepts every database.
    pub accepted_databases: Option<Vec<String>>,
    /// The auth plugin the frontend names in its handshake. Anything other than
    /// `mysql_native_password` must be refused by the proxy.
    pub auth_plugin: String,
}

impl Default for FakeDorisConfig {
    fn default() -> Self {
        FakeDorisConfig {
            accepted_users: vec!["analyst".to_string()],
            current_user_overrides: HashMap::new(),
            accepted_databases: None,
            auth_plugin: "mysql_native_password".to_string(),
        }
    }
}

pub struct FakeDoris {
    pub addr: String,
    pub recorded: Arc<Mutex<Recorded>>,
}

impl FakeDoris {
    pub async fn start(config: FakeDorisConfig) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let recorded = Arc::new(Mutex::new(Recorded::default()));
        let recorded_for_task = Arc::clone(&recorded);

        tokio::spawn(async move {
            let mut index = 0usize;
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                let recorded = Arc::clone(&recorded_for_task);
                let config = config.clone();
                let this_index = index;
                index += 1;
                recorded.lock().unwrap().ensure(this_index);
                tokio::spawn(async move {
                    let _ = serve_backend(stream, this_index, config, Arc::clone(&recorded)).await;
                    recorded.lock().unwrap().ensure(this_index).closed = true;
                });
            }
        });

        FakeDoris { addr, recorded }
    }

    pub fn sessions(&self) -> Vec<BackendSession> {
        self.recorded.lock().unwrap().sessions.clone()
    }

    pub fn session_count(&self) -> usize {
        self.recorded.lock().unwrap().sessions.len()
    }

    /// Wait until `predicate` holds, or fail the test. Every cross-task
    /// observation in these tests goes through here rather than a bare sleep.
    pub async fn wait_for(&self, what: &str, predicate: impl Fn(&[BackendSession]) -> bool) {
        for _ in 0..400 {
            if predicate(&self.sessions()) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!(
            "timed out waiting for {what}; sessions = {:#?}",
            self.sessions()
        );
    }
}

fn handshake_packet(salt: &[u8; 20], auth_plugin: &str) -> Vec<u8> {
    let caps = CLIENT_PROTOCOL_41
        | CLIENT_SECURE_CONNECTION
        | CLIENT_PLUGIN_AUTH
        | CLIENT_CONNECT_WITH_DB
        | CLIENT_DEPRECATE_EOF;
    let bits = caps.to_le_bytes();
    let mut body = vec![10];
    body.extend_from_slice(b"8.0.33-Doris-fake\0");
    body.extend_from_slice(&4242u32.to_le_bytes());
    body.extend_from_slice(&salt[..8]);
    body.push(0x00);
    body.extend_from_slice(&bits[..2]);
    body.push(0x21);
    body.extend_from_slice(&[0x00, 0x00]);
    body.extend_from_slice(&bits[2..]);
    body.push(21); // auth-plugin-data length, including the NUL
    body.extend_from_slice(&[0u8; 10]);
    body.extend_from_slice(&salt[8..]);
    body.push(0x00);
    body.extend_from_slice(auth_plugin.as_bytes());
    body.push(0x00);
    body
}

/// Decode a `HandshakeResponse41` payload.
pub fn parse_handshake_response(body: &[u8]) -> (u32, String, Vec<u8>, Option<String>) {
    let mut r = Reader::new(body);
    let capabilities = r.u32_le();
    let _max_packet = r.u32_le();
    let _charset = r.u8();
    r.take(23);
    let username = String::from_utf8_lossy(r.nul_terminated()).into_owned();

    let auth_response = if capabilities & CLIENT_PLUGIN_AUTH_LENENC_CLIENT_DATA != 0 {
        r.lenenc_bytes().to_vec()
    } else if capabilities & CLIENT_SECURE_CONNECTION != 0 {
        let len = r.u8() as usize;
        r.take(len).to_vec()
    } else {
        r.nul_terminated().to_vec()
    };

    let database = if capabilities & CLIENT_CONNECT_WITH_DB != 0 {
        Some(String::from_utf8_lossy(r.nul_terminated()).into_owned())
    } else {
        None
    };

    (capabilities, username, auth_response, database)
}

async fn serve_backend(
    mut stream: TcpStream,
    index: usize,
    config: FakeDorisConfig,
    recorded: Arc<Mutex<Recorded>>,
) -> std::io::Result<()> {
    write_packet(
        &mut stream,
        0,
        &handshake_packet(DORIS_SALT, &config.auth_plugin),
    )
    .await?;

    let (_, response) = read_packet(&mut stream).await?;
    let (capabilities, username, auth_response, _db) = parse_handshake_response(&response);
    {
        let mut guard = recorded.lock().unwrap();
        let session = guard.ensure(index);
        session.username.clone_from(&username);
        session.auth_response.clone_from(&auth_response);
        session.capabilities = capabilities;
    }

    if !config.accepted_users.contains(&username) {
        write_packet(
            &mut stream,
            2,
            &err_packet(
                1045,
                "28000",
                &format!("Access denied for user '{username}'"),
            ),
        )
        .await?;
        return Ok(());
    }
    write_packet(&mut stream, 2, &ok_packet()).await?;
    recorded.lock().unwrap().ensure(index).authenticated = true;

    loop {
        let (_, packet) = match read_packet(&mut stream).await {
            Ok(packet) => packet,
            Err(_) => return Ok(()), // the proxy closed the connection
        };
        let Some(&command) = packet.first() else {
            return Ok(());
        };
        recorded
            .lock()
            .unwrap()
            .ensure(index)
            .commands
            .push(command);

        match command {
            // COM_QUERY
            0x03 => {
                let sql = String::from_utf8_lossy(&packet[1..]).into_owned();
                recorded
                    .lock()
                    .unwrap()
                    .ensure(index)
                    .queries
                    .push(sql.clone());

                if sql.eq_ignore_ascii_case("SELECT CURRENT_USER()") {
                    let reported = config
                        .current_user_overrides
                        .get(&username)
                        .cloned()
                        .unwrap_or_else(|| format!("{username}@%"));
                    for (i, p) in single_value_result_set("CURRENT_USER()", &reported)
                        .into_iter()
                        .enumerate()
                    {
                        write_packet(&mut stream, (i + 1) as u8, &p).await?;
                    }
                } else {
                    for (i, p) in single_value_result_set("echo", &sql)
                        .into_iter()
                        .enumerate()
                    {
                        write_packet(&mut stream, (i + 1) as u8, &p).await?;
                    }
                }
            }
            // COM_INIT_DB
            0x02 => {
                let db = String::from_utf8_lossy(&packet[1..]).into_owned();
                recorded
                    .lock()
                    .unwrap()
                    .ensure(index)
                    .init_dbs
                    .push(db.clone());
                let accepted = config
                    .accepted_databases
                    .as_ref()
                    .is_none_or(|allowed| allowed.contains(&db));
                if accepted {
                    write_packet(&mut stream, 1, &ok_packet()).await?;
                } else {
                    write_packet(
                        &mut stream,
                        1,
                        &err_packet(1049, "42000", &format!("Unknown database '{db}'")),
                    )
                    .await?;
                }
            }
            // COM_QUIT
            0x01 => return Ok(()),
            _ => write_packet(&mut stream, 1, &ok_packet()).await?,
        }
    }
}

// ---------------------------------------------------------------------------
// A raw client that talks to the proxy
// ---------------------------------------------------------------------------

pub struct RawClient {
    stream: TcpStream,
    pub salt: Vec<u8>,
    pub server_capabilities: u32,
    pub server_version: String,
    pub auth_plugin: String,
}

impl RawClient {
    /// Connect and read the proxy's Initial Handshake Packet.
    pub async fn connect(addr: &str) -> std::io::Result<Self> {
        let mut stream = TcpStream::connect(addr).await?;
        let (_, body) = read_packet(&mut stream).await?;
        assert_ne!(
            body.first(),
            Some(&0xff),
            "proxy refused the connection: {}",
            String::from_utf8_lossy(&body)
        );

        let mut r = Reader::new(&body);
        assert_eq!(r.u8(), 10, "expected HandshakeV10");
        let server_version = String::from_utf8_lossy(r.nul_terminated()).into_owned();
        let _connection_id = r.u32_le();
        let mut salt = r.take(8).to_vec();
        r.u8(); // filler
        let caps_low = r.u16_le();
        r.u8(); // charset
        r.u16_le(); // status
        let caps_high = r.u16_le();
        let _auth_plugin_data_len = r.u8();
        r.take(10);
        let part_2 = r.take(12).to_vec();
        r.u8(); // NUL terminating part 2
        let auth_plugin = String::from_utf8_lossy(r.nul_terminated()).into_owned();
        salt.extend_from_slice(&part_2);

        Ok(RawClient {
            stream,
            salt,
            server_capabilities: u32::from(caps_low) | (u32::from(caps_high) << 16),
            server_version,
            auth_plugin,
        })
    }

    /// Read whatever the proxy sends without completing a handshake first.
    pub async fn connect_expecting_refusal(addr: &str) -> std::io::Result<Vec<u8>> {
        let mut stream = TcpStream::connect(addr).await?;
        let (_, body) = read_packet(&mut stream).await?;
        Ok(body)
    }

    /// Send a `HandshakeResponse41` carrying `auth_response` verbatim.
    ///
    /// No hashing: the bytes are chosen by the test so the fake frontend can
    /// assert they arrived unchanged.
    pub async fn send_handshake_response(
        &mut self,
        username: &str,
        auth_response: &[u8],
        database: Option<&str>,
    ) -> std::io::Result<()> {
        let mut caps = CLIENT_PROTOCOL_41 | CLIENT_SECURE_CONNECTION | CLIENT_PLUGIN_AUTH;
        if database.is_some() {
            caps |= CLIENT_CONNECT_WITH_DB;
        }
        let mut body = Vec::new();
        body.extend_from_slice(&caps.to_le_bytes());
        body.extend_from_slice(&0x00ff_ffffu32.to_le_bytes());
        body.push(0x21);
        body.extend_from_slice(&[0u8; 23]);
        body.extend_from_slice(username.as_bytes());
        body.push(0x00);
        body.push(auth_response.len() as u8);
        body.extend_from_slice(auth_response);
        if let Some(db) = database {
            body.extend_from_slice(db.as_bytes());
            body.push(0x00);
        }
        body.extend_from_slice(b"mysql_native_password\0");
        write_packet(&mut self.stream, 1, &body).await
    }

    pub async fn read_packet(&mut self) -> std::io::Result<(u8, Vec<u8>)> {
        read_packet(&mut self.stream).await
    }

    pub async fn send_command(&mut self, command: u8, payload: &[u8]) -> std::io::Result<()> {
        let mut body = vec![command];
        body.extend_from_slice(payload);
        write_packet(&mut self.stream, 0, &body).await
    }

    /// Send a statement and consume its **entire** response, returning the first
    /// packet.
    ///
    /// Draining matters: a result set is several packets, and leaving the tail
    /// unread would make the next command read a stale one — which looks like a
    /// proxy bug and is not.
    ///
    /// `RawClient` negotiates without `CLIENT_DEPRECATE_EOF`, so the framing is
    /// the classical one: column count, column definitions, EOF, rows, EOF.
    pub async fn query(&mut self, sql: &str) -> std::io::Result<(u8, Vec<u8>)> {
        self.send_command(0x03, sql.as_bytes()).await?;
        let (seq, first) = self.read_packet().await?;
        match first.first() {
            Some(0x00) | Some(0xff) | None => return Ok((seq, first)),
            _ => {}
        }

        let columns = Reader::new(&first).lenenc_int() as usize;
        for _ in 0..columns {
            self.read_packet().await?;
        }
        // EOF after the column definitions, then rows until the second EOF.
        for _ in 0..2 {
            loop {
                let (_, packet) = self.read_packet().await?;
                let terminator = matches!(packet.first(), Some(0xfe)) && packet.len() < 9;
                if terminator || matches!(packet.first(), Some(0xff)) {
                    break;
                }
            }
        }
        Ok((seq, first))
    }

    /// Complete a handshake and assert it succeeded.
    pub async fn authenticate(
        addr: &str,
        username: &str,
        auth_response: &[u8],
    ) -> std::io::Result<Self> {
        let mut client = RawClient::connect(addr).await?;
        client
            .send_handshake_response(username, auth_response, None)
            .await?;
        let (_, reply) = client.read_packet().await?;
        assert_eq!(
            reply.first(),
            Some(&0x00),
            "expected the proxy to accept the session, got {}",
            String::from_utf8_lossy(&reply)
        );
        Ok(client)
    }
}

/// The SQLSTATE carried by an ERR packet.
pub fn err_sqlstate(body: &[u8]) -> String {
    assert_eq!(body.first(), Some(&0xff), "not an ERR packet: {body:?}");
    assert_eq!(body[3], b'#', "ERR packet has no SQL state marker");
    String::from_utf8_lossy(&body[4..9]).into_owned()
}

/// The human-readable part of an ERR packet.
pub fn err_message(body: &[u8]) -> String {
    assert_eq!(body.first(), Some(&0xff), "not an ERR packet: {body:?}");
    String::from_utf8_lossy(&body[9..]).into_owned()
}
