//! The executable-comment invariant, asserted as a property rather than as
//! plumbing.
//!
//! # The invariant
//!
//! **A statement containing a `/*! … */` gate is either forwarded verbatim or
//! refused. It is never rewritten.**
//!
//! # Why that is the right thing to assert
//!
//! MySQL runs the contents of `/*!NNNNN … */` only when the server version is at
//! least `NNNNN` — a question about the backend that only the backend can
//! answer. `sqlparser` parses the contents as ordinary SQL and throws the gate
//! away, so re-rendering the AST emits the fragment unconditionally: the proxy
//! would have answered that question on the backend's behalf, and answered it
//! "yes" whatever the truth.
//!
//! Forwarding the original text preserves the gate exactly. So the hazard is not
//! executable comments; it is **re-rendering** them, and the rule follows from
//! that rather than from any judgement about how dangerous the syntax is.
//!
//! An earlier version of this file asserted the plumbing — that the scanner was
//! wired into `parse_single`, and that it ran before parsing. Both were true and
//! both stopped being true when the check moved to where rendered text is
//! produced. The property did not change. Assert the property.
//!
//! # The half that makes it non-trivial
//!
//! "Never rewritten" would be satisfied by refusing every executable comment,
//! which is what the proxy used to do — and it made `mysqldump` unusable, since
//! three of its first five statements are `/*! … */`. So the verbatim-forward
//! case is tested alongside, and it is the one that would silently regress if
//! someone re-broadened the rule.

use doris_row_filter_proxy::policy::PolicySet;
use doris_row_filter_proxy::rewrite::rewrite_statement;

const POLICY: &str = r#"
[[policy]]
user = "analyst"
database = "sales"
table = "orders"
column = "region"
permitted_values = ["APAC", "EMEA"]
"#;

fn policies() -> PolicySet {
    PolicySet::from_toml_str(POLICY, "analyze_comment_scanner.rs").expect("valid policy")
}

/// What the proxy would send to Doris, or `None` if it refused.
fn forwarded(sql: &str) -> Option<String> {
    rewrite_statement(sql, "analyst", Some("sales"), &policies()).ok()
}

/// The refusing half: a statement whose gate we would have discarded never goes
/// out.
///
/// Each of these reads a policy-bearing table, so it would be rewritten — and
/// rewriting is exactly what destroys the gate.
#[test]
fn a_statement_carrying_a_gate_is_never_rewritten() {
    for sql in [
        "SELECT /*!99999 id, */ region FROM sales.orders",
        "SELECT /*!50000 1, */ * FROM sales.orders",
        "SELECT * FROM sales.products /*! UNION SELECT * FROM sales.orders */",
        "SELECT * FROM sales.products /*!99999 UNION SELECT * FROM sales.orders */",
        "/*!50000 SELECT * FROM sales.orders */",
    ] {
        assert_eq!(
            forwarded(sql),
            None,
            "{sql:?} would have been rewritten, discarding its version gate"
        );
    }
}

/// The forwarding half, and the reason the rule is narrow rather than a blanket
/// refusal.
///
/// These are `mysqldump`'s first three statements, captured off the wire. None
/// reads a policy table, so none is rewritten, so the gate survives untouched
/// and the backend still decides whether to run it. Refusing them — which the
/// proxy did until this invariant was stated properly — made `mysqldump`
/// unusable at statement one.
#[test]
fn a_statement_carrying_a_gate_is_forwarded_byte_for_byte() {
    for sql in [
        "/*!40100 SET @@SQL_MODE='' */",
        "/*!40103 SET TIME_ZONE='+00:00' */",
        "/*!80000 SET SESSION information_schema_stats_expiry=0 */",
        "SELECT /*!99999 1, */ * FROM sales.products",
    ] {
        assert_eq!(
            forwarded(sql).as_deref(),
            Some(sql),
            "{sql:?} must reach the backend unchanged, gate intact"
        );
    }
}

/// The measurement the whole approach rests on.
///
/// Analysing a gated fragment is safe *because* `sqlparser` parses its contents
/// as real SQL — the walk sees a superset of what may execute, so a gated
/// reference to a policy table is caught even when the gate would have skipped
/// it. If `sqlparser` instead skipped the contents, a gated read of a restricted
/// table would be invisible to the walk and this file's first test would pass
/// for the wrong reason.
///
/// Pinned here rather than left as a claim in a doc comment, because it is the
/// assumption that would fail silently on a dependency upgrade.
#[test]
fn a_gated_reference_to_a_policy_table_is_seen_by_the_walk() {
    // Nothing outside the gate touches a policy table. If the gate's contents
    // were skipped, this would enumerate nothing, be judged unrestricted, and be
    // forwarded — which is the shape of the bug.
    assert_eq!(
        forwarded("SELECT * FROM sales.products /*!99999 UNION SELECT * FROM sales.orders */"),
        None,
        "the gated reference to sales.orders must be visible to the walk"
    );
}

/// Ordinary comments are untouched by any of this. The rule is about the gate,
/// not about comments, and a reader who thinks otherwise will widen or delete it
/// for the wrong reasons.
#[test]
fn an_ordinary_comment_does_not_prevent_rewriting() {
    let rewritten =
        forwarded("SELECT * FROM sales.orders /* an ordinary comment */").expect("not refused");
    assert!(
        rewritten.contains("`region` IN ('APAC', 'EMEA')"),
        "an ordinary comment must not stop the guard being injected: {rewritten}"
    );
}

/// The scanner is byte-wise, which is sound only because every byte of a
/// multi-byte UTF-8 sequence has the high bit set and so cannot collide with the
/// ASCII delimiters it matches. That assumption is invisible in the code and
/// load-bearing: if it were wrong, a gate after any non-ASCII literal would be
/// missed — the fail-open direction, on input a client fully controls.
#[test]
fn a_gate_after_a_non_ascii_literal_is_still_found() {
    for sql in [
        "SELECT 'ordres_é' /*!99999 1, */ * FROM sales.orders",
        "SELECT '日本語' /*!99999 1, */ * FROM sales.orders",
        "SELECT `Ördérs`.* /*!99999 , 1 */ FROM sales.orders",
    ] {
        assert_eq!(
            forwarded(sql),
            None,
            "the gate after the non-ASCII literal in {sql:?} must still be found"
        );
    }
}

/// The converse. A multi-byte sequence must not end a literal early and let its
/// contents be read as a gate — that direction costs compatibility rather than
/// confidentiality, but it is still wrong.
#[test]
fn a_marker_inside_a_non_ascii_literal_is_still_inert() {
    for sql in [
        "SELECT * FROM sales.orders WHERE note = 'é /*!99999 x */'",
        "SELECT * FROM sales.orders WHERE note = '日本語 /*!99999 x */'",
    ] {
        let rewritten = forwarded(sql).unwrap_or_else(|| {
            panic!("{sql:?} must not be refused; the marker is inside a string")
        });
        assert!(
            rewritten.contains("`region` IN ('APAC', 'EMEA')"),
            "{sql:?} should have been rewritten normally: {rewritten}"
        );
    }
}
