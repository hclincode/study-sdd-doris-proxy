//! The executable-comment scanner, tested over its decision surface rather than
//! through a handful of whole statements.
//!
//! # Why this file exists
//!
//! `contains_executable_comment` decides whether a statement is refused, and it
//! runs *before* parsing because the version gate is discarded the moment
//! `sqlparser` builds an AST. It is therefore a hand-written byte scanner in the
//! refusal path — the least forgiving place to have an untested state machine.
//!
//! Mutation testing found **22 survivors** in it. Not one of them is an exotic
//! branch: they include deleting the `#` line-comment arm entirely, forcing the
//! `--` guard to `false`, forcing the `/*` guard to `true`, and a long list of
//! corruptions of the index cursor. The four existing tests in
//! `analyze_walk.rs` pin the verdict for a handful of inputs, so almost any
//! internal step can be broken without a test noticing.
//!
//! The lesson from the rest of this exercise applies here in a specific form:
//! a scanner exercised only through statements that *should* be refused will
//! not notice a branch that has stopped refusing correctly. So every case below
//! is stated as a pair — what the scanner must say, and why — and the file is
//! deliberately half negative.
//!
//! # What is being asserted
//!
//! Only the scanner's verdict, isolated from parsing: whether the refusal is
//! `UnsupportedShape { construct: "conditionally-executed comment" }`. Several
//! inputs here are not valid SQL at all, which is fine and deliberate — the
//! scanner runs first, so its answer must be right for input the parser would
//! reject.

use doris_row_filter_proxy::analyze::parse_single;
use doris_row_filter_proxy::error::RefusalReason;

/// Whether the scanner refused `sql` for carrying an executable comment.
///
/// Distinguished from every other refusal: a statement rejected as unparseable
/// or as multi-statement tells us nothing about the scanner, and treating any
/// error as agreement would let this whole file pass on inputs the scanner
/// never examined.
fn refused_as_executable_comment(sql: &str) -> bool {
    matches!(
        parse_single(sql),
        Err(RefusalReason::UnsupportedShape { ref construct })
            if construct == "conditionally-executed comment"
    )
}

fn assert_refused(cases: &[(&str, &str)]) {
    for (sql, why) in cases {
        assert!(
            refused_as_executable_comment(sql),
            "expected {sql:?} to be refused as an executable comment ({why})"
        );
    }
}

fn assert_not_refused(cases: &[(&str, &str)]) {
    for (sql, why) in cases {
        assert!(
            !refused_as_executable_comment(sql),
            "expected {sql:?} NOT to be refused as an executable comment ({why})"
        );
    }
}

#[test]
fn a_marker_in_code_position_is_refused() {
    assert_refused(&[
        ("/*!99999 SELECT 1 */", "at the very start of the input"),
        ("SELECT /*!99999 id, */ region FROM t", "mid-statement"),
        ("SELECT 1 /*!", "truncated marker at the very end"),
        ("SELECT * FROM t /*!40101 UNION SELECT 1 */", "trailing"),
        ("/*!", "marker is the entire input"),
    ]);
}

#[test]
fn a_marker_after_a_closed_string_is_refused() {
    // Each closing delimiter must return the scanner to code position. If any
    // one of them fails to, a marker after a perfectly ordinary literal is
    // missed — the fail-open direction.
    assert_refused(&[
        ("SELECT 'a' /*!99999 1 */", "after a single-quoted string"),
        ("SELECT \"a\" /*!99999 1 */", "after a double-quoted string"),
        ("SELECT `a` /*!99999 1 */", "after a backtick identifier"),
        (
            "SELECT 'it''s' /*!99999 1 */",
            "after a string containing a doubled quote",
        ),
        ("SELECT '' /*!99999 1 */", "after an empty string"),
        ("SELECT 'a' 'b' /*!99999 1 */", "after two adjacent strings"),
    ]);
}

#[test]
fn a_marker_after_a_closed_comment_is_refused() {
    assert_refused(&[
        (
            "SELECT 1 # comment\n/*!99999 2 */",
            "after a # line comment ends at the newline",
        ),
        (
            "SELECT 1 -- comment\n/*!99999 2 */",
            "after a -- line comment ends at the newline",
        ),
        (
            "SELECT 1 /* comment */ /*!99999 2 */",
            "after a block comment closes",
        ),
        (
            "SELECT 1 /* a */ /* b */ /*!99999 2 */",
            "after two block comments",
        ),
    ]);
}

#[test]
fn a_double_minus_without_following_whitespace_is_not_a_comment() {
    // MySQL's own rule: `--` starts a comment only when whitespace follows.
    // Treating it unconditionally would skip the rest of the line and miss the
    // marker — which is why the guard exists and why it is worth pinning from
    // both sides.
    assert_refused(&[
        (
            "SELECT 1--2 /*!99999 3 */",
            "`--2` is an operator, not a comment",
        ),
        (
            "SELECT 1--1 /*!99999 2 */",
            "double negation, not a comment",
        ),
    ]);
    assert_not_refused(&[
        (
            "SELECT 1 -- /*!99999 2 */",
            "`-- ` with a space does start a comment",
        ),
        (
            "SELECT 1 --\t/*!99999 2 */",
            "a tab counts as whitespace after --",
        ),
    ]);
}

#[test]
fn a_marker_inside_a_string_is_not_an_executable_comment() {
    assert_not_refused(&[
        ("SELECT '/*!99999 x */'", "inside a single-quoted string"),
        ("SELECT \"/*!99999 x */\"", "inside a double-quoted string"),
        ("SELECT `/*!99999 x */`", "inside a backtick identifier"),
        (
            "SELECT 'it''s /*!99999 x */'",
            "after a doubled quote, still inside the same string",
        ),
    ]);
}

#[test]
fn a_marker_inside_a_comment_is_not_an_executable_comment() {
    assert_not_refused(&[
        ("SELECT 1 # /*!99999 x */", "inside a # line comment"),
        ("SELECT 1 -- /*!99999 x */", "inside a -- line comment"),
        (
            "SELECT 1 /* /*!99999 x */",
            "inside a block comment — MySQL does not nest them",
        ),
    ]);
}

#[test]
fn ordinary_input_is_not_refused() {
    // The control. Without it every negative assertion above could pass because
    // the scanner refuses nothing at all.
    assert_not_refused(&[
        ("SELECT 1", "no comment of any kind"),
        ("SELECT /* plain */ 1", "an ordinary block comment"),
        ("SELECT 1 -- plain", "an ordinary line comment"),
        ("SELECT 1 # plain", "an ordinary # comment"),
        ("SELECT 1 / 2", "a division operator, not a comment opener"),
        ("SELECT 'a*/b'", "a comment closer inside a string"),
    ]);
}

#[test]
fn truncated_and_empty_input_does_not_panic_or_false_positive() {
    // The scanner indexes ahead by one and two bytes in several places. These
    // inputs end in the middle of every such lookahead. Invariant 2 forbids a
    // panic on client input, and a false positive here would refuse ordinary
    // fragments.
    assert_not_refused(&[
        ("", "empty input"),
        (" ", "whitespace only"),
        ("/", "a lone slash"),
        ("/*", "an unterminated block comment"),
        ("-", "a lone minus"),
        ("--", "a bare double minus at end of input"),
        ("#", "a bare hash at end of input"),
        ("SELECT 'abc", "an unterminated string"),
        ("SELECT `abc", "an unterminated identifier"),
        (
            "SELECT /* abc",
            "an unterminated block comment with content",
        ),
        ("SELECT 'a", "an unterminated string of one character"),
        ("*/", "a stray comment closer"),
    ]);
}

#[test]
fn a_marker_is_not_found_inside_an_unterminated_construct() {
    // An unterminated string or comment swallows the rest of the input, so a
    // marker after it is genuinely inside it. The scanner must not "recover"
    // and start refusing — that would be a false positive on input the parser
    // will reject anyway, and it would mask the real error.
    assert_not_refused(&[
        ("SELECT 'abc /*!99999 1 */", "inside an unterminated string"),
        (
            "SELECT /* abc /*!99999 1 */",
            "inside an unterminated block comment, which the marker does not close",
        ),
        (
            "SELECT # abc /*!99999 1 */",
            "inside a # comment to end of input",
        ),
    ]);
}

#[test]
fn multi_byte_characters_do_not_shift_the_scanner() {
    // The scanner works byte-wise, which is safe only because every byte of a
    // multi-byte UTF-8 sequence has the high bit set and so cannot collide with
    // the ASCII delimiters. If that assumption were wrong, a marker after a
    // non-ASCII literal would be missed.
    assert_refused(&[
        (
            "SELECT 'ordres_é' /*!99999 1 */",
            "after a non-ASCII string",
        ),
        (
            "SELECT `Ördérs` /*!99999 1 */",
            "after a non-ASCII identifier",
        ),
        (
            "SELECT '日本語' /*!99999 1 */",
            "after a multi-byte CJK string",
        ),
    ]);
    assert_not_refused(&[(
        "SELECT 'é /*!99999 x */'",
        "a marker inside a non-ASCII string is still inside it",
    )]);
}
