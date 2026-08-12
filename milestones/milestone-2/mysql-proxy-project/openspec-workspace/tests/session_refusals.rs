//! Tasks 6.11–6.13 — the refusal path, end to end through a real proxy.
//!
//! The rewrite tests assert that `rewrite_statement` returns an `Err`. That is
//! not the same claim as the one the spec makes. A refusal is only a control if
//! the statement **did not reach Doris**, and only `rewrite_statement` returning
//! an error does not prove the session layer honoured it. So every test here
//! goes through the whole proxy — client socket, gate, backend socket — and
//! asserts against the fake frontend's record of what it was actually asked to
//! run.
//!
//! The fake frontend does no filtering of its own. If a statement reaches it, it
//! runs; so "the backend saw nothing" is a real assertion rather than a
//! restatement of the client-visible error.

use std::sync::Arc;

use doris_row_filter_proxy::policy::PolicySet;
use doris_row_filter_proxy::session::{PolicyGate, ProxyServer};
use tokio::net::TcpListener;

#[path = "session_fake_doris.rs"]
mod fake;

use fake::{err_message, err_sqlstate, FakeDoris, FakeDorisConfig, RawClient};

const CLIENT_SCRAMBLE: &[u8; 20] = b"CLIENT-SCRAMBLE-1234";

/// `analyst` is restricted on `sales.orders`; `auditor`'s policy exists only so
/// task 6.13 can check that a refusal shown to `analyst` never mentions it.
const POLICY_FILE: &str = r#"
[[policy]]
user = "analyst"
database = "sales"
table = "orders"
column = "region"
permitted_values = ["APAC", "EMEA"]

[[policy]]
user = "auditor"
database = "sales"
table = "salaries"
column = "department"
permitted_values = ["FINANCE"]
"#;

/// The only statement the proxy issues on its own account, when resolving the
/// session identity. Anything else in the backend's record was forwarded.
const IDENTITY_QUERY: &str = "SELECT CURRENT_USER()";

struct Harness {
    doris: FakeDoris,
    proxy: String,
}

impl Harness {
    async fn start() -> Self {
        let doris = FakeDoris::start(FakeDorisConfig {
            accepted_users: vec!["analyst".to_string(), "reporting".to_string()],
            ..Default::default()
        })
        .await;

        let policies = PolicySet::from_toml_str(POLICY_FILE, "session_refusals.rs").unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy = listener.local_addr().unwrap().to_string();
        let server = Arc::new(ProxyServer::new(
            doris.addr.clone(),
            Arc::new(PolicyGate::new(policies)),
        ));
        tokio::spawn(async move {
            let _ = server.run(listener).await;
        });

        Harness { doris, proxy }
    }

    async fn connect(&self, user: &str) -> RawClient {
        RawClient::authenticate(&self.proxy, user, CLIENT_SCRAMBLE)
            .await
            .unwrap()
    }

    /// Every statement this session's backend connection was asked to run,
    /// excluding the proxy's own identity lookup.
    fn forwarded_to_backend(&self) -> Vec<String> {
        self.doris
            .sessions()
            .into_iter()
            .flat_map(|session| session.queries)
            .filter(|sql| sql != IDENTITY_QUERY)
            .collect()
    }
}

/// Task 6.11 and 6.12 together, over every rejection category.
///
/// One assertion is the SQLSTATE, which design D5 fixes at `42000` so drivers
/// see a consistent class. The other is that the backend's record is empty —
/// the assertion the spec actually cares about.
#[tokio::test]
async fn every_refusal_category_is_42000_and_never_reaches_the_backend() {
    let cases: &[(&str, &str)] = &[
        // Unparseable, for a user with a policy (task 6.1, design D6).
        ("unparseable", "SELCT * FROM sales.orders"),
        // A shape the allowlist walk cannot analyse (task 6.3).
        (
            "unsupported shape",
            "SELECT * FROM sales.orders LATERAL VIEW explode(items) t AS item",
        ),
        // Writes against a policy table (tasks 6.4–6.7).
        (
            "insert",
            "INSERT INTO sales.orders (id, region) VALUES (1, 'AMER')",
        ),
        (
            "insert select",
            "INSERT INTO sales.orders SELECT * FROM sales.orders",
        ),
        (
            "update",
            "UPDATE sales.orders SET region = 'AMER' WHERE id = 1",
        ),
        ("delete", "DELETE FROM sales.orders WHERE id = 1"),
        (
            "replace",
            "REPLACE INTO sales.orders (id, region) VALUES (1, 'AMER')",
        ),
        // Multi-statement, including comment smuggling (tasks 6.10, 7.6).
        ("multi statement", "SELECT * FROM sales.orders; SELECT 1"),
        (
            "comment-smuggled second statement",
            "SELECT * FROM sales.orders /* harmless */ ; SELECT 1",
        ),
    ];

    for (label, sql) in cases {
        let harness = Harness::start().await;
        let mut client = harness.connect("analyst").await;

        let (_, reply) = client.query(sql).await.unwrap();
        assert_eq!(
            reply.first(),
            Some(&0xff),
            "{label}: expected an ERR packet for {sql:?}"
        );
        assert_eq!(
            err_sqlstate(&reply),
            "42000",
            "{label}: design D5 fixes the SQLSTATE of every refusal"
        );

        harness
            .doris
            .wait_for("the backend to settle", |sessions| {
                sessions.first().is_some_and(|s| s.authenticated)
            })
            .await;
        assert_eq!(
            harness.forwarded_to_backend(),
            Vec::<String>::new(),
            "{label}: {sql:?} reached the backend despite being refused"
        );
    }
}

/// The control for the test above.
///
/// Without it, "the backend saw nothing" could be passing because nothing ever
/// reaches the backend — a broken forward path would look like perfect
/// enforcement. Here the statement is admitted, and it arrives **rewritten**.
#[tokio::test]
async fn an_admitted_statement_reaches_the_backend_constrained() {
    let harness = Harness::start().await;
    let mut client = harness.connect("analyst").await;

    client.query("SELECT * FROM sales.orders").await.unwrap();

    harness
        .doris
        .wait_for("the statement to arrive", |sessions| {
            sessions.first().is_some_and(|s| s.queries.len() >= 2)
        })
        .await;

    assert_eq!(
        harness.forwarded_to_backend(),
        vec![
            "SELECT * FROM (SELECT * FROM sales.orders WHERE `region` IN ('APAC', 'EMEA')) \
             AS orders"
                .to_string()
        ],
        "the statement Doris ran was not the constrained one"
    );
}

/// Design D6: a user with no policy at all has nothing to protect, so a
/// statement the proxy cannot analyse is forwarded as written (task 6.2).
///
/// The counterpart of the unparseable case above, and the reason that case is
/// about the *user* rather than about the SQL.
#[tokio::test]
async fn an_unanalysable_statement_from_an_unrestricted_user_is_forwarded() {
    let harness = Harness::start().await;
    let mut client = harness.connect("reporting").await;

    let sql = "SELECT * FROM sales.orders LATERAL VIEW explode(items) t AS item";
    client.query(sql).await.unwrap();

    harness
        .doris
        .wait_for("the statement to arrive", |sessions| {
            sessions.first().is_some_and(|s| s.queries.len() >= 2)
        })
        .await;
    assert_eq!(
        harness.forwarded_to_backend(),
        vec![sql.to_string()],
        "a user with no policy should have this forwarded unmodified"
    );
}

/// Task 6.13 — a refusal message discloses nothing about any policy.
///
/// The message is a channel an attacker can query at will: they choose the
/// statement, so they choose which refusal to read. If it named the table it
/// declined to expose, or a permitted value, the refusal itself would enumerate
/// the policy set (design D5).
#[tokio::test]
async fn refusal_messages_disclose_no_policy_contents() {
    let harness = Harness::start().await;
    let mut client = harness.connect("analyst").await;

    for sql in [
        "SELCT * FROM sales.orders",
        "SELECT * FROM sales.orders LATERAL VIEW explode(items) t AS item",
        "UPDATE sales.orders SET region = 'AMER'",
        "SELECT * FROM sales.orders; SELECT 1",
    ] {
        let (_, reply) = client.query(sql).await.unwrap();
        assert_eq!(reply.first(), Some(&0xff), "{sql:?} should be refused");
        let message = err_message(&reply);

        // Nothing from this user's policy, and nothing from anyone else's.
        for secret in [
            "sales",
            "orders",
            "region",
            "APAC",
            "EMEA", // analyst's
            "auditor",
            "salaries",
            "department",
            "FINANCE", // auditor's
        ] {
            assert!(
                !message.contains(secret),
                "refusal for {sql:?} leaks {secret:?}: {message}"
            );
        }
    }
}

/// Design D5 again, and the risk `design.md` names explicitly: an operator must
/// be able to tell a compatibility gap from a policy denial, or every allowlist
/// rejection looks like a proxy bug.
#[tokio::test]
async fn a_compatibility_gap_reads_differently_from_a_policy_denial() {
    let harness = Harness::start().await;
    let mut client = harness.connect("analyst").await;

    let (_, unsupported) = client
        .query("SELECT * FROM sales.orders LATERAL VIEW explode(items) t AS item")
        .await
        .unwrap();
    let (_, denied) = client
        .query("UPDATE sales.orders SET region = 'AMER'")
        .await
        .unwrap();

    let unsupported = err_message(&unsupported);
    let denied = err_message(&denied);

    assert_ne!(
        unsupported, denied,
        "an unsupported shape and a policy denial must not be indistinguishable"
    );
    assert!(
        unsupported.contains("cannot analyse"),
        "unsupported shape should say the proxy could not analyse it: {unsupported}"
    );
    assert!(
        denied.contains("not permitted"),
        "a policy denial should say it is not permitted: {denied}"
    );
}

/// Task 3.7 meets `policy-config`: with no current database, an unqualified
/// reference cannot be resolved, and the statement must not reach Doris.
///
/// This is the bypass the `PolicyLookup` seam could not express. Asserted here
/// at the level that matters — Doris was never asked to run it.
#[tokio::test]
async fn an_unqualified_reference_with_no_current_database_never_reaches_the_backend() {
    let harness = Harness::start().await;
    let mut client = harness.connect("analyst").await;

    let (_, reply) = client.query("SELECT * FROM orders").await.unwrap();
    assert_eq!(reply.first(), Some(&0xff), "expected a refusal");
    assert_eq!(err_sqlstate(&reply), "42000");

    assert_eq!(
        harness.forwarded_to_backend(),
        Vec::<String>::new(),
        "an unresolvable reference reached Doris unconstrained"
    );

    // Once the session has a database the same reference resolves, and is
    // constrained rather than refused — so the refusal was about resolution.
    client.send_command(0x02, b"sales").await.unwrap();
    let (_, reply) = client.read_packet().await.unwrap();
    assert_eq!(reply.first(), Some(&0x00), "expected USE to succeed");

    client.query("SELECT * FROM orders").await.unwrap();
    assert_eq!(
        harness.forwarded_to_backend(),
        vec![
            "SELECT * FROM (SELECT * FROM orders WHERE `region` IN ('APAC', 'EMEA')) AS orders"
                .to_string()
        ]
    );
}

/// Policies are per user. `auditor`'s policy on `sales.salaries` constrains
/// `auditor`; for `analyst` that table has no policy and passes through
/// untouched.
///
/// Written as a live end-to-end check rather than a unit test on `lookup`
/// because the identity the gate keys on is supplied by the session layer, and
/// this asserts the whole chain agrees on who is asking.
#[tokio::test]
async fn another_users_policy_does_not_constrain_this_user() {
    let harness = Harness::start().await;
    let mut client = harness.connect("analyst").await;

    let sql = "SELECT * FROM sales.salaries";
    let (_, reply) = client.query(sql).await.unwrap();
    assert_eq!(
        reply.first(),
        Some(&0x01),
        "expected a result set, not a refusal: {}",
        String::from_utf8_lossy(&reply)
    );

    harness
        .doris
        .wait_for("the statement to arrive", |sessions| {
            sessions.first().is_some_and(|s| s.queries.len() >= 2)
        })
        .await;
    assert_eq!(
        harness.forwarded_to_backend(),
        vec![sql.to_string()],
        "another user's policy was applied to this session"
    );
}

/// The assembled product: the real binary, its own arguments, its own policy
/// file, serving a real session.
///
/// Every other test here builds `ProxyServer` in-process, so none of them would
/// notice if `main.rs` wired the wrong gate, loaded the wrong file, or passed
/// the arguments in the wrong order. This is the only test that runs what
/// actually ships.
#[tokio::test]
async fn the_binary_serves_a_constrained_session_end_to_end() {
    use std::io::Write;
    use std::process::{Command, Stdio};
    use std::time::Duration;

    let doris = FakeDoris::start(FakeDorisConfig {
        accepted_users: vec!["analyst".to_string()],
        ..Default::default()
    })
    .await;

    let policy_path = std::env::temp_dir().join(format!(
        "doris-proxy-e2e-{}-{:?}.toml",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::File::create(&policy_path)
        .unwrap()
        .write_all(POLICY_FILE.as_bytes())
        .unwrap();

    // Bind and release to find an address nothing is using.
    let listen = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let listen_addr = listen.local_addr().unwrap().to_string();
    drop(listen);

    let mut child = Command::new(env!("CARGO_BIN_EXE_doris-row-filter-proxy"))
        .arg("--policy")
        .arg(&policy_path)
        .arg("--listen")
        .arg(&listen_addr)
        .arg("--backend")
        .arg(&doris.addr)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to launch the proxy binary");

    // Wait for the listener rather than sleeping a fixed amount.
    let mut client = None;
    for _ in 0..200 {
        if let Ok(connected) =
            RawClient::authenticate(&listen_addr, "analyst", CLIENT_SCRAMBLE).await
        {
            client = Some(connected);
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let mut client = client.expect("the binary never accepted a session");

    let outcome = client.query("SELECT * FROM sales.orders").await;

    let forwarded: Vec<String> = doris
        .sessions()
        .into_iter()
        .flat_map(|session| session.queries)
        .filter(|sql| sql != IDENTITY_QUERY)
        .collect();

    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_file(&policy_path);

    outcome.expect("the query should have been answered");
    assert_eq!(
        forwarded,
        vec![
            "SELECT * FROM (SELECT * FROM sales.orders WHERE `region` IN ('APAC', 'EMEA')) \
             AS orders"
                .to_string()
        ],
        "the shipped binary did not constrain the statement"
    );
}
