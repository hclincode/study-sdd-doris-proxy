//! Test scaffolding: a scriptable MySQL backend and a minimal client.
//!
//! These speak just enough of the protocol to drive the proxy end to end
//! without a database, which keeps the behavioural tests deterministic and
//! runnable anywhere. Verification against real MySQL and real drivers is a
//! separate, Docker-backed exercise.

#![allow(dead_code)]

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;

use mysql_proxy::config::ListenerConfig;
use mysql_proxy::logging::writer;
use mysql_proxy::pipeline::{ObserveStage, Pipeline};
use mysql_proxy::row_filter::{RowFilterStage, RuleSet};
use mysql_proxy::protocol::capabilities::*;
use mysql_proxy::proxy::{self, ListenerContext};

// ---------------------------------------------------------------- wire helpers

pub fn packet(seq: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + payload.len());
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes()[..3]);
    out.push(seq);
    out.extend_from_slice(payload);
    out
}

pub async fn read_packet(stream: &mut TcpStream) -> std::io::Result<(u8, Vec<u8>)> {
    let mut header = [0u8; 4];
    stream.read_exact(&mut header).await?;
    let len = u32::from_le_bytes([header[0], header[1], header[2], 0]) as usize;
    let mut payload = vec![0u8; len];
    stream.read_exact(&mut payload).await?;
    Ok((header[3], payload))
}

pub fn ok_packet() -> Vec<u8> {
    vec![0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00]
}

pub fn err_packet(code: u16, state: &str, msg: &str) -> Vec<u8> {
    let mut p = vec![0xFF];
    p.extend_from_slice(&code.to_le_bytes());
    p.push(b'#');
    p.extend_from_slice(state.as_bytes());
    p.extend_from_slice(msg.as_bytes());
    p
}

fn column_def(name: &str) -> Vec<u8> {
    let mut p = Vec::new();
    for part in ["def", "", "", "", name, name] {
        p.push(part.len() as u8);
        p.extend_from_slice(part.as_bytes());
    }
    p.push(0x0C);
    p.extend_from_slice(&33u16.to_le_bytes());
    p.extend_from_slice(&64u32.to_le_bytes());
    p.push(0xFD);
    p.extend_from_slice(&0u16.to_le_bytes());
    p.push(0);
    p.extend_from_slice(&0u16.to_le_bytes());
    p
}

fn text_row(value: &str) -> Vec<u8> {
    let mut p = vec![value.len() as u8];
    p.extend_from_slice(value.as_bytes());
    p
}

fn eof_packet(status: u16) -> Vec<u8> {
    let mut p = vec![0xFE];
    p.extend_from_slice(&0u16.to_le_bytes());
    p.extend_from_slice(&status.to_le_bytes());
    p
}

// ------------------------------------------------------------- mock backend

/// Capabilities the mock advertises. Deliberately includes the bits the proxy
/// is required to mask so tests can observe the masking.
pub const MOCK_SERVER_CAPS: u32 = CLIENT_PROTOCOL_41
    | CLIENT_SECURE_CONNECTION
    | CLIENT_PLUGIN_AUTH
    | CLIENT_CONNECT_WITH_DB
    | CLIENT_TRANSACTIONS
    | CLIENT_MULTI_RESULTS
    | CLIENT_SSL
    | CLIENT_COMPRESS
    | CLIENT_LOCAL_FILES
    | CLIENT_QUERY_ATTRIBUTES;

pub struct MockBackend {
    pub addr: SocketAddr,
    pub connections: Arc<AtomicU64>,
    /// Statement text as it actually arrived, so tests can assert what the
    /// proxy forwarded rather than only what the client sent.
    pub statements: Arc<std::sync::Mutex<Vec<String>>>,
}

impl MockBackend {
    pub fn seen(&self) -> Vec<String> {
        self.statements.lock().unwrap().clone()
    }

    pub fn last_statement(&self) -> String {
        self.seen().last().cloned().unwrap_or_default()
    }
}

impl MockBackend {
    /// Starts a backend that answers queries according to their text:
    ///
    /// * `rows:N` returns a one-column result set of N rows
    /// * `fail` returns an ERR packet
    /// * anything else returns OK
    pub async fn start() -> MockBackend {
        Self::start_with(MOCK_SERVER_CAPS, 42).await
    }

    pub async fn start_with(caps: u32, connection_id: u32) -> MockBackend {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let connections = Arc::new(AtomicU64::new(0));
        let counter = Arc::clone(&connections);
        let statements = Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = Arc::clone(&statements);

        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                counter.fetch_add(1, Ordering::Relaxed);
                tokio::spawn(serve_backend(
                    stream,
                    caps,
                    connection_id,
                    Arc::clone(&seen),
                ));
            }
        });

        MockBackend {
            addr,
            connections,
            statements,
        }
    }
}

async fn serve_backend(
    mut stream: TcpStream,
    caps: u32,
    connection_id: u32,
    statements: Arc<std::sync::Mutex<Vec<String>>>,
) {
    // Initial handshake.
    let mut p = vec![10];
    p.extend_from_slice(b"8.0.36-mock\0");
    p.extend_from_slice(&connection_id.to_le_bytes());
    p.extend_from_slice(b"12345678");
    p.push(0);
    p.extend_from_slice(&(caps as u16).to_le_bytes());
    p.push(0x21);
    p.extend_from_slice(&2u16.to_le_bytes());
    p.extend_from_slice(&((caps >> 16) as u16).to_le_bytes());
    p.push(21);
    p.extend_from_slice(&[0u8; 10]);
    p.extend_from_slice(b"abcdefghijkl\0");
    p.extend_from_slice(b"mysql_native_password\0");
    if stream.write_all(&packet(0, &p)).await.is_err() {
        return;
    }

    // Client handshake response, then OK.
    if read_packet(&mut stream).await.is_err() {
        return;
    }
    if stream.write_all(&packet(2, &ok_packet())).await.is_err() {
        return;
    }

    // Command loop.
    loop {
        let Ok((seq, payload)) = read_packet(&mut stream).await else {
            return;
        };
        let reply_seq = seq.wrapping_add(1);
        let code = payload.first().copied().unwrap_or(0);

        if matches!(code, 0x03 | 0x16) {
            statements
                .lock()
                .unwrap()
                .push(String::from_utf8_lossy(&payload[1..]).to_string());
        }

        let response: Vec<Vec<u8>> = match code {
            0x01 => return, // COM_QUIT
            0x0E => vec![ok_packet()],
            // COM_STMT_PREPARE: a header claiming no parameters and no columns.
            0x16 => {
                let mut header = vec![0x00];
                header.extend_from_slice(&1u32.to_le_bytes());
                header.extend_from_slice(&0u16.to_le_bytes());
                header.extend_from_slice(&0u16.to_le_bytes());
                header.push(0x00);
                header.extend_from_slice(&0u16.to_le_bytes());
                vec![header]
            }
            0x03 => {
                let sql = String::from_utf8_lossy(&payload[1..]).to_string();
                if sql.trim() == "fail" {
                    vec![err_packet(1146, "42S02", "table does not exist")]
                } else if let Some(n) = sql.trim().strip_prefix("rows:") {
                    let n: usize = n.trim().parse().unwrap_or(0);
                    let mut out = vec![vec![0x01], column_def("v"), eof_packet(0)];
                    for i in 0..n {
                        out.push(text_row(&i.to_string()));
                    }
                    out.push(eof_packet(0));
                    out
                } else {
                    vec![ok_packet()]
                }
            }
            _ => vec![ok_packet()],
        };

        for (i, payload) in response.iter().enumerate() {
            if stream
                .write_all(&packet(reply_seq.wrapping_add(i as u8), payload))
                .await
                .is_err()
            {
                return;
            }
        }
    }
}

// ------------------------------------------------------------------- client

/// A client that speaks the minimum needed to reach the command phase.
#[derive(Debug)]
pub struct TestClient {
    pub stream: TcpStream,
    pub server_capabilities: u32,
}

impl TestClient {
    pub async fn connect(addr: SocketAddr) -> std::io::Result<TestClient> {
        Self::connect_with(addr, 0).await
    }

    /// `extra_caps` lets a test assert a capability the proxy masked off.
    pub async fn connect_with(
        addr: SocketAddr,
        extra_caps: u32,
    ) -> std::io::Result<TestClient> {
        let mut stream = TcpStream::connect(addr).await?;
        let (_, greeting) = read_packet(&mut stream).await?;

        // A server that refuses the connection outright answers with ERR in
        // place of a handshake, and so does the proxy when it cannot reach the
        // backend.
        if greeting.first() == Some(&0xFF) {
            return Err(std::io::Error::other(format!(
                "connection refused: {}",
                String::from_utf8_lossy(&greeting[9.min(greeting.len())..])
            )));
        }

        // Capability flags sit after the version string, connection id, salt
        // and filler; the upper half follows charset and status.
        let version_end = greeting[1..].iter().position(|&b| b == 0).unwrap() + 1;
        let lower_at = version_end + 1 + 4 + 8 + 1;
        let lower = u16::from_le_bytes([greeting[lower_at], greeting[lower_at + 1]]) as u32;
        let upper_at = lower_at + 2 + 1 + 2;
        let upper = u16::from_le_bytes([greeting[upper_at], greeting[upper_at + 1]]) as u32;
        let server_capabilities = lower | (upper << 16);

        let caps = (CLIENT_PROTOCOL_41
            | CLIENT_SECURE_CONNECTION
            | CLIENT_PLUGIN_AUTH
            | CLIENT_TRANSACTIONS)
            | extra_caps;

        let mut p = Vec::new();
        p.extend_from_slice(&caps.to_le_bytes());
        p.extend_from_slice(&(16u32 * 1024 * 1024).to_le_bytes());
        p.push(0x21);
        p.extend_from_slice(&[0u8; 23]);
        p.extend_from_slice(b"app\0");
        p.push(0);
        p.extend_from_slice(b"mysql_native_password\0");
        stream.write_all(&packet(1, &p)).await?;

        let (_, reply) = read_packet(&mut stream).await?;
        if reply.first() == Some(&0xFF) {
            return Err(std::io::Error::other(format!(
                "auth failed: {}",
                String::from_utf8_lossy(&reply[9..])
            )));
        }

        Ok(TestClient {
            stream,
            server_capabilities,
        })
    }

    pub async fn query(&mut self, sql: &str) -> std::io::Result<Vec<(u8, Vec<u8>)>> {
        let mut payload = vec![0x03];
        payload.extend_from_slice(sql.as_bytes());
        self.stream.write_all(&packet(0, &payload)).await?;
        self.read_response().await
    }

    pub async fn command(&mut self, code: u8, rest: &[u8]) -> std::io::Result<Vec<(u8, Vec<u8>)>> {
        let mut payload = vec![code];
        payload.extend_from_slice(rest);
        self.stream.write_all(&packet(0, &payload)).await?;
        self.read_response().await
    }

    /// Reads packets until the response is complete, using the same rules the
    /// proxy does.
    async fn read_response(&mut self) -> std::io::Result<Vec<(u8, Vec<u8>)>> {
        let mut out = Vec::new();
        let first = read_packet(&mut self.stream).await?;
        let kind = first.1.first().copied().unwrap_or(0);
        out.push(first);

        if matches!(kind, 0x00 | 0xFF) {
            return Ok(out);
        }

        // Column definitions, EOF, rows, EOF.
        let columns = kind as usize;
        for _ in 0..columns {
            out.push(read_packet(&mut self.stream).await?);
        }
        out.push(read_packet(&mut self.stream).await?);
        loop {
            let p = read_packet(&mut self.stream).await?;
            let done = p.1.first() == Some(&0xFE) && p.1.len() < 9;
            out.push(p);
            if done {
                break;
            }
        }
        Ok(out)
    }
}

// -------------------------------------------------------------- proxy harness

pub struct RunningProxy {
    pub addr: SocketAddr,
    pub log_path: PathBuf,
    shutdown: watch::Sender<bool>,
}

impl RunningProxy {
    pub async fn start(backend: SocketAddr, capacity: usize) -> RunningProxy {
        Self::start_with_filters(backend, capacity, &[]).await
    }

    /// Starts a proxy whose listener carries the given row-filter rules.
    pub async fn start_with_filters(
        backend: SocketAddr,
        capacity: usize,
        filters: &[(&str, &str)],
    ) -> RunningProxy {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        // A process-wide counter, not a timestamp: the clock is not fine
        // grained enough to keep parallel tests apart, and two proxies sharing
        // a log path read each other's records and delete each other's files.
        static NEXT_LOG: AtomicU64 = AtomicU64::new(0);
        let mut log_path = std::env::temp_dir();
        log_path.push(format!(
            "mysql-proxy-it-{}-{}.jsonl",
            std::process::id(),
            NEXT_LOG.fetch_add(1, Ordering::Relaxed)
        ));

        let row_filters: std::collections::HashMap<String, String> = filters
            .iter()
            .map(|(t, p)| (t.to_string(), p.to_string()))
            .collect();
        let config = ListenerConfig {
            name: "test".into(),
            bind: addr.to_string(),
            backend: backend.to_string(),
            log_file: log_path.clone(),
            log_channel_capacity: capacity,
            row_filters: row_filters.clone(),
        };

        let reopen = Arc::new(tokio::sync::Notify::new());
        let (log, _writer_task) = writer::spawn(
            config.name.clone(),
            config.log_file.clone(),
            config.log_channel_capacity,
            reopen,
        )
        .await
        .unwrap();

        let rules = RuleSet::compile(&row_filters).expect("filters should compile");
        let pipeline = if rules.is_empty() {
            Pipeline::observe_only()
        } else {
            Pipeline::new(vec![
                Box::new(ObserveStage),
                Box::new(RowFilterStage::new(rules)),
            ])
        };

        let (shutdown, shutdown_rx) = watch::channel(false);
        let ctx = Arc::new(ListenerContext {
            config,
            log,
            pipeline: Arc::new(pipeline),
        });
        tokio::spawn(proxy::serve(listener, ctx, shutdown_rx));

        RunningProxy {
            addr,
            log_path,
            shutdown,
        }
    }

    /// Waits until the log file contains at least `n` records, then returns them.
    ///
    /// Records are written asynchronously after the response reaches the
    /// client, so a test that has just seen its reply may still be ahead of the
    /// writer. The window is generous because the whole suite runs in parallel
    /// and a loaded machine can delay the writer well past the common case.
    pub async fn records(&self, n: usize) -> Vec<serde_json::Value> {
        for _ in 0..250 {
            let found = self.read_records();
            if found.len() >= n {
                return found;
            }
            tokio::time::sleep(std::time::Duration::from_millis(40)).await;
        }
        self.read_records()
    }

    pub fn read_records(&self) -> Vec<serde_json::Value> {
        let Ok(text) = std::fs::read_to_string(&self.log_path) else {
            return Vec::new();
        };
        text.lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).expect("every line must be valid JSON"))
            .collect()
    }
}

impl Drop for RunningProxy {
    fn drop(&mut self) {
        let _ = self.shutdown.send(true);
        let _ = std::fs::remove_file(&self.log_path);
    }
}
