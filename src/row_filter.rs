//! Row-filter predicate injection.
//!
//! A listener may carry a map from table name to a predicate expression. When a
//! statement is a `SELECT` over exactly one table that has a rule, the
//! predicate is spliced into its `WHERE` clause before the command reaches the
//! backend.
//!
//! This is best-effort by construction: a statement that cannot be rewritten is
//! forwarded unchanged and counted, never rejected. It reduces accidental
//! cross-tenant reads in ordinary traffic and does not resist a client trying
//! to evade it — the backend's own grants remain the access control.

use std::collections::HashMap;

use crate::pipeline::{FilterOutcome, Stage, StageContext, StageOutput};
use crate::protocol::command::{Command, COM_QUERY, COM_STMT_PREPARE};
use crate::protocol::framing::packet_count;
use crate::sql::analyze::{analyze, Analysis, Outcome, SkipReason, TableRef};
use crate::sql::tokenizer::{tokenize, TokenKind};

/// Why a configured predicate was rejected at startup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PredicateError {
    Empty,
    NotTokenizable(String),
    UnbalancedParentheses,
    StatementSeparator,
    Placeholder,
    Comment,
}

impl std::fmt::Display for PredicateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PredicateError::Empty => write!(f, "predicate is empty"),
            PredicateError::NotTokenizable(e) => write!(f, "predicate cannot be parsed: {e}"),
            PredicateError::UnbalancedParentheses => {
                write!(f, "predicate has unbalanced parentheses")
            }
            PredicateError::StatementSeparator => write!(
                f,
                "predicate contains ';', which would terminate the statement it is spliced into"
            ),
            PredicateError::Placeholder => write!(
                f,
                "predicate contains '?', which would change the parameter count reported to a prepared statement's client"
            ),
            PredicateError::Comment => write!(
                f,
                "predicate contains a comment, which could comment out the rest of the statement it is spliced into"
            ),
        }
    }
}

/// Checks that a configured predicate is structurally safe to splice.
///
/// This cannot check that the predicate is semantically a boolean expression,
/// or that the column it names exists — the proxy never reads the backend's
/// catalog. Those failures surface as a loud query-time error on the tables
/// carrying the rule. What is checked here is everything that would instead
/// fail silently or corrupt the surrounding statement.
pub fn validate_predicate(predicate: &str) -> Result<(), PredicateError> {
    let trimmed = predicate.trim();
    if trimmed.is_empty() {
        return Err(PredicateError::Empty);
    }

    let tokens = tokenize(trimmed.as_bytes())
        .map_err(|e| PredicateError::NotTokenizable(e.to_string()))?;

    let mut depth: i32 = 0;
    for t in &tokens {
        match t.kind {
            // Any comment form is refused. The predicate is wrapped in
            // parentheses when spliced, so a line comment inside it would
            // comment out the closing parenthesis and the rest of the client's
            // statement. A block comment is harmless, but a rule that depends
            // on which kind you used is a rule nobody will remember.
            TokenKind::Comment | TokenKind::ExecCommentOpen | TokenKind::ExecCommentClose => {
                return Err(PredicateError::Comment)
            }
            TokenKind::Punct => match t.text(trimmed.as_bytes()) {
                b"(" => depth += 1,
                b")" => {
                    depth -= 1;
                    if depth < 0 {
                        return Err(PredicateError::UnbalancedParentheses);
                    }
                }
                b";" => return Err(PredicateError::StatementSeparator),
                b"?" => return Err(PredicateError::Placeholder),
                _ => {}
            },
            _ => {}
        }
    }
    if depth != 0 {
        return Err(PredicateError::UnbalancedParentheses);
    }

    Ok(())
}

/// A rule key: a table name, optionally qualified by a schema. Both are
/// normalized to lowercase with backticks stripped.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct RuleKey {
    schema: Option<String>,
    name: String,
}

impl RuleKey {
    /// Parses a configured key such as `orders` or `shop.orders`.
    fn parse(raw: &str) -> RuleKey {
        let cleaned = raw.replace('`', "");
        let lowered = cleaned.trim().to_ascii_lowercase();
        match lowered.split_once('.') {
            Some((schema, name)) => RuleKey {
                schema: Some(schema.trim().to_string()),
                name: name.trim().to_string(),
            },
            None => RuleKey {
                schema: None,
                name: lowered,
            },
        }
    }
}

/// The compiled rules for one listener.
#[derive(Debug, Clone, Default)]
pub struct RuleSet {
    rules: HashMap<RuleKey, String>,
    /// Bare table names across all rules, for the cheap "might a rule have
    /// applied here?" test used when analysis gave up before finding a table.
    names: Vec<String>,
}

impl RuleSet {
    /// Compiles and validates configured rules.
    pub fn compile(raw: &HashMap<String, String>) -> Result<RuleSet, (String, PredicateError)> {
        let mut rules = HashMap::new();
        let mut names = Vec::new();
        for (table, predicate) in raw {
            validate_predicate(predicate).map_err(|e| (table.clone(), e))?;
            let key = RuleKey::parse(table);
            if !names.contains(&key.name) {
                names.push(key.name.clone());
            }
            rules.insert(key, predicate.trim().to_string());
        }
        Ok(RuleSet { rules, names })
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// Finds the predicate for a table reference.
    ///
    /// A qualified reference prefers a qualified rule and falls back to a bare
    /// one, which lets a single rule cover a table across schemas. An
    /// unqualified reference matches only a bare rule: the statement names no
    /// schema, so applying a schema-specific policy would be a guess.
    pub fn lookup(&self, table: &TableRef) -> Option<&str> {
        if let Some(schema) = &table.schema {
            let qualified = RuleKey {
                schema: Some(schema.clone()),
                name: table.name.clone(),
            };
            if let Some(p) = self.rules.get(&qualified) {
                return Some(p);
            }
        }
        self.rules
            .get(&RuleKey {
                schema: None,
                name: table.name.clone(),
            })
            .map(String::as_str)
    }

    /// Whether any ruled table name appears in the statement text.
    ///
    /// Used only to decide whether a skip is worth counting. When analysis
    /// declines it usually has no table to report, so this answers the weaker
    /// question "might a filter have been wanted here?" — deliberately loose,
    /// because it feeds a counter and not a decision.
    fn mentions_a_ruled_table(&self, sql: &[u8]) -> bool {
        let lowered = String::from_utf8_lossy(sql).to_ascii_lowercase();
        self.names.iter().any(|name| {
            lowered.match_indices(name.as_str()).any(|(at, _)| {
                let before = lowered[..at].chars().next_back();
                let after = lowered[at + name.len()..].chars().next();
                // `is_none_or` would read better but is newer than the
                // declared MSRV.
                let boundary = |c: Option<char>| {
                    !c.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '$')
                };
                boundary(before) && boundary(after)
            })
        })
    }
}

/// Builds the rewritten statement.
///
/// Both the original condition and the injected predicate are parenthesized,
/// always. Without that, `WHERE a = 1 OR b = 2` combined with `tenant_id = 7`
/// binds as `a = 1 OR (b = 2 AND tenant_id = 7)` and returns every tenant's
/// rows where `a = 1` — a silent widening that produces valid SQL and correct
/// looking output.
pub fn splice(sql: &[u8], analysis: &Analysis, predicate: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(sql.len() + predicate.len() + 16);

    // The remainder is whatever followed the splice point. A `WHERE` extent
    // runs up to the start of the next clause keyword, so the whitespace that
    // separated them sits inside the extent and the injected text would
    // otherwise butt straight against that keyword.
    let push_remainder = |out: &mut Vec<u8>, from: usize| {
        let rest = &sql[from..];
        if rest.first().is_some_and(|b| !b.is_ascii_whitespace()) {
            out.push(b' ');
        }
        out.extend_from_slice(rest);
    };

    match analysis.where_clause {
        None => {
            out.extend_from_slice(&sql[..analysis.insert_at]);
            out.extend_from_slice(b" WHERE (");
            out.extend_from_slice(predicate.as_bytes());
            out.push(b')');
            push_remainder(&mut out, analysis.insert_at);
        }
        Some(w) => {
            out.extend_from_slice(&sql[..w.after_keyword]);
            out.extend_from_slice(b" (");
            out.extend_from_slice(&sql[w.after_keyword..w.end]);
            out.extend_from_slice(b") AND (");
            out.extend_from_slice(predicate.as_bytes());
            out.push(b')');
            push_remainder(&mut out, w.end);
        }
    }
    out
}

/// The pipeline stage that applies row filters.
#[derive(Debug)]
pub struct RowFilterStage {
    rules: RuleSet,
}

impl RowFilterStage {
    pub fn new(rules: RuleSet) -> Self {
        Self { rules }
    }
}

impl Stage for RowFilterStage {
    fn apply(&self, cmd: &Command, payload: &[u8], ctx: &mut StageContext) -> StageOutput {
        // Only the two commands that carry a statement as plain trailing text.
        if !matches!(cmd.code, COM_QUERY | COM_STMT_PREPARE) {
            return StageOutput::Unchanged;
        }
        let Some(sql) = cmd.statement() else {
            return StageOutput::Unchanged;
        };

        let analysis = match analyze(sql) {
            Outcome::NoTable => return StageOutput::Unchanged,
            Outcome::Skip(reason) => {
                // Only count skips where a filter plausibly should have applied,
                // so the counter measures coverage this feature is missing
                // rather than traffic nobody meant to filter.
                if self.rules.mentions_a_ruled_table(sql) {
                    ctx.filter = FilterOutcome::Skipped(reason);
                }
                return StageOutput::Unchanged;
            }
            Outcome::Eligible(a) => a,
        };

        let Some(predicate) = self.rules.lookup(&analysis.table) else {
            return StageOutput::Unchanged;
        };

        let rewritten_sql = splice(sql, &analysis, predicate);
        let mut rewritten = Vec::with_capacity(1 + rewritten_sql.len());
        rewritten.push(cmd.code);
        rewritten.extend_from_slice(&rewritten_sql);

        // Growing across a packet boundary would shift every later sequence
        // identifier. Skipping instead is what lets the relay stay free of
        // renumbering state.
        if packet_count(rewritten.len()) != packet_count(payload.len()) {
            ctx.filter = FilterOutcome::Skipped(SkipReason::PacketCountChanged);
            return StageOutput::Unchanged;
        }

        ctx.filter = FilterOutcome::Rewritten {
            table: analysis.table.name.clone(),
            forwarded: String::from_utf8_lossy(&rewritten_sql).into_owned(),
        };
        StageOutput::Replaced(rewritten)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;

    fn rules(pairs: &[(&str, &str)]) -> RuleSet {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        RuleSet::compile(&map).expect("rules should compile")
    }

    fn command(code: u8, sql: &str) -> Command {
        let mut payload = vec![code];
        payload.extend_from_slice(sql.as_bytes());
        Command::new(Bytes::from(payload), 0, 1)
    }

    fn rewrite(rules: &[(&str, &str)], sql: &str) -> (String, FilterOutcome) {
        let stage = RowFilterStage::new(super::tests::rules(rules));
        let cmd = command(COM_QUERY, sql);
        let mut ctx = StageContext::default();
        let out = stage.apply(&cmd, &cmd.payload, &mut ctx);
        let text = match out {
            StageOutput::Replaced(bytes) => String::from_utf8(bytes[1..].to_vec()).unwrap(),
            StageOutput::Unchanged => sql.to_string(),
        };
        (text, ctx.filter)
    }

    // --- predicate validation ------------------------------------------------

    #[test]
    fn accepts_ordinary_predicates() {
        for p in [
            "tenant_id = 7",
            "tenant_id = 7 AND deleted_at IS NULL",
            "org IN (1, 2, 3)",
            "(a = 1 OR b = 2)",
            "region = 'eu-west'",
            "created_at > '2020-01-01'",
        ] {
            assert!(validate_predicate(p).is_ok(), "should accept {p:?}");
        }
    }

    #[test]
    fn rejects_an_empty_predicate() {
        assert_eq!(validate_predicate("").unwrap_err(), PredicateError::Empty);
        assert_eq!(validate_predicate("   ").unwrap_err(), PredicateError::Empty);
    }

    #[test]
    fn rejects_a_statement_separator() {
        assert_eq!(
            validate_predicate("1=1; DROP TABLE orders").unwrap_err(),
            PredicateError::StatementSeparator
        );
    }

    #[test]
    fn rejects_a_placeholder() {
        assert_eq!(
            validate_predicate("tenant_id = ?").unwrap_err(),
            PredicateError::Placeholder
        );
    }

    #[test]
    fn rejects_every_comment_form() {
        // A trailing line comment is the dangerous one: it would swallow the
        // closing parenthesis the splice adds.
        for p in [
            "tenant_id = 7 -- note",
            "tenant_id = 7 # note",
            "tenant_id = /* note */ 7",
            "tenant_id = 7 /*!40001 x */",
        ] {
            assert_eq!(
                validate_predicate(p).unwrap_err(),
                PredicateError::Comment,
                "should reject {p:?}"
            );
        }
    }

    #[test]
    fn rejects_unbalanced_parentheses() {
        assert_eq!(
            validate_predicate("(a = 1").unwrap_err(),
            PredicateError::UnbalancedParentheses
        );
        assert_eq!(
            validate_predicate("a = 1)").unwrap_err(),
            PredicateError::UnbalancedParentheses
        );
    }

    #[test]
    fn rejects_an_unterminated_literal() {
        assert!(matches!(
            validate_predicate("name = 'unterminated").unwrap_err(),
            PredicateError::NotTokenizable(_)
        ));
    }

    // --- rule matching -------------------------------------------------------

    #[test]
    fn rule_keys_are_normalized() {
        let set = rules(&[("Orders", "t = 1"), ("`Shop`.`Invoices`", "t = 2")]);
        assert_eq!(
            set.lookup(&TableRef { schema: None, name: "orders".into() }),
            Some("t = 1")
        );
        assert_eq!(
            set.lookup(&TableRef {
                schema: Some("shop".into()),
                name: "invoices".into()
            }),
            Some("t = 2")
        );
    }

    #[test]
    fn qualification_matching_follows_the_design_table() {
        let bare = rules(&[("orders", "bare")]);
        let qualified = rules(&[("shop.orders", "qualified")]);

        let unqualified_ref = TableRef { schema: None, name: "orders".into() };
        let qualified_ref = TableRef {
            schema: Some("shop".into()),
            name: "orders".into(),
        };

        assert_eq!(bare.lookup(&unqualified_ref), Some("bare"));
        assert_eq!(bare.lookup(&qualified_ref), Some("bare"), "bare rule is the fallback");
        assert_eq!(qualified.lookup(&qualified_ref), Some("qualified"));
        assert_eq!(
            qualified.lookup(&unqualified_ref),
            None,
            "a schema-specific rule must not apply to an unqualified reference"
        );
    }

    #[test]
    fn a_qualified_rule_wins_over_a_bare_one() {
        let set = rules(&[("orders", "bare"), ("shop.orders", "qualified")]);
        assert_eq!(
            set.lookup(&TableRef {
                schema: Some("shop".into()),
                name: "orders".into()
            }),
            Some("qualified")
        );
        assert_eq!(
            set.lookup(&TableRef {
                schema: Some("other".into()),
                name: "orders".into()
            }),
            Some("bare")
        );
    }

    #[test]
    fn compile_reports_which_table_had_the_bad_predicate() {
        let map: HashMap<String, String> =
            [("orders".to_string(), "1=1; DROP TABLE x".to_string())]
                .into_iter()
                .collect();
        let (table, err) = RuleSet::compile(&map).unwrap_err();
        assert_eq!(table, "orders");
        assert_eq!(err, PredicateError::StatementSeparator);
    }

    // --- splicing ------------------------------------------------------------

    #[test]
    fn injects_a_where_clause_when_there_is_none() {
        let (sql, outcome) = rewrite(&[("orders", "tenant_id = 7")], "SELECT * FROM orders");
        assert_eq!(sql, "SELECT * FROM orders WHERE (tenant_id = 7)");
        assert!(matches!(outcome, FilterOutcome::Rewritten { .. }));
    }

    #[test]
    fn combines_with_an_existing_where_clause() {
        let (sql, _) = rewrite(
            &[("orders", "tenant_id = 7")],
            "SELECT * FROM orders WHERE id = 1",
        );
        assert_eq!(sql, "SELECT * FROM orders WHERE ( id = 1) AND (tenant_id = 7)");
    }

    #[test]
    fn parenthesizes_an_original_condition_using_or() {
        // Without the parentheses this reads as `a = 1 OR (b = 2 AND tenant = 7)`
        // and leaks every tenant's rows where a = 1.
        let (sql, _) = rewrite(
            &[("orders", "tenant_id = 7")],
            "SELECT * FROM orders WHERE a = 1 OR b = 2",
        );
        assert_eq!(
            sql,
            "SELECT * FROM orders WHERE ( a = 1 OR b = 2) AND (tenant_id = 7)"
        );
    }

    #[test]
    fn parenthesizes_a_predicate_using_or() {
        let (sql, _) = rewrite(
            &[("orders", "a = 1 OR b = 2")],
            "SELECT * FROM orders WHERE c = 3",
        );
        assert_eq!(
            sql,
            "SELECT * FROM orders WHERE ( c = 3) AND (a = 1 OR b = 2)"
        );
    }

    #[test]
    fn inserts_before_trailing_clauses() {
        let (sql, _) = rewrite(
            &[("orders", "tenant_id = 7")],
            "SELECT * FROM orders ORDER BY id LIMIT 10",
        );
        assert_eq!(
            sql,
            "SELECT * FROM orders WHERE (tenant_id = 7) ORDER BY id LIMIT 10"
        );

        let (sql, _) = rewrite(
            &[("orders", "tenant_id = 7")],
            "SELECT * FROM orders WHERE a = 1 ORDER BY id",
        );
        assert_eq!(
            sql,
            "SELECT * FROM orders WHERE ( a = 1 ) AND (tenant_id = 7) ORDER BY id"
        );
    }

    #[test]
    fn preserves_comments_hints_and_whitespace() {
        let (sql, _) = rewrite(
            &[("orders", "tenant_id = 7")],
            "SELECT /*+ MAX_EXECUTION_TIME(1000) */  *   FROM orders -- trailing",
        );
        assert_eq!(
            sql,
            "SELECT /*+ MAX_EXECUTION_TIME(1000) */  *   FROM orders WHERE (tenant_id = 7) -- trailing"
        );
    }

    #[test]
    fn rewrites_an_aliased_table() {
        let (sql, _) = rewrite(
            &[("orders", "tenant_id = 7")],
            "SELECT o.* FROM orders o WHERE o.id = 1",
        );
        assert_eq!(
            sql,
            "SELECT o.* FROM orders o WHERE ( o.id = 1) AND (tenant_id = 7)"
        );
    }

    // --- stage behaviour -----------------------------------------------------

    #[test]
    fn statements_on_unruled_tables_pass_through_without_a_reason() {
        let (sql, outcome) = rewrite(&[("orders", "t = 1")], "SELECT * FROM currencies");
        assert_eq!(sql, "SELECT * FROM currencies");
        assert!(matches!(outcome, FilterOutcome::NotApplicable));
    }

    #[test]
    fn skips_on_ruled_tables_are_counted_with_a_reason() {
        let cases = [
            ("SELECT * FROM orders o JOIN items i ON o.id = i.oid", SkipReason::MultipleTables),
            ("SELECT * FROM orders WHERE id IN (SELECT oid FROM items)", SkipReason::UnsupportedStructure),
            ("SELECT * FROM orders UNION SELECT * FROM archive_orders", SkipReason::UnsupportedStructure),
            ("SELECT 1; SELECT * FROM orders", SkipReason::MultipleStatements),
            ("SELECT * FROM orders WHERE a = 'unterminated", SkipReason::TokenizeFailed),
            ("UPDATE orders SET a = 1", SkipReason::NotSelect),
        ];
        for (sql, want) in cases {
            let (out, outcome) = rewrite(&[("orders", "tenant_id = 7")], sql);
            assert_eq!(out, sql, "must be forwarded unchanged: {sql}");
            match outcome {
                FilterOutcome::Skipped(reason) => assert_eq!(reason, want, "{sql}"),
                other => panic!("expected a counted skip for {sql:?}, got {other:?}"),
            }
        }
    }

    #[test]
    fn skips_on_unruled_tables_are_not_counted() {
        let (_, outcome) = rewrite(
            &[("orders", "t = 1")],
            "SELECT * FROM currencies c JOIN regions r ON c.r = r.id",
        );
        assert!(
            matches!(outcome, FilterOutcome::NotApplicable),
            "a skip on tables nobody wanted filtered must not be counted, got {outcome:?}"
        );
    }

    #[test]
    fn ruled_table_name_must_match_on_a_word_boundary() {
        let set = rules(&[("orders", "t = 1")]);
        assert!(set.mentions_a_ruled_table(b"SELECT * FROM orders o JOIN x ON 1"));
        assert!(!set.mentions_a_ruled_table(b"SELECT * FROM back_orders_archive j JOIN x ON 1"));
    }

    #[test]
    fn prepared_statements_are_rewritten_too() {
        let stage = RowFilterStage::new(rules(&[("orders", "tenant_id = 7")]));
        let cmd = command(COM_STMT_PREPARE, "SELECT * FROM orders WHERE id = ?");
        let mut ctx = StageContext::default();
        match stage.apply(&cmd, &cmd.payload, &mut ctx) {
            StageOutput::Replaced(bytes) => {
                assert_eq!(bytes[0], COM_STMT_PREPARE);
                assert_eq!(
                    String::from_utf8(bytes[1..].to_vec()).unwrap(),
                    "SELECT * FROM orders WHERE ( id = ?) AND (tenant_id = 7)"
                );
            }
            other => panic!("expected a rewrite, got {other:?}"),
        }
    }

    #[test]
    fn other_commands_are_never_analyzed() {
        let stage = RowFilterStage::new(rules(&[("orders", "tenant_id = 7")]));
        for code in [0x0E_u8, 0x02, 0x17, 0x01] {
            let cmd = command(code, "SELECT * FROM orders");
            let mut ctx = StageContext::default();
            assert!(matches!(
                stage.apply(&cmd, &cmd.payload, &mut ctx),
                StageOutput::Unchanged
            ));
            assert!(matches!(ctx.filter, FilterOutcome::NotApplicable));
        }
    }

    #[test]
    fn a_rewrite_crossing_a_packet_boundary_is_skipped() {
        use crate::protocol::framing::MAX_PAYLOAD;
        let stage = RowFilterStage::new(rules(&[("orders", "tenant_id = 7")]));

        // Pad the statement so the command fills a packet exactly, leaving no
        // room for the injected text.
        let tail = " WHERE a = 1";
        let predicate_growth = " () AND (tenant_id = 7)".len();
        let base = format!("SELECT /*{}*/ * FROM orders{tail}", "p".repeat(64));
        let pad = MAX_PAYLOAD - 1 - base.len() + 1;
        let sql = base.replace(
            &"p".repeat(64),
            &"p".repeat(64 + pad.min(predicate_growth.max(1))),
        );

        let cmd = command(COM_QUERY, &sql);
        let mut ctx = StageContext::default();
        // Force the guard by claiming the original occupied fewer packets than
        // the rewrite will: a payload one byte under the boundary.
        let payload = vec![0u8; MAX_PAYLOAD];
        let out = stage.apply(&cmd, &payload, &mut ctx);
        assert!(matches!(out, StageOutput::Unchanged));
        assert!(
            matches!(ctx.filter, FilterOutcome::Skipped(SkipReason::PacketCountChanged)),
            "got {:?}",
            ctx.filter
        );
    }
}
