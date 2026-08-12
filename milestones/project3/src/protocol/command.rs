//! Client command classification.

use bytes::Bytes;

// Command bytes the proxy names explicitly.
pub const COM_QUIT: u8 = 0x01;
pub const COM_INIT_DB: u8 = 0x02;
pub const COM_QUERY: u8 = 0x03;
pub const COM_FIELD_LIST: u8 = 0x04;
pub const COM_STATISTICS: u8 = 0x09;
pub const COM_PROCESS_KILL: u8 = 0x0C;
pub const COM_DEBUG: u8 = 0x0D;
pub const COM_PING: u8 = 0x0E;
pub const COM_CHANGE_USER: u8 = 0x11;
pub const COM_BINLOG_DUMP: u8 = 0x12;
pub const COM_TABLE_DUMP: u8 = 0x13;
pub const COM_CONNECT_OUT: u8 = 0x14;
pub const COM_REGISTER_SLAVE: u8 = 0x15;
pub const COM_STMT_PREPARE: u8 = 0x16;
pub const COM_STMT_EXECUTE: u8 = 0x17;
pub const COM_STMT_SEND_LONG_DATA: u8 = 0x18;
pub const COM_STMT_CLOSE: u8 = 0x19;
pub const COM_STMT_RESET: u8 = 0x1A;
pub const COM_SET_OPTION: u8 = 0x1B;
pub const COM_STMT_FETCH: u8 = 0x1C;
pub const COM_BINLOG_DUMP_GTID: u8 = 0x1E;
pub const COM_RESET_CONNECTION: u8 = 0x1F;
pub const COM_CLONE: u8 = 0x20;

/// The shape of the response a command produces, which decides how the
/// response state machine reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponseShape {
    /// The backend sends nothing at all.
    None,
    /// A single packet of free-form text with no status header.
    Statistics,
    /// The `COM_STMT_PREPARE` response: a header, then parameter and column
    /// definitions.
    PrepareOk,
    /// Column definitions terminated by EOF, with no preceding column count.
    FieldList,
    /// OK, ERR, or a result set.
    Generic,
}

/// A command read from the client, buffered whole.
#[derive(Debug, Clone)]
pub struct Command {
    pub code: u8,
    pub payload: Bytes,
    /// Sequence id of the command's first packet.
    pub first_seq: u8,
    /// How many packets it occupied on the wire.
    pub packet_count: usize,
}

impl Command {
    pub fn new(payload: Bytes, first_seq: u8, packet_count: usize) -> Self {
        let code = payload.first().copied().unwrap_or(0xFF);
        Self {
            code,
            payload,
            first_seq,
            packet_count,
        }
    }

    /// Statement text, for the commands that carry SQL as a plain trailing
    /// string. `CLIENT_QUERY_ATTRIBUTES` is masked off during the handshake, so
    /// `COM_QUERY` has no binary parameter block in front of the statement.
    pub fn statement(&self) -> Option<&[u8]> {
        match self.code {
            COM_QUERY | COM_STMT_PREPARE => Some(&self.payload[1..]),
            _ => None,
        }
    }

    /// True for commands whose response the proxy cannot follow. These are
    /// refused rather than forwarded: a command that puts the connection into a
    /// streaming mode would desynchronize the state machine permanently, and
    /// the proxy would have no way to notice.
    pub fn is_refused(&self) -> bool {
        matches!(
            self.code,
            COM_BINLOG_DUMP
                | COM_BINLOG_DUMP_GTID
                | COM_REGISTER_SLAVE
                | COM_TABLE_DUMP
                | COM_CONNECT_OUT
                | COM_CLONE
                | COM_CHANGE_USER
        )
    }

    pub fn response_shape(&self) -> ResponseShape {
        match self.code {
            COM_QUIT | COM_STMT_CLOSE | COM_STMT_SEND_LONG_DATA => ResponseShape::None,
            COM_STATISTICS => ResponseShape::Statistics,
            COM_STMT_PREPARE => ResponseShape::PrepareOk,
            COM_FIELD_LIST => ResponseShape::FieldList,
            _ => ResponseShape::Generic,
        }
    }

    pub fn name(&self) -> &'static str {
        match self.code {
            COM_QUIT => "COM_QUIT",
            COM_INIT_DB => "COM_INIT_DB",
            COM_QUERY => "COM_QUERY",
            COM_FIELD_LIST => "COM_FIELD_LIST",
            COM_STATISTICS => "COM_STATISTICS",
            COM_PROCESS_KILL => "COM_PROCESS_KILL",
            COM_DEBUG => "COM_DEBUG",
            COM_PING => "COM_PING",
            COM_CHANGE_USER => "COM_CHANGE_USER",
            COM_BINLOG_DUMP => "COM_BINLOG_DUMP",
            COM_TABLE_DUMP => "COM_TABLE_DUMP",
            COM_CONNECT_OUT => "COM_CONNECT_OUT",
            COM_REGISTER_SLAVE => "COM_REGISTER_SLAVE",
            COM_STMT_PREPARE => "COM_STMT_PREPARE",
            COM_STMT_EXECUTE => "COM_STMT_EXECUTE",
            COM_STMT_SEND_LONG_DATA => "COM_STMT_SEND_LONG_DATA",
            COM_STMT_CLOSE => "COM_STMT_CLOSE",
            COM_STMT_RESET => "COM_STMT_RESET",
            COM_SET_OPTION => "COM_SET_OPTION",
            COM_STMT_FETCH => "COM_STMT_FETCH",
            COM_BINLOG_DUMP_GTID => "COM_BINLOG_DUMP_GTID",
            COM_RESET_CONNECTION => "COM_RESET_CONNECTION",
            COM_CLONE => "COM_CLONE",
            _ => "COM_UNKNOWN",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd(bytes: &[u8]) -> Command {
        Command::new(Bytes::copy_from_slice(bytes), 0, 1)
    }

    #[test]
    fn extracts_statement_text() {
        let c = cmd(b"\x03SELECT 1");
        assert_eq!(c.statement(), Some(&b"SELECT 1"[..]));
        assert_eq!(c.name(), "COM_QUERY");

        let c = cmd(b"\x16SELECT ?");
        assert_eq!(c.statement(), Some(&b"SELECT ?"[..]));
    }

    #[test]
    fn commands_without_sql_have_no_statement() {
        assert!(cmd(b"\x0e").statement().is_none());
        assert!(cmd(b"\x02shop").statement().is_none());
        assert!(cmd(b"\x17\x01\x00\x00\x00").statement().is_none());
    }

    #[test]
    fn empty_query_yields_empty_statement() {
        assert_eq!(cmd(b"\x03").statement(), Some(&b""[..]));
    }

    #[test]
    fn replication_and_user_change_commands_are_refused() {
        for code in [0x11u8, 0x12, 0x13, 0x14, 0x15, 0x1E, 0x20] {
            assert!(cmd(&[code]).is_refused(), "code {code:#x}");
        }
        for code in [0x03u8, 0x0E, 0x16, 0x17, 0x02] {
            assert!(!cmd(&[code]).is_refused(), "code {code:#x}");
        }
    }

    #[test]
    fn response_shapes_match_command_kinds() {
        assert_eq!(cmd(b"\x01").response_shape(), ResponseShape::None);
        assert_eq!(cmd(b"\x19").response_shape(), ResponseShape::None);
        assert_eq!(cmd(b"\x18").response_shape(), ResponseShape::None);
        assert_eq!(cmd(b"\x09").response_shape(), ResponseShape::Statistics);
        assert_eq!(cmd(b"\x16x").response_shape(), ResponseShape::PrepareOk);
        assert_eq!(cmd(b"\x04t").response_shape(), ResponseShape::FieldList);
        assert_eq!(cmd(b"\x03x").response_shape(), ResponseShape::Generic);
        assert_eq!(cmd(b"\x17").response_shape(), ResponseShape::Generic);
    }
}
