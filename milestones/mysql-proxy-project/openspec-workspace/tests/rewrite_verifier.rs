//! The fail-closed re-check (task 5.3), tested as a safety net rather than as a
//! pass-through.
//!
//! # Why this file exists
//!
//! `rewrite_statement` re-analyses its own rendered output and refuses if any
//! policy-bearing reference is not inside a guard. Every other test reaches that
//! check *after* a correct wrapping, so the check always answers "yes" and its
//! answer is never load-bearing. Mutation testing made the consequence concrete:
//! `replace is_guard -> bool with true` **survived** the entire suite. That
//! mutant makes the verifier treat every derived table as a guard, which is
//! invisible while the wrapper works and catastrophic the moment it does not —
//! precisely the situation the re-check exists for.
//!
//! So these tests drive the verifier directly, with SQL in which a policy
//! reference is deliberately *not* guarded. They assert the net catches what the
//! wrapper would have missed. A test here failing means the last line of defence
//! is gone, not that a rewrite is cosmetically different.

#[path = "rewrite_fixture.rs"]
mod fixture;

/// The verifier's answer for `sql` as `analyst`, failing the test if the SQL
/// could not be analysed at all — an analysis failure would refuse the statement
/// anyway, so it is not the case under test here.
fn guarded(sql: &str) -> bool {
    fixture::TestPolicies::analyst()
        .guarded(sql)
        .expect("statement should be analysable")
}

#[test]
fn a_correctly_wrapped_statement_is_recognised_as_guarded() {
    // The control. Without it, every assertion below could pass because the
    // verifier answers "not guarded" to everything.
    assert!(
        guarded(&fixture::forwarded("SELECT * FROM sales.orders")),
        "a statement the rewriter produced must pass its own re-check"
    );
}

#[test]
fn an_unwrapped_policy_reference_is_not_guarded() {
    assert!(
        !guarded("SELECT * FROM sales.orders"),
        "a bare policy-bearing reference must not be accepted as guarded"
    );
}

#[test]
fn a_policy_reference_inside_a_plain_derived_table_is_not_guarded() {
    // Kills `replace is_guard -> bool with true`: with that mutant the verifier
    // stops at the derived table, never sees the reference inside it, and
    // reports the statement fully guarded.
    assert!(
        !guarded("SELECT * FROM (SELECT * FROM sales.orders) AS t"),
        "a derived table is not a guard merely by being a derived table"
    );
}

#[test]
fn a_derived_table_with_the_wrong_permitted_values_is_not_guarded() {
    // Near-miss: correct shape, wrong predicate. Accepting this would let a
    // user supply their own widened predicate and have it mistaken for ours.
    assert!(
        !guarded("SELECT * FROM (SELECT * FROM sales.orders WHERE region IN ('AMER')) AS orders"),
        "a guard-shaped subquery with different permitted values is not a guard"
    );
}

#[test]
fn a_derived_table_over_the_wrong_column_is_not_guarded() {
    assert!(
        !guarded("SELECT * FROM (SELECT * FROM sales.orders WHERE owner IN ('APAC')) AS orders"),
        "a guard-shaped subquery over a different column is not a guard"
    );
}

#[test]
fn a_guard_carrying_an_extra_clause_is_not_guarded() {
    // The whole `Query` is compared, so anything the guard would not have
    // produced must fail recognition and be descended into.
    assert!(
        !guarded(
            "SELECT * FROM (SELECT * FROM sales.orders WHERE region IN ('APAC', 'EMEA') LIMIT 1) AS orders"
        ),
        "a guard with an extra clause is not the guard the rewriter builds"
    );
}

#[test]
fn an_unguarded_reference_beneath_a_real_guard_is_still_found() {
    // A correct guard must not shelter an unguarded sibling: recognition stops
    // the walk inside *that* subquery only.
    let sql = format!(
        "SELECT * FROM {} AS orders JOIN (SELECT * FROM sales.orders) AS t ON 1 = 1",
        fixture::ORDERS_GUARD
    );
    assert!(
        !guarded(&sql),
        "recognising one guard must not stop the walk reaching an unguarded reference elsewhere"
    );
}

#[test]
fn an_insert_target_on_a_policy_table_is_not_guarded() {
    // The statement that actually reaches `Verifier::visit_write_target`.
    //
    // Which write statement is used here is not interchangeable, and the
    // distinction is invisible from this file. `walk_insert` hands the target
    // to `visit_write_target` and never walks it as a relation, so an `INSERT`
    // is the only shape whose refusal depends on that method. Writes are
    // refused earlier in `rewrite_statement`, so this path is otherwise
    // unexercised — and it is what would matter if that earlier refusal were
    // ever relaxed.
    assert!(
        !guarded("INSERT INTO sales.orders (id, region) VALUES (1, 'AMER')"),
        "an INSERT into a policy-bearing table must never be reported as guarded"
    );
}

#[test]
fn an_update_target_on_a_policy_table_is_not_guarded() {
    // `walk_update` walks its target through `walk_table_with_joins`, so this
    // reaches `visit_relation` rather than `visit_write_target` — a different
    // path to the same refusal. Kept alongside the `INSERT` case because the
    // two are easy to mistake for each other, and because a change to
    // `walk_update` that routed the target to `visit_write_target` instead
    // should not silently leave this shape untested.
    assert!(
        !guarded("UPDATE sales.orders SET total = 1"),
        "an UPDATE against a policy-bearing table must never be reported as guarded"
    );
}

#[test]
fn a_table_with_no_policy_for_this_user_is_guarded_vacuously() {
    // The other direction: the verifier must not refuse statements that carry
    // no policy-bearing reference at all, or every unrestricted query would be
    // rejected once the re-check ran.
    assert!(
        guarded("SELECT * FROM sales.products"),
        "a statement touching no policy-bearing table has nothing to guard"
    );
}

// ---------------------------------------------------------------------------
// `is_guard`'s early exits
//
// The four near-miss tests above all fail recognition at the *final* equality
// comparison. `is_guard` returns `false` from seven places, and each earlier one
// is a separate opportunity for a mutant to report "this is a guard" about a
// subquery that is not one — which stops the walk and hides whatever reference
// is inside. These reach the earlier exits instead.
// ---------------------------------------------------------------------------

#[test]
fn a_derived_table_whose_body_is_a_set_operation_is_not_guarded() {
    // Fails at the `SetExpr::Select` binding: a guard's body is a single
    // `SELECT`, never a union.
    assert!(
        !guarded(
            "SELECT * FROM (SELECT * FROM sales.orders UNION ALL SELECT * FROM sales.orders) AS orders"
        ),
        "a union inside a derived table is not a guard"
    );
}

#[test]
fn a_derived_table_over_two_relations_is_not_guarded() {
    // Fails at the single-element slice pattern. Two comma-separated relations
    // means one of them could be anything at all.
    assert!(
        !guarded("SELECT * FROM (SELECT * FROM sales.orders, sales.products) AS orders"),
        "a derived table reading two relations is not a guard"
    );
}

#[test]
fn a_derived_table_containing_a_join_is_not_guarded() {
    // Fails at `!joins.is_empty()`. A guard has exactly one relation and no
    // joins; a join could widen what the subquery returns.
    assert!(
        !guarded(
            "SELECT * FROM (SELECT * FROM sales.orders JOIN sales.products ON 1 = 1) AS orders"
        ),
        "a join inside a derived table is not a guard"
    );
}

#[test]
fn a_derived_table_wrapping_another_derived_table_is_not_guarded() {
    // Fails at the `TableFactor::Table` binding: a guard wraps a named table,
    // not another subquery.
    assert!(
        !guarded("SELECT * FROM (SELECT * FROM (SELECT * FROM sales.orders) AS x) AS orders"),
        "nesting a derived table inside another is not a guard"
    );
}

#[test]
fn a_derived_table_whose_relation_is_aliased_is_not_guarded() {
    // Fails at `alias.is_some()`. A real guard strips the alias off the inner
    // relation and puts it on the wrapper, so an alias surviving inside means
    // this subquery was not built by `guard`.
    //
    // This exit is load-bearing in a way the others are not: it sits *before*
    // the whole-query equality comparison, so a mutant that returns `true` here
    // accepts the subquery without ever checking its predicate. The statement
    // below has no predicate at all, and would be waved through.
    assert!(
        !guarded("SELECT * FROM (SELECT * FROM sales.orders o) AS orders"),
        "an alias on the inner relation means this is not the guard we built"
    );
}
