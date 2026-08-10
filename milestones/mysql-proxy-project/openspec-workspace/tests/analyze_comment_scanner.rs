//! Two things the scanner's own unit tests cannot cover from inside the module.
//!
//! # Where the decision surface is tested
//!
//! **Not here.** `contains_executable_comment` is private, and its verdicts are
//! pinned exhaustively by `executable_comment_scanner_decision_surface` and
//! `executable_comment_scanner_never_panics_on_partial_input` in
//! `src/analyze.rs` — roughly fifty input/expected rows reaching the function
//! directly. That is the right level for a state machine, and this file
//! deliberately does not duplicate it: two test sets over one decision surface
//! means two places to update and no way for a reader to tell which is
//! authoritative.
//!
//! What is left is the part those unit tests are structurally unable to assert.
//!
//! # 1. The wiring
//!
//! A unit test can prove the scanner returns `true`. It cannot prove that
//! `parse_single` *calls* it, that the call happens before parsing, or that the
//! refusal a client sees names this construct rather than some other. Those are
//! claims about the seam, and a scanner that is correct but unwired refuses
//! nothing at all.
//!
//! # 2. Multi-byte input
//!
//! The scanner is byte-wise, which is sound only because every byte of a
//! multi-byte UTF-8 sequence has the high bit set and so cannot collide with the
//! ASCII delimiters it matches. That assumption is invisible in the code and
//! load-bearing: if it were wrong, a marker after any non-ASCII literal would be
//! missed — the fail-open direction, on input a client fully controls.

use doris_row_filter_proxy::analyze::parse_single;
use doris_row_filter_proxy::error::RefusalReason;

/// Whether `sql` was refused *specifically* for carrying an executable comment.
///
/// The construct string is the point: a statement refused as unparseable or as
/// multi-statement would also be an `Err`, and treating any error as agreement
/// would let this file pass on input the scanner never examined.
fn refused_as_executable_comment(sql: &str) -> bool {
    matches!(
        parse_single(sql),
        Err(RefusalReason::UnsupportedShape { ref construct })
            if construct == "conditionally-executed comment"
    )
}

#[test]
fn the_scanner_is_wired_into_parse_single() {
    assert!(
        refused_as_executable_comment("SELECT /*!99999 id, */ region FROM t"),
        "parse_single must consult the scanner and surface its refusal"
    );
}

#[test]
fn the_scanner_runs_before_parsing() {
    // Input that is not valid SQL and also carries a marker. If parsing ran
    // first this would come back `Unparseable`, and the executable comment —
    // the more specific and more actionable diagnosis — would be lost.
    assert!(
        refused_as_executable_comment("SELECT FROM WHERE /*!99999 1 */"),
        "the scanner must run before the parser, or its verdict is unreachable \
         for any statement the parser also rejects"
    );
}

#[test]
fn an_ordinary_statement_reaches_the_parser() {
    // The control for both tests above: without it, they would pass just as
    // well if `parse_single` refused everything.
    assert!(
        parse_single("SELECT 1").is_ok(),
        "a statement with no executable comment must not be refused"
    );
}

#[test]
fn a_marker_after_a_non_ascii_literal_is_still_found() {
    // The byte-wise assumption, exercised in the direction that fails open.
    for sql in [
        "SELECT 'ordres_é' /*!99999 1 */",
        "SELECT `Ördérs` /*!99999 1 */",
        "SELECT '日本語' /*!99999 1 */",
        "SELECT 'é' /*!99999 1 */",
    ] {
        assert!(
            refused_as_executable_comment(sql),
            "a marker after the non-ASCII literal in {sql:?} must still be found"
        );
    }
}

#[test]
fn a_marker_inside_a_non_ascii_literal_is_still_inert() {
    // The converse. A multi-byte sequence must not end a literal early and let
    // its contents be read as code — the fail-closed direction, which costs
    // compatibility rather than confidentiality but is still wrong.
    // Asserted as `is_ok`, not as "not refused for this reason". The weaker
    // form is also satisfied by a refusal for some *other* reason, so it would
    // keep passing if one of these inputs stopped parsing — accidentally
    // correct rather than robust, and one `sqlparser` upgrade from silently
    // testing nothing.
    for sql in [
        "SELECT 'é /*!99999 x */'",
        "SELECT '日本語 /*!99999 x */'",
        "SELECT `Ördérs /*!99999 x */`",
    ] {
        assert!(
            parse_single(sql).is_ok(),
            "the marker inside the non-ASCII literal in {sql:?} must stay inert, \
             and the statement must still parse"
        );
    }
}
