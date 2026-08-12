//! Log record types.

use serde::Serialize;

fn is_false(b: &bool) -> bool {
    !*b
}

/// Outcome strings used in records. The protocol-level outcomes come from the
/// response state machine; the last two are decisions the proxy itself made.
pub mod outcome {
    /// The connection was torn down before the response completed.
    pub const TERMINATED: &str = "terminated";
    /// The proxy declined to forward the command.
    pub const REFUSED: &str = "refused";
}

#[derive(Debug, Clone, Serialize)]
pub struct CommandRecord {
    pub ts: String,
    pub ts_unix_ms: i64,
    pub listener: String,
    /// Stable for the life of the client connection.
    pub connection_id: u64,
    /// The backend's own connection id, for correlating with its logs.
    pub backend_connection_id: u32,
    pub client_addr: String,
    pub username: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub database: Option<String>,
    pub command: &'static str,
    /// Complete statement text as submitted, for the commands that carry SQL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub statement: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub digest_hash: Option<String>,
    /// Present and true when the statement could not be normalized. The record
    /// is still emitted; only the digest is missing.
    #[serde(skip_serializing_if = "is_false")]
    pub digest_unavailable: bool,
    pub duration_us: u64,
    pub outcome: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub affected_rows: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub returned_rows: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_sets: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sql_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

/// Written whenever the discard counter has advanced, so a reader can tell the
/// record stream in front of them is incomplete.
#[derive(Debug, Clone, Serialize)]
pub struct DroppedRecord {
    pub ts: String,
    pub ts_unix_ms: i64,
    pub listener: String,
    /// Records discarded since the proxy started.
    pub dropped_total: u64,
    /// Records discarded since the previous such record.
    pub dropped_since_last: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Record {
    Command(Box<CommandRecord>),
    Dropped(DroppedRecord),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::timestamp::Timestamp;

    fn sample() -> CommandRecord {
        let ts = Timestamp { unix_ms: 1_700_000_000_000 };
        CommandRecord {
            ts: ts.to_rfc3339(),
            ts_unix_ms: ts.unix_ms,
            listener: "primary".into(),
            connection_id: 7,
            backend_connection_id: 91,
            client_addr: "127.0.0.1:5000".into(),
            username: "app".into(),
            database: Some("shop".into()),
            command: "COM_QUERY",
            statement: Some("SELECT * FROM t WHERE id = 1".into()),
            digest: Some("SELECT * FROM t WHERE id = ?".into()),
            digest_hash: Some("00ff00ff00ff00ff".into()),
            digest_unavailable: false,
            duration_us: 1234,
            outcome: "result_set".into(),
            affected_rows: None,
            returned_rows: Some(3),
            result_sets: Some(1),
            error_code: None,
            sql_state: None,
            error_message: None,
        }
    }

    #[test]
    fn serializes_as_one_json_object_on_one_line() {
        let line = serde_json::to_string(&Record::Command(Box::new(sample()))).unwrap();
        assert!(!line.contains('\n'));
        let parsed: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert!(parsed.is_object());
        assert_eq!(parsed["type"], "command");
        assert_eq!(parsed["listener"], "primary");
        assert_eq!(parsed["returned_rows"], 3);
        assert_eq!(parsed["statement"], "SELECT * FROM t WHERE id = 1");
    }

    #[test]
    fn absent_optional_fields_are_omitted() {
        let mut r = sample();
        r.database = None;
        r.returned_rows = None;
        let v: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&Record::Command(Box::new(r))).unwrap())
                .unwrap();
        assert!(v.get("database").is_none());
        assert!(v.get("returned_rows").is_none());
        assert!(v.get("digest_unavailable").is_none());
    }

    #[test]
    fn unavailable_digest_is_marked_explicitly() {
        let mut r = sample();
        r.digest = None;
        r.digest_hash = None;
        r.digest_unavailable = true;
        let v: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&Record::Command(Box::new(r))).unwrap())
                .unwrap();
        assert_eq!(v["digest_unavailable"], true);
        assert!(v.get("digest").is_none());
        assert_eq!(v["statement"], "SELECT * FROM t WHERE id = 1");
    }

    #[test]
    fn dropped_records_are_distinguishable() {
        let d = Record::Dropped(DroppedRecord {
            ts: "1970-01-01T00:00:00.000Z".into(),
            ts_unix_ms: 0,
            listener: "primary".into(),
            dropped_total: 12,
            dropped_since_last: 5,
        });
        let v: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&d).unwrap()).unwrap();
        assert_eq!(v["type"], "dropped");
        assert_eq!(v["dropped_total"], 12);
    }
}
