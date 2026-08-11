//! `connection-routing` behaviour, exercised over real sockets against a fake
//! Doris frontend (`session_fake_doris.rs`).
//!
//! What is genuinely proved here and what is not:
//!
//! - The proxy's own bytes are real. Every assertion below is made against what
//!   the proxy actually wrote to a socket, decoded by a client that shares no
//!   code with it.
//! - The *frontend* is a fake. It performs no password hashing, so these tests
//!   prove the proxy relays a challenge and a response unchanged — which is
//!   exactly design D1's claim — but they cannot prove that a real Doris accepts
//!   the result. That needs task 8.1's integration harness.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use doris_row_filter_proxy::session::{
    Admission, ProxyServer, RefuseAllGate, SessionIdentity, StatementGate, UnfilteredGate,
};
use tokio::net::TcpListener;

#[path = "session_fake_doris.rs"]
mod fake;

use fake::{
    err_message, err_sqlstate, FakeDoris, FakeDorisConfig, RawClient, CLIENT_MULTI_RESULTS,
    CLIENT_MULTI_STATEMENTS, DORIS_SALT,
};

/// A scramble that no hash function would produce, so a test that sees it
/// arrive at the backend knows it was relayed rather than recomputed.
const CLIENT_SCRAMBLE: &[u8; 20] = b"CLIENT-SCRAMBLE-1234";

/// One question the session layer put to the gate: who is asking, where they
/// are, and what they sent.
type Consultation = (SessionIdentity, Option<String>, String);
type Consultations = Arc<Mutex<Vec<Consultation>>>;

/// Records what the session layer asked about each statement, then answers with
/// a fixed verdict. Stands in for `analyze`/`rewrite`.
struct RecordingGate {
    verdict: Box<dyn Fn(&str) -> Admission + Send + Sync>,
    seen: Consultations,
}

impl RecordingGate {
    fn forwarding() -> (Arc<Self>, Consultations) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let gate = Arc::new(RecordingGate {
            verdict: Box::new(|sql| Admission::Forward(sql.to_string())),
            seen: Arc::clone(&seen),
        });
        (gate, seen)
    }
}

impl StatementGate for RecordingGate {
    fn admit(
        &self,
        identity: &SessionIdentity,
        current_database: Option<&str>,
        sql: &str,
    ) -> Admission {
        self.seen.lock().unwrap().push((
            identity.clone(),
            current_database.map(str::to_string),
            sql.to_string(),
        ));
        (self.verdict)(sql)
    }
}

async fn start_proxy(backend_addr: &str, gate: Arc<dyn StatementGate>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let server = Arc::new(ProxyServer::new(backend_addr, gate));
    tokio::spawn(async move {
        let _ = server.run(listener).await;
    });
    addr
}

/// Design D1, task 3.2 — the whole security property in one test.
#[tokio::test]
async fn doris_salt_is_presented_to_the_client_and_the_scramble_relayed_verbatim() {
    let doris = FakeDoris::start(FakeDorisConfig::default()).await;
    let proxy = start_proxy(&doris.addr, Arc::new(UnfilteredGate)).await;

    let mut client = RawClient::connect(&proxy).await.unwrap();

    // The proxy must challenge the client with Doris's salt, not one of its own.
    assert_eq!(
        client.salt.as_slice(),
        DORIS_SALT.as_slice(),
        "proxy presented its own salt instead of relaying Doris's"
    );
    assert_eq!(client.auth_plugin, "mysql_native_password");
    assert_eq!(
        client.server_version, "8.0.33-Doris-fake",
        "proxy should report the backend's version so clients behave identically"
    );

    client
        .send_handshake_response("analyst", CLIENT_SCRAMBLE, None)
        .await
        .unwrap();
    let (_, reply) = client.read_packet().await.unwrap();
    assert_eq!(reply.first(), Some(&0x00), "expected an OK packet");

    doris
        .wait_for("the backend to authenticate", |s| {
            s.first().is_some_and(|s| s.authenticated)
        })
        .await;

    let session = &doris.sessions()[0];
    assert_eq!(session.username, "analyst");
    assert_eq!(
        session.auth_response.as_slice(),
        CLIENT_SCRAMBLE.as_slice(),
        "the client's scramble reached Doris altered; passthrough is broken"
    );
}

/// Design D4, task 3.3.
///
/// This test exists **because we do not control these bits.** `opensrv-mysql`
/// hardcodes the advertised capability set (`src/lib.rs:308-313`) with no hook
/// to change it; the two multi-statement flags happen to be absent from it. So
/// the property D4 asks for currently holds by accident of a dependency, and a
/// version bump could reintroduce either flag with nothing in the proxy to stop
/// it. Reading the flags back off the handshake bytes turns that into a build
/// failure instead of a silently weakened control.
///
/// Note what this does *not* buy: clearing an advertised bit is advisory, and a
/// client may set it anyway. SQL-level multi-statement rejection is the actual
/// control and lives in `analyze`.
#[tokio::test]
async fn advertised_capabilities_exclude_multi_statement() {
    let doris = FakeDoris::start(FakeDorisConfig::default()).await;
    let proxy = start_proxy(&doris.addr, Arc::new(UnfilteredGate)).await;

    let client = RawClient::connect(&proxy).await.unwrap();
    assert_eq!(
        client.server_capabilities & CLIENT_MULTI_STATEMENTS,
        0,
        "proxy advertised CLIENT_MULTI_STATEMENTS to the client"
    );
    assert_eq!(
        client.server_capabilities & CLIENT_MULTI_RESULTS,
        0,
        "proxy advertised CLIENT_MULTI_RESULTS to the client"
    );
}

/// Design D4 again, on the other side: even if a statement got past analysis,
/// Doris would refuse a multi-statement payload on this connection.
#[tokio::test]
async fn backend_connection_does_not_negotiate_multi_statement_support() {
    let doris = FakeDoris::start(FakeDorisConfig::default()).await;
    let proxy = start_proxy(&doris.addr, Arc::new(UnfilteredGate)).await;

    let _client = RawClient::authenticate(&proxy, "analyst", CLIENT_SCRAMBLE)
        .await
        .unwrap();
    doris
        .wait_for("the backend handshake", |s| {
            s.first().is_some_and(|s| s.authenticated)
        })
        .await;

    let capabilities = doris.sessions()[0].capabilities;
    assert_eq!(capabilities & CLIENT_MULTI_STATEMENTS, 0);
    assert_eq!(capabilities & CLIENT_MULTI_RESULTS, 0);
}

/// Task 3.1 / 3.8 — one backend connection per client session, never shared.
#[tokio::test]
async fn each_client_session_gets_its_own_backend_connection() {
    let doris = FakeDoris::start(FakeDorisConfig::default()).await;
    let proxy = start_proxy(&doris.addr, Arc::new(UnfilteredGate)).await;

    let mut first = RawClient::authenticate(&proxy, "analyst", CLIENT_SCRAMBLE)
        .await
        .unwrap();
    let mut second = RawClient::authenticate(&proxy, "analyst", CLIENT_SCRAMBLE)
        .await
        .unwrap();

    first.query("SELECT 'first'").await.unwrap();
    second.query("SELECT 'second'").await.unwrap();

    doris
        .wait_for("both statements to arrive", |s| {
            s.len() == 2 && s.iter().all(|s| s.queries.len() >= 2)
        })
        .await;

    let sessions = doris.sessions();
    assert_eq!(
        sessions.len(),
        2,
        "expected exactly two backend connections"
    );
    // Each statement executed only on its own session's connection.
    assert!(sessions[0].queries.contains(&"SELECT 'first'".to_string()));
    assert!(!sessions[0].queries.contains(&"SELECT 'second'".to_string()));
    assert!(sessions[1].queries.contains(&"SELECT 'second'".to_string()));
    assert!(!sessions[1].queries.contains(&"SELECT 'first'".to_string()));
}

/// Task 3.8 — credentials Doris rejects must not produce a session that can
/// issue statements.
#[tokio::test]
async fn credentials_rejected_by_doris_yield_no_usable_session() {
    let doris = FakeDoris::start(FakeDorisConfig {
        accepted_users: vec!["analyst".to_string()],
        ..Default::default()
    })
    .await;
    let proxy = start_proxy(&doris.addr, Arc::new(UnfilteredGate)).await;

    let mut client = RawClient::connect(&proxy).await.unwrap();
    client
        .send_handshake_response("intruder", CLIENT_SCRAMBLE, None)
        .await
        .unwrap();
    let (_, reply) = client.read_packet().await.unwrap();
    assert_eq!(
        reply.first(),
        Some(&0xff),
        "expected an ERR packet for rejected credentials"
    );

    doris
        .wait_for("the backend to see the attempt", |s| {
            s.first().is_some_and(|s| !s.username.is_empty())
        })
        .await;
    assert!(
        !doris.sessions()[0].authenticated,
        "the backend never authenticated this user"
    );

    // The session must not reach a state where statements can be issued: either
    // the socket is already closed, or nothing gets through it.
    let _ = client.send_command(0x03, b"SELECT 1").await;
    assert!(
        client.read_packet().await.is_err(),
        "the proxy kept the session alive after Doris rejected it"
    );
    assert!(
        doris.sessions()[0].queries.is_empty(),
        "a statement reached the backend on an unauthenticated session"
    );
}

/// Task 3.4 / 3.8 — the identity policy is keyed on comes from Doris, not from
/// what the client typed.
#[tokio::test]
async fn policy_is_keyed_on_the_backend_authenticated_user() {
    let mut overrides = HashMap::new();
    // The client will claim `admin`; the frontend resolves the session to
    // `analyst`, as host-pattern account matching can.
    overrides.insert("admin".to_string(), "'analyst'@'%'".to_string());

    let doris = FakeDoris::start(FakeDorisConfig {
        accepted_users: vec!["admin".to_string()],
        current_user_overrides: overrides,
        accepted_databases: None,
        ..Default::default()
    })
    .await;

    let (gate, seen) = RecordingGate::forwarding();
    let proxy = start_proxy(&doris.addr, gate).await;

    let mut client = RawClient::authenticate(&proxy, "admin", CLIENT_SCRAMBLE)
        .await
        .unwrap();
    client.query("SELECT 1").await.unwrap();

    let recorded = seen.lock().unwrap().clone();
    let (identity, _, sql) = recorded.first().expect("gate was never consulted");
    assert_eq!(sql, "SELECT 1");
    assert_eq!(
        identity.authenticated_user(),
        "analyst",
        "policy would have been chosen by the name the client typed"
    );
    assert_eq!(identity.claimed_user(), "admin");
}

/// Task 3.5 — no backend, no session.
#[tokio::test]
async fn session_is_refused_when_the_backend_is_unavailable() {
    // Bind and immediately drop, so the address is certainly not listening.
    let dead = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let dead_addr = dead.local_addr().unwrap().to_string();
    drop(dead);

    let proxy = start_proxy(&dead_addr, Arc::new(UnfilteredGate)).await;

    let body = RawClient::connect_expecting_refusal(&proxy).await.unwrap();
    assert_eq!(
        body.first(),
        Some(&0xff),
        "expected an ERR packet in place of a handshake, got {body:?}"
    );
    assert!(
        err_message(&body).contains("Doris frontend"),
        "unexpected refusal message: {}",
        err_message(&body)
    );
}

/// A frontend requiring an auth plugin the proxy cannot relay is refused, and
/// the refusal must not read as a credential failure.
///
/// The distinction matters operationally: no password would work here, and the
/// fix is to change the Doris account's plugin, not to try again. Milestone 1
/// reached the same conclusion (`discussions/milestone-1/02-proxy-scope.md` §C2).
#[tokio::test]
async fn an_unrelayable_auth_plugin_is_refused_as_a_proxy_limitation() {
    let doris = FakeDoris::start(FakeDorisConfig {
        auth_plugin: "caching_sha2_password".to_string(),
        ..Default::default()
    })
    .await;
    let proxy = start_proxy(&doris.addr, Arc::new(UnfilteredGate)).await;

    let body = RawClient::connect_expecting_refusal(&proxy).await.unwrap();
    assert_eq!(
        body.first(),
        Some(&0xff),
        "expected an ERR packet in place of a handshake"
    );

    let message = err_message(&body);
    assert!(
        message.contains("caching_sha2_password") && message.contains("proxy limitation"),
        "refusal does not identify itself as a proxy limitation: {message}"
    );
    assert_ne!(
        err_sqlstate(&body),
        "28000",
        "an access-denied SQLSTATE would misreport this as a credential problem"
    );
}

/// Task 3.6 — the backend connection does not outlive the client session.
#[tokio::test]
async fn client_disconnect_releases_the_backend_connection() {
    let doris = FakeDoris::start(FakeDorisConfig::default()).await;
    let proxy = start_proxy(&doris.addr, Arc::new(UnfilteredGate)).await;

    let client = RawClient::authenticate(&proxy, "analyst", CLIENT_SCRAMBLE)
        .await
        .unwrap();
    doris
        .wait_for("the backend to authenticate", |s| {
            s.first().is_some_and(|s| s.authenticated)
        })
        .await;
    assert!(!doris.sessions()[0].closed);

    drop(client);

    doris
        .wait_for("the backend connection to close", |s| {
            s.first().is_some_and(|s| s.closed)
        })
        .await;
}

/// Task 6.9 / design D4 — prepared statements are refused, and the refusal never
/// reaches the backend.
#[tokio::test]
async fn prepared_statements_are_refused_without_reaching_the_backend() {
    let doris = FakeDoris::start(FakeDorisConfig::default()).await;
    let proxy = start_proxy(&doris.addr, Arc::new(UnfilteredGate)).await;

    let mut client = RawClient::authenticate(&proxy, "analyst", CLIENT_SCRAMBLE)
        .await
        .unwrap();

    // COM_STMT_PREPARE
    client
        .send_command(0x16, b"SELECT * FROM t WHERE id = ?")
        .await
        .unwrap();
    let (_, reply) = client.read_packet().await.unwrap();

    assert_eq!(reply.first(), Some(&0xff), "expected an ERR packet");
    assert_eq!(
        err_sqlstate(&reply),
        "42000",
        "design D5 mandates SQLSTATE 42000"
    );
    assert!(
        err_message(&reply).contains("prepared statements are not supported"),
        "unexpected message: {}",
        err_message(&reply)
    );

    doris
        .wait_for("the backend to settle", |s| {
            s.first().is_some_and(|s| s.authenticated)
        })
        .await;
    // The only statement the proxy issues on its own account is the identity
    // lookup; the prepare must not have been forwarded.
    assert_eq!(
        doris.sessions()[0].queries,
        vec!["SELECT CURRENT_USER()".to_string()],
        "COM_STMT_PREPARE reached the backend"
    );
}

/// Task 3.7 — the current database follows what the backend accepted.
#[tokio::test]
async fn current_database_follows_the_backend_accepted_change() {
    let doris = FakeDoris::start(FakeDorisConfig {
        accepted_users: vec!["analyst".to_string()],
        current_user_overrides: HashMap::new(),
        accepted_databases: Some(vec!["sales".to_string()]),
        ..Default::default()
    })
    .await;

    let (gate, seen) = RecordingGate::forwarding();
    let proxy = start_proxy(&doris.addr, gate).await;

    let mut client = RawClient::authenticate(&proxy, "analyst", CLIENT_SCRAMBLE)
        .await
        .unwrap();

    // Before any USE, there is no current database to resolve names against.
    client.query("SELECT 1").await.unwrap();
    assert_eq!(seen.lock().unwrap()[0].1, None);

    // COM_INIT_DB
    client.send_command(0x02, b"sales").await.unwrap();
    let (_, reply) = client.read_packet().await.unwrap();
    assert_eq!(reply.first(), Some(&0x00), "expected the backend to accept");

    client.query("SELECT 2").await.unwrap();
    assert_eq!(
        seen.lock().unwrap()[1].1.as_deref(),
        Some("sales"),
        "the session did not track the database change"
    );

    // A database the backend refuses must not move the proxy's idea of where it
    // is: an unqualified table name would then resolve against the wrong schema.
    client.send_command(0x02, b"staging").await.unwrap();
    let (_, reply) = client.read_packet().await.unwrap();
    assert_eq!(
        reply.first(),
        Some(&0xff),
        "expected the backend's rejection"
    );

    client.query("SELECT 3").await.unwrap();
    assert_eq!(
        seen.lock().unwrap()[2].1.as_deref(),
        Some("sales"),
        "a rejected USE moved the session's current database"
    );
}

/// Task 3.7 — a database named in the handshake (`CLIENT_CONNECT_WITH_DB`) is
/// tracked too, not only one selected later.
///
/// `None` has to keep meaning "genuinely unset": `policy-config` returns a
/// distinct `Unresolvable` outcome for an unqualified table reference with no
/// current database, and the rewriter refuses on it. A placeholder that compared
/// equal to a real database name would turn that refusal into a wrong match.
#[tokio::test]
async fn a_database_named_in_the_handshake_becomes_the_current_database() {
    let doris = FakeDoris::start(FakeDorisConfig::default()).await;
    let (gate, seen) = RecordingGate::forwarding();
    let proxy = start_proxy(&doris.addr, gate).await;

    let mut client = RawClient::connect(&proxy).await.unwrap();
    client
        .send_handshake_response("analyst", CLIENT_SCRAMBLE, Some("sales"))
        .await
        .unwrap();
    let (_, reply) = client.read_packet().await.unwrap();
    assert_eq!(reply.first(), Some(&0x00), "expected the session to open");

    client.query("SELECT 1").await.unwrap();
    assert_eq!(
        seen.lock().unwrap()[0].1.as_deref(),
        Some("sales"),
        "the database named at handshake time was not tracked"
    );

    // The proxy relays it as COM_INIT_DB rather than in its own handshake
    // response, so there is exactly one code path that sets this.
    doris
        .wait_for("the backend to receive the database", |s| {
            s.first().is_some_and(|s| !s.init_dbs.is_empty())
        })
        .await;
    assert_eq!(doris.sessions()[0].init_dbs, vec!["sales".to_string()]);
}

/// A session that never named a database has `None`, not an empty string.
#[tokio::test]
async fn a_session_with_no_database_reports_none_not_a_placeholder() {
    let doris = FakeDoris::start(FakeDorisConfig::default()).await;
    let (gate, seen) = RecordingGate::forwarding();
    let proxy = start_proxy(&doris.addr, gate).await;

    let mut client = RawClient::authenticate(&proxy, "analyst", CLIENT_SCRAMBLE)
        .await
        .unwrap();
    client.query("SELECT 1").await.unwrap();

    assert_eq!(
        seen.lock().unwrap()[0].1,
        None,
        "an unset database must be None, so `Unresolvable` stays reachable"
    );
}

/// A `USE` written as a statement takes the same path as `COM_INIT_DB`.
#[tokio::test]
async fn use_statement_updates_the_current_database() {
    let doris = FakeDoris::start(FakeDorisConfig::default()).await;
    let (gate, seen) = RecordingGate::forwarding();
    let proxy = start_proxy(&doris.addr, gate).await;

    let mut client = RawClient::authenticate(&proxy, "analyst", CLIENT_SCRAMBLE)
        .await
        .unwrap();
    let (_, reply) = client.query("USE sales").await.unwrap();
    assert_eq!(reply.first(), Some(&0x00));

    client.query("SELECT 1").await.unwrap();
    assert_eq!(seen.lock().unwrap()[0].1.as_deref(), Some("sales"));
}

/// The fail-closed default: a build whose analysis stage is not wired forwards
/// nothing.
#[tokio::test]
async fn an_unwired_proxy_refuses_statements_rather_than_forwarding_them() {
    let doris = FakeDoris::start(FakeDorisConfig::default()).await;
    let proxy = start_proxy(&doris.addr, Arc::new(RefuseAllGate)).await;

    let mut client = RawClient::authenticate(&proxy, "analyst", CLIENT_SCRAMBLE)
        .await
        .unwrap();
    let (_, reply) = client.query("SELECT 1").await.unwrap();

    assert_eq!(reply.first(), Some(&0xff));
    assert_eq!(err_sqlstate(&reply), "42000");
    assert_eq!(
        doris.sessions()[0].queries,
        vec!["SELECT CURRENT_USER()".to_string()],
        "an unexamined statement reached the backend"
    );
}

/// `connection-routing` — the client sees Doris's error, not a proxy substitute.
#[tokio::test]
async fn backend_errors_reach_the_client() {
    let doris = FakeDoris::start(FakeDorisConfig {
        accepted_users: vec!["analyst".to_string()],
        current_user_overrides: HashMap::new(),
        accepted_databases: Some(vec!["sales".to_string()]),
        ..Default::default()
    })
    .await;
    let proxy = start_proxy(&doris.addr, Arc::new(UnfilteredGate)).await;

    let mut client = RawClient::authenticate(&proxy, "analyst", CLIENT_SCRAMBLE)
        .await
        .unwrap();
    client.send_command(0x02, b"nonexistent").await.unwrap();
    let (_, reply) = client.read_packet().await.unwrap();

    assert_eq!(reply.first(), Some(&0xff));
    assert!(
        err_message(&reply).contains("Unknown database 'nonexistent'"),
        "the backend's own message did not reach the client: {}",
        err_message(&reply)
    );
}

/// Statements sent before the handshake completes never reach the backend.
#[tokio::test]
async fn statements_before_authentication_do_not_reach_the_backend() {
    let doris = FakeDoris::start(FakeDorisConfig::default()).await;
    let proxy = start_proxy(&doris.addr, Arc::new(UnfilteredGate)).await;

    let mut client = RawClient::connect(&proxy).await.unwrap();
    // A COM_QUERY where the handshake response belongs.
    client.send_command(0x03, b"SELECT 1").await.unwrap();
    let _ = client.read_packet().await;

    doris
        .wait_for("the backend connection to be dropped", |s| {
            s.first().is_some_and(|s| s.closed)
        })
        .await;
    assert!(!doris.sessions()[0].authenticated);
    assert!(
        doris.sessions()[0].queries.is_empty(),
        "a statement reached the backend before authentication"
    );
}
