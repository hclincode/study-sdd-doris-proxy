//! Matching a table reference *as it was written in SQL* to a policy.
//!
//! Covers `specs/policy-config`, requirement "Table identity in configuration is
//! unambiguous". The references here are taken from statements parsed with the
//! same parser the proxy uses, so the qualified/unqualified/aliased spellings
//! under test are real spellings rather than hand-built structs.

use doris_row_filter_proxy::policy::{PolicyDecision, PolicySet, TableRef};
use sqlparser::ast::{ObjectName, Query, SetExpr, Statement, TableFactor};
use sqlparser::dialect::MySqlDialect;
use sqlparser::parser::Parser;

const CONFIG: &str = r#"
[[policy]]
user = "analyst"
database = "sales"
table = "orders"
column = "region"
permitted_values = ["APAC", "EMEA"]
"#;

fn policies() -> PolicySet {
    PolicySet::from_toml_str(CONFIG, "policy_lookup.rs").expect("valid configuration")
}

/// The first table reference in a `SELECT`, plus the alias it was given.
///
/// Deliberately minimal — this stands in for the enumeration the rewriter will
/// do, only far enough to produce one reference to look up.
fn first_reference(sql: &str) -> (TableRef, Option<String>) {
    let statements = Parser::parse_sql(&MySqlDialect {}, sql).expect("statement parses");
    let [Statement::Query(query)] = statements.as_slice() else {
        panic!("expected exactly one query");
    };
    let Query { body: set_expr, .. } = query.as_ref();
    let SetExpr::Select(select) = set_expr.as_ref() else {
        panic!("expected a plain SELECT");
    };
    let relation = &select.from.first().expect("a FROM clause").relation;
    let TableFactor::Table { name, alias, .. } = relation else {
        panic!("expected a plain table reference");
    };

    (
        table_ref(name),
        alias.as_ref().map(|alias| alias.name.value.clone()),
    )
}

fn table_ref(name: &ObjectName) -> TableRef {
    let parts: Vec<&str> = name
        .0
        .iter()
        .map(|part| part.as_ident().expect("a plain identifier").value.as_str())
        .collect();

    match parts.as_slice() {
        [table] => TableRef::unqualified(*table),
        [database, table] => TableRef::qualified(*database, *table),
        other => panic!("unexpected object name with {} parts", other.len()),
    }
}

#[test]
fn qualified_reference_matches_the_qualified_policy() {
    let (reference, _) = first_reference("SELECT * FROM sales.orders");

    assert!(policies()
        .lookup("analyst", &reference, None)
        .is_restricted());
}

#[test]
fn unqualified_reference_resolves_to_the_qualified_policy() {
    let (reference, _) = first_reference("SELECT * FROM orders");

    assert_eq!(reference, TableRef::unqualified("orders"));
    assert!(policies()
        .lookup("analyst", &reference, Some("sales"))
        .is_restricted());
}

#[test]
fn unqualified_reference_under_another_current_database_is_not_matched() {
    let (reference, _) = first_reference("SELECT * FROM orders");

    assert_eq!(
        policies().lookup("analyst", &reference, Some("staging")),
        PolicyDecision::Unrestricted
    );
}

#[test]
fn same_named_table_in_another_database_is_not_matched() {
    let (reference, _) = first_reference("SELECT * FROM staging.orders");

    assert_eq!(
        policies().lookup("analyst", &reference, Some("sales")),
        PolicyDecision::Unrestricted
    );
}

#[test]
fn aliased_reference_is_still_matched() {
    let (reference, alias) = first_reference("SELECT o.total FROM sales.orders AS o");

    assert_eq!(alias.as_deref(), Some("o"));
    assert!(
        policies()
            .lookup("analyst", &reference, None)
            .is_restricted(),
        "an alias renames the relation, not the table"
    );
}

#[test]
fn aliased_unqualified_reference_is_matched_under_the_right_current_database() {
    let (reference, alias) = first_reference("SELECT o.total FROM orders o");

    assert_eq!(alias.as_deref(), Some("o"));
    assert!(policies()
        .lookup("analyst", &reference, Some("sales"))
        .is_restricted());
    assert_eq!(
        policies().lookup("analyst", &reference, Some("staging")),
        PolicyDecision::Unrestricted
    );
}

#[test]
fn backtick_quoted_reference_is_matched() {
    // Quoting is a spelling of the same table, and must not evade the policy.
    let (reference, _) = first_reference("SELECT * FROM `sales`.`orders`");

    assert!(policies()
        .lookup("analyst", &reference, None)
        .is_restricted());
}

#[test]
fn case_respelled_reference_is_matched() {
    let (reference, _) = first_reference("SELECT * FROM SALES.Orders");

    assert!(policies()
        .lookup("analyst", &reference, None)
        .is_restricted());
}

#[test]
fn unqualified_reference_with_no_current_database_is_unresolvable() {
    // Not "unrestricted": this reference might be sales.orders, and the proxy
    // cannot show that it is not. The rewriter must reject the statement.
    let (reference, _) = first_reference("SELECT * FROM orders");

    assert_eq!(
        policies().lookup("analyst", &reference, None),
        PolicyDecision::Unresolvable
    );
}

#[test]
fn an_unrestricted_user_is_never_unresolvable() {
    let (reference, _) = first_reference("SELECT * FROM orders");

    assert_eq!(
        policies().lookup("reporting", &reference, None),
        PolicyDecision::Unrestricted
    );
}
