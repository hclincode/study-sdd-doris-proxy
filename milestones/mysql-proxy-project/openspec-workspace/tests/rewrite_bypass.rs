//! Tasks 7.1–7.5 and 7.7 — the bypass attempts.
//!
//! These are the ways a SQL-rewriting row filter leaks. Each test states the
//! attack and asserts the shape of the statement that would actually reach
//! Doris, because with no Doris in this suite the transformation is the only
//! thing observable. What each one checks is the same property in a different
//! syntactic disguise: **the reference to the policy table is not reachable
//! except through a predicate the user did not write and cannot reach.**

#[path = "rewrite_fixture.rs"]
mod fixture;

use doris_row_filter_proxy::analyze::analyze;
use fixture::{assert_fully_guarded, guard_count, TestPolicies, ORDERS_GUARD};
use proptest::prelude::*;

/// Task 7.1. `OR 1=1` is the vector that defeats appending `AND <policy>` to the
/// user's `WHERE`: `region = 'AMER' OR 1 = 1 AND policy` binds as
/// `region = 'AMER' OR (1 = 1 AND policy)`, leaving the first disjunct free.
/// The guard is not in the user's `WHERE` at all, so the user's operators cannot
/// reach it.
#[test]
fn a_user_disjunction_cannot_widen_the_row_set() {
    let forwarded = assert_fully_guarded("SELECT * FROM sales.orders WHERE region = 'AMER' OR 1=1");
    assert_eq!(
        forwarded,
        format!("SELECT * FROM {ORDERS_GUARD} AS orders WHERE region = 'AMER' OR 1 = 1")
    );

    // The policy predicate appears exactly once, and inside the guard — never
    // as a conjunct of the user's predicate where precedence could reach it.
    assert_eq!(forwarded.matches("`region` IN").count(), 1);
    let guard_end = forwarded.find(" AS orders").unwrap();
    assert!(
        forwarded.find("`region` IN").unwrap() < guard_end,
        "the policy predicate escaped the guard: {forwarded}"
    );
}

/// Task 7.2. Counting through a derived table is the classic way to recover a
/// restricted population when only the outer query is constrained.
#[test]
fn a_subquery_count_cannot_count_restricted_rows() {
    let forwarded = assert_fully_guarded("SELECT COUNT(*) FROM (SELECT * FROM sales.orders) t");
    assert_eq!(
        forwarded,
        format!("SELECT COUNT(*) FROM (SELECT * FROM {ORDERS_GUARD} AS orders) t")
    );
}

/// Task 7.3. An aggregate projects no individual row, so an implementation that
/// filtered "the rows the client sees" rather than the relation would leave it
/// unconstrained.
#[test]
fn an_aggregate_with_no_row_projection_is_computed_only_over_permitted_rows() {
    for sql in [
        "SELECT COUNT(*) FROM sales.orders",
        "SELECT SUM(total) FROM sales.orders",
        "SELECT MAX(total), MIN(total), AVG(total) FROM sales.orders",
        "SELECT COUNT(DISTINCT region) FROM sales.orders",
        "SELECT region, SUM(total) FROM sales.orders GROUP BY region HAVING SUM(total) > 0",
    ] {
        let forwarded = assert_fully_guarded(sql);
        assert_eq!(
            guard_count(&forwarded),
            1,
            "aggregate left its relation unconstrained: {forwarded}"
        );
    }
}

/// Task 7.4. Injecting the policy into the enclosing `WHERE` would have turned
/// this into an inner join, which changes the result set. Leaving it out
/// entirely would let a restricted row produce a NULL-extended output row —
/// disclosing that the row exists without showing it.
///
/// The guard does neither: it filters inside the operand, so the preserved side
/// is untouched and a restricted right-hand row is simply not there to match.
#[test]
fn a_left_join_discloses_no_restricted_row_including_through_null_extension() {
    let forwarded = assert_fully_guarded(
        "SELECT p.id, o.total FROM sales.products p LEFT JOIN sales.orders o ON p.id = o.pid",
    );
    assert_eq!(
        forwarded,
        format!(
            "SELECT p.id, o.total FROM sales.products p \
             LEFT JOIN {ORDERS_GUARD} AS o ON p.id = o.pid"
        )
    );
    // The join type is unchanged, so preserved-side rows still appear...
    assert!(forwarded.contains("LEFT JOIN"));
    // ...and no predicate was appended to the outer statement, which is what
    // would have silently converted it to an inner join. The only `WHERE` in
    // the statement is the guard's own.
    assert!(forwarded.ends_with("ON p.id = o.pid"), "{forwarded}");
    assert_eq!(forwarded.matches("WHERE").count(), 1);

    // The same holds with the policy table on the preserved side.
    let forwarded = assert_fully_guarded(
        "SELECT o.id FROM sales.orders o LEFT JOIN sales.products p ON p.id = o.pid",
    );
    assert_eq!(
        forwarded,
        format!(
            "SELECT o.id FROM {ORDERS_GUARD} AS o \
             LEFT JOIN sales.products p ON p.id = o.pid"
        )
    );
}

/// Task 7.5. A correlated `EXISTS` never returns a restricted row, only a
/// boolean about it — which is enough to enumerate the restricted set one probe
/// at a time if the subquery's relation is left unconstrained.
#[test]
fn a_correlated_exists_cannot_infer_the_existence_of_a_restricted_row() {
    let forwarded = assert_fully_guarded(
        "SELECT p.id FROM sales.products p \
         WHERE EXISTS (SELECT 1 FROM sales.orders o WHERE o.pid = p.id AND o.region = 'AMER')",
    );
    assert_eq!(
        forwarded,
        format!(
            "SELECT p.id FROM sales.products p \
             WHERE EXISTS (SELECT 1 FROM {ORDERS_GUARD} AS o WHERE o.pid = p.id AND o.region = 'AMER')"
        )
    );

    // `NOT EXISTS` is the same probe inverted.
    let forwarded = assert_fully_guarded(
        "SELECT p.id FROM sales.products p \
         WHERE NOT EXISTS (SELECT 1 FROM sales.orders o WHERE o.pid = p.id)",
    );
    assert_eq!(guard_count(&forwarded), 1);
}

/// A CTE whose body reads the policy table, used several times, is constrained
/// once at the body and therefore at every use.
#[test]
fn a_cte_cannot_launder_restricted_rows_to_its_uses() {
    let forwarded = assert_fully_guarded(
        "WITH c AS (SELECT * FROM sales.orders) SELECT * FROM c x JOIN c y ON x.id = y.id",
    );
    assert_eq!(guard_count(&forwarded), 1, "the CTE body is guarded once");
    assert!(
        forwarded.contains("FROM c x JOIN c y"),
        "the CTE uses should be untouched: {forwarded}"
    );
}

/// The same table reached through a different spelling is still the same table.
#[test]
fn an_alias_or_unqualified_spelling_does_not_evade_the_policy() {
    for sql in [
        "SELECT * FROM sales.orders",
        "SELECT * FROM sales.orders o",
        "SELECT * FROM sales.orders AS anything",
        "SELECT * FROM orders",
        "SELECT * FROM orders AS o",
    ] {
        let forwarded = assert_fully_guarded(sql);
        assert!(
            forwarded.contains("`region` IN ('APAC', 'EMEA')"),
            "{sql:?} escaped the policy: {forwarded}"
        );
    }
}

/// A hand-written subquery shaped like a guard is still descended into, so a
/// *near*-guard cannot be used to stop the verifier short of a real reference.
#[test]
fn a_hand_written_lookalike_does_not_pass_for_a_guard() {
    // The user writes something that resembles the guard but permits more.
    let forwarded =
        assert_fully_guarded("SELECT * FROM (SELECT * FROM sales.orders WHERE 1 = 1) AS orders");
    assert_eq!(
        forwarded,
        format!("SELECT * FROM (SELECT * FROM {ORDERS_GUARD} AS orders WHERE 1 = 1) AS orders")
    );

    // And a statement that was never rewritten does not verify, even though it
    // contains text spelled exactly like a guard around a *second*,
    // unconstrained reference.
    let unrewritten =
        format!("SELECT * FROM {ORDERS_GUARD} AS orders JOIN sales.orders o ON 1 = 1");
    assert!(
        !TestPolicies::analyst().guarded(&unrewritten).unwrap(),
        "verification missed the unguarded second reference"
    );
}

// ---------------------------------------------------------------------------
// Task 7.7 — property test
// ---------------------------------------------------------------------------

/// Statement shapes built around the policy table, in every position design D2
/// claims to cover plus a few it does not.
fn statement_against_the_policy_table() -> impl Strategy<Value = String> {
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

proptest! {
    /// Task 7.7. Every generated statement is either refused or reaches Doris
    /// with every reference to the policy table inside a guard. There is no
    /// third outcome — no statement that is forwarded while still able to read
    /// a row outside the permitted values.
    #[test]
    fn every_generated_statement_is_refused_or_reads_only_permitted_rows(
        sql in statement_against_the_policy_table()
    ) {
        let policies = TestPolicies::analyst();
        match policies.rewrite(&sql) {
            Err(_) => {}
            Ok(forwarded) => {
                prop_assert!(
                    policies.guarded(&forwarded).unwrap(),
                    "forwarded {forwarded:?} still reads sales.orders unconstrained"
                );
                // And the forwarded statement is itself analysable, so the
                // guarantee is not resting on text the proxy cannot re-read.
                prop_assert!(analyze(&forwarded).is_ok());
            }
        }
    }
}
