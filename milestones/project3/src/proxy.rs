//! Connection handling: accepting clients, pairing each with a backend
//! connection, and driving the command loop.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;

use crate::config::ListenerConfig;
use crate::logging::record::{outcome as outcome_str, CommandRecord};
use crate::logging::writer::LogHandle;
use crate::pipeline::{FilterOutcome, Pipeline, StageContext};
use crate::protocol::command::{Command, COM_QUIT};
use crate::protocol::connection_phase::{self, err_payload, HandshakeError, Session};
use crate::protocol::framing::{PacketReader, PacketWriter};
use crate::protocol::response::{self, OutcomeKind, ResponseError};
use crate::timestamp::Timestamp;

/// Server-side error codes the proxy raises on its own behalf.
const ER_UNKNOWN_ERROR: u16 = 1105;
const ER_NOT_SUPPORTED_YET: u16 = 1235;

static NEXT_CONNECTION_ID: AtomicU64 = AtomicU64::new(1);

/// Everything a connection task needs.
#[derive(Debug)]
pub struct ListenerContext {
    pub config: ListenerConfig,
    pub log: LogHandle,
    pub pipeline: Arc<Pipeline>,
}

/// Accepts connections until shutdown is signalled.
pub async fn serve(
    listener: TcpListener,
    ctx: Arc<ListenerContext>,
    mut shutdown: watch::Receiver<bool>,
) {
    loop {
        let accepted = tokio::select! {
            biased;
            _ = shutdown.changed() => break,
            r = listener.accept() => r,
        };

        match accepted {
            Ok((stream, peer)) => {
                let ctx = Arc::clone(&ctx);
                let shutdown = shutdown.clone();
                tokio::spawn(async move {
                    let id = NEXT_CONNECTION_ID.fetch_add(1, Ordering::Relaxed);
                    if let Err(e) = handle_connection(stream, peer, ctx, shutdown, id).await {
                        // Connection-level failures are ordinary; the records
                        // already emitted describe what the client saw.
                        eprintln!("connection {id} ended: {e}");
                    }
                });
            }
            Err(e) => {
                eprintln!("accept failed: {e}");
                // Back off briefly so a persistent accept error does not spin.
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        }
    }
}

/// Reasons a connection ended, for the operator-facing message only.
#[derive(Debug)]
pub enum ConnectionError {
    Io(std::io::Error),
    Handshake(HandshakeError),
    Response(ResponseError),
}

impl std::fmt::Display for ConnectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConnectionError::Io(e) => write!(f, "{e}"),
            ConnectionError::Handshake(e) => write!(f, "{e}"),
            ConnectionError::Response(e) => write!(f, "{e}"),
        }
    }
}

impl From<std::io::Error> for ConnectionError {
    fn from(e: std::io::Error) -> Self {
        ConnectionError::Io(e)
    }
}

impl From<HandshakeError> for ConnectionError {
    fn from(e: HandshakeError) -> Self {
        ConnectionError::Handshake(e)
    }
}

impl From<ResponseError> for ConnectionError {
    fn from(e: ResponseError) -> Self {
        ConnectionError::Response(e)
    }
}

async fn handle_connection(
    client: TcpStream,
    peer: SocketAddr,
    ctx: Arc<ListenerContext>,
    mut shutdown: watch::Receiver<bool>,
    connection_id: u64,
) -> Result<(), ConnectionError> {
    let _ = client.set_nodelay(true);

    let (client_read, client_write) = client.into_split();
    let mut client_rx = PacketReader::new(client_read);
    let mut client_tx = PacketWriter::new(client_write);

    // A backend that cannot be reached is reported to the client the way a
    // server would report a refusal: an ERR packet as the first thing it sees.
    let backend = match TcpStream::connect(&ctx.config.backend).await {
        Ok(s) => s,
        Err(e) => {
            let msg = format!("proxy cannot reach backend {}: {e}", ctx.config.backend);
            let _ = client_tx
                .write_packet(0, &err_payload(ER_UNKNOWN_ERROR, "HY000", &msg))
                .await;
            return Err(ConnectionError::Io(e));
        }
    };
    let _ = backend.set_nodelay(true);

    let (backend_read, backend_write) = backend.into_split();
    let mut backend_rx = PacketReader::new(backend_read);
    let mut backend_tx = PacketWriter::new(backend_write);

    let session = match connection_phase::run(
        &mut client_rx,
        &mut client_tx,
        &mut backend_rx,
        &mut backend_tx,
    )
    .await
    {
        Ok(s) => s,
        Err(e) => {
            // A client that asserted a masked capability gets a reason; the
            // other cases have already been relayed from the backend.
            if let HandshakeError::ForbiddenCapability(names) = &e {
                let msg = format!(
                    "proxy does not support {}; connect without it",
                    names.join(", ")
                );
                let _ = client_tx
                    .write_packet(2, &err_payload(ER_NOT_SUPPORTED_YET, "0A000", &msg))
                    .await;
            }
            return Err(e.into());
        }
    };

    let conn = ConnectionInfo {
        listener: ctx.config.name.clone(),
        connection_id,
        client_addr: peer.to_string(),
        session,
    };

    command_loop(
        &mut client_rx,
        &mut client_tx,
        &mut backend_rx,
        &mut backend_tx,
        &ctx,
        &conn,
        &mut shutdown,
    )
    .await
}

/// Connection-scoped fields that appear on every record.
struct ConnectionInfo {
    listener: String,
    connection_id: u64,
    client_addr: String,
    session: Session,
}

/// Fields of a record that are known before the response is read.
struct PendingRecord {
    ts: Timestamp,
    started: Instant,
    command: &'static str,
    statement: Option<String>,
    ctx: StageContext,
}

impl PendingRecord {
    fn finish(self, conn: &ConnectionInfo, outcome: String) -> CommandRecord {
        // A rewrite and a skip are mutually exclusive, and "no rule applied"
        // leaves all four fields absent so records for unfiltered traffic keep
        // the shape they had before row filtering existed.
        let (rewritten, forwarded_statement, filter_table, filter_skipped) = match self.ctx.filter
        {
            FilterOutcome::NotApplicable => (false, None, None, None),
            FilterOutcome::Rewritten { table, forwarded } => {
                (true, Some(forwarded), Some(table), None)
            }
            FilterOutcome::Skipped(reason) => (false, None, None, Some(reason.as_str())),
        };

        CommandRecord {
            ts: self.ts.to_rfc3339(),
            ts_unix_ms: self.ts.unix_ms,
            listener: conn.listener.clone(),
            connection_id: conn.connection_id,
            backend_connection_id: conn.session.backend_connection_id,
            client_addr: conn.client_addr.clone(),
            username: conn.session.username.clone(),
            database: conn.session.database.clone(),
            command: self.command,
            statement: self.statement,
            digest: self.ctx.digest.as_ref().map(|d| d.text.clone()),
            digest_hash: self.ctx.digest.as_ref().map(|d| d.hash_hex()),
            digest_unavailable: self.ctx.digest_unavailable,
            duration_us: self.started.elapsed().as_micros() as u64,
            outcome,
            affected_rows: None,
            returned_rows: None,
            result_sets: None,
            error_code: None,
            sql_state: None,
            error_message: None,
            rewritten,
            forwarded_statement,
            filter_table,
            filter_skipped,
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn command_loop<CR, CW, BR, BW>(
    client_rx: &mut PacketReader<CR>,
    client_tx: &mut PacketWriter<CW>,
    backend_rx: &mut PacketReader<BR>,
    backend_tx: &mut PacketWriter<BW>,
    ctx: &ListenerContext,
    conn: &ConnectionInfo,
    shutdown: &mut watch::Receiver<bool>,
) -> Result<(), ConnectionError>
where
    CR: tokio::io::AsyncRead + Unpin,
    CW: tokio::io::AsyncWrite + Unpin,
    BR: tokio::io::AsyncRead + Unpin,
    BW: tokio::io::AsyncWrite + Unpin,
{
    loop {
        // While idle, watch the backend too: it may time the connection out,
        // and the client must not be left believing the session is alive.
        // Both futures are cancel-safe, so losing the race costs nothing.
        let next = tokio::select! {
            biased;
            _ = shutdown.changed() => return Ok(()),
            backend = backend_rx.wait_for_activity() => {
                return match backend {
                    Ok(()) => Err(ConnectionError::Response(ResponseError::Protocol(
                        "backend sent data while no command was in flight".into(),
                    ))),
                    Err(e) => Err(ConnectionError::Io(e)),
                };
            }
            client = client_rx.next_message() => client?,
        };

        let Some(message) = next else {
            // Client hung up without COM_QUIT.
            return Ok(());
        };

        let command = Command::new(message.payload, message.first_seq, message.packet_count);
        let started = Instant::now();
        let ts = Timestamp::now();
        let statement = command
            .statement()
            .map(|s| String::from_utf8_lossy(s).into_owned());

        // The backend answers a command starting at the sequence id after its
        // last packet; the proxy uses the same numbering when replying itself.
        let reply_seq = command
            .first_seq
            .wrapping_add(command.packet_count as u8);

        if command.is_refused() {
            let msg = format!(
                "{} is not supported through this proxy",
                command.name()
            );
            client_tx
                .write_packet(reply_seq, &err_payload(ER_NOT_SUPPORTED_YET, "0A000", &msg))
                .await?;

            let pending = PendingRecord {
                ts,
                started,
                command: command.name(),
                statement,
                ctx: StageContext::default(),
            };
            let mut record = pending.finish(conn, outcome_str::REFUSED.to_string());
            record.error_code = Some(ER_NOT_SUPPORTED_YET);
            record.sql_state = Some("0A000".into());
            record.error_message = Some(msg);
            ctx.log.emit(record);
            continue;
        }

        let mut stage_ctx = StageContext::default();
        let payload = ctx.pipeline.run(&command, &mut stage_ctx);

        let pending = PendingRecord {
            ts,
            started,
            command: command.name(),
            statement,
            ctx: stage_ctx,
        };

        if let Err(e) = backend_tx.write_message(command.first_seq, &payload).await {
            ctx.log
                .emit(pending.finish(conn, outcome_str::TERMINATED.to_string()));
            return Err(ConnectionError::Io(e));
        }

        let shape = command.response_shape();
        let result = response::relay(backend_rx, client_tx, &conn.session, shape).await;

        match result {
            Ok(outcome) => {
                let kind = outcome.kind.unwrap_or(OutcomeKind::NoResponse);
                let mut record = pending.finish(conn, kind.as_str().to_string());
                record.affected_rows = outcome.affected_rows;
                record.returned_rows = outcome.returned_rows;
                record.result_sets = (outcome.result_set_count > 0).then_some(outcome.result_set_count);
                record.error_code = outcome.error_code;
                record.sql_state = outcome.sql_state;
                record.error_message = outcome.error_message;
                ctx.log.emit(record);

                if command.code == COM_QUIT {
                    return Ok(());
                }
            }
            Err(e) => {
                ctx.log
                    .emit(pending.finish(conn, outcome_str::TERMINATED.to_string()));
                return Err(e.into());
            }
        }
    }
}
