//! Statement digests.
//!
//! A digest is the statement with its literal values replaced by placeholders,
//! comments removed, and whitespace normalized, so that statements differing
//! only in their values collapse to one form. It is what makes the log usable
//! for grouping: raw statement text has unbounded cardinality.

use super::tokenizer::{tokenize, Token, TokenKind};

/// Reserved words that are case-folded so that `select` and `SELECT` group
/// together.
///
/// Only reserved words are folded. Non-reserved keywords can legally be
/// unquoted identifiers, and folding those would merge distinct column or table
/// names on case-sensitive schemas.
const RESERVED: &[&str] = &[
    "ADD", "ALL", "ALTER", "AND", "AS", "ASC", "BETWEEN", "BY", "CALL", "CASE", "CREATE",
    "CROSS", "DELETE", "DESC", "DESCRIBE", "DISTINCT", "DROP", "ELSE", "EXISTS", "EXPLAIN",
    "FOR", "FOREIGN", "FROM", "GROUP", "HAVING", "IN", "INDEX", "INNER", "INSERT", "INTERVAL",
    "INTO", "IS", "JOIN", "KEY", "LEFT", "LIKE", "LIMIT", "LOCK", "NATURAL", "NOT", "ON",
    "OR", "ORDER", "OUTER", "PRIMARY", "REFERENCES", "REPLACE", "RIGHT", "SELECT", "SET",
    "SHOW", "TABLE", "THEN", "UNION", "UPDATE", "USING", "VALUES", "WHEN", "WHERE", "WITH",
];

const PLACEHOLDER: &str = "?";
const COLLAPSED_LIST: &str = "(...)";

/// A normalized statement and a stable hash of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Digest {
    pub text: String,
    pub hash: u64,
}

impl Digest {
    /// Lowercase hex, for the log record.
    pub fn hash_hex(&self) -> String {
        format!("{:016x}", self.hash)
    }
}

/// FNV-1a. Stable across runs and platforms, which is all the digest hash needs
/// to be; it is a grouping key, not a security primitive.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn normalize_token(src: &[u8], token: &Token) -> Option<String> {
    match token.kind {
        TokenKind::Whitespace | TokenKind::Comment => None,
        TokenKind::String
        | TokenKind::Number
        | TokenKind::HexLiteral
        | TokenKind::BitLiteral => Some(PLACEHOLDER.to_string()),
        TokenKind::Word => {
            let text = String::from_utf8_lossy(token.text(src)).into_owned();
            let upper = text.to_ascii_uppercase();
            if RESERVED.contains(&upper.as_str()) {
                Some(upper)
            } else {
                Some(text)
            }
        }
        _ => Some(String::from_utf8_lossy(token.text(src)).into_owned()),
    }
}

/// Replaces `( ? , ? , ? )` with a single marker so that lists of different
/// lengths share a digest.
fn collapse_literal_lists(tokens: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(tokens.len());
    let mut i = 0;
    while i < tokens.len() {
        if tokens[i] == "(" {
            let mut j = i + 1;
            let mut placeholders = 0;
            let mut only_placeholders = true;
            while j < tokens.len() && tokens[j] != ")" {
                match tokens[j].as_str() {
                    PLACEHOLDER => placeholders += 1,
                    "," => {}
                    _ => {
                        only_placeholders = false;
                        break;
                    }
                }
                j += 1;
            }
            if only_placeholders && placeholders > 0 && j < tokens.len() && tokens[j] == ")" {
                out.push(COLLAPSED_LIST.to_string());
                i = j + 1;
                continue;
            }
        }
        out.push(tokens[i].clone());
        i += 1;
    }
    out
}

fn join(tokens: &[String]) -> String {
    let mut out = String::new();
    for (i, tok) in tokens.iter().enumerate() {
        let no_space_before = matches!(tok.as_str(), "," | ")" | "." | ";");
        let prev_suppresses = i > 0 && matches!(tokens[i - 1].as_str(), "(" | ".");
        if i > 0 && !no_space_before && !prev_suppresses {
            out.push(' ');
        }
        out.push_str(tok);
    }
    out
}

/// Computes the digest of a statement, or `None` when it cannot be tokenized.
///
/// Failure is not an error for the caller: the log record is still emitted, it
/// simply records that no digest was available.
pub fn digest(sql: &[u8]) -> Option<Digest> {
    let tokens = tokenize(sql).ok()?;
    let normalized: Vec<String> = tokens
        .iter()
        .filter_map(|t| normalize_token(sql, t))
        .collect();
    let collapsed = collapse_literal_lists(normalized);
    let text = join(&collapsed);
    let hash = fnv1a(text.as_bytes());
    Some(Digest { text, hash })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(sql: &str) -> String {
        digest(sql.as_bytes()).unwrap().text
    }

    #[test]
    fn replaces_literals_with_placeholders() {
        assert_eq!(
            d("SELECT * FROM users WHERE id = 42 AND name = 'bob'"),
            "SELECT * FROM users WHERE id = ? AND name = ?"
        );
    }

    #[test]
    fn statements_differing_only_in_literals_share_a_digest() {
        let a = digest(b"SELECT * FROM t WHERE id = 1 AND s = 'x'").unwrap();
        let b = digest(b"SELECT * FROM t WHERE id = 99999 AND s = 'yyyy'").unwrap();
        assert_eq!(a.text, b.text);
        assert_eq!(a.hash, b.hash);
    }

    #[test]
    fn structurally_different_statements_differ() {
        let a = digest(b"SELECT a FROM t").unwrap();
        let b = digest(b"SELECT b FROM t").unwrap();
        assert_ne!(a.text, b.text);
        assert_ne!(a.hash, b.hash);

        let c = digest(b"SELECT a FROM other").unwrap();
        assert_ne!(a.hash, c.hash);
    }

    #[test]
    fn literal_lists_of_different_lengths_collapse_alike() {
        let a = digest(b"SELECT * FROM t WHERE id IN (1, 2, 3)").unwrap();
        let b = digest(b"SELECT * FROM t WHERE id IN (7)").unwrap();
        let c = digest(b"SELECT * FROM t WHERE id IN (1,2,3,4,5,6,7,8)").unwrap();
        assert_eq!(a.text, "SELECT * FROM t WHERE id IN (...)");
        assert_eq!(a.hash, b.hash);
        assert_eq!(a.hash, c.hash);
    }

    #[test]
    fn a_parenthesized_expression_is_not_collapsed() {
        assert_eq!(
            d("SELECT * FROM t WHERE (a = 1 OR b = 2)"),
            "SELECT * FROM t WHERE (a = ? OR b = ?)"
        );
    }

    #[test]
    fn comments_are_removed() {
        assert_eq!(d("SELECT /* trace=abc */ 1 FROM t"), "SELECT ? FROM t");
        assert_eq!(d("SELECT 1 -- note\nFROM t"), "SELECT ? FROM t");
        assert_eq!(d("SELECT 1 # note\nFROM t"), "SELECT ? FROM t");
    }

    #[test]
    fn version_gated_comment_contents_survive() {
        let text = d("SELECT /*!40001 SQL_NO_CACHE */ a FROM t");
        assert!(text.contains("SQL_NO_CACHE"), "got {text}");
    }

    #[test]
    fn whitespace_is_normalized() {
        let a = digest(b"SELECT   a\n\nFROM\tt").unwrap();
        let b = digest(b"SELECT a FROM t").unwrap();
        assert_eq!(a.hash, b.hash);
    }

    #[test]
    fn reserved_words_are_case_folded_but_identifiers_are_not() {
        let a = digest(b"select a from t").unwrap();
        let b = digest(b"SELECT a FROM t").unwrap();
        assert_eq!(a.hash, b.hash);

        let lower = digest(b"SELECT a FROM t").unwrap();
        let upper = digest(b"SELECT A FROM T").unwrap();
        assert_ne!(lower.hash, upper.hash, "identifier case must be preserved");
    }

    #[test]
    fn null_and_booleans_are_kept_as_structure() {
        assert_eq!(d("SELECT * FROM t WHERE a IS NULL"), "SELECT * FROM t WHERE a IS NULL");
    }

    #[test]
    fn qualified_names_stay_readable() {
        assert_eq!(d("SELECT o.id FROM shop.orders o"), "SELECT o.id FROM shop.orders o");
    }

    #[test]
    fn quoted_identifiers_are_preserved() {
        assert_eq!(d("SELECT `from` FROM `t`"), "SELECT `from` FROM `t`");
    }

    #[test]
    fn hex_and_bit_literals_are_placeholders() {
        assert_eq!(d("SELECT * FROM t WHERE a = 0xFF AND b = B'01'"),
                   "SELECT * FROM t WHERE a = ? AND b = ?");
    }

    #[test]
    fn unparsable_statements_yield_no_digest() {
        assert!(digest(b"SELECT 'unterminated").is_none());
        assert!(digest(b"SELECT /* unterminated").is_none());
    }

    #[test]
    fn hash_is_stable_and_hex_formatted() {
        let a = digest(b"SELECT 1").unwrap();
        let b = digest(b"SELECT 1").unwrap();
        assert_eq!(a.hash, b.hash);
        assert_eq!(a.hash_hex().len(), 16);
    }
}
