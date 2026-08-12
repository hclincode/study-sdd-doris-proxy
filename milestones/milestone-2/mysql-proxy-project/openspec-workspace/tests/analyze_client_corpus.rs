//! What real MySQL clients actually send, asserted to be analysable.
//!
//! # Why this file exists
//!
//! Three blockers of one class were found in this project, each by a real client
//! hours after the code was written, and each time the proxy was unusable for an
//! ordinary connection until it was fixed:
//!
//! - `SET NAMES utf8mb4` refused, so no connector could complete a handshake.
//! - `COMMIT` refused while `SET autocommit=0` was forwarded, so a client could
//!   open a transaction it could not close.
//! - `SHOW COLUMNS` refused, so anything that introspects broke.
//!
//! None of them was a flaw in the enumeration, which is the part with property
//! tests and mutation coverage. They failed at *classification*, so enumeration
//! was never asked. **No test we had could have caught them, because none of
//! them asked a question a client asks.** This file is that question, and its
//! job is to turn "found by a real client three hours later" into 200
//! milliseconds of `cargo test`.
//!
//! # What it asserts, and what it deliberately does not
//!
//! **Analysability only.** Each statement must classify and enumerate — the
//! proxy must be able to *decide*. Whether the decision is forward or refuse
//! depends on the policy set and the session, and belongs to the tests that own
//! those rules. A corpus that pinned dispositions would fail every time a
//! ruling changed, and would be deleted for noise before it caught anything.
//!
//! # Provenance
//!
//! Every entry says where it came from, because "JDBC sends this on connect" is
//! a claim, and a corpus of invented statements would be the same mistake as a
//! fixture whose types are narrower than reality. Two levels:
//!
//! - [`Source::Measured`] — captured off the wire on this machine, by putting a
//!   minimal MySQL server in front of the real client and logging its
//!   `COM_QUERY` payloads. These are facts.
//! - [`Source::Documented`] — from the connector's published behaviour, for
//!   clients not installed here. These are claims, and are marked as such so
//!   nobody later mistakes them for observations.
//!
//! Measuring corrected me twice while assembling this. The `mysql` CLI does
//! **not** send `SET NAMES` as a statement — it sets the charset in the
//! handshake packet, and I had asserted otherwise from an earlier end-to-end run
//! where the `SET NAMES` was one I had typed myself. And `mysqldump` opens with
//! three executable comments, not with `START TRANSACTION`.

use doris_row_filter_proxy::analyze::analyze;

/// Where an entry came from. See the module docs — the distinction is the point.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Source {
    /// Captured off the wire on this machine.
    Measured,
    /// From the connector's published behaviour; not observed here.
    Documented,
}

/// Whether the proxy can analyse this statement today.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Expected {
    Analysable,
    /// Known-failing, with the reason. Recorded rather than omitted: a corpus
    /// containing only passing cases cannot tell you what is broken, and these
    /// are the entries that say what a real client cannot do through the proxy.
    KnownFailing(&'static str),
}

struct Entry {
    sql: &'static str,
    source: Source,
    client: &'static str,
    expected: Expected,
}

const fn ok(sql: &'static str, source: Source, client: &'static str) -> Entry {
    Entry {
        sql,
        source,
        client,
        expected: Expected::Analysable,
    }
}

const fn failing(
    sql: &'static str,
    source: Source,
    client: &'static str,
    why: &'static str,
) -> Entry {
    Entry {
        sql,
        source,
        client,
        expected: Expected::KnownFailing(why),
    }
}

const CORPUS: &[Entry] = &[
    // -- connection startup ------------------------------------------------
    //
    // The `mysql` CLI sends exactly one statement before the user's first
    // query, with or without `--default-character-set`. Captured on this
    // machine; the charset is negotiated in the handshake packet, not here.
    ok(
        "select @@version_comment limit 1",
        Source::Measured,
        "mysql CLI 8.x, on connect",
    ),
    // Connectors that do issue charset and mode statements at startup. These
    // are the shapes that made the proxy unusable in 10.1.
    ok("SET NAMES utf8mb4", Source::Documented, "JDBC, Python, Go"),
    ok(
        "SET NAMES utf8mb4 COLLATE utf8mb4_unicode_ci",
        Source::Documented,
        "JDBC with connectionCollation",
    ),
    ok("SET autocommit=1", Source::Documented, "JDBC, ODBC"),
    ok("SET SESSION sql_mode=''", Source::Documented, "JDBC"),
    ok(
        "SET @@session.time_zone='+00:00'",
        Source::Documented,
        "JDBC with serverTimezone",
    ),
    ok(
        "SET character_set_results=NULL",
        Source::Documented,
        "Connector/J character-set negotiation",
    ),
    ok("SELECT @@version_comment", Source::Documented, "Python, Go"),
    ok("SELECT VERSION()", Source::Documented, "ODBC, BI tools"),
    ok("SELECT DATABASE()", Source::Documented, "JDBC, ODBC"),
    ok("SELECT @@sql_mode", Source::Documented, "JDBC"),
    ok(
        "SET @@sql_select_limit=DEFAULT",
        Source::Documented,
        "Connector/J after setMaxRows",
    ),
    ok(
        "SET SESSION NET_READ_TIMEOUT= 86400, SESSION NET_WRITE_TIMEOUT= 86400",
        Source::Measured,
        "mysqldump 8.x",
    ),
    // -- transaction lifecycle ---------------------------------------------
    //
    // The pairing from 10.3: every statement here must be analysable, or a
    // client can enter a state it cannot leave.
    ok("SET autocommit=0", Source::Documented, "JDBC setAutoCommit"),
    ok("START TRANSACTION", Source::Documented, "JDBC, Python"),
    ok("BEGIN", Source::Documented, "Go, ODBC"),
    ok("COMMIT", Source::Documented, "every connector"),
    ok("ROLLBACK", Source::Documented, "every connector"),
    ok(
        "SET SESSION TRANSACTION ISOLATION LEVEL READ COMMITTED",
        Source::Documented,
        "JDBC setTransactionIsolation",
    ),
    ok("SAVEPOINT sp1", Source::Documented, "JDBC setSavepoint"),
    ok(
        "ROLLBACK TO SAVEPOINT sp1",
        Source::Documented,
        "JDBC rollback(Savepoint)",
    ),
    ok(
        "RELEASE SAVEPOINT sp1",
        Source::Documented,
        "JDBC releaseSavepoint",
    ),
    // -- introspection -----------------------------------------------------
    //
    // What BI and admin tools ask before they can show anything. 10.7 was this
    // group.
    ok("SHOW DATABASES", Source::Documented, "BI tools, DBeaver"),
    ok("SHOW TABLES", Source::Documented, "BI tools"),
    ok("SHOW TABLES FROM sales", Source::Documented, "DBeaver"),
    ok(
        "SHOW FULL TABLES WHERE Table_type = 'BASE TABLE'",
        Source::Documented,
        "MySQL Workbench",
    ),
    ok(
        "SHOW COLUMNS FROM sales.orders",
        Source::Documented,
        "JDBC metadata",
    ),
    ok(
        "SHOW CREATE TABLE sales.orders",
        Source::Documented,
        "DBeaver DDL view",
    ),
    ok(
        "SHOW VARIABLES LIKE 'gtid_mode'",
        Source::Measured,
        "mysqldump 8.x",
    ),
    ok(
        "SHOW SESSION STATUS",
        Source::Documented,
        "monitoring tools",
    ),
    ok("SHOW COLLATION", Source::Documented, "Connector/J"),
    ok("SHOW CHARACTER SET", Source::Documented, "ODBC"),
    ok("SHOW ENGINES", Source::Documented, "MySQL Workbench"),
    ok(
        "SHOW WARNINGS",
        Source::Documented,
        "every connector after an error",
    ),
    ok(
        "SELECT * FROM information_schema.columns WHERE table_schema='sales'",
        Source::Documented,
        "JDBC DatabaseMetaData",
    ),
    ok(
        "SELECT table_name FROM information_schema.tables WHERE table_schema='sales'",
        Source::Documented,
        "BI tools",
    ),
    // -- known-failing -----------------------------------------------------
    //
    // These are what a real client cannot do through the proxy today. They are
    // in the corpus so the cost is visible and so a later `sqlparser` upgrade
    // that fixes one turns this test red and forces a decision, rather than
    // quietly widening what is accepted.
    // Reclassified from `failing` to `ok` deliberately, in the change that made
    // it true — not as later maintenance.
    //
    // These carry a `/*!NNNNN */` gate, and they read no policy table, so they
    // are **forwarded verbatim with the gate intact** rather than rewritten.
    // The 10.6 hazard was never the comment; it was re-rendering the AST, which
    // discards a condition only the backend can evaluate. Forward-verbatim does
    // not re-render, so the hazard is absent. See
    // `tests/analyze_comment_scanner.rs`.
    ok(
        "/*!40100 SET @@SQL_MODE='' */",
        Source::Measured,
        "mysqldump 8.x, first statement",
    ),
    ok(
        "/*!40103 SET TIME_ZONE='+00:00' */",
        Source::Measured,
        "mysqldump 8.x, second statement",
    ),
    ok(
        "/*!80000 SET SESSION information_schema_stats_expiry=0 */",
        Source::Measured,
        "mysqldump 8.x, third statement",
    ),
    // The measured form, which is not the one I first recorded: the
    // `WITH CONSISTENT SNAPSHOT` is itself inside a gate. Captured from
    // `mysqldump`'s own error quoting the statement it sent. `sqlparser` parses
    // the gate contents, so this is equivalent to
    // `START TRANSACTION WITH CONSISTENT SNAPSHOT`, which it cannot parse.
    failing(
        "START TRANSACTION /*!40100 WITH CONSISTENT SNAPSHOT */",
        Source::Measured,
        "mysqldump --single-transaction",
        "10.7 — `MySqlDialect` cannot parse it; a parser gap, not an allowlist \
         gap, so no classification work fixes it",
    ),
    // Where plain `mysqldump` stops once the gates are forwarded. Observed
    // end-to-end against the live Doris; the statement text is `mysqldump`'s
    // documented form rather than one I captured, since it never reaches a
    // sniffer that cannot return result sets.
    failing(
        "LOCK TABLES `orders` READ /*!32311 LOCAL */",
        Source::Documented,
        "mysqldump without --single-transaction",
        "10.9/10.11 — `LOCK TABLES` is an unclassified statement kind, so it \
         refuses at classification without the walk ever being asked. It names \
         a table and reads no row, so the default change should decide it",
    ),
    failing(
        "UNLOCK TABLES",
        Source::Documented,
        "mysqldump, after dumping",
        "10.9/10.11 — unclassified kind, and the other half of a pair: a client \
         allowed to LOCK must be allowed to UNLOCK",
    ),
    failing(
        "SHOW CREATE DATABASE `sales`",
        Source::Documented,
        "mysqldump --databases, DBeaver",
        "10.7 — `MySqlDialect` cannot parse it",
    ),
    failing(
        "COMMIT RELEASE",
        Source::Documented,
        "clients closing a session",
        "10.7 — `MySqlDialect` cannot parse it",
    ),
    failing(
        "ROLLBACK RELEASE",
        Source::Documented,
        "clients closing a session",
        "10.7 — `MySqlDialect` cannot parse it",
    ),
];

/// The corpus, checked against the analyser.
///
/// A failure here means a real client cannot use the proxy. That is a different
/// and more urgent signal than a unit test failing, which is why the message
/// says so.
#[test]
fn every_statement_real_clients_send_is_analysable() {
    let mut broken = Vec::new();
    let mut unexpectedly_fixed = Vec::new();

    for entry in CORPUS {
        let analysed = analyze(entry.sql).is_ok();
        match entry.expected {
            Expected::Analysable if !analysed => {
                let reason = analyze(entry.sql).unwrap_err();
                broken.push(format!(
                    "  {:?} ({}, {:?})\n      -> {reason}",
                    entry.sql, entry.client, entry.source
                ));
            }
            Expected::KnownFailing(why) if analysed => {
                unexpectedly_fixed.push(format!(
                    "  {:?} ({})\n      was: {why}",
                    entry.sql, entry.client
                ));
            }
            _ => {}
        }
    }

    assert!(
        broken.is_empty(),
        "statements real clients send can no longer be analysed — the proxy is \
         broken for those clients, not merely stricter:\n{}",
        broken.join("\n")
    );

    assert!(
        unexpectedly_fixed.is_empty(),
        "known-failing entries now analyse. That is good news, but decide what \
         the disposition should be and move them to `ok(...)` deliberately \
         rather than letting the allowlist widen unnoticed:\n{}",
        unexpectedly_fixed.join("\n")
    );
}

/// The corpus is only worth what its provenance is worth.
///
/// Guards against the failure mode where someone adds a plausible-looking
/// statement nobody has seen a client send: every entry must name a client, and
/// enough of the corpus must be measured rather than recalled that the file
/// stays anchored to observation.
#[test]
fn every_corpus_entry_records_where_it_came_from() {
    for entry in CORPUS {
        assert!(
            !entry.client.is_empty(),
            "{:?} does not say which client sends it",
            entry.sql
        );
    }

    let measured = CORPUS
        .iter()
        .filter(|e| e.source == Source::Measured)
        .count();
    assert!(
        measured >= 6,
        "only {measured} entries are measured; the corpus is drifting towards \
         recollection. Capture more off the wire before adding claims."
    );
}

/// The specific sequence that made the proxy unusable in 10.1, kept together so
/// the regression is legible as a *sequence* rather than as scattered cases.
///
/// A connector issues these before its first query. If any one is unanalysable,
/// the client never reaches a state where it can query at all.
#[test]
fn a_connector_can_complete_its_startup_sequence() {
    for sql in [
        "SET NAMES utf8mb4",
        "SET autocommit=1",
        "SET SESSION sql_mode=''",
        "SELECT @@version_comment",
        "SELECT DATABASE()",
    ] {
        assert!(
            analyze(sql).is_ok(),
            "{sql:?} is part of connector startup; refusing it means no client can connect"
        );
    }
}

/// The pairing from 10.3, kept as a sequence for the same reason: the defect was
/// not that `COMMIT` was refused, it was that `SET autocommit=0` was *forwarded*
/// while `COMMIT` was refused. Neither statement alone shows that.
#[test]
fn a_client_can_close_the_transaction_it_opened() {
    for sql in [
        "SET autocommit=0",
        "START TRANSACTION",
        "COMMIT",
        "ROLLBACK",
        "SET autocommit=1",
    ] {
        assert!(
            analyze(sql).is_ok(),
            "{sql:?} is part of the transaction lifecycle; a client that can open \
             one must be able to close it"
        );
    }
}
