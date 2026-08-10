//! Shared fixture for the `row-filter-rewrite` tests.
//!
//! Included by the other test files with `#[path = "rewrite_fixture.rs"] mod
//! fixture;`, so it also compiles as a test target of its own with no tests in
//! it. That is why the file allows dead code: any one includer uses only part
//! of it.
//!
//! # Why this builds a real `PolicySet`
//!
//! This fixture used to define its own `TestPolicies` against a two-state
//! `PolicyLookup` trait — `Option<RowPolicy>`, policy or no policy. The real
//! policy layer has **three** states, and the third is a refusal: an unqualified
//! reference that the session cannot resolve because it has no current database.
//! A mock that cannot express that state cannot test it, and every rewrite test
//! passed without ever exercising the one case where forwarding is a disclosure.
//!
//! So the fixture now goes through `PolicySet::from_toml_str` — the same
//! validation the running proxy uses. Two things follow, and both are the point
//! rather than an inconvenience:
//!
//! - The current database is `Option<String>`, and [`TestPolicies::with_no_current_database`]
//!   makes the refusal case reachable from a test.
//! - An empty permitted-value set can no longer be constructed at all. The
//!   loader rejects it as a configuration error, so a rewriter branch rendering
//!   `WHERE FALSE` for it was dead code describing an unrepresentable state.

#![allow(dead_code)]

use doris_row_filter_proxy::analyze::AnalysisResult;
use doris_row_filter_proxy::error::RefusalReason;
use doris_row_filter_proxy::policy::PolicySet;
use doris_row_filter_proxy::rewrite::{all_policy_refs_guarded, rewrite_statement};
use proptest::prelude::*;

/// The guard text the rewriter produces for `analyst`'s policy on
/// `sales.orders`. Spelled out once so the position tests can assert on the
/// exact SQL that reaches Doris rather than on a paraphrase of it.
pub const ORDERS_GUARD: &str = "(SELECT * FROM sales.orders WHERE `region` IN ('APAC', 'EMEA'))";

/// One `[[policy]]` entry, kept as written so the set can be rebuilt whenever a
/// builder method adds another.
struct Row {
    database: String,
    table: String,
    column: String,
    values: Vec<String>,
}

/// A validated policy set plus the session state a lookup needs: which user is
/// asking and which database they are in.
pub struct TestPolicies {
    user: String,
    current_database: Option<String>,
    rows: Vec<Row>,
    policies: PolicySet,
}

impl TestPolicies {
    /// `analyst`, restricted on `sales.orders` to `region IN ('APAC','EMEA')`,
    /// with `sales` as the session's current database.
    pub fn analyst() -> Self {
        Self::for_user("analyst").with("sales", "orders", "region", &["APAC", "EMEA"])
    }

    /// `reporting`, who has no configured policy for any table.
    pub fn unrestricted() -> Self {
        Self::for_user("reporting")
    }

    pub fn for_user(user: &str) -> Self {
        TestPolicies {
            user: user.to_string(),
            current_database: Some("sales".to_string()),
            rows: Vec::new(),
            policies: PolicySet::empty(),
        }
    }

    /// Add a policy with text permitted values, or replace the one already
    /// configured for this table.
    ///
    /// Replacing rather than appending is not cosmetic: the loader rejects two
    /// policies for the same user and table as a configuration error, so
    /// `TestPolicies::analyst().with("sales", "orders", ...)` — the shape most
    /// tests use to override the default policy — would otherwise fail to
    /// build. The old test double kept policies in a `HashMap` and overwrote
    /// silently, which is why this only surfaced once the real loader was used.
    pub fn with(mut self, database: &str, table: &str, column: &str, values: &[&str]) -> Self {
        let row = Row {
            database: database.to_string(),
            table: table.to_string(),
            column: column.to_string(),
            values: values.iter().map(|v| (*v).to_string()).collect(),
        };
        match self
            .rows
            .iter_mut()
            .find(|existing| existing.database == row.database && existing.table == row.table)
        {
            Some(existing) => *existing = row,
            None => self.rows.push(row),
        }
        self.rebuild()
    }

    pub fn in_database(mut self, database: &str) -> Self {
        self.current_database = Some(database.to_string());
        self
    }

    /// A session that has issued no `USE` and named no database at handshake.
    ///
    /// Not an empty string — `None` is what makes an unqualified reference
    /// unresolvable rather than resolvable against a database called "".
    pub fn with_no_current_database(mut self) -> Self {
        self.current_database = None;
        self
    }

    fn rebuild(mut self) -> Self {
        let mut source = String::new();
        for row in &self.rows {
            source.push_str("[[policy]]\n");
            source.push_str(&format!("user = {}\n", toml_string(&self.user)));
            source.push_str(&format!("database = {}\n", toml_string(&row.database)));
            source.push_str(&format!("table = {}\n", toml_string(&row.table)));
            source.push_str(&format!("column = {}\n", toml_string(&row.column)));
            let values: Vec<String> = row.values.iter().map(|v| toml_string(v)).collect();
            source.push_str(&format!("permitted_values = [{}]\n\n", values.join(", ")));
        }
        self.policies = PolicySet::from_toml_str(&source, "rewrite_fixture.rs")
            .expect("fixture built an invalid policy configuration");
        self
    }

    pub fn user(&self) -> &str {
        &self.user
    }

    pub fn current_database(&self) -> Option<&str> {
        self.current_database.as_deref()
    }

    pub fn policy_set(&self) -> &PolicySet {
        &self.policies
    }

    /// Rewrite `sql` as this user, in this session.
    pub fn rewrite(&self, sql: &str) -> AnalysisResult<String> {
        rewrite_statement(sql, &self.user, self.current_database(), &self.policies)
    }

    /// Whether every policy-bearing reference in `sql` sits inside a guard.
    pub fn guarded(&self, sql: &str) -> AnalysisResult<bool> {
        all_policy_refs_guarded(sql, &self.user, self.current_database(), &self.policies)
    }
}

/// Render a value as a TOML basic string.
///
/// Written out rather than relying on `format!("{value:?}")`: Rust's `Debug`
/// escaping is close to TOML's but not the same, and these tests deliberately
/// feed the loader values containing backslashes and quotes.
fn toml_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Rewrite as `analyst`.
pub fn rewrite(sql: &str) -> Result<String, RefusalReason> {
    TestPolicies::analyst().rewrite(sql)
}

/// Rewrite as `analyst` and fail the test if the statement was refused.
pub fn forwarded(sql: &str) -> String {
    match rewrite(sql) {
        Ok(forwarded) => forwarded,
        Err(reason) => panic!("expected {sql:?} to be forwarded, but it was refused: {reason}"),
    }
}

/// Refusal reason for a statement issued by `analyst`.
pub fn refusal(sql: &str) -> RefusalReason {
    match rewrite(sql) {
        Ok(forwarded) => {
            panic!("expected {sql:?} to be refused, but it was forwarded as {forwarded:?}")
        }
        Err(reason) => reason,
    }
}

/// Rewrite as `analyst`, then re-parse the result and assert that no reference
/// to a policy-bearing table survives outside a guard.
///
/// The assertion is made against the *rendered* SQL, which is what Doris would
/// see. Returns the forwarded statement so callers can assert on its text too.
pub fn assert_fully_guarded(sql: &str) -> String {
    let policies = TestPolicies::analyst();
    let forwarded = match policies.rewrite(sql) {
        Ok(forwarded) => forwarded,
        Err(reason) => panic!("expected {sql:?} to be forwarded, but it was refused: {reason}"),
    };
    assert!(
        policies.guarded(&forwarded).unwrap(),
        "policy-bearing reference left unconstrained in {forwarded:?}"
    );
    forwarded
}

/// How many guards the rewritten statement contains.
pub fn guard_count(sql: &str) -> usize {
    sql.matches(ORDERS_GUARD).count()
}

// ---------------------------------------------------------------------------
// Shared generator
// ---------------------------------------------------------------------------

/// Statement shapes built around the policy table, in every position design D2
/// claims to cover plus a few it does not.
pub fn statement_against_the_policy_table() -> impl Strategy<Value = String> {
    let relation = prop_oneof![
        Just("sales.orders".to_string()),
        Just("sales.orders o".to_string()),
        Just("orders".to_string()),
        Just("sales.products p".to_string()),
        Just("scratch.tmp".to_string()),
    ];
    let predicate = prop_oneof![
        Just(String::new()),
        Just(" WHERE 1 = 1".to_string()),
        Just(" WHERE region = 'AMER' OR 1 = 1".to_string()),
        Just(" WHERE region = 'AMER'".to_string()),
        Just(" WHERE EXISTS (SELECT 1 FROM sales.orders x WHERE x.id = 1)".to_string()),
        Just(" WHERE id IN (SELECT id FROM sales.orders)".to_string()),
    ];
    let projection = prop_oneof![
        Just("*".to_string()),
        Just("COUNT(*)".to_string()),
        Just("SUM(total)".to_string()),
        Just("(SELECT MAX(total) FROM sales.orders)".to_string()),
    ];

    (relation, predicate, projection, 0u8..6).prop_map(|(rel, pred, proj, shape)| match shape {
        0 => format!("SELECT {proj} FROM {rel}{pred}"),
        1 => format!("SELECT {proj} FROM (SELECT * FROM {rel}{pred}) d"),
        2 => format!("SELECT {proj} FROM {rel}{pred} UNION ALL SELECT {proj} FROM sales.orders"),
        3 => format!("WITH c AS (SELECT * FROM {rel}{pred}) SELECT {proj} FROM c"),
        4 => format!("SELECT {proj} FROM {rel} LEFT JOIN sales.orders j ON 1 = 1{pred}"),
        _ => format!("SELECT {proj} FROM sales.orders a JOIN {rel} ON 1 = 1{pred}"),
    })
}
