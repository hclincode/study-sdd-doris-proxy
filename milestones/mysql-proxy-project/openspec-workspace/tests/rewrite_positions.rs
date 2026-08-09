//! Tasks 5.5–5.8 — one test per position in design D2's table, plus the
//! rewriting-preserves-everything requirement.
//!
//! Every assertion here is on the SQL that would reach Doris. There is no Doris
//! instance in this suite, so the claim under test is the transformation, not
//! the rows: that the policy predicate lands inside the relation, that the
//! relation keeps its effective name, and that nothing else about the statement
//! moves.

#[path = "rewrite_fixture.rs"]
mod fixture;

use doris_row_filter_proxy::analyze::parse_single;
use doris_row_filter_proxy::policy::PolicySet;
use fixture::{assert_fully_guarded, forwarded, guard_count, ORDERS_GUARD};

// ---------------------------------------------------------------------------
// Design D2 position table
// ---------------------------------------------------------------------------

/// Spec: "Simple select is constrained".
#[test]
fn simple_select_is_constrained() {
    assert_eq!(
        assert_fully_guarded("SELECT * FROM sales.orders"),
        format!("SELECT * FROM {ORDERS_GUARD} AS orders")
    );
}

/// The effective name is the user's alias when there is one, so column
/// references written against the alias still resolve.
#[test]
fn an_aliased_relation_keeps_its_alias_as_the_guard_name() {
    assert_eq!(
        assert_fully_guarded("SELECT o.total FROM sales.orders AS o"),
        format!("SELECT o.total FROM {ORDERS_GUARD} AS o")
    );
}

/// Spec: "User disjunction does not widen the permitted set". The vector that
/// kills naive `AND` injection: `WHERE a OR b AND policy` binds as
/// `a OR (b AND policy)`. Here the user's predicate is left exactly as written
/// and the policy sits inside the relation, so its precedence is irrelevant.
#[test]
fn user_disjunction_does_not_widen_the_permitted_set() {
    let forwarded = assert_fully_guarded("SELECT * FROM sales.orders WHERE region = 'AMER' OR 1=1");
    assert_eq!(
        forwarded,
        format!("SELECT * FROM {ORDERS_GUARD} AS orders WHERE region = 'AMER' OR 1 = 1")
    );
    // The user's disjunction is intact and *outside* the guard: the rewriter did
    // not try to merge the two predicates.
    assert!(forwarded.contains("AS orders WHERE region = 'AMER' OR 1 = 1"));
}

#[test]
fn an_inner_join_operand_is_constrained() {
    assert_eq!(
        assert_fully_guarded(
            "SELECT * FROM sales.products p INNER JOIN sales.orders o ON p.id = o.pid"
        ),
        format!("SELECT * FROM sales.products p INNER JOIN {ORDERS_GUARD} AS o ON p.id = o.pid")
    );
}

/// Spec: "Outer join does not reveal restricted rows". Injecting into the
/// enclosing `WHERE` would have turned this `LEFT JOIN` into an inner join and
/// dropped preserved-side rows. Filtering inside the operand keeps the join
/// type, so the preserved side is unchanged and restricted rows contribute
/// nothing — not even a NULL-extended row asserting their existence.
#[test]
fn outer_join_does_not_reveal_restricted_rows() {
    let forwarded = assert_fully_guarded(
        "SELECT * FROM sales.products p LEFT JOIN sales.orders o ON p.id = o.pid",
    );
    assert_eq!(
        forwarded,
        format!("SELECT * FROM sales.products p LEFT JOIN {ORDERS_GUARD} AS o ON p.id = o.pid")
    );
    // The join type survives, and no predicate was added to the outer statement.
    assert!(forwarded.contains("LEFT JOIN"));
    assert!(!forwarded.contains("AS o ON p.id = o.pid WHERE"));
}

/// Spec: "Subquery reference is constrained".
#[test]
fn subquery_reference_is_constrained() {
    assert_eq!(
        assert_fully_guarded("SELECT COUNT(*) FROM (SELECT * FROM sales.orders) t"),
        format!("SELECT COUNT(*) FROM (SELECT * FROM {ORDERS_GUARD} AS orders) t")
    );
}

/// Spec: "Common table expression reference is constrained". The CTE's own
/// `FROM` is rewritten, so every use of the CTE reads constrained rows; the use
/// site itself is a CTE reference, not a table, and is left alone.
#[test]
fn common_table_expression_reference_is_constrained() {
    assert_eq!(
        assert_fully_guarded("WITH c AS (SELECT * FROM sales.orders) SELECT * FROM c"),
        format!("WITH c AS (SELECT * FROM {ORDERS_GUARD} AS orders) SELECT * FROM c")
    );
}

/// Spec: "Every branch of a set operation is constrained".
#[test]
fn every_branch_of_a_set_operation_is_constrained() {
    let forwarded = assert_fully_guarded(
        "SELECT id FROM sales.orders UNION ALL SELECT id FROM sales.orders WHERE region = 'AMER'",
    );
    assert_eq!(guard_count(&forwarded), 2, "both branches must be guarded");
    assert_eq!(
        forwarded,
        format!(
            "SELECT id FROM {ORDERS_GUARD} AS orders \
             UNION ALL SELECT id FROM {ORDERS_GUARD} AS orders WHERE region = 'AMER'"
        )
    );
}

/// Spec: "Correlated subquery cannot test restricted rows". The subquery's own
/// relation is wrapped, so the existence test evaluates over permitted rows
/// only and its outcome discloses nothing about the rest.
#[test]
fn correlated_subquery_cannot_test_restricted_rows() {
    assert_eq!(
        assert_fully_guarded(
            "SELECT * FROM sales.products p \
             WHERE EXISTS (SELECT 1 FROM sales.orders o WHERE o.pid = p.id)"
        ),
        format!(
            "SELECT * FROM sales.products p \
             WHERE EXISTS (SELECT 1 FROM {ORDERS_GUARD} AS o WHERE o.pid = p.id)"
        )
    );
}

#[test]
fn a_correlated_in_subquery_is_constrained() {
    assert_eq!(
        assert_fully_guarded(
            "SELECT * FROM sales.products p WHERE p.id IN (SELECT pid FROM sales.orders)"
        ),
        format!(
            "SELECT * FROM sales.products p \
             WHERE p.id IN (SELECT pid FROM {ORDERS_GUARD} AS orders)"
        )
    );
}

/// Spec: "Both operands of a self-join are constrained".
#[test]
fn both_operands_of_a_self_join_are_constrained() {
    let forwarded = assert_fully_guarded(
        "SELECT a.id, b.id FROM sales.orders a JOIN sales.orders b ON a.id = b.id",
    );
    assert_eq!(guard_count(&forwarded), 2, "both operands must be guarded");
    assert_eq!(
        forwarded,
        format!(
            "SELECT a.id, b.id FROM {ORDERS_GUARD} AS a JOIN {ORDERS_GUARD} AS b ON a.id = b.id"
        )
    );
}

/// Spec: "Restricted rows cannot be recovered through an aggregate". The
/// aggregate has no relation of its own to constrain — the guard sits under it,
/// so the aggregate sees only permitted rows.
#[test]
fn restricted_rows_cannot_be_recovered_through_an_aggregate() {
    assert_eq!(
        assert_fully_guarded("SELECT SUM(total), MAX(total) FROM sales.orders"),
        format!("SELECT SUM(total), MAX(total) FROM {ORDERS_GUARD} AS orders")
    );
}

// ---------------------------------------------------------------------------
// Task 5.6 — references without a policy
// ---------------------------------------------------------------------------

/// Spec: a table with no policy for the requesting user is unrestricted.
#[test]
fn a_table_with_no_policy_for_the_user_is_forwarded_unwrapped() {
    let sql = "SELECT * FROM sales.products WHERE id = 1";
    assert_eq!(forwarded(sql), sql);
}

#[test]
fn only_the_policy_bearing_operand_of_a_join_is_wrapped() {
    let forwarded =
        assert_fully_guarded("SELECT * FROM sales.products p JOIN sales.orders o ON p.id = o.pid");
    assert_eq!(guard_count(&forwarded), 1);
    assert!(
        forwarded.contains("sales.products p JOIN ("),
        "the unrestricted operand should be untouched: {forwarded}"
    );
}

/// The same table name under a different database is a different table.
#[test]
fn a_policy_on_one_database_does_not_constrain_the_same_name_elsewhere() {
    let sql = "SELECT * FROM staging.orders";
    assert_eq!(forwarded(sql), sql);
}

/// An unqualified name resolves against the session's current database, so
/// `orders` under `sales` is the policy table and is constrained.
#[test]
fn an_unqualified_reference_resolves_against_the_current_database() {
    assert_eq!(
        assert_fully_guarded("SELECT * FROM orders"),
        "SELECT * FROM (SELECT * FROM orders WHERE `region` IN ('APAC', 'EMEA')) AS orders"
    );
}

// ---------------------------------------------------------------------------
// Task 5.7, 5.8, and "Rewriting preserves everything except restricted rows"
// ---------------------------------------------------------------------------

/// Spec: "Column shape is unchanged by rewriting". The projection is copied
/// through untouched, and the guard projects `*`, so the column list, its order
/// and its types are whatever Doris would have produced for the original.
#[test]
fn column_shape_is_unchanged_by_rewriting() {
    let forwarded = assert_fully_guarded("SELECT id, region, total FROM sales.orders");
    assert_eq!(
        forwarded,
        format!("SELECT id, region, total FROM {ORDERS_GUARD} AS orders")
    );
    assert!(forwarded.starts_with("SELECT id, region, total FROM"));
}

/// Spec: "Ordering among permitted rows is preserved". The `ORDER BY` stays on
/// the outer query, where it still orders the whole result; the guard adds no
/// ordering of its own that could interact with it.
#[test]
fn ordering_among_permitted_rows_is_preserved() {
    assert_eq!(
        assert_fully_guarded("SELECT id FROM sales.orders ORDER BY total DESC, id ASC"),
        format!("SELECT id FROM {ORDERS_GUARD} AS orders ORDER BY total DESC, id ASC")
    );
}

/// Spec: "Placeholder count and order are unchanged". Invariant 3: the permitted
/// values are emitted as literals, never as `?`, so a client's parameter binding
/// computed against the statement it wrote stays correct.
#[test]
fn placeholder_count_and_order_are_unchanged() {
    let original = "SELECT * FROM sales.orders WHERE id = ? AND total > ? AND note LIKE ?";
    let forwarded = assert_fully_guarded(original);
    assert_eq!(
        original.matches('?').count(),
        forwarded.matches('?').count(),
        "placeholder count changed: {forwarded}"
    );
    assert_eq!(
        forwarded,
        format!(
            "SELECT * FROM {ORDERS_GUARD} AS orders \
             WHERE id = ? AND total > ? AND note LIKE ?"
        )
    );
}

/// Spec: "A user permitted all present values sees the same rows as without the
/// proxy".
///
/// With no Doris to query, the checkable form of that claim is structural: the
/// forwarded statement is the original with the relation replaced by a filtered
/// copy of *that same relation*, projecting `*` and adding nothing else.
/// Substituting the relation back reproduces the original statement exactly, so
/// a predicate that excludes no rows leaves the result identical.
#[test]
fn a_user_permitted_all_present_values_sees_the_same_rows_as_without_the_proxy() {
    for original in [
        "SELECT id, region, total FROM sales.orders WHERE total > 10 ORDER BY id",
        "SELECT COUNT(*) FROM sales.orders GROUP BY region HAVING COUNT(*) > 1",
        "SELECT * FROM sales.products p LEFT JOIN sales.orders ON p.id = orders.pid",
    ] {
        let forwarded = assert_fully_guarded(original);
        let restored = forwarded.replace(&format!("{ORDERS_GUARD} AS orders"), "sales.orders");
        assert_eq!(
            restored,
            parse_single(original).unwrap().to_string(),
            "the rewrite of {original:?} differs from the original by more than the guard"
        );
    }
}

/// The guard predicate is built from configured literals. A value containing a
/// quote is escaped rather than closing the string early.
///
/// This test used to include `back\slash` and assert it was emitted doubled.
/// `policy-config` now refuses such a value at load time — see
/// `a_backslash_value_is_refused_by_the_loader_not_escaped_by_the_rewriter`
/// below — so the doubled form is unreachable and asserting on it would be
/// asserting on a state the proxy cannot be in.
#[test]
fn permitted_values_are_emitted_as_escaped_literals() {
    let policies =
        fixture::TestPolicies::analyst().with("sales", "orders", "owner", &["O'Hare", "Smith"]);
    let forwarded = policies.rewrite("SELECT * FROM sales.orders").unwrap();
    assert_eq!(
        forwarded,
        "SELECT * FROM (SELECT * FROM sales.orders \
         WHERE `owner` IN ('O''Hare', 'Smith')) AS orders"
    );
}

/// The two layers must not both try to solve backslashes.
///
/// The rewriter deliberately does **not** double a backslash: doing so is
/// correct under MySQL's default `sql_mode` and wrong under
/// `NO_BACKSLASH_ESCAPES`, and the proxy does not control the backend's mode.
/// `policy-config` refuses the value instead, which is correct under both. This
/// test pins which layer owns the problem, so a later change cannot quietly move
/// it back into the renderer.
#[test]
fn a_backslash_value_is_refused_by_the_loader_not_escaped_by_the_rewriter() {
    let error = PolicySet::from_toml_str(
        "[[policy]]\n\
         user = \"analyst\"\n\
         database = \"sales\"\n\
         table = \"orders\"\n\
         column = \"region\"\n\
         permitted_values = [\"back\\\\slash\"]\n",
        "a_backslash_value_is_refused_by_the_loader_not_escaped_by_the_rewriter",
    )
    .expect_err("a backslash in a permitted value must not load");
    assert!(
        error.to_string().contains("backslash"),
        "diagnostic does not name the problem: {error}"
    );
}

/// Every configured value survives the trip into the emitted literal and back,
/// including the three shapes `sqlparser`'s own renderer gets wrong: a
/// backslash, a value ending in a backslash, and a pair of consecutive quotes.
///
/// The round trip is the assertion, not a hand-written expected string: the
/// literal is parsed back with `MySqlDialect`, whose backslash-escape handling
/// matches Doris's default `sql_mode`. A value that came back different would be
/// a policy matching rows the operator never configured.
#[test]
fn every_permitted_value_round_trips_through_the_emitted_literal() {
    use sqlparser::ast::{Expr, Value};
    use sqlparser::dialect::MySqlDialect;
    use sqlparser::parser::Parser;

    // Only values `policy-config` accepts. The shapes it refuses — a backslash
    // anywhere, and a `''` pair — are the ones whose rendering cannot be made
    // correct under every `sql_mode`, and they are asserted to be refused in
    // `every_value_the_loader_refuses_is_refused_at_load` below rather than
    // round-tripped here.
    for value in [
        "APAC",
        "O'Hare",
        "a'b'c",
        "'",
        "trailing'",
        "100%",
        "1'; DROP TABLE x --",
    ] {
        let policies = fixture::TestPolicies::analyst().with("sales", "orders", "region", &[value]);
        let forwarded = policies.rewrite("SELECT * FROM sales.orders").unwrap();

        let literal = forwarded
            .split_once("IN (")
            .and_then(|(_, rest)| rest.rsplit_once("))"))
            .map(|(literal, _)| literal)
            .unwrap_or_else(|| panic!("no literal found in {forwarded}"));

        let expression = Parser::new(&MySqlDialect {})
            .try_with_sql(literal)
            .unwrap()
            .parse_expr()
            .unwrap_or_else(|e| panic!("emitted literal {literal} does not parse: {e}"));
        let Expr::Value(parsed) = expression else {
            panic!("emitted literal {literal} is not a value");
        };
        assert_eq!(
            parsed.value,
            Value::SingleQuotedString(value.to_string()),
            "value {value:?} was emitted as {literal}, which reads back as something else"
        );
    }
}

/// The other half of the round trip: every shape the emitter could not render
/// faithfully is refused before it can become a policy.
///
/// Without this, narrowing what the loader accepts would silently shrink what
/// the round-trip test above covers, and nothing would notice.
#[test]
fn every_value_the_loader_refuses_is_refused_at_load() {
    for value in ["a\\\\b", "a\\\\", "a''b", "\\\\'; DROP TABLE x --"] {
        let source = format!(
            "[[policy]]\nuser = \"analyst\"\ndatabase = \"sales\"\ntable = \"orders\"\n\
             column = \"region\"\npermitted_values = [{}]\n",
            toml_basic_string(value)
        );
        PolicySet::from_toml_str(&source, "every_value_the_loader_refuses_is_refused_at_load")
            .expect_err(&format!("{value:?} should not load as a permitted value"));
    }
}

/// TOML basic-string quoting, so the values above reach the loader as written.
fn toml_basic_string(value: &str) -> String {
    let mut out = String::from("\"");
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// An empty permitted set never reaches the rewriter: `policy-config` rejects it
/// at load time, so there is no "permits nothing" policy for a guard to render.
///
/// This replaces a test that asserted the rewriter emitted `WHERE false` for
/// such a policy. That test passed only because the old test double could
/// construct a state the real loader refuses; the branch it exercised was dead
/// code describing an unrepresentable configuration. The behaviour is still
/// pinned — one layer down, where it actually lives.
#[test]
fn an_empty_permitted_set_is_rejected_before_any_rewriting() {
    let error = PolicySet::from_toml_str(
        "[[policy]]\n\
         user = \"analyst\"\n\
         database = \"sales\"\n\
         table = \"orders\"\n\
         column = \"region\"\n\
         permitted_values = []\n",
        "an_empty_permitted_set_is_rejected_before_any_rewriting",
    )
    .expect_err("an empty permitted set must not load");
    assert!(
        error.to_string().contains("permitted_values"),
        "diagnostic does not name the offending field: {error}"
    );
}
