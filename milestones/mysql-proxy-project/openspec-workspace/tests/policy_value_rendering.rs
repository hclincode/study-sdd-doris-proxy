//! How a configured permitted value becomes a SQL literal, and which values are
//! refused because they would not survive the trip.
//!
//! Covers `specs/policy-config`, requirement "Permitted values must survive
//! transmission unaltered".
//!
//! # Two halves, and the second one pins a hazard
//!
//! The first half checks values that load, and that they render as the operator
//! wrote them. The second half checks that the values which would *not* render
//! faithfully are rejected at load — and then, separately, pins what
//! `sqlparser`'s renderer would have done with them.
//!
//! Those pinning assertions are of behaviour that is **wrong at the backend**.
//! They are here so the reason for the load-time rule is written down somewhere
//! executable, and so a `sqlparser` upgrade that changes the escaping breaks the
//! build rather than shipping. They construct `PermittedValue` directly, because
//! such a value can no longer come from a configuration file.
//!
//! The rewriter emits permitted values by mapping `policy::PermittedValue` onto
//! a `sqlparser` `Value` and letting the renderer quote it, so `rendered` below
//! is the real output path — and the only one. `policy.rs` renders no SQL at
//! all, so these assertions pin `sqlparser`'s behaviour rather than ours.

use doris_row_filter_proxy::error::ProxyError;
use doris_row_filter_proxy::policy::{PermittedValue, PolicySet, TableRef};
use sqlparser::ast::{Expr, Value};

/// Render a permitted value the way the rewriter does.
fn rendered(value: &PermittedValue) -> String {
    let literal = match value {
        PermittedValue::Text(text) => Value::SingleQuotedString(text.clone()),
        PermittedValue::Integer(number) => Value::Number(number.to_string(), false),
    };
    Expr::Value(literal.with_empty_span()).to_string()
}

fn config(toml_array: &str) -> String {
    format!(
        r#"
[[policy]]
user = "analyst"
database = "sales"
table = "orders"
column = "region"
permitted_values = {toml_array}
"#
    )
}

/// The permitted values of a configuration that is expected to load.
fn permitted_values(toml_array: &str) -> Vec<PermittedValue> {
    let policies = PolicySet::from_toml_str(&config(toml_array), "policy_value_rendering.rs")
        .expect("valid configuration");
    policies
        .lookup("analyst", &TableRef::qualified("sales", "orders"), None)
        .policy()
        .expect("policy applies")
        .permitted_values()
        .to_vec()
}

/// The diagnostic from a configuration that is expected to be refused.
fn rejection(toml_array: &str) -> String {
    match PolicySet::from_toml_str(&config(toml_array), "policy_value_rendering.rs") {
        Err(ProxyError::Config(message)) => message,
        Err(other) => panic!("expected a configuration error, got {other:?}"),
        Ok(_) => panic!("expected a configuration error, configuration loaded"),
    }
}

// ---------------------------------------------------------------------------
// Values that load, and render as configured.
// ---------------------------------------------------------------------------

#[test]
fn a_plain_value_renders_as_a_quoted_literal() {
    let values = permitted_values(r#"["APAC", "EMEA"]"#);

    assert_eq!(rendered(&values[0]), "'APAC'");
    assert_eq!(rendered(&values[1]), "'EMEA'");
}

#[test]
fn an_integer_value_renders_unquoted() {
    let values = permitted_values("[17, -3]");

    assert_eq!(rendered(&values[0]), "17");
    assert_eq!(rendered(&values[1]), "-3");
}

#[test]
fn an_ordinary_value_containing_a_quote_still_loads_and_renders_correctly() {
    // The spec is explicit that O'Brien must keep working: an isolated quote is
    // doubled correctly, so refusing it would cost function for nothing.
    let values = permitted_values(r#"["O'Brien"]"#);

    assert_eq!(values[0], PermittedValue::Text("O'Brien".into()));
    assert_eq!(rendered(&values[0]), "'O''Brien'");
}

#[test]
fn a_value_cannot_alter_the_structure_of_the_injected_predicate() {
    // The value one would reach for to ask "can configuration inject SQL?".
    // It cannot: the quote is doubled, so the literal stays closed and the
    // trailing punctuation is data.
    let values = permitted_values(r#"["a')--"]"#);

    assert_eq!(rendered(&values[0]), "'a'')--'");
}

// ---------------------------------------------------------------------------
// Values refused at load, because they would not survive rendering.
// ---------------------------------------------------------------------------

#[test]
fn a_value_containing_a_backslash_is_rejected() {
    let message = rejection(r#"["a\\b"]"#);

    assert!(message.contains("contains a backslash"), "{message}");
    assert!(message.contains("policy #1"), "{message}");
    assert!(message.contains("sales.orders"), "{message}");
}

#[test]
fn a_value_ending_in_a_backslash_is_rejected() {
    // The worst of them: rendered as 'a\', the literal would never terminate.
    let message = rejection(r#"["a\\"]"#);

    assert!(message.contains("contains a backslash"), "{message}");
}

#[test]
fn a_value_containing_a_doubled_quote_is_rejected() {
    let message = rejection(r#"["a''b"]"#);

    assert!(
        message.contains("two consecutive single quotes"),
        "{message}"
    );
    assert!(
        message.contains("O'Brien"),
        "the diagnostic should say which quoted values are fine: {message}"
    );
}

// ---------------------------------------------------------------------------
// Below: what the renderer WOULD do with the refused values. Asserted behaviour
// that is wrong at the backend — this is why the rule above exists.
// ---------------------------------------------------------------------------

#[test]
fn the_renderer_would_not_escape_a_backslash() {
    // HAZARD, unreachable from configuration. Rendered as 'a\b'; under MySQL's
    // default sql_mode `\b` is a backspace escape, so the permitted value would
    // become "a" + 0x08 rather than the configured "a\b" — a widening if any
    // row holds the transformed value.
    let value = PermittedValue::Text("a\\b".into());

    assert_eq!(rendered(&value), "'a\\b'");
    assert_ne!(
        rendered(&value),
        "'a\\\\b'",
        "if this fails, sqlparser started escaping backslashes — revisit the \
         load-time rejection in src/policy.rs and the module docs"
    );
}

#[test]
fn the_renderer_would_leave_a_value_ending_in_a_backslash_unterminated() {
    let value = PermittedValue::Text("a\\".into());

    assert_eq!(rendered(&value), "'a\\'");
}

#[test]
fn the_renderer_would_leave_an_already_doubled_quote_alone() {
    // sqlparser treats a pre-existing '' as already escaped, so the configured
    // four-character "a''b" would render as 'a''b' and be read back as "a'b".
    let value = PermittedValue::Text("a''b".into());

    assert_eq!(rendered(&value), "'a''b'");
}

// `to_sql_literal_escapes_differently_from_the_renderer` lived here. It guarded
// against using `PermittedValue::to_sql_literal` to build AST literals, which
// would have double-escaped. That method has been deleted, so the mistake is no
// longer possible and a guard against it would only puzzle a later reader. The
// HAZARD tests above stay: they pin `sqlparser`, which is still here.
