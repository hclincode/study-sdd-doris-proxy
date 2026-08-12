//! Statement shape analysis for row-filter injection.
//!
//! This is not a parser. It recognizes one narrow statement shape — a `SELECT`
//! reading exactly one table — and declines everything else. That asymmetry is
//! the point: a construct the scanner has never seen is skipped rather than
//! rewritten approximately, so new or exotic syntax can cost coverage but never
//! correctness.
//!
//! Every byte offset returned here is copied from a token's recorded span,
//! never computed by arithmetic on one. That is what guarantees a splice cannot
//! land inside a string literal or a comment: the tokenizer already decided
//! where those are, and a clause keyword appearing inside one is not a token of
//! that kind.

use super::tokenizer::{tokenize, Token, TokenKind};

/// Why a statement was not rewritten, when a rewrite might have been wanted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    NotSelect,
    MultipleTables,
    UnsupportedStructure,
    MultipleStatements,
    TokenizeFailed,
    NoInsertionPoint,
    /// Raised by the splicer rather than here.
    PacketCountChanged,
}

impl SkipReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            SkipReason::NotSelect => "not_select",
            SkipReason::MultipleTables => "multiple_tables",
            SkipReason::UnsupportedStructure => "unsupported_structure",
            SkipReason::MultipleStatements => "multiple_statements",
            SkipReason::TokenizeFailed => "tokenize_failed",
            SkipReason::NoInsertionPoint => "no_insertion_point",
            SkipReason::PacketCountChanged => "packet_count_changed",
        }
    }
}

/// A table as written in the statement, normalized for rule lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableRef {
    pub schema: Option<String>,
    pub name: String,
}

/// Byte offsets bounding an existing `WHERE` clause.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WhereExtent {
    /// Just after the `WHERE` keyword.
    pub after_keyword: usize,
    /// Where the clause ends: the next top-level clause keyword, or the end of
    /// the statement body.
    pub end: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Analysis {
    pub table: TableRef,
    /// Where a new `WHERE` clause would be inserted.
    pub insert_at: usize,
    pub where_clause: Option<WhereExtent>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The statement can be rewritten.
    Eligible(Analysis),
    /// Nothing to filter — no table is read, so no rule could apply. Not a skip.
    NoTable,
    Skip(SkipReason),
}

/// Keywords that may follow the table reference and therefore bound a `WHERE`
/// clause. `WHERE` itself is included so the scan can find it.
const CLAUSE_KEYWORDS: &[&str] = &[
    "WHERE", "GROUP", "HAVING", "WINDOW", "ORDER", "LIMIT", "OFFSET", "PROCEDURE", "INTO",
    "FOR", "LOCK", "UNION", "INTERSECT", "EXCEPT",
];

/// Words that mean the `FROM` clause names more than one table.
const JOIN_KEYWORDS: &[&str] = &[
    "JOIN", "INNER", "LEFT", "RIGHT", "CROSS", "FULL", "NATURAL", "STRAIGHT_JOIN",
];

/// Words that may follow a table reference but that this scanner does not
/// model, such as index hints. Their presence means "give up", not "alias".
const POST_TABLE_UNSUPPORTED: &[&str] = &["USE", "FORCE", "IGNORE", "PARTITION", "TABLESAMPLE"];

fn word_eq(src: &[u8], token: &Token, want: &str) -> bool {
    token.kind == TokenKind::Word && token.text(src).eq_ignore_ascii_case(want.as_bytes())
}

fn word_in(src: &[u8], token: &Token, set: &[&str]) -> bool {
    token.kind == TokenKind::Word && set.iter().any(|w| token.text(src).eq_ignore_ascii_case(w.as_bytes()))
}

fn is_punct(src: &[u8], token: &Token, want: u8) -> bool {
    token.kind == TokenKind::Punct && token.text(src) == [want]
}

/// Strips backticks and lowercases an identifier for rule matching.
fn normalize_ident(raw: &[u8]) -> String {
    let text = String::from_utf8_lossy(raw);
    let trimmed = text.trim_matches('`').replace("``", "`");
    trimmed.to_ascii_lowercase()
}

/// Significant tokens paired with the parenthesis depth at which each appears.
///
/// The markers around a version-gated comment are dropped: their contents are
/// real SQL and are scanned like any other tokens, but the markers themselves
/// are not keywords.
fn significant(src: &[u8], tokens: &[Token]) -> Vec<(Token, usize)> {
    let mut out = Vec::with_capacity(tokens.len());
    let mut depth = 0usize;
    for t in tokens {
        match t.kind {
            TokenKind::Whitespace
            | TokenKind::Comment
            | TokenKind::ExecCommentOpen
            | TokenKind::ExecCommentClose => continue,
            TokenKind::Punct => {
                if is_punct(src, t, b'(') {
                    out.push((*t, depth));
                    depth += 1;
                    continue;
                }
                if is_punct(src, t, b')') {
                    depth = depth.saturating_sub(1);
                    out.push((*t, depth));
                    continue;
                }
                out.push((*t, depth));
            }
            _ => out.push((*t, depth)),
        }
    }
    out
}

/// Analyzes a statement and reports whether a predicate can be injected.
pub fn analyze(sql: &[u8]) -> Outcome {
    let Ok(tokens) = tokenize(sql) else {
        return Outcome::Skip(SkipReason::TokenizeFailed);
    };
    let toks = significant(sql, &tokens);
    if toks.is_empty() {
        return Outcome::NoTable;
    }

    // A leading parenthesis introduces a parenthesized query block, and `WITH`
    // introduces a common table expression. Neither is a shape this handles.
    if is_punct(sql, &toks[0].0, b'(') || word_eq(sql, &toks[0].0, "WITH") {
        return Outcome::Skip(SkipReason::UnsupportedStructure);
    }
    if !word_eq(sql, &toks[0].0, "SELECT") {
        return Outcome::Skip(SkipReason::NotSelect);
    }

    // A trailing separator is part of this statement; anything after one means
    // the command carries more than a single statement.
    let mut body: &[(Token, usize)] = &toks;
    if let Some(pos) = toks.iter().position(|(t, d)| *d == 0 && is_punct(sql, t, b';')) {
        if pos + 1 < toks.len() {
            return Outcome::Skip(SkipReason::MultipleStatements);
        }
        body = &toks[..pos];
    }

    // Any subquery at all, anywhere.
    for pair in body.windows(2) {
        if is_punct(sql, &pair[0].0, b'(')
            && (word_eq(sql, &pair[1].0, "SELECT") || word_eq(sql, &pair[1].0, "WITH"))
        {
            return Outcome::Skip(SkipReason::UnsupportedStructure);
        }
    }

    // Set operators at the top level put the tables beyond one reference.
    if body.iter().any(|(t, d)| {
        *d == 0 && word_in(sql, t, &["UNION", "INTERSECT", "EXCEPT"])
    }) {
        return Outcome::Skip(SkipReason::UnsupportedStructure);
    }

    let from_positions: Vec<usize> = body
        .iter()
        .enumerate()
        .filter(|(_, (t, d))| *d == 0 && word_eq(sql, t, "FROM"))
        .map(|(i, _)| i)
        .collect();
    match from_positions.len() {
        0 => return Outcome::NoTable,
        1 => {}
        _ => return Outcome::Skip(SkipReason::UnsupportedStructure),
    }
    let from_at = from_positions[0];

    // --- the table reference ------------------------------------------------
    let mut i = from_at + 1;
    let Some((first, _)) = body.get(i) else {
        return Outcome::Skip(SkipReason::UnsupportedStructure);
    };
    // A derived table opens with a parenthesis.
    if is_punct(sql, first, b'(') {
        return Outcome::Skip(SkipReason::UnsupportedStructure);
    }
    if !matches!(first.kind, TokenKind::Word | TokenKind::QuotedIdent) {
        return Outcome::Skip(SkipReason::UnsupportedStructure);
    }
    if word_eq(sql, first, "DUAL") {
        return Outcome::NoTable;
    }

    let mut schema = None;
    let mut name = normalize_ident(first.text(sql));
    let mut last_ref_end = first.end;
    i += 1;

    // Optional schema qualification.
    if let Some((dot, _)) = body.get(i) {
        if is_punct(sql, dot, b'.') {
            let Some((second, _)) = body.get(i + 1) else {
                return Outcome::Skip(SkipReason::UnsupportedStructure);
            };
            if !matches!(second.kind, TokenKind::Word | TokenKind::QuotedIdent) {
                return Outcome::Skip(SkipReason::UnsupportedStructure);
            }
            schema = Some(name);
            name = normalize_ident(second.text(sql));
            last_ref_end = second.end;
            i += 2;
        }
    }

    // Optional alias, with or without AS.
    if let Some((next, _)) = body.get(i) {
        if word_eq(sql, next, "AS") {
            let Some((alias, _)) = body.get(i + 1) else {
                return Outcome::Skip(SkipReason::UnsupportedStructure);
            };
            if !matches!(alias.kind, TokenKind::Word | TokenKind::QuotedIdent) {
                return Outcome::Skip(SkipReason::UnsupportedStructure);
            }
            last_ref_end = alias.end;
            i += 2;
        } else if matches!(next.kind, TokenKind::QuotedIdent)
            || (next.kind == TokenKind::Word
                && !word_in(sql, next, CLAUSE_KEYWORDS)
                && !word_in(sql, next, JOIN_KEYWORDS)
                && !word_in(sql, next, POST_TABLE_UNSUPPORTED))
        {
            last_ref_end = next.end;
            i += 1;
        }
    }

    // Whatever follows the table reference decides whether this is one table.
    if let Some((next, _)) = body.get(i) {
        if is_punct(sql, next, b',') || word_in(sql, next, JOIN_KEYWORDS) {
            return Outcome::Skip(SkipReason::MultipleTables);
        }
        if !word_in(sql, next, CLAUSE_KEYWORDS) {
            // Index hints and anything else unmodelled.
            return Outcome::Skip(SkipReason::UnsupportedStructure);
        }
    }

    // --- insertion point and WHERE extent ------------------------------------
    let insert_at = last_ref_end;
    let body_end = body.last().map(|(t, _)| t.end).unwrap_or(insert_at);

    let clause_at = |from: usize| -> Option<usize> {
        body[from..]
            .iter()
            .position(|(t, d)| *d == 0 && word_in(sql, t, CLAUSE_KEYWORDS))
            .map(|p| p + from)
    };

    let where_clause = match clause_at(i) {
        Some(pos) if word_eq(sql, &body[pos].0, "WHERE") => {
            let after_keyword = body[pos].0.end;
            let end = clause_at(pos + 1)
                .map(|next| body[next].0.start)
                .unwrap_or(body_end);
            if end < after_keyword {
                return Outcome::Skip(SkipReason::NoInsertionPoint);
            }
            Some(WhereExtent { after_keyword, end })
        }
        _ => None,
    };

    Outcome::Eligible(Analysis {
        table: TableRef { schema, name },
        insert_at,
        where_clause,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outcome(sql: &str) -> Outcome {
        analyze(sql.as_bytes())
    }

    fn eligible(sql: &str) -> Analysis {
        match outcome(sql) {
            Outcome::Eligible(a) => a,
            other => panic!("expected eligible for {sql:?}, got {other:?}"),
        }
    }

    fn skip(sql: &str) -> SkipReason {
        match outcome(sql) {
            Outcome::Skip(r) => r,
            other => panic!("expected skip for {sql:?}, got {other:?}"),
        }
    }

    #[test]
    fn plain_select_is_eligible() {
        let a = eligible("SELECT * FROM orders");
        assert_eq!(a.table.name, "orders");
        assert!(a.table.schema.is_none());
        assert!(a.where_clause.is_none());
        assert_eq!(a.insert_at, "SELECT * FROM orders".len());
    }

    #[test]
    fn select_with_where_is_eligible() {
        let sql = "SELECT * FROM orders WHERE id = 1";
        let a = eligible(sql);
        let w = a.where_clause.unwrap();
        assert_eq!(&sql[w.after_keyword..w.end], " id = 1");
    }

    #[test]
    fn qualified_and_quoted_names_are_normalized() {
        let a = eligible("SELECT * FROM Shop.Orders");
        assert_eq!(a.table.schema.as_deref(), Some("shop"));
        assert_eq!(a.table.name, "orders");

        let a = eligible("SELECT * FROM `Weird Name`");
        assert_eq!(a.table.name, "weird name");

        let a = eligible("SELECT * FROM `shop`.`orders`");
        assert_eq!(a.table.schema.as_deref(), Some("shop"));
        assert_eq!(a.table.name, "orders");
    }

    #[test]
    fn aliases_are_accepted_and_move_the_insertion_point() {
        for sql in [
            "SELECT * FROM orders o",
            "SELECT * FROM orders AS o",
            "SELECT * FROM orders `o`",
        ] {
            let a = eligible(sql);
            assert_eq!(a.table.name, "orders", "{sql}");
            assert_eq!(a.insert_at, sql.len(), "{sql}");
        }
    }

    #[test]
    fn joins_are_multiple_tables() {
        assert_eq!(skip("SELECT * FROM a JOIN b ON a.id = b.id"), SkipReason::MultipleTables);
        assert_eq!(skip("SELECT * FROM a, b"), SkipReason::MultipleTables);
        assert_eq!(skip("SELECT * FROM a LEFT JOIN b ON a.id = b.id"), SkipReason::MultipleTables);
        assert_eq!(skip("SELECT * FROM a AS x INNER JOIN b ON x.id = b.id"), SkipReason::MultipleTables);
        assert_eq!(skip("SELECT * FROM a NATURAL JOIN b"), SkipReason::MultipleTables);
    }

    #[test]
    fn subqueries_unions_and_ctes_are_unsupported() {
        assert_eq!(skip("SELECT * FROM (SELECT 1) x"), SkipReason::UnsupportedStructure);
        assert_eq!(skip("SELECT * FROM t WHERE id IN (SELECT id FROM u)"), SkipReason::UnsupportedStructure);
        assert_eq!(skip("SELECT (SELECT 1) FROM t"), SkipReason::UnsupportedStructure);
        assert_eq!(skip("SELECT * FROM a UNION SELECT * FROM b"), SkipReason::UnsupportedStructure);
        assert_eq!(skip("WITH c AS (SELECT 1) SELECT * FROM c"), SkipReason::UnsupportedStructure);
        assert_eq!(skip("(SELECT * FROM a) UNION (SELECT * FROM b)"), SkipReason::UnsupportedStructure);
    }

    #[test]
    fn non_select_statements_are_skipped_as_not_select() {
        assert_eq!(skip("UPDATE t SET a = 1"), SkipReason::NotSelect);
        assert_eq!(skip("DELETE FROM t WHERE id = 1"), SkipReason::NotSelect);
        assert_eq!(skip("INSERT INTO t VALUES (1)"), SkipReason::NotSelect);
        assert_eq!(skip("SHOW TABLES"), SkipReason::NotSelect);
        assert_eq!(skip("CALL p()"), SkipReason::NotSelect);
    }

    #[test]
    fn several_statements_in_one_command_are_skipped() {
        assert_eq!(skip("SELECT 1; SELECT * FROM t"), SkipReason::MultipleStatements);
        assert_eq!(skip("SELECT * FROM t; DROP TABLE t"), SkipReason::MultipleStatements);
    }

    #[test]
    fn a_trailing_separator_is_tolerated() {
        let a = eligible("SELECT * FROM orders;");
        assert_eq!(a.insert_at, "SELECT * FROM orders".len());
    }

    #[test]
    fn unparsable_statements_report_tokenize_failure() {
        assert_eq!(skip("SELECT * FROM t WHERE a = 'unterminated"), SkipReason::TokenizeFailed);
    }

    #[test]
    fn statements_without_a_table_are_not_skips() {
        assert_eq!(outcome("SELECT 1"), Outcome::NoTable);
        assert_eq!(outcome("SELECT NOW()"), Outcome::NoTable);
        assert_eq!(outcome("SELECT 1 FROM DUAL"), Outcome::NoTable);
        assert_eq!(outcome(""), Outcome::NoTable);
    }

    #[test]
    fn index_hints_are_unsupported_rather_than_treated_as_an_alias() {
        assert_eq!(skip("SELECT * FROM t USE INDEX (i) WHERE a = 1"), SkipReason::UnsupportedStructure);
        assert_eq!(skip("SELECT * FROM t FORCE INDEX (i)"), SkipReason::UnsupportedStructure);
    }

    #[test]
    fn insertion_point_precedes_trailing_clauses() {
        for (sql, tail) in [
            ("SELECT * FROM orders ORDER BY id", " ORDER BY id"),
            ("SELECT * FROM orders GROUP BY a", " GROUP BY a"),
            ("SELECT * FROM orders LIMIT 10", " LIMIT 10"),
            ("SELECT * FROM orders FOR UPDATE", " FOR UPDATE"),
            ("SELECT * FROM orders LOCK IN SHARE MODE", " LOCK IN SHARE MODE"),
            ("SELECT * FROM orders INTO OUTFILE '/tmp/x'", " INTO OUTFILE '/tmp/x'"),
        ] {
            let a = eligible(sql);
            assert_eq!(&sql[a.insert_at..], tail, "{sql}");
        }
    }

    #[test]
    fn where_extent_stops_at_the_next_clause() {
        let sql = "SELECT * FROM orders WHERE a = 1 ORDER BY id LIMIT 5";
        let w = eligible(sql).where_clause.unwrap();
        // The extent runs from just after the keyword to the start of the next
        // clause, so it carries the surrounding whitespace. That is harmless:
        // the splice wraps it in parentheses either way.
        assert_eq!(sql[w.after_keyword..w.end].trim(), "a = 1");
    }

    #[test]
    fn where_extent_covers_the_rest_when_no_clause_follows() {
        let sql = "SELECT * FROM orders WHERE a = 1 OR b = 2";
        let w = eligible(sql).where_clause.unwrap();
        assert_eq!(&sql[w.after_keyword..w.end], " a = 1 OR b = 2");
    }

    #[test]
    fn clause_keywords_inside_literals_are_not_boundaries() {
        let sql = "SELECT * FROM orders WHERE note = 'ORDER BY x'";
        let w = eligible(sql).where_clause.unwrap();
        assert_eq!(&sql[w.after_keyword..w.end], " note = 'ORDER BY x'");

        let sql = "SELECT * FROM orders WHERE note = 'a' /* ORDER BY y */";
        let w = eligible(sql).where_clause.unwrap();
        assert_eq!(sql[w.after_keyword..w.end].trim(), "note = 'a'");

        let a = eligible("SELECT * FROM `order by`");
        assert_eq!(a.table.name, "order by");
    }

    #[test]
    fn clause_keywords_nested_in_parentheses_are_not_top_level() {
        let sql = "SELECT * FROM orders WHERE a IN (1, 2) AND b = GROUP_TEST(3)";
        let w = eligible(sql).where_clause.unwrap();
        assert_eq!(&sql[w.after_keyword..w.end], " a IN (1, 2) AND b = GROUP_TEST(3)");
    }

    #[test]
    fn version_gated_comment_contents_are_scanned_as_sql() {
        // The markers are transparent; the table is still found.
        let a = eligible("SELECT /*!40001 SQL_NO_CACHE */ * FROM orders");
        assert_eq!(a.table.name, "orders");
    }

    #[test]
    fn comments_do_not_move_the_insertion_point_past_them() {
        let sql = "SELECT * FROM orders -- trailing note";
        let a = eligible(sql);
        assert_eq!(&sql[..a.insert_at], "SELECT * FROM orders");
    }

    #[test]
    fn lowercase_and_mixed_case_keywords_are_recognized() {
        let a = eligible("select * from orders where id = 1");
        assert_eq!(a.table.name, "orders");
        assert!(a.where_clause.is_some());
        assert_eq!(skip("select * from a join b on a.id = b.id"), SkipReason::MultipleTables);
    }
}
