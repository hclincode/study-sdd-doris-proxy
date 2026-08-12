//! End-to-end behaviour of row-filter injection.
//!
//! These assert what actually reached the backend, not only what the proxy
//! logged, so a rewrite that never left the process would fail here.

mod support;

use support::{MockBackend, RunningProxy, TestClient};

const TENANT: &str = "tenant_id = 7";

#[tokio::test]
async fn a_select_without_a_where_clause_gains_one() {
    let backend = MockBackend::start().await;
    let proxy = RunningProxy::start_with_filters(backend.addr, 1024, &[("orders", TENANT)]).await;

    let mut client = TestClient::connect(proxy.addr).await.unwrap();
    client.query("SELECT * FROM orders").await.unwrap();

    assert_eq!(
        backend.last_statement(),
        "SELECT * FROM orders WHERE (tenant_id = 7)"
    );

    let r = &proxy.records(1).await[0];
    assert_eq!(r["rewritten"], true);
    assert_eq!(r["filter_table"], "orders");
    assert_eq!(
        r["forwarded_statement"],
        "SELECT * FROM orders WHERE (tenant_id = 7)"
    );
    assert_eq!(
        r["statement"], "SELECT * FROM orders",
        "the client's own text must be preserved"
    );
}

#[tokio::test]
async fn an_existing_where_clause_is_parenthesized_and_combined() {
    let backend = MockBackend::start().await;
    let proxy = RunningProxy::start_with_filters(backend.addr, 1024, &[("orders", TENANT)]).await;

    let mut client = TestClient::connect(proxy.addr).await.unwrap();
    client
        .query("SELECT * FROM orders WHERE id = 1")
        .await
        .unwrap();

    assert_eq!(
        backend.last_statement(),
        "SELECT * FROM orders WHERE ( id = 1) AND (tenant_id = 7)"
    );
}

#[tokio::test]
async fn an_or_condition_is_parenthesized_so_the_filter_still_narrows() {
    let backend = MockBackend::start().await;
    let proxy = RunningProxy::start_with_filters(backend.addr, 1024, &[("orders", TENANT)]).await;

    let mut client = TestClient::connect(proxy.addr).await.unwrap();
    client
        .query("SELECT * FROM orders WHERE a = 1 OR b = 2")
        .await
        .unwrap();

    // Without the parentheses this would bind as `a = 1 OR (b = 2 AND ...)`.
    assert_eq!(
        backend.last_statement(),
        "SELECT * FROM orders WHERE ( a = 1 OR b = 2) AND (tenant_id = 7)"
    );
}

#[tokio::test]
async fn trailing_clauses_survive_the_injection() {
    let backend = MockBackend::start().await;
    let proxy = RunningProxy::start_with_filters(backend.addr, 1024, &[("orders", TENANT)]).await;

    let mut client = TestClient::connect(proxy.addr).await.unwrap();
    client
        .query("SELECT * FROM orders ORDER BY id LIMIT 10")
        .await
        .unwrap();

    assert_eq!(
        backend.last_statement(),
        "SELECT * FROM orders WHERE (tenant_id = 7) ORDER BY id LIMIT 10"
    );
}

#[tokio::test]
async fn prepared_statements_are_rewritten_at_prepare_time() {
    let backend = MockBackend::start().await;
    let proxy = RunningProxy::start_with_filters(backend.addr, 1024, &[("orders", TENANT)]).await;

    let mut client = TestClient::connect(proxy.addr).await.unwrap();
    client
        .command(0x16, b"SELECT * FROM orders WHERE id = ?")
        .await
        .unwrap();

    assert_eq!(
        backend.last_statement(),
        "SELECT * FROM orders WHERE ( id = ?) AND (tenant_id = 7)"
    );

    let r = &proxy.records(1).await[0];
    assert_eq!(r["command"], "COM_STMT_PREPARE");
    assert_eq!(r["rewritten"], true);
}

#[tokio::test]
async fn each_unsupported_construct_is_forwarded_unchanged_with_a_reason() {
    let cases: &[(&str, &str)] = &[
        ("SELECT * FROM orders o JOIN items i ON o.id = i.oid", "multiple_tables"),
        ("SELECT * FROM orders, items", "multiple_tables"),
        ("SELECT * FROM orders WHERE id IN (SELECT oid FROM items)", "unsupported_structure"),
        ("SELECT * FROM orders UNION SELECT * FROM archive", "unsupported_structure"),
        ("SELECT 1; SELECT * FROM orders", "multiple_statements"),
        ("SELECT * FROM orders WHERE a = 'unterminated", "tokenize_failed"),
        ("UPDATE orders SET a = 1", "not_select"),
    ];

    for (sql, reason) in cases {
        let backend = MockBackend::start().await;
        let proxy =
            RunningProxy::start_with_filters(backend.addr, 1024, &[("orders", TENANT)]).await;
        let mut client = TestClient::connect(proxy.addr).await.unwrap();
        client.query(sql).await.unwrap();

        assert_eq!(
            backend.last_statement(),
            *sql,
            "must reach the backend unchanged: {sql}"
        );
        let r = &proxy.records(1).await[0];
        assert_eq!(r["filter_skipped"], *reason, "for {sql}");
        assert!(r.get("rewritten").is_none(), "for {sql}");
        assert!(r.get("forwarded_statement").is_none(), "for {sql}");
    }
}

#[tokio::test]
async fn a_join_over_a_filtered_table_returns_unfiltered_rows() {
    // The specified best-effort behaviour: the proxy declines rather than
    // rejecting, so the client does receive rows the predicate would exclude.
    let backend = MockBackend::start().await;
    let proxy = RunningProxy::start_with_filters(backend.addr, 1024, &[("orders", TENANT)]).await;

    let mut client = TestClient::connect(proxy.addr).await.unwrap();
    let response = client
        .query("SELECT * FROM orders o JOIN items i ON o.id = i.oid")
        .await
        .unwrap();

    assert_eq!(response[0].1[0], 0x00, "the statement must still execute");
    assert_eq!(
        proxy.records(1).await[0]["filter_skipped"],
        "multiple_tables"
    );
}

#[tokio::test]
async fn a_table_with_no_rule_is_untouched_and_records_no_reason() {
    let backend = MockBackend::start().await;
    let proxy = RunningProxy::start_with_filters(backend.addr, 1024, &[("orders", TENANT)]).await;

    let mut client = TestClient::connect(proxy.addr).await.unwrap();
    client.query("SELECT * FROM currencies").await.unwrap();

    assert_eq!(backend.last_statement(), "SELECT * FROM currencies");
    let r = &proxy.records(1).await[0];
    assert!(r.get("rewritten").is_none());
    assert!(
        r.get("filter_skipped").is_none(),
        "traffic nobody meant to filter must not count as missing coverage"
    );
}

#[tokio::test]
async fn a_skip_on_unruled_tables_is_not_counted_either() {
    let backend = MockBackend::start().await;
    let proxy = RunningProxy::start_with_filters(backend.addr, 1024, &[("orders", TENANT)]).await;

    let mut client = TestClient::connect(proxy.addr).await.unwrap();
    client
        .query("SELECT * FROM currencies c JOIN regions r ON c.r = r.id")
        .await
        .unwrap();

    assert!(proxy.records(1).await[0].get("filter_skipped").is_none());
}

#[tokio::test]
async fn a_listener_without_rules_behaves_exactly_as_before() {
    let backend = MockBackend::start().await;
    let proxy = RunningProxy::start(backend.addr, 1024).await;

    let mut client = TestClient::connect(proxy.addr).await.unwrap();
    client.query("SELECT * FROM orders").await.unwrap();

    assert_eq!(backend.last_statement(), "SELECT * FROM orders");
    let r = &proxy.records(1).await[0];
    for absent in ["rewritten", "forwarded_statement", "filter_table", "filter_skipped"] {
        assert!(r.get(absent).is_none(), "{absent} must be absent");
    }
}

#[tokio::test]
async fn two_listeners_can_filter_the_same_table_differently() {
    let backend = MockBackend::start().await;
    let seven =
        RunningProxy::start_with_filters(backend.addr, 1024, &[("orders", "tenant_id = 7")]).await;
    let eight =
        RunningProxy::start_with_filters(backend.addr, 1024, &[("orders", "tenant_id = 8")]).await;

    TestClient::connect(seven.addr)
        .await
        .unwrap()
        .query("SELECT * FROM orders")
        .await
        .unwrap();
    let first = backend.last_statement();

    TestClient::connect(eight.addr)
        .await
        .unwrap()
        .query("SELECT * FROM orders")
        .await
        .unwrap();
    let second = backend.last_statement();

    assert_eq!(first, "SELECT * FROM orders WHERE (tenant_id = 7)");
    assert_eq!(second, "SELECT * FROM orders WHERE (tenant_id = 8)");
}

#[tokio::test]
async fn the_digest_describes_the_clients_statement_not_the_rewrite() {
    let backend = MockBackend::start().await;
    let filtered =
        RunningProxy::start_with_filters(backend.addr, 1024, &[("orders", TENANT)]).await;
    let plain = RunningProxy::start(backend.addr, 1024).await;

    TestClient::connect(filtered.addr)
        .await
        .unwrap()
        .query("SELECT * FROM orders WHERE id = 1")
        .await
        .unwrap();
    TestClient::connect(plain.addr)
        .await
        .unwrap()
        .query("SELECT * FROM orders WHERE id = 1")
        .await
        .unwrap();

    let a = &filtered.records(1).await[0];
    let b = &plain.records(1).await[0];
    assert_eq!(
        a["digest_hash"], b["digest_hash"],
        "grouping must not depend on whether a rule happened to apply"
    );
}

#[tokio::test]
async fn comments_and_aliases_survive_a_rewrite() {
    let backend = MockBackend::start().await;
    let proxy = RunningProxy::start_with_filters(backend.addr, 1024, &[("orders", TENANT)]).await;

    let mut client = TestClient::connect(proxy.addr).await.unwrap();
    client
        .query("SELECT /*+ HINT(x) */ o.* FROM orders o -- trailing")
        .await
        .unwrap();

    assert_eq!(
        backend.last_statement(),
        "SELECT /*+ HINT(x) */ o.* FROM orders o WHERE (tenant_id = 7) -- trailing"
    );
}
