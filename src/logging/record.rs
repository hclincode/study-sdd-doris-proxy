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
    /// Present and true when a row filter was injected. Absent means the
    /// statement was forwarded as written, so records for traffic no rule
    /// touched keep exactly the shape they had before row filtering existed.
    #[serde(skip_serializing_if = "is_false")]
    pub rewritten: bool,
    /// The statement as forwarded to the backend. Present only on a rewrite;
    /// `statement` always holds what the client submitted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub forwarded_statement: Option<String>,
    /// The table whose rule was applied.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter_table: Option<String>,
    /// Why a wanted rewrite did not happen. Present only when a rule plausibly
    /// applied, so these can be counted as missing coverage.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter_skipped: Option<&'static str>,
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
            rewritten: false,
            forwarded_statement: None,
            filter_table: None,
            filter_skipped: None,
        }
    }

    fn json(r: CommandRecord) -> serde_json::Value {
        serde_json::from_str(&serde_json::to_string(&Record::Command(Box::new(r))).unwrap())
            .unwrap()
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
    fn a_rewritten_statement_reports_both_forms() {
        let mut r = sample();
        r.rewritten = true;
        r.forwarded_statement =
            Some("SELECT * FROM t WHERE ( id = 1) AND (tenant_id = 7)".into());
        r.filter_table = Some("t".into());
        let v = json(r);
        assert_eq!(v["rewritten"], true);
        assert_eq!(v["filter_table"], "t");
        assert_eq!(
            v["forwarded_statement"],
            "SELECT * FROM t WHERE ( id = 1) AND (tenant_id = 7)"
        );
        assert_eq!(
            v["statement"], "SELECT * FROM t WHERE id = 1",
            "the client's own text must survive alongside the rewrite"
        );
        assert!(v.get("filter_skipped").is_none());
    }

    #[test]
    fn a_skipped_rewrite_reports_a_reason_and_no_rewrite() {
        let mut r = sample();
        r.filter_skipped = Some("multiple_tables");
        let v = json(r);
        assert_eq!(v["filter_skipped"], "multiple_tables");
        assert!(v.get("rewritten").is_none(), "absence means not rewritten");
        assert!(v.get("forwarded_statement").is_none());
    }

    #[test]
    fn traffic_no_rule_touched_keeps_its_original_shape() {
        let v = json(sample());
        for absent in ["rewritten", "forwarded_statement", "filter_table", "filter_skipped"] {
            assert!(v.get(absent).is_none(), "{absent} must be omitted");
        }
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
