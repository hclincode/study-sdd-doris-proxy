//! The response state machine.
//!
//! Responses are forwarded packet by packet as they arrive — a result set is
//! never accumulated in memory. The machine tracks only where it is in the
//! response, plus counters, so per-row work is a header check and a write.
//! Column values are never decoded.

use std::io;

use bytes::Bytes;
use tokio::io::{AsyncRead, AsyncWrite};

use super::capabilities::{CLIENT_PROTOCOL_41, CLIENT_TRANSACTIONS, SERVER_MORE_RESULTS_EXISTS};
use super::command::ResponseShape;
use super::connection_phase::Session;
use super::framing::{Cursor, PacketReader, PacketWriter, MAX_PAYLOAD};

/// How a command ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutcomeKind {
    /// The backend sends no response for this command.
    NoResponse,
    Ok,
    Error,
    ResultSet,
    Statistics,
    Prepared,
}

impl OutcomeKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            OutcomeKind::NoResponse => "no_response",
            OutcomeKind::Ok => "ok",
            OutcomeKind::Error => "error",
            OutcomeKind::ResultSet => "result_set",
            OutcomeKind::Statistics => "statistics",
            OutcomeKind::Prepared => "prepared",
        }
    }
}

/// What the proxy observed about a completed response.
#[derive(Debug, Clone, Default)]
pub struct Outcome {
    pub kind: Option<OutcomeKind>,
    pub affected_rows: Option<u64>,
    pub returned_rows: Option<u64>,
    pub result_set_count: usize,
    pub error_code: Option<u16>,
    pub sql_state: Option<String>,
    pub error_message: Option<String>,
}

#[derive(Debug)]
pub enum ResponseError {
    Io(io::Error),
    Protocol(String),
    /// The backend asked the client to send a local file. `CLIENT_LOCAL_FILES`
    /// was masked off, so this should be unreachable; if it happens the
    /// connection is torn down rather than relayed.
    LocalInfileRequested,
}

impl From<io::Error> for ResponseError {
    fn from(e: io::Error) -> Self {
        ResponseError::Io(e)
    }
}

impl std::fmt::Display for ResponseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResponseError::Io(e) => write!(f, "io error reading response: {e}"),
            ResponseError::Protocol(m) => write!(f, "malformed response: {m}"),
            ResponseError::LocalInfileRequested => {
                write!(f, "backend requested a local file despite the capability being masked")
            }
        }
    }
}

/// Forwards packets from backend to client while tracking message boundaries.
struct Forwarder<'a, BR, CW> {
    rx: &'a mut PacketReader<BR>,
    tx: &'a mut PacketWriter<CW>,
}

impl<BR, CW> Forwarder<'_, BR, CW>
where
    BR: AsyncRead + Unpin,
    CW: AsyncWrite + Unpin,
{
    /// Forwards one complete message and returns its first packet's payload.
    ///
    /// Continuation packets are forwarded but not returned: classification only
    /// ever needs the head, and a split message must count as one row, not
    /// several.
    async fn next_message_head(&mut self) -> Result<Bytes, ResponseError> {
        let first = self
            .rx
            .next_packet()
            .await?
            .ok_or_else(|| ResponseError::Protocol("backend closed mid-response".into()))?;
        self.tx.write_packet(first.seq, &first.payload).await?;

        let mut continues = first.payload.len() == MAX_PAYLOAD;
        while continues {
            let next = self
                .rx
                .next_packet()
                .await?
                .ok_or_else(|| ResponseError::Protocol("backend closed mid-message".into()))?;
            self.tx.write_packet(next.seq, &next.payload).await?;
            continues = next.payload.len() == MAX_PAYLOAD;
        }

        Ok(first.payload)
    }
}

/// True when a packet in the row phase terminates the result set.
///
/// A genuine text row could only begin with 0xFE if its first column value were
/// at least 2^24 bytes, which forces the packet to be split and therefore to be
/// exactly full. Checking the length against a full packet distinguishes the
/// two, and works whether or not `CLIENT_DEPRECATE_EOF` is in force — the OK
/// packet that replaces EOF also carries the 0xFE header.
fn is_result_terminator(payload: &[u8]) -> bool {
    payload.first() == Some(&0xFE) && payload.len() < MAX_PAYLOAD
}

fn parse_err(payload: &[u8], session: &Session, out: &mut Outcome) {
    let mut c = Cursor::new(&payload[1..]);
    out.error_code = c.u16_le();
    if session.has(CLIENT_PROTOCOL_41) && c.rest().first() == Some(&b'#') {
        c.u8();
        out.sql_state = c
            .bytes(5)
            .map(|b| String::from_utf8_lossy(b).into_owned());
    }
    out.error_message = Some(String::from_utf8_lossy(c.rest()).into_owned());
}

/// Reads an OK packet's affected-row count and status flags. `body` starts
/// after the header byte, which is 0x00 for a real OK and 0xFE when an OK
/// stands in for EOF.
fn parse_ok_body(body: &[u8], session: &Session) -> (Option<u64>, u16) {
    let mut c = Cursor::new(body);
    let affected = c.lenenc_int().flatten();
    let _last_insert_id = c.lenenc_int();
    let status = if session.has(CLIENT_PROTOCOL_41) || session.has(CLIENT_TRANSACTIONS) {
        c.u16_le().unwrap_or(0)
    } else {
        0
    };
    (affected, status)
}

/// Status flags carried by whatever packet terminated a result set.
fn terminator_status(payload: &[u8], session: &Session) -> u16 {
    if session.deprecate_eof() {
        parse_ok_body(&payload[1..], session).1
    } else {
        // EOF packet: warning count then status flags.
        let mut c = Cursor::new(&payload[1..]);
        c.u16_le();
        c.u16_le().unwrap_or(0)
    }
}

fn more_results(status: u16) -> bool {
    status & SERVER_MORE_RESULTS_EXISTS != 0
}

/// Reads the column definitions of one result set, plus the EOF that follows
/// them when `CLIENT_DEPRECATE_EOF` is not in force.
async fn skip_column_definitions<BR, CW>(
    fwd: &mut Forwarder<'_, BR, CW>,
    count: u64,
    session: &Session,
) -> Result<(), ResponseError>
where
    BR: AsyncRead + Unpin,
    CW: AsyncWrite + Unpin,
{
    for _ in 0..count {
        fwd.next_message_head().await?;
    }
    if count > 0 && !session.deprecate_eof() {
        fwd.next_message_head().await?;
    }
    Ok(())
}

/// Relays a complete response and reports what it contained.
pub async fn relay<BR, CW>(
    backend_rx: &mut PacketReader<BR>,
    client_tx: &mut PacketWriter<CW>,
    session: &Session,
    shape: ResponseShape,
) -> Result<Outcome, ResponseError>
where
    BR: AsyncRead + Unpin,
    CW: AsyncWrite + Unpin,
{
    let mut out = Outcome::default();
    if shape == ResponseShape::None {
        out.kind = Some(OutcomeKind::NoResponse);
        return Ok(out);
    }

    let mut fwd = Forwarder {
        rx: backend_rx,
        tx: client_tx,
    };

    match shape {
        ResponseShape::None => unreachable!("handled above"),

        ResponseShape::Statistics => {
            fwd.next_message_head().await?;
            out.kind = Some(OutcomeKind::Statistics);
        }

        ResponseShape::PrepareOk => {
            let head = fwd.next_message_head().await?;
            match head.first() {
                Some(0xFF) => {
                    out.kind = Some(OutcomeKind::Error);
                    parse_err(&head, session, &mut out);
                }
                Some(0x00) => {
                    let mut c = Cursor::new(&head[1..]);
                    c.u32_le();
                    let num_columns = c.u16_le().unwrap_or(0) as u64;
                    let num_params = c.u16_le().unwrap_or(0) as u64;
                    skip_column_definitions(&mut fwd, num_params, session).await?;
                    skip_column_definitions(&mut fwd, num_columns, session).await?;
                    out.kind = Some(OutcomeKind::Prepared);
                }
                other => {
                    return Err(ResponseError::Protocol(format!(
                        "unexpected prepare response header {other:?}"
                    )))
                }
            }
        }

        ResponseShape::FieldList => {
            let mut fields = 0u64;
            loop {
                let head = fwd.next_message_head().await?;
                if head.first() == Some(&0xFF) {
                    out.kind = Some(OutcomeKind::Error);
                    parse_err(&head, session, &mut out);
                    break;
                }
                if is_result_terminator(&head) {
                    out.kind = Some(OutcomeKind::ResultSet);
                    out.returned_rows = Some(fields);
                    break;
                }
                fields += 1;
            }
        }

        ResponseShape::Generic => {
            let mut rows_total = 0u64;
            let mut saw_result_set = false;

            loop {
                let head = fwd.next_message_head().await?;
                match head.first() {
                    Some(0xFF) => {
                        out.kind = Some(OutcomeKind::Error);
                        parse_err(&head, session, &mut out);
                        break;
                    }
                    Some(0xFB) => return Err(ResponseError::LocalInfileRequested),
                    Some(0x00) => {
                        let (affected, status) = parse_ok_body(&head[1..], session);
                        out.affected_rows =
                            Some(out.affected_rows.unwrap_or(0) + affected.unwrap_or(0));
                        if !saw_result_set {
                            out.kind = Some(OutcomeKind::Ok);
                        }
                        if more_results(status) {
                            continue;
                        }
                        break;
                    }
                    Some(_) => {
                        // Anything else is a column count introducing a result set.
                        let column_count = Cursor::new(&head[..])
                            .lenenc_int()
                            .flatten()
                            .ok_or_else(|| {
                                ResponseError::Protocol("malformed column count".into())
                            })?;
                        saw_result_set = true;
                        out.result_set_count += 1;
                        skip_column_definitions(&mut fwd, column_count, session).await?;

                        let status = loop {
                            let row = fwd.next_message_head().await?;
                            if row.first() == Some(&0xFF) {
                                out.kind = Some(OutcomeKind::Error);
                                parse_err(&row, session, &mut out);
                                out.returned_rows = Some(rows_total);
                                return Ok(out);
                            }
                            if is_result_terminator(&row) {
                                break terminator_status(&row, session);
                            }
                            rows_total += 1;
                        };

                        out.kind = Some(OutcomeKind::ResultSet);
                        out.returned_rows = Some(rows_total);
                        if more_results(status) {
                            continue;
                        }
                        break;
                    }
                    None => {
                        return Err(ResponseError::Protocol(
                            "empty packet where a response was expected".into(),
                        ))
                    }
                }
            }
        }
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::capabilities::*;

    fn session(caps: u32) -> Session {
        Session {
            capabilities: caps | CLIENT_PROTOCOL_41,
            username: "u".into(),
            database: None,
            backend_connection_id: 1,
        }
    }

    /// Serializes payloads into a packet stream with ascending sequence ids.
    fn wire(payloads: &[Vec<u8>]) -> Vec<u8> {
        let mut out = Vec::new();
        for (i, p) in payloads.iter().enumerate() {
            out.extend_from_slice(&(p.len() as u32).to_le_bytes()[..3]);
            out.push(i as u8 + 1);
            out.extend_from_slice(p);
        }
        out
    }

    fn ok_packet(affected: u8, status: u16) -> Vec<u8> {
        let mut p = vec![0x00, affected, 0x00];
        p.extend_from_slice(&status.to_le_bytes());
        p.extend_from_slice(&0u16.to_le_bytes());
        p
    }

    fn eof_packet(status: u16) -> Vec<u8> {
        let mut p = vec![0xFE];
        p.extend_from_slice(&0u16.to_le_bytes());
        p.extend_from_slice(&status.to_le_bytes());
        p
    }

    fn eof_as_ok(status: u16) -> Vec<u8> {
        let mut p = vec![0xFE, 0x00, 0x00];
        p.extend_from_slice(&status.to_le_bytes());
        p.extend_from_slice(&0u16.to_le_bytes());
        p
    }

    fn err_packet(code: u16, msg: &str) -> Vec<u8> {
        let mut p = vec![0xFF];
        p.extend_from_slice(&code.to_le_bytes());
        p.push(b'#');
        p.extend_from_slice(b"42S02");
        p.extend_from_slice(msg.as_bytes());
        p
    }

    fn column_def() -> Vec<u8> {
        vec![0x03, b'd', b'e', b'f', 0x00, 0x00, 0x00]
    }

    fn text_row(value: &str) -> Vec<u8> {
        let mut p = vec![value.len() as u8];
        p.extend_from_slice(value.as_bytes());
        p
    }

    async fn run(packets: &[Vec<u8>], caps: u32, shape: ResponseShape) -> (Outcome, Vec<u8>) {
        let input = wire(packets);
        let mut rx = PacketReader::new(&input[..]);
        let mut sink = Vec::new();
        let mut tx = PacketWriter::new(&mut sink);
        let outcome = relay(&mut rx, &mut tx, &session(caps), shape).await.unwrap();
        (outcome, sink)
    }

    #[tokio::test]
    async fn ok_response() {
        let (o, forwarded) = run(&[ok_packet(3, 0)], 0, ResponseShape::Generic).await;
        assert_eq!(o.kind, Some(OutcomeKind::Ok));
        assert_eq!(o.affected_rows, Some(3));
        assert_eq!(forwarded, wire(&[ok_packet(3, 0)]));
    }

    #[tokio::test]
    async fn error_response() {
        let (o, _) = run(&[err_packet(1146, "no such table")], 0, ResponseShape::Generic).await;
        assert_eq!(o.kind, Some(OutcomeKind::Error));
        assert_eq!(o.error_code, Some(1146));
        assert_eq!(o.sql_state.as_deref(), Some("42S02"));
        assert_eq!(o.error_message.as_deref(), Some("no such table"));
    }

    #[tokio::test]
    async fn result_set_without_deprecate_eof() {
        let packets = vec![
            vec![0x01],
            column_def(),
            eof_packet(0),
            text_row("a"),
            text_row("b"),
            eof_packet(0),
        ];
        let (o, forwarded) = run(&packets, 0, ResponseShape::Generic).await;
        assert_eq!(o.kind, Some(OutcomeKind::ResultSet));
        assert_eq!(o.returned_rows, Some(2));
        assert_eq!(o.result_set_count, 1);
        assert_eq!(forwarded, wire(&packets), "response must pass through verbatim");
    }

    #[tokio::test]
    async fn result_set_with_deprecate_eof() {
        let packets = vec![
            vec![0x01],
            column_def(),
            text_row("a"),
            text_row("b"),
            text_row("c"),
            eof_as_ok(0),
        ];
        let (o, _) = run(&packets, CLIENT_DEPRECATE_EOF, ResponseShape::Generic).await;
        assert_eq!(o.kind, Some(OutcomeKind::ResultSet));
        assert_eq!(o.returned_rows, Some(3));
    }

    #[tokio::test]
    async fn empty_result_set() {
        let packets = vec![vec![0x01], column_def(), eof_packet(0), eof_packet(0)];
        let (o, _) = run(&packets, 0, ResponseShape::Generic).await;
        assert_eq!(o.returned_rows, Some(0));
    }

    #[tokio::test]
    async fn multiple_result_sets() {
        let packets = vec![
            vec![0x01],
            column_def(),
            eof_packet(0),
            text_row("a"),
            eof_packet(SERVER_MORE_RESULTS_EXISTS),
            vec![0x01],
            column_def(),
            eof_packet(0),
            text_row("b"),
            text_row("c"),
            eof_packet(0),
        ];
        let (o, _) = run(&packets, 0, ResponseShape::Generic).await;
        assert_eq!(o.result_set_count, 2);
        assert_eq!(o.returned_rows, Some(3));
    }

    #[tokio::test]
    async fn ok_then_another_result() {
        let packets = vec![
            ok_packet(1, SERVER_MORE_RESULTS_EXISTS),
            ok_packet(2, 0),
        ];
        let (o, _) = run(&packets, 0, ResponseShape::Generic).await;
        assert_eq!(o.affected_rows, Some(3));
        assert_eq!(o.kind, Some(OutcomeKind::Ok));
    }

    #[tokio::test]
    async fn error_midway_through_rows() {
        let packets = vec![
            vec![0x01],
            column_def(),
            eof_packet(0),
            text_row("a"),
            err_packet(1317, "interrupted"),
        ];
        let (o, _) = run(&packets, 0, ResponseShape::Generic).await;
        assert_eq!(o.kind, Some(OutcomeKind::Error));
        assert_eq!(o.error_code, Some(1317));
        assert_eq!(o.returned_rows, Some(1));
    }

    #[tokio::test]
    async fn local_infile_request_is_rejected() {
        let input = wire(&[vec![0xFB, b'/', b't', b'm', b'p']]);
        let mut rx = PacketReader::new(&input[..]);
        let mut sink = Vec::new();
        let mut tx = PacketWriter::new(&mut sink);
        let err = relay(&mut rx, &mut tx, &session(0), ResponseShape::Generic)
            .await
            .unwrap_err();
        assert!(matches!(err, ResponseError::LocalInfileRequested));
    }

    #[tokio::test]
    async fn prepare_response_with_params_and_columns() {
        let mut header = vec![0x00];
        header.extend_from_slice(&1u32.to_le_bytes());
        header.extend_from_slice(&2u16.to_le_bytes()); // columns
        header.extend_from_slice(&1u16.to_le_bytes()); // params
        header.push(0x00);
        header.extend_from_slice(&0u16.to_le_bytes());

        let packets = vec![
            header,
            column_def(),
            eof_packet(0),
            column_def(),
            column_def(),
            eof_packet(0),
        ];
        let (o, _) = run(&packets, 0, ResponseShape::PrepareOk).await;
        assert_eq!(o.kind, Some(OutcomeKind::Prepared));
    }

    #[tokio::test]
    async fn prepare_response_error() {
        let (o, _) = run(&[err_packet(1064, "syntax")], 0, ResponseShape::PrepareOk).await;
        assert_eq!(o.kind, Some(OutcomeKind::Error));
        assert_eq!(o.error_code, Some(1064));
    }

    #[tokio::test]
    async fn no_response_commands_return_immediately() {
        let (o, forwarded) = run(&[], 0, ResponseShape::None).await;
        assert_eq!(o.kind, Some(OutcomeKind::NoResponse));
        assert!(forwarded.is_empty());
    }

    #[tokio::test]
    async fn statistics_is_a_single_packet() {
        let packets = vec![b"Uptime: 100".to_vec()];
        let (o, forwarded) = run(&packets, 0, ResponseShape::Statistics).await;
        assert_eq!(o.kind, Some(OutcomeKind::Statistics));
        assert_eq!(forwarded, wire(&packets));
    }

    #[test]
    fn full_length_packet_starting_with_fe_is_a_row_not_a_terminator() {
        let mut row = vec![0xFE];
        row.resize(MAX_PAYLOAD, b'x');
        assert!(!is_result_terminator(&row));
        assert!(is_result_terminator(&[0xFE, 0, 0, 0, 0]));
    }
}
