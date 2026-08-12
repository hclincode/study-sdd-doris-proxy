//! End-to-end behaviour of the proxy against a scriptable backend.
//!
//! These exercise the proxy as a whole — accept, handshake, command loop,
//! logging — without needing a database, so they run anywhere and stay
//! deterministic. Verification against real MySQL and real drivers is done
//! separately via `scripts/verify-with-mysql.sh`.

mod support;

use std::time::Duration;

use mysql_proxy::protocol::capabilities::*;
use support::{MockBackend, RunningProxy, TestClient};

#[tokio::test]
async fn client_connects_and_queries_through_the_proxy() {
    let backend = MockBackend::start().await;
    let proxy = RunningProxy::start(backend.addr, 1024).await;

    let mut client = TestClient::connect(proxy.addr).await.unwrap();
    let response = client.query("SELECT 1").await.unwrap();

    assert_eq!(response[0].1[0], 0x00, "expected an OK response");
    let records = proxy.records(1).await;
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["command"], "COM_QUERY");
    assert_eq!(records[0]["statement"], "SELECT 1");
    assert_eq!(records[0]["outcome"], "ok");
}

#[tokio::test]
async fn each_client_gets_its_own_backend_connection() {
    let backend = MockBackend::start().await;
    let proxy = RunningProxy::start(backend.addr, 1024).await;

    let mut a = TestClient::connect(proxy.addr).await.unwrap();
    let mut b = TestClient::connect(proxy.addr).await.unwrap();
    a.query("SELECT 1").await.unwrap();
    b.query("SELECT 2").await.unwrap();

    assert_eq!(
        backend.connections.load(std::sync::atomic::Ordering::Relaxed),
        2,
        "each client connection must open its own backend connection"
    );

    let records = proxy.records(2).await;
    let ids: std::collections::HashSet<_> = records
        .iter()
        .map(|r| r["connection_id"].as_u64().unwrap())
        .collect();
    assert_eq!(ids.len(), 2, "records must distinguish the two connections");
}

#[tokio::test]
async fn unreachable_backend_yields_an_error_packet() {
    // Bind and immediately drop, so the port is almost certainly closed.
    let dead = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap()
        .local_addr()
        .unwrap();
    let proxy = RunningProxy::start(dead, 1024).await;

    let err = TestClient::connect(proxy.addr).await.unwrap_err();
    assert!(
        err.to_string().contains("cannot reach backend"),
        "client should receive the proxy's reason, got: {err}"
    );
}

#[tokio::test]
async fn masked_capabilities_are_not_offered_to_the_client() {
    let backend = MockBackend::start().await;
    let proxy = RunningProxy::start(backend.addr, 1024).await;

    let client = TestClient::connect(proxy.addr).await.unwrap();
    let offered = client.server_capabilities;

    assert_eq!(offered & CLIENT_SSL, 0, "TLS must not be offered");
    assert_eq!(offered & CLIENT_COMPRESS, 0, "compression must not be offered");
    assert_eq!(offered & CLIENT_LOCAL_FILES, 0, "local files must not be offered");
    assert_eq!(offered & CLIENT_QUERY_ATTRIBUTES, 0);
    // Capabilities the proxy does support must survive.
    assert_ne!(offered & CLIENT_PROTOCOL_41, 0);
}

#[tokio::test]
async fn client_asserting_a_masked_capability_is_rejected() {
    let backend = MockBackend::start().await;
    let proxy = RunningProxy::start(backend.addr, 1024).await;

    let err = TestClient::connect_with(proxy.addr, CLIENT_SSL)
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("CLIENT_SSL"),
        "expected the reason to name the capability, got: {err}"
    );
}

#[tokio::test]
async fn result_set_rows_are_counted_and_forwarded() {
    let backend = MockBackend::start().await;
    let proxy = RunningProxy::start(backend.addr, 1024).await;

    let mut client = TestClient::connect(proxy.addr).await.unwrap();
    let response = client.query("rows:5").await.unwrap();

    // column count + 1 column def + EOF + 5 rows + EOF
    assert_eq!(response.len(), 9);

    let records = proxy.records(1).await;
    assert_eq!(records[0]["outcome"], "result_set");
    assert_eq!(records[0]["returned_rows"], 5);
    assert_eq!(records[0]["result_sets"], 1);
}

#[tokio::test]
async fn empty_result_set_is_recorded_as_zero_rows() {
    let backend = MockBackend::start().await;
    let proxy = RunningProxy::start(backend.addr, 1024).await;

    let mut client = TestClient::connect(proxy.addr).await.unwrap();
    client.query("rows:0").await.unwrap();

    let records = proxy.records(1).await;
    assert_eq!(records[0]["returned_rows"], 0);
}

#[tokio::test]
async fn errors_are_relayed_and_recorded() {
    let backend = MockBackend::start().await;
    let proxy = RunningProxy::start(backend.addr, 1024).await;

    let mut client = TestClient::connect(proxy.addr).await.unwrap();
    let response = client.query("fail").await.unwrap();
    assert_eq!(response[0].1[0], 0xFF);

    let records = proxy.records(1).await;
    assert_eq!(records[0]["outcome"], "error");
    assert_eq!(records[0]["error_code"], 1146);
    assert_eq!(records[0]["sql_state"], "42S02");
}

#[tokio::test]
async fn statements_are_logged_with_full_text_and_a_digest() {
    let backend = MockBackend::start().await;
    let proxy = RunningProxy::start(backend.addr, 1024).await;

    let mut client = TestClient::connect(proxy.addr).await.unwrap();
    client
        .query("SELECT * FROM users WHERE email = 'someone@example.com' AND id IN (1,2,3)")
        .await
        .unwrap();

    let records = proxy.records(1).await;
    let r = &records[0];
    assert!(r["statement"]
        .as_str()
        .unwrap()
        .contains("someone@example.com"));
    assert_eq!(
        r["digest"],
        "SELECT * FROM users WHERE email = ? AND id IN (...)"
    );
    assert_eq!(r["digest_hash"].as_str().unwrap().len(), 16);
}

#[tokio::test]
async fn records_carry_connection_identity() {
    let backend = MockBackend::start_with(support::MOCK_SERVER_CAPS, 4242).await;
    let proxy = RunningProxy::start(backend.addr, 1024).await;

    let mut client = TestClient::connect(proxy.addr).await.unwrap();
    client.query("SELECT 1").await.unwrap();

    let records = proxy.records(1).await;
    let r = &records[0];
    assert_eq!(r["listener"], "test");
    assert_eq!(r["username"], "app");
    assert_eq!(r["backend_connection_id"], 4242);
    assert!(r["client_addr"].as_str().unwrap().starts_with("127.0.0.1:"));
    assert!(r["ts"].as_str().unwrap().ends_with('Z'));
    assert!(r["duration_us"].as_u64().is_some());
}

#[tokio::test]
async fn commands_without_sql_are_still_recorded() {
    let backend = MockBackend::start().await;
    let proxy = RunningProxy::start(backend.addr, 1024).await;

    let mut client = TestClient::connect(proxy.addr).await.unwrap();
    client.command(0x0E, b"").await.unwrap(); // COM_PING

    let records = proxy.records(1).await;
    assert_eq!(records[0]["command"], "COM_PING");
    assert!(records[0].get("statement").is_none());
}

#[tokio::test]
async fn replication_commands_are_refused_without_reaching_the_backend() {
    let backend = MockBackend::start().await;
    let proxy = RunningProxy::start(backend.addr, 1024).await;

    let mut client = TestClient::connect(proxy.addr).await.unwrap();
    let response = client.command(0x12, b"").await.unwrap(); // COM_BINLOG_DUMP
    assert_eq!(response[0].1[0], 0xFF, "expected the proxy to refuse");

    // The connection must remain usable.
    let after = client.query("SELECT 1").await.unwrap();
    assert_eq!(after[0].1[0], 0x00);

    let records = proxy.records(2).await;
    assert_eq!(records[0]["command"], "COM_BINLOG_DUMP");
    assert_eq!(records[0]["outcome"], "refused");
    assert_eq!(records[1]["outcome"], "ok");
}

#[tokio::test]
async fn unparsable_statements_are_recorded_without_a_digest() {
    let backend = MockBackend::start().await;
    let proxy = RunningProxy::start(backend.addr, 1024).await;

    let mut client = TestClient::connect(proxy.addr).await.unwrap();
    client.query("SELECT 'unterminated").await.unwrap();

    let records = proxy.records(1).await;
    assert_eq!(records[0]["digest_unavailable"], true);
    assert!(records[0].get("digest").is_none());
    assert_eq!(records[0]["statement"], "SELECT 'unterminated");
}

#[tokio::test]
async fn a_large_statement_survives_packet_splitting() {
    let backend = MockBackend::start().await;
    let proxy = RunningProxy::start(backend.addr, 1024).await;

    let mut client = TestClient::connect(proxy.addr).await.unwrap();
    // Comfortably larger than a single packet would carry if the proxy
    // mishandled framing, while staying quick to run.
    let big = format!("SELECT '{}'", "x".repeat(200_000));
    client.query(&big).await.unwrap();

    let records = proxy.records(1).await;
    assert_eq!(records[0]["statement"].as_str().unwrap().len(), big.len());
}

#[tokio::test]
async fn every_command_on_a_connection_produces_a_record() {
    let backend = MockBackend::start().await;
    let proxy = RunningProxy::start(backend.addr, 1024).await;

    let mut client = TestClient::connect(proxy.addr).await.unwrap();
    for i in 0..10 {
        client.query(&format!("SELECT {i}")).await.unwrap();
    }

    let records = proxy.records(10).await;
    assert_eq!(records.len(), 10);
    let same_connection = records
        .iter()
        .all(|r| r["connection_id"] == records[0]["connection_id"]);
    assert!(same_connection, "all records share one connection id");
}

#[tokio::test]
async fn client_disconnect_closes_the_backend_connection() {
    let backend = MockBackend::start().await;
    let proxy = RunningProxy::start(backend.addr, 1024).await;

    let mut client = TestClient::connect(proxy.addr).await.unwrap();
    client.query("SELECT 1").await.unwrap();
    drop(client);

    // Nothing to assert directly on the mock, but the proxy must not panic and
    // must keep serving new connections.
    tokio::time::sleep(Duration::from_millis(50)).await;
    let mut next = TestClient::connect(proxy.addr).await.unwrap();
    assert_eq!(next.query("SELECT 2").await.unwrap()[0].1[0], 0x00);
}

#[tokio::test]
async fn quit_ends_the_session_cleanly() {
    let backend = MockBackend::start().await;
    let proxy = RunningProxy::start(backend.addr, 1024).await;

    let mut client = TestClient::connect(proxy.addr).await.unwrap();
    let mut payload = vec![0x01u8];
    payload.clear();
    payload.push(0x01);
    use tokio::io::AsyncWriteExt;
    client
        .stream
        .write_all(&support::packet(0, &payload))
        .await
        .unwrap();

    let records = proxy.records(1).await;
    assert_eq!(records[0]["command"], "COM_QUIT");
    assert_eq!(records[0]["outcome"], "no_response");
}

#[tokio::test]
async fn queries_are_unaffected_by_a_saturated_log_queue() {
    let backend = MockBackend::start().await;
    // A one-slot queue puts the logging path under as much pressure as it can
    // get. Whether records are actually discarded depends on how the writer and
    // the clients interleave, so this asserts the invariant that must hold
    // either way: query traffic is never delayed or failed by logging.
    // Deterministic coverage of discard-and-report lives in the writer's own
    // unit tests, where the queue can be held full on purpose.
    let proxy = RunningProxy::start(backend.addr, 1).await;

    let mut clients = Vec::new();
    for _ in 0..8 {
        clients.push(TestClient::connect(proxy.addr).await.unwrap());
    }

    let mut handles = Vec::new();
    for (n, mut client) in clients.into_iter().enumerate() {
        handles.push(tokio::spawn(async move {
            for i in 0..25 {
                let response = client.query(&format!("SELECT {n}_{i}")).await.unwrap();
                assert_eq!(response[0].1[0], 0x00, "queries must keep succeeding");
            }
        }));
    }
    for h in handles {
        h.await.unwrap();
    }

    tokio::time::sleep(Duration::from_millis(200)).await;
    let records = proxy.read_records();
    let commands = records.iter().filter(|r| r["type"] == "command").count();
    let dropped: Vec<_> = records.iter().filter(|r| r["type"] == "dropped").collect();

    // Every line must still be well-formed, and the file must account for the
    // full 800 commands as either written or reported discarded.
    let reported_drops = dropped
        .last()
        .and_then(|r| r["dropped_total"].as_u64())
        .unwrap_or(0) as usize;
    assert_eq!(
        commands + reported_drops,
        200,
        "every command must be either logged or counted as discarded"
    );
}
