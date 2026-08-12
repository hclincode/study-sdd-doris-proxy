//! A MySQL-specific SQL tokenizer.
//!
//! This walks lexical structure only — there is no grammar here. Tokens carry
//! byte spans into the original statement, which is what the digest normalizer
//! consumes and what a future rewriter would use to locate a splice point
//! without re-rendering the statement.
//!
//! The default SQL mode is assumed: backslash escapes are active inside string
//! literals, and double quotes delimit strings rather than identifiers. A
//! session running with `NO_BACKSLASH_ESCAPES` or `ANSI_QUOTES` can therefore
//! produce a digest that differs from MySQL's own, which affects grouping only.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    /// Unquoted keyword or identifier.
    Word,
    /// Backtick-quoted identifier.
    QuotedIdent,
    /// String literal in single or double quotes.
    String,
    Number,
    HexLiteral,
    BitLiteral,
    Punct,
    /// A comment in any of MySQL's three forms. Not executable.
    Comment,
    /// The opening `/*!` of a version-gated comment. Its contents are real SQL
    /// and are tokenized normally, so this is a marker, not a span of skipped
    /// text.
    ExecCommentOpen,
    /// The `*/` closing a version-gated comment.
    ExecCommentClose,
    Whitespace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    pub start: usize,
    pub end: usize,
}

impl Token {
    pub fn text<'a>(&self, src: &'a [u8]) -> &'a [u8] {
        &src[self.start..self.end]
    }

    pub fn is_significant(&self) -> bool {
        !matches!(self.kind, TokenKind::Whitespace | TokenKind::Comment)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenizeError {
    pub at: usize,
    pub reason: &'static str,
}

impl std::fmt::Display for TokenizeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} at byte {}", self.reason, self.at)
    }
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_' || b == b'$' || b >= 0x80
}

fn is_ident_continue(b: u8) -> bool {
    is_ident_start(b) || b.is_ascii_digit()
}

/// Tokenizes a statement, returning tokens in source order.
pub fn tokenize(src: &[u8]) -> Result<Vec<Token>, TokenizeError> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    let mut in_exec_comment = false;

    while pos < src.len() {
        let start = pos;
        let b = src[pos];

        // Closing of a version-gated comment, whose body we tokenize as SQL.
        if in_exec_comment && b == b'*' && src.get(pos + 1) == Some(&b'/') {
            pos += 2;
            in_exec_comment = false;
            out.push(Token {
                kind: TokenKind::ExecCommentClose,
                start,
                end: pos,
            });
            continue;
        }

        if b.is_ascii_whitespace() {
            while pos < src.len() && src[pos].is_ascii_whitespace() {
                pos += 1;
            }
            out.push(Token {
                kind: TokenKind::Whitespace,
                start,
                end: pos,
            });
            continue;
        }

        // `#` runs to end of line.
        if b == b'#' {
            while pos < src.len() && src[pos] != b'\n' {
                pos += 1;
            }
            out.push(Token {
                kind: TokenKind::Comment,
                start,
                end: pos,
            });
            continue;
        }

        // `--` only starts a comment when followed by whitespace or a control
        // character; otherwise it is two minus signs.
        if b == b'-' && src.get(pos + 1) == Some(&b'-') {
            let third = src.get(pos + 2);
            let is_comment = match third {
                None => true,
                Some(&c) => c.is_ascii_whitespace() || c < 0x20,
            };
            if is_comment {
                while pos < src.len() && src[pos] != b'\n' {
                    pos += 1;
                }
                out.push(Token {
                    kind: TokenKind::Comment,
                    start,
                    end: pos,
                });
                continue;
            }
        }

        if b == b'/' && src.get(pos + 1) == Some(&b'*') {
            // `/*!` and `/*!50000` introduce SQL that MySQL executes.
            if src.get(pos + 2) == Some(&b'!') {
                pos += 3;
                while pos < src.len() && src[pos].is_ascii_digit() {
                    pos += 1;
                }
                in_exec_comment = true;
                out.push(Token {
                    kind: TokenKind::ExecCommentOpen,
                    start,
                    end: pos,
                });
                continue;
            }
            // Ordinary block comment, including `/*+ hints */`.
            pos += 2;
            loop {
                if pos + 1 >= src.len() {
                    return Err(TokenizeError {
                        at: start,
                        reason: "unterminated block comment",
                    });
                }
                if src[pos] == b'*' && src[pos + 1] == b'/' {
                    pos += 2;
                    break;
                }
                pos += 1;
            }
            out.push(Token {
                kind: TokenKind::Comment,
                start,
                end: pos,
            });
            continue;
        }

        // Quoted string or identifier.
        if b == b'\'' || b == b'"' || b == b'`' {
            let quote = b;
            let backslash_escapes = quote != b'`';
            pos += 1;
            loop {
                let Some(&c) = src.get(pos) else {
                    return Err(TokenizeError {
                        at: start,
                        reason: "unterminated quoted literal",
                    });
                };
                if backslash_escapes && c == b'\\' {
                    pos += 2;
                    continue;
                }
                if c == quote {
                    // A doubled quote is a literal quote, not a terminator.
                    if src.get(pos + 1) == Some(&quote) {
                        pos += 2;
                        continue;
                    }
                    pos += 1;
                    break;
                }
                pos += 1;
            }
            out.push(Token {
                kind: if quote == b'`' {
                    TokenKind::QuotedIdent
                } else {
                    TokenKind::String
                },
                start,
                end: pos,
            });
            continue;
        }

        // Introduced literals: X'..' / B'..' and their lowercase forms.
        if (b == b'x' || b == b'X' || b == b'b' || b == b'B') && src.get(pos + 1) == Some(&b'\'') {
            let kind = if b == b'x' || b == b'X' {
                TokenKind::HexLiteral
            } else {
                TokenKind::BitLiteral
            };
            pos += 2;
            loop {
                let Some(&c) = src.get(pos) else {
                    return Err(TokenizeError {
                        at: start,
                        reason: "unterminated introduced literal",
                    });
                };
                pos += 1;
                if c == b'\'' {
                    break;
                }
            }
            out.push(Token {
                kind,
                start,
                end: pos,
            });
            continue;
        }

        // 0x / 0b prefixed literals.
        if b == b'0' && matches!(src.get(pos + 1), Some(b'x') | Some(b'X')) {
            pos += 2;
            while pos < src.len() && src[pos].is_ascii_hexdigit() {
                pos += 1;
            }
            out.push(Token {
                kind: TokenKind::HexLiteral,
                start,
                end: pos,
            });
            continue;
        }
        if b == b'0' && matches!(src.get(pos + 1), Some(b'b') | Some(b'B')) {
            pos += 2;
            while pos < src.len() && matches!(src[pos], b'0' | b'1') {
                pos += 1;
            }
            out.push(Token {
                kind: TokenKind::BitLiteral,
                start,
                end: pos,
            });
            continue;
        }

        // Numbers, including a leading-dot form and exponents.
        let starts_number = b.is_ascii_digit()
            || (b == b'.' && src.get(pos + 1).is_some_and(u8::is_ascii_digit));
        if starts_number {
            while pos < src.len() && src[pos].is_ascii_digit() {
                pos += 1;
            }
            if src.get(pos) == Some(&b'.') {
                pos += 1;
                while pos < src.len() && src[pos].is_ascii_digit() {
                    pos += 1;
                }
            }
            if matches!(src.get(pos), Some(b'e') | Some(b'E')) {
                let mut probe = pos + 1;
                if matches!(src.get(probe), Some(b'+') | Some(b'-')) {
                    probe += 1;
                }
                if src.get(probe).is_some_and(u8::is_ascii_digit) {
                    pos = probe;
                    while pos < src.len() && src[pos].is_ascii_digit() {
                        pos += 1;
                    }
                }
            }
            out.push(Token {
                kind: TokenKind::Number,
                start,
                end: pos,
            });
            continue;
        }

        if is_ident_start(b) {
            while pos < src.len() && is_ident_continue(src[pos]) {
                pos += 1;
            }
            out.push(Token {
                kind: TokenKind::Word,
                start,
                end: pos,
            });
            continue;
        }

        pos += 1;
        out.push(Token {
            kind: TokenKind::Punct,
            start,
            end: pos,
        });
    }

    if in_exec_comment {
        return Err(TokenizeError {
            at: src.len(),
            reason: "unterminated version-gated comment",
        });
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::TokenKind as K;

    fn kinds(sql: &str) -> Vec<TokenKind> {
        tokenize(sql.as_bytes())
            .unwrap()
            .into_iter()
            .filter(|t| t.kind != K::Whitespace)
            .map(|t| t.kind)
            .collect()
    }

    fn texts(sql: &str) -> Vec<String> {
        let src = sql.as_bytes();
        tokenize(src)
            .unwrap()
            .into_iter()
            .filter(|t| t.kind != K::Whitespace)
            .map(|t| String::from_utf8_lossy(t.text(src)).into_owned())
            .collect()
    }

    #[test]
    fn spans_index_the_original_bytes() {
        let src = b"SELECT a FROM t";
        let toks = tokenize(src).unwrap();
        for t in &toks {
            assert!(t.end <= src.len());
            assert!(t.start < t.end);
        }
        let words: Vec<_> = toks
            .iter()
            .filter(|t| t.kind == K::Word)
            .map(|t| t.text(src))
            .collect();
        assert_eq!(words, vec![&b"SELECT"[..], b"a", b"FROM", b"t"]);
    }

    #[test]
    fn single_quoted_string_with_backslash_escape() {
        assert_eq!(texts(r"SELECT 'a\'b'"), vec!["SELECT", r"'a\'b'"]);
        assert_eq!(kinds(r"SELECT 'a\'b'"), vec![K::Word, K::String]);
    }

    #[test]
    fn single_quoted_string_with_doubled_quote() {
        assert_eq!(texts("SELECT 'a''b'"), vec!["SELECT", "'a''b'"]);
    }

    #[test]
    fn backslash_before_terminator_does_not_end_the_string() {
        // The trailing quote is escaped, so the literal runs to the last quote.
        assert_eq!(texts(r"'a\\' , 'b'").len(), 3);
        assert_eq!(kinds(r"'a\\' , 'b'"), vec![K::String, K::Punct, K::String]);
    }

    #[test]
    fn double_quoted_is_a_string_in_default_mode() {
        assert_eq!(kinds(r#"SELECT "abc""#), vec![K::Word, K::String]);
    }

    #[test]
    fn backtick_identifier_with_doubled_backtick() {
        let src = "SELECT `we``ird`";
        assert_eq!(kinds(src), vec![K::Word, K::QuotedIdent]);
        assert_eq!(texts(src)[1], "`we``ird`");
    }

    #[test]
    fn backslash_is_literal_inside_backticks() {
        let src = r"`a\`";
        // The backslash is not an escape, so the identifier closes at the tick.
        assert_eq!(kinds(src), vec![K::QuotedIdent]);
    }

    #[test]
    fn hash_comment_runs_to_end_of_line() {
        assert_eq!(
            kinds("SELECT 1 # trailing\nFROM t"),
            vec![K::Word, K::Number, K::Comment, K::Word, K::Word]
        );
    }

    #[test]
    fn double_dash_needs_whitespace_to_be_a_comment() {
        assert_eq!(
            kinds("SELECT 1 -- note\nFROM t"),
            vec![K::Word, K::Number, K::Comment, K::Word, K::Word]
        );
        // Without the space it is arithmetic, not a comment.
        assert_eq!(kinds("SELECT 1--2"), vec![K::Word, K::Number, K::Punct, K::Punct, K::Number]);
    }

    #[test]
    fn double_dash_at_end_of_input_is_a_comment() {
        assert_eq!(kinds("SELECT 1 --"), vec![K::Word, K::Number, K::Comment]);
    }

    #[test]
    fn block_comment_and_hint_are_comments() {
        assert_eq!(kinds("SELECT /* c */ 1"), vec![K::Word, K::Comment, K::Number]);
        assert_eq!(
            kinds("SELECT /*+ HINT(t) */ 1"),
            vec![K::Word, K::Comment, K::Number]
        );
    }

    #[test]
    fn unterminated_block_comment_is_an_error() {
        assert!(tokenize(b"SELECT /* oops").is_err());
    }

    #[test]
    fn version_gated_comment_contents_are_tokenized_as_sql() {
        let src = "SELECT /*!40001 SQL_NO_CACHE */ a";
        assert_eq!(
            kinds(src),
            vec![K::Word, K::ExecCommentOpen, K::Word, K::ExecCommentClose, K::Word]
        );
    }

    #[test]
    fn unterminated_version_gated_comment_is_an_error() {
        assert!(tokenize(b"SELECT /*!40001 a").is_err());
    }

    #[test]
    fn numeric_forms() {
        assert_eq!(kinds("1 2.5 .5 1e10 1E-3"), vec![K::Number; 5]);
        assert_eq!(texts("1e10")[0], "1e10");
        // A bare `e` after digits is not an exponent without digits following.
        assert_eq!(kinds("1e"), vec![K::Number, K::Word]);
    }

    #[test]
    fn hex_and_bit_literals() {
        assert_eq!(kinds("0xFF 0b1010"), vec![K::HexLiteral, K::BitLiteral]);
        assert_eq!(kinds("X'1f' B'01'"), vec![K::HexLiteral, K::BitLiteral]);
        assert_eq!(kinds("x'1f' b'01'"), vec![K::HexLiteral, K::BitLiteral]);
    }

    #[test]
    fn identifier_starting_with_b_is_not_a_bit_literal() {
        assert_eq!(kinds("balance"), vec![K::Word]);
        assert_eq!(kinds("x"), vec![K::Word]);
    }

    #[test]
    fn unterminated_string_is_an_error() {
        assert!(tokenize(b"SELECT 'abc").is_err());
        assert!(tokenize(b"SELECT `abc").is_err());
        assert!(tokenize(b"SELECT X'ab").is_err());
    }

    #[test]
    fn keywords_inside_strings_are_not_tokens() {
        let src = "SELECT 'ORDER BY x' FROM t";
        assert_eq!(kinds(src), vec![K::Word, K::String, K::Word, K::Word]);
    }

    #[test]
    fn empty_input_yields_no_tokens() {
        assert!(tokenize(b"").unwrap().is_empty());
    }
}
