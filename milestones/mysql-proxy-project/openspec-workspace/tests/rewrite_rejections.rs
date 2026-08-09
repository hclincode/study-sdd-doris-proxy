//! Tasks 6.1–6.8 — the SQL-level rejection paths.
//!
//! "Nothing reached the backend" is asserted here as "the rewriter produced no
//! statement to forward". `rewrite_statement` is the only source of forwardable
//! SQL on this path, so an `Err` from it is exactly the property: there is no
//! second return value, and no partially-rewritten statement is handed back
//! alongside the error.

#[path = "rewrite_fixture.rs"]
mod fixture;

use doris_row_filter_proxy::error::RefusalReason;
use fixture::{forwarded, refusal, TestPolicies};

/// Spec: "Unparseable statement from a policy-bearing user is rejected".
#[test]
fn unparseable_statement_from_a_policy_bearing_user_is_rejected() {
    for sql in [
        "SELCT * FROM sales.orders",
        "SELECT * FROM sales.orders WHERE",
        "SELECT * FROM sales.orders LATERAL VIEW explode(items) t AS item",
        "((((",
    ] {
        let reason = refusal(sql);
        assert!(
            matches!(
                reason,
                RefusalReason::Unparseable | RefusalReason::UnsupportedShape { .. }
            ),
            "{sql:?} refused with the wrong reason: {reason}"
        );
    }
}

/// Spec: "Unparseable statement from an unrestricted user is forwarded".
///
/// Design D6: the rule is keyed on whether the user has *any* policy, because
/// that is decidable without parsing. Asking "does it touch a policy table?"
/// would require understanding SQL that has just failed to parse.
#[test]
fn unparseable_statement_from_an_unrestricted_user_is_forwarded() {
    let unrestricted = TestPolicies::unrestricted();
    for sql in [
        "SELCT * FROM sales.orders",
        "SELECT * FROM sales.orders LATERAL VIEW explode(items) t AS item",
        "SHOW TABLES",
        "SELECT * FROM sales.orders; SELECT 1",
    ] {
        assert_eq!(
            unrestricted.rewrite(sql).unwrap(),
            sql,
            "{sql:?} should be forwarded unmodified for a user with no policy"
        );
    }
}

/// Spec: "Statement shape the rewriter does not support is rejected".
///
/// Each of these parses. None of them is on the allowlist, so the proxy cannot
/// show that it enumerated every table reference, and refuses.
#[test]
fn statement_shape_the_rewriter_does_not_support_is_rejected() {
    for sql in [
        // An expression kind the walk does not descend into.
        "SELECT * FROM sales.orders o WHERE MATCH (o.note) AGAINST ('x')",
        // A relation kind that is not a table or a derived table.
        "SELECT * FROM UNNEST(ARRAY[1,2]) AS t",
        // A join operator design D2's position table does not cover.
        "SELECT * FROM sales.orders LEFT SEMI JOIN sales.products ON 1 = 1",
        // Partition selection: the wrap would drop it.
        "SELECT * FROM sales.orders PARTITION (p0)",
        // A three-part name the policy layer cannot resolve.
        "SELECT * FROM internal.sales.orders",
        // A statement kind outside SELECT and the four write forms.
        "SHOW TABLES",
        "SET autocommit = 1",
    ] {
        assert!(
            matches!(refusal(sql), RefusalReason::UnsupportedShape { .. }),
            "{sql:?} should refuse as an unsupported shape"
        );
    }
}

/// Spec: "Rejection names the reason without disclosing policy contents".
///
/// Covers only the message this module produces. The MySQL error packet that
/// carries it is task 6.11, which belongs to the protocol layer.
#[test]
fn rejection_names_the_reason_without_disclosing_policy_contents() {
    for sql in [
        "SELCT nonsense",
        "SELECT * FROM sales.orders o WHERE MATCH (o.note) AGAINST ('x')",
        "UPDATE sales.orders SET total = 1",
        "SELECT * FROM sales.orders; SELECT 1",
    ] {
        let message = refusal(sql).client_message();
        assert!(
            message.starts_with("proxy refused statement:"),
            "the message should say the proxy refused, not why the user is denied: {message}"
        );
        for secret in ["region", "APAC", "EMEA", "analyst", "reporting"] {
            assert!(
                !message.contains(secret),
                "message discloses policy content {secret:?}: {message}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Tasks 6.4–6.7 — writes against a policy table
// ---------------------------------------------------------------------------

/// Spec: "Update against a policy table is rejected".
#[test]
fn update_against_a_policy_table_is_rejected() {
    assert!(matches!(
        refusal("UPDATE sales.orders SET total = 1 WHERE id = 2"),
        RefusalReason::WriteToRestrictedTable
    ));
}

/// Spec: "Insert selecting from a policy table is rejected". The policy table is
/// only *read* here, and the target has no policy — but the read is
/// unconstrained on the write path, so the statement does not run.
#[test]
fn insert_selecting_from_a_policy_table_is_rejected() {
    assert!(matches!(
        refusal("INSERT INTO scratch.copy SELECT * FROM sales.orders"),
        RefusalReason::WriteToRestrictedTable
    ));
}

#[test]
fn insert_into_a_policy_table_is_rejected() {
    assert!(matches!(
        refusal("INSERT INTO sales.orders (id, region) VALUES (1, 'AMER')"),
        RefusalReason::WriteToRestrictedTable
    ));
}

#[test]
fn replace_into_a_policy_table_is_rejected() {
    assert!(matches!(
        refusal("REPLACE INTO sales.orders (id, region) VALUES (1, 'AMER')"),
        RefusalReason::WriteToRestrictedTable
    ));
}

#[test]
fn delete_against_a_policy_table_is_rejected() {
    assert!(matches!(
        refusal("DELETE FROM sales.orders WHERE id = 1"),
        RefusalReason::WriteToRestrictedTable
    ));
}

#[test]
fn a_write_reading_a_policy_table_in_a_subquery_is_rejected() {
    assert!(matches!(
        refusal("UPDATE sales.products SET flag = 1 WHERE id IN (SELECT pid FROM sales.orders)"),
        RefusalReason::WriteToRestrictedTable
    ));
    assert!(matches!(
        refusal("DELETE FROM scratch.tmp WHERE id IN (SELECT id FROM sales.orders)"),
        RefusalReason::WriteToRestrictedTable
    ));
}

/// Spec: "Write against a table with no policy is forwarded".
#[test]
fn write_against_a_table_with_no_policy_is_forwarded() {
    for sql in [
        "UPDATE sales.products SET price = 1 WHERE id = 2",
        "INSERT INTO scratch.tmp (id) VALUES (1)",
        "DELETE FROM scratch.tmp WHERE id = 1",
        "REPLACE INTO scratch.tmp (id) VALUES (1)",
    ] {
        assert_eq!(
            forwarded(sql),
            sql,
            "{sql:?} should be forwarded unmodified"
        );
    }
}

// ---------------------------------------------------------------------------
// One request, one statement
// ---------------------------------------------------------------------------

/// Spec: "Multi-statement request is rejected".
///
/// Design D4 keeps `CLIENT_MULTI_STATEMENTS` from being negotiated at all, so a
/// client should never be able to send one. This is the SQL-level backstop for
/// the case where one arrives anyway; the protocol half is task 6.10.
#[test]
fn multi_statement_request_is_rejected() {
    assert!(matches!(
        refusal("SELECT * FROM sales.orders; SELECT 1"),
        RefusalReason::MultiStatement
    ));
    assert!(matches!(
        refusal("SELECT 1; SELECT * FROM sales.orders"),
        RefusalReason::MultiStatement
    ));
}

/// Spec: "Comment cannot conceal a second statement". The parser sees through
/// the comment, so the second statement is counted and the request refused
/// rather than the first statement being forwarded on its own.
#[test]
fn comment_cannot_conceal_a_second_statement() {
    for sql in [
        "SELECT * FROM sales.orders /* harmless */ ; SELECT 1",
        "SELECT * FROM sales.orders -- trailing\n; SELECT 1",
        "SELECT * FROM sales.orders; /* second */ SELECT * FROM sales.orders",
    ] {
        assert!(
            matches!(refusal(sql), RefusalReason::MultiStatement),
            "{sql:?} should be refused as a multi-statement request"
        );
    }
}

// ---------------------------------------------------------------------------
// Unresolvable references — `policy-config`, "An unqualified reference with no
// current database is refused"
// ---------------------------------------------------------------------------

/// The case the old two-state test double could not express.
///
/// A client connects without naming a database and never issues `USE`, then
/// queries `FROM orders`. The proxy cannot tell whether `orders` is the
/// `sales.orders` it is meant to constrain, so it must refuse. Forwarding here
/// would return the policy table's rows unfiltered.
///
/// This is asserted on the refusal, not merely on "not the happy path": a
/// statement that failed to parse would also be an `Err`, and would prove
/// nothing about resolution.
#[test]
fn an_unqualified_reference_with_no_current_database_is_refused() {
    let session = TestPolicies::analyst().with_no_current_database();

    let reason = session
        .rewrite("SELECT * FROM orders")
        .expect_err("an unresolvable reference must not be forwarded");

    assert!(
        matches!(reason, RefusalReason::UnresolvableTableReference),
        "expected an unresolvable-reference refusal, got {reason:?}"
    );

    // Design D5: a refusal must not disclose policy contents. This one is
    // returned for a table that *might* be policy-bearing, so naming anything
    // from the policy would confirm which tables are.
    let message = reason.client_message();
    for secret in ["sales", "orders", "region", "APAC", "EMEA"] {
        assert!(
            !message.contains(secret),
            "refusal leaks {secret:?} from the policy: {message}"
        );
    }

    // And it is indistinguishable from an unsupported shape to the client. The
    // variant is what an operator reads; the wording is what the client reads,
    // and telling them apart from outside would say which references the proxy
    // could resolve.
    assert_eq!(
        message,
        RefusalReason::UnsupportedShape {
            construct: "irrelevant".to_string()
        }
        .client_message(),
        "the client can tell an unresolvable reference from an unsupported shape"
    );
}

/// The refusal is about resolution, not about the table being restricted: a
/// reference the session *can* resolve to a table with no policy still passes.
#[test]
fn the_same_reference_is_forwarded_once_the_session_has_a_database() {
    let sql = "SELECT * FROM products";
    let session = TestPolicies::analyst().in_database("sales");
    assert_eq!(
        session.rewrite(sql).expect("resolvable and unrestricted"),
        sql
    );
}

/// A qualified reference needs no current database, so it is unaffected.
#[test]
fn a_qualified_reference_is_unaffected_by_the_absence_of_a_current_database() {
    let session = TestPolicies::analyst().with_no_current_database();
    assert_eq!(
        session
            .rewrite("SELECT * FROM sales.orders")
            .expect("a qualified reference resolves without a current database"),
        "SELECT * FROM (SELECT * FROM sales.orders WHERE `region` IN ('APAC', 'EMEA')) AS orders"
    );
}

/// Spec: "A user with no policies is unaffected by an unresolvable reference".
///
/// The refusal exists to protect policy tables. A user who has none has nothing
/// to protect, so the same statement is forwarded rather than refused — and the
/// two cases must not be collapsed in either direction.
#[test]
fn a_user_with_no_policies_is_unaffected_by_an_unresolvable_reference() {
    let sql = "SELECT * FROM orders";
    let session = TestPolicies::unrestricted().with_no_current_database();
    assert_eq!(session.rewrite(sql).expect("nothing to protect"), sql);
}

/// The pre-check is the layer that matters here.
///
/// `rewrite_statement` returns the original SQL as soon as nothing looks
/// restricted, *before* the wrapper walks the statement. So an unresolvable
/// reference has to be caught in that pre-check; catching it only in the
/// wrapper would leave the statement forwarded untouched one layer earlier.
/// This asserts the whole statement is refused even though the reference that
/// cannot be resolved sits beside one that plainly carries no policy.
#[test]
fn an_unresolvable_reference_is_caught_before_the_wrapper_runs() {
    let session = TestPolicies::analyst().with_no_current_database();
    assert!(
        session
            .rewrite("SELECT * FROM sales.products p JOIN orders o ON p.id = o.pid")
            .is_err(),
        "an unresolvable reference must refuse the statement, not pass the pre-check"
    );
}
