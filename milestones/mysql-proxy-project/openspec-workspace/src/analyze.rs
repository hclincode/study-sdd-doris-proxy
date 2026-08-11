//! Statement analysis: parse, then enumerate every table reference via an
//! allowlist walk that refuses anything unrecognised (design D3).
//!
//! Owned by the `row-filter-rewrite` capability. See
//! `openspec/changes/add-row-filter-proxy-mvp/specs/row-filter-rewrite/spec.md`.
//!
//! # The allowlist rule
//!
//! Every `match` in this module over a `sqlparser` node kind is exhaustive and
//! written out variant by variant. There is **no `_ => {}` arm that continues
//! the walk**. The only catch-all arms in the file end in
//! [`RefusalReason::UnsupportedShape`]. A future `sqlparser` release that adds a
//! node kind therefore turns previously-working queries into *rejections*, never
//! into silent bypasses — the failure direction design D3 asks for.
//!
//! Struct fields get the same treatment: a node kind is only on the allowlist if
//! every one of its fields is either walked or explicitly asserted to be
//! absent. Adding a field to a struct upstream will not compile-error here, so
//! the field checks are written as guards that refuse when a field is populated.

use std::fmt;

use sqlparser::ast::{
    CeilFloorKind, Cte, Distinct, Expr, FromTable, FunctionArg, FunctionArgExpr, FunctionArguments,
    GroupByExpr, Ident, Insert, Join, JoinConstraint, JoinOperator, LimitClause, ObjectName,
    OrderBy, OrderByExpr, OrderByKind, Query, Select, SelectFlavor, SelectItem, SelectModifiers,
    SetExpr, Statement, TableFactor, TableObject, TableWithJoins, Update, UpdateTableFromKind,
    WindowType,
};
use sqlparser::dialect::MySqlDialect;
use sqlparser::parser::Parser;

use crate::error::RefusalReason;

/// Result of an analysis step. The error side is always a *refusal*: there is no
/// "could not analyse, carry on" outcome.
pub type AnalysisResult<T> = std::result::Result<T, RefusalReason>;

/// Refuse, naming the construct responsible. The name is derived from the AST
/// variant, never from the user's SQL text, so it cannot echo query contents
/// back to the client (design D5).
fn unsupported<T>(construct: impl Into<String>) -> AnalysisResult<T> {
    Err(RefusalReason::UnsupportedShape {
        construct: construct.into(),
    })
}

/// The leading identifier of a value's `Debug` rendering, which for an enum is
/// its variant name. Used to name the offending construct in a refusal without
/// quoting any user-supplied text.
fn variant_name<T: fmt::Debug>(value: &T) -> String {
    format!("{value:?}")
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect()
}

/// A table name exactly as it was written in the statement: one part when
/// unqualified (`orders`), two when qualified (`sales.orders`).
///
/// Resolution of an unqualified name against the session's current database is
/// the policy layer's job, not this module's — see `PolicySet::lookup` in
/// [`crate::policy`]. This type carries only what the statement said.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TableName {
    parts: Vec<String>,
}

impl TableName {
    /// Read a table name off a parsed `ObjectName`, refusing anything the
    /// policy layer could not resolve.
    pub fn from_object_name(name: &ObjectName) -> AnalysisResult<Self> {
        let mut parts = Vec::with_capacity(name.0.len());
        for part in &name.0 {
            match part.as_ident() {
                Some(ident) => parts.push(ident.value.clone()),
                // `ObjectNamePart::Function` — a name computed by a function call.
                None => return unsupported("function-valued name part"),
            }
        }
        match parts.len() {
            1 | 2 => Ok(Self { parts }),
            // Doris supports `catalog.db.table`; this proxy does not resolve
            // three-part names against policy, so it refuses them.
            _ => unsupported("table name qualified beyond `database.table`"),
        }
    }

    /// The table part of the name, ignoring any database qualifier.
    pub fn leaf(&self) -> &str {
        self.parts.last().expect("TableName is never empty")
    }

    /// The database qualifier, when the reference was written qualified.
    pub fn qualifier(&self) -> Option<&str> {
        (self.parts.len() == 2).then(|| self.parts[0].as_str())
    }

    /// Whether the reference was written without a database qualifier.
    pub fn is_unqualified(&self) -> bool {
        self.parts.len() == 1
    }
}

impl fmt::Display for TableName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.parts.join("."))
    }
}

/// Where in the statement a table reference sits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefPosition {
    /// A relation in a `FROM` or join position. The only position a derived-table
    /// wrap (design D2) can be applied to.
    Relation,
    /// The target of a write statement. Enumerated so a policy can be detected,
    /// never wrapped — writes against policy tables are refused outright.
    WriteTarget,
}

/// One enumerated reference to a named relation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableRef {
    /// The name as written.
    pub name: TableName,
    /// The user's alias, when present.
    pub alias: Option<String>,
    /// True when this reference resolves to a common table expression defined in
    /// an enclosing `WITH`, rather than to a stored table. A CTE is not a table
    /// and carries no policy of its own; its body is walked and constrained
    /// separately, so its rows are already filtered by the time the CTE is read.
    pub is_cte_reference: bool,
    /// Whether a constraint can be injected here.
    pub position: RefPosition,
}

impl TableRef {
    /// The name by which columns of this reference are qualified in the rest of
    /// the statement: the user's alias if present, else the bare table name.
    /// This is the name a derived-table wrap must be aliased to (design D2).
    pub fn effective_name(&self) -> &str {
        self.alias.as_deref().unwrap_or_else(|| self.name.leaf())
    }
}

/// What kind of statement was parsed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatementKind {
    Select,
    Insert,
    Replace,
    Update,
    Delete,
    /// `SET` — configures the session. Analysed like anything else rather than
    /// waved through: `SET @x = (SELECT total FROM sales.orders)` reads a table,
    /// and `SELECT @x` returns it afterwards.
    Session,
    /// `SHOW` — reports metadata.
    Metadata,
}

impl StatementKind {
    /// `INSERT`, `REPLACE`, `UPDATE` and `DELETE` are writes. The MVP refuses
    /// them against policy-bearing tables rather than constraining them.
    pub fn is_write(self) -> bool {
        matches!(
            self,
            StatementKind::Insert
                | StatementKind::Replace
                | StatementKind::Update
                | StatementKind::Delete
        )
    }

    /// Whether a policy-bearing reference in this kind of statement must be
    /// **refused** rather than constrained by wrapping.
    ///
    /// Only `SELECT` can be constrained. A write is out of scope for the MVP. A
    /// `SET` or `SHOW` puts its result somewhere the proxy does not track —
    /// session state, or a metadata result whose shape the guard does not
    /// apply to — so wrapping the relation inside it would not bound what the
    /// client can read afterwards. Refusing is the honest answer.
    pub fn must_refuse_policy_tables(self) -> bool {
        !matches!(self, StatementKind::Select)
    }
}

/// A fully analysed statement: parsed, classified, and with every table
/// reference in it enumerated.
#[derive(Clone, Debug)]
pub struct Analysis {
    /// The parsed statement, owned so the rewriter can mutate it in place.
    pub statement: Statement,
    pub kind: StatementKind,
    /// Every table reference in the statement, in walk order.
    pub tables: Vec<TableRef>,
}

/// Parse exactly one statement.
///
/// More than one statement is [`RefusalReason::MultiStatement`]. This is the
/// **only** place multi-statement smuggling is closed. Design D4 claimed
/// capability negotiation closed it at the protocol level; that turned out to be
/// wrong — the server crate hands the whole `COM_QUERY` payload to the query
/// handler whatever the client negotiated, and the advertised capability bit is
/// advisory in any case. So this check is load-bearing, not a backstop.
///
/// A *trailing* semicolon is punctuation, not a second statement:
/// `Parser::parse_sql` returns one statement for `SELECT 1;`, and still one for
/// `SELECT 1; ;` or `SELECT 1; -- comment`. Refusing those would be a false
/// rejection of the most ordinary thing a client sends. Pinned by
/// `a_trailing_semicolon_is_not_a_second_statement` in `tests/analyze_walk.rs`.
pub fn parse_single(sql: &str) -> AnalysisResult<Statement> {
    if contains_executable_comment(sql) {
        return unsupported("conditionally-executed comment");
    }
    let mut statements =
        Parser::parse_sql(&MySqlDialect {}, sql).map_err(|_| RefusalReason::Unparseable)?;
    match statements.len() {
        // Nothing to parse — an empty request, whitespace, or nothing but a
        // comment. Reported as `Unparseable` because that is the outcome the
        // client sees and there is no separate variant for it; the name is a
        // little off, since no parse error occurred. Refusing rather than
        // forwarding matches MySQL, which errors on an empty query, so it costs
        // no compatibility, and "I found no statement here" must not become a
        // forwarding path for a policy-bearing user.
        0 => Err(RefusalReason::Unparseable),
        1 => Ok(statements.pop().expect("length checked")),
        _ => Err(RefusalReason::MultiStatement),
    }
}

/// Whether `sql` contains a MySQL *executable* comment, `/*! ... */`.
///
/// # Why this is refused rather than handled
///
/// MySQL runs the contents of `/*!NNNNN ... */` only when the server version is
/// at least `NNNNN`, and `/*! ... */` always. **`sqlparser` parses the contents
/// as ordinary SQL and discards the gate**, so the fragment is enumerated and
/// constrained like any other SQL — but re-rendering the AST emits it with the
/// gate gone, and the statement reaching Doris executes unconditionally what the
/// original might never have run. Measured:
///
/// ```text
/// in   SELECT /*!99999 id, */ region FROM sales.orders
/// out  SELECT id, region FROM (SELECT * FROM sales.orders WHERE `region` IN (...)) AS orders
/// ```
///
/// One column becomes two. A gated `UNION` branch becomes an unconditional one.
/// That breaks invariant 4 and the "column shape is unchanged by rewriting"
/// scenario, so the statement must not be forwarded.
///
/// This is not excessive caution about comments — ordinary `/* */` and `--`
/// comments are discarded identically by `sqlparser` and by Doris and are fine.
/// It is specifically that the *gate* is a condition the backend evaluates about
/// itself, and re-rendering resolves it on the backend's behalf. Preserving it
/// would mean re-emitting `/*!NNNNN */` around a fragment of the AST, which is a
/// great deal of machinery for a construct that has no business crossing a
/// filtering proxy.
///
/// # Why the scan is textual, and why it is deliberately trigger-happy
///
/// The gate is gone by the time a `Statement` exists, so this has to run on the
/// text. The scanner skips string literals, quoted identifiers and ordinary
/// comments, so a `/*!` *inside* a literal is not a false positive.
///
/// Two rules are set so that mis-tracking errs toward flagging rather than
/// missing:
///
/// - **Backslash escapes are not honoured inside string literals.** Honouring
///   them is correct only under MySQL's default `sql_mode`; under
///   `NO_BACKSLASH_ESCAPES` a backslash is literal. Guessing wrong in that
///   direction would keep the scanner *inside* a string past its real end and
///   could step over a marker. Not honouring them ends strings earlier, which
///   can only produce a spurious refusal. Same reasoning `policy-config` applies
///   to permitted values: prefer the rule that is correct under every
///   `sql_mode`.
/// - **`--` starts a comment only when followed by whitespace or end of input**,
///   which is MySQL's own rule. Treating it as a comment unconditionally would
///   skip the rest of the line in `SELECT 1--2 /*!99999 ... */` and miss the
///   marker.
///
/// # Termination
///
/// Every branch advances `i`, which is what makes the loop terminate on
/// arbitrary input — and this function runs on attacker-controlled bytes before
/// anything has validated them, so invariant 2 covers a hang as much as a panic.
/// The property is easy to break silently: a cursor that moved *backwards* on
/// leaving a block comment loops forever on `/*/**/`, because it re-enters the
/// comment it just left. Mutation testing produced exactly that, and
/// `SELECT 1 /*/**/` in the decision-surface table is what catches it.
fn contains_executable_comment(sql: &str) -> bool {
    #[derive(Clone, Copy, PartialEq)]
    enum State {
        Normal,
        Quoted(u8),
        LineComment,
        BlockComment,
    }

    // Byte-wise is safe for UTF-8: every byte of a multi-byte sequence has the
    // high bit set, so none can collide with the ASCII delimiters matched here.
    let bytes = sql.as_bytes();
    let mut state = State::Normal;
    let mut i = 0;

    while i < bytes.len() {
        let byte = bytes[i];
        match state {
            State::Normal => match byte {
                b'\'' | b'"' | b'`' => {
                    state = State::Quoted(byte);
                    i += 1;
                }
                b'#' => {
                    state = State::LineComment;
                    i += 1;
                }
                b'-' if bytes.get(i + 1) == Some(&b'-')
                    && bytes.get(i + 2).is_none_or(u8::is_ascii_whitespace) =>
                {
                    state = State::LineComment;
                    i += 2;
                }
                b'/' if bytes.get(i + 1) == Some(&b'*') => {
                    if bytes.get(i + 2) == Some(&b'!') {
                        return true;
                    }
                    state = State::BlockComment;
                    i += 2;
                }
                _ => i += 1,
            },
            State::Quoted(quote) => {
                if byte == quote {
                    // A doubled delimiter is a literal one and does not close.
                    if bytes.get(i + 1) == Some(&quote) {
                        i += 2;
                    } else {
                        state = State::Normal;
                        i += 1;
                    }
                } else {
                    i += 1;
                }
            }
            State::LineComment => {
                if byte == b'\n' {
                    state = State::Normal;
                }
                i += 1;
            }
            State::BlockComment => {
                if byte == b'*' && bytes.get(i + 1) == Some(&b'/') {
                    state = State::Normal;
                    i += 2;
                } else {
                    i += 1;
                }
            }
        }
    }
    false
}

/// Parse a statement and enumerate every table reference in it.
///
/// Returns [`RefusalReason::UnsupportedShape`] naming the offending construct
/// for anything the allowlist walk does not recognise.
pub fn analyze(sql: &str) -> AnalysisResult<Analysis> {
    let mut statement = parse_single(sql)?;
    let kind = statement_kind(&statement)?;
    let mut collector = Collector::default();
    walk_statement(&mut statement, &mut collector)?;
    Ok(Analysis {
        statement,
        kind,
        tables: collector.tables,
    })
}

fn statement_kind(statement: &Statement) -> AnalysisResult<StatementKind> {
    Ok(match statement {
        Statement::Query(_) => StatementKind::Select,
        Statement::Insert(insert) => {
            if insert.replace_into {
                StatementKind::Replace
            } else {
                StatementKind::Insert
            }
        }
        Statement::Update(_) => StatementKind::Update,
        Statement::Delete(_) => StatementKind::Delete,
        Statement::Set(_) => StatementKind::Session,
        Statement::ShowTables { .. }
        | Statement::ShowColumns { .. }
        | Statement::ShowDatabases { .. }
        | Statement::ShowSchemas { .. }
        | Statement::ShowViews { .. }
        | Statement::ShowVariables { .. }
        | Statement::ShowStatus { .. }
        | Statement::ShowCollation { .. }
        | Statement::ShowCharset(_)
        | Statement::ShowVariable { .. }
        | Statement::ShowCreate { .. } => StatementKind::Metadata,
        other => return unsupported(format!("{} statement", variant_name(other))),
    })
}

#[derive(Default)]
struct Collector {
    tables: Vec<TableRef>,
}

impl SiteVisitor for Collector {
    fn visit_relation(
        &mut self,
        factor: &mut TableFactor,
        shadowed_by_cte: bool,
    ) -> AnalysisResult<()> {
        self.tables
            .push(relation_table_ref(factor, shadowed_by_cte)?);
        Ok(())
    }

    fn visit_write_target(&mut self, name: &ObjectName) -> AnalysisResult<()> {
        self.tables.push(TableRef {
            name: TableName::from_object_name(name)?,
            alias: None,
            is_cte_reference: false,
            position: RefPosition::WriteTarget,
        });
        Ok(())
    }
}

/// Describe a `TableFactor::Table` site as a [`TableRef`].
pub fn relation_table_ref(factor: &TableFactor, shadowed_by_cte: bool) -> AnalysisResult<TableRef> {
    match factor {
        TableFactor::Table { name, alias, .. } => Ok(TableRef {
            name: TableName::from_object_name(name)?,
            alias: alias.as_ref().map(|a| a.name.value.clone()),
            is_cte_reference: shadowed_by_cte,
            position: RefPosition::Relation,
        }),
        other => unsupported(format!("{} in a relation position", variant_name(other))),
    }
}

/// Receives each enumerated reference site as the walk reaches it.
///
/// The walk hands out `&mut TableFactor` so an implementation can *replace* a
/// relation with a derived table in place — that is how design D2's wrapping is
/// applied without a second traversal that could disagree with this one about
/// which sites exist.
pub trait SiteVisitor {
    /// A named relation in a `FROM` or join position.
    ///
    /// `shadowed_by_cte` is true when the name resolves to a common table
    /// expression visible at this point rather than to a stored table.
    fn visit_relation(
        &mut self,
        factor: &mut TableFactor,
        shadowed_by_cte: bool,
    ) -> AnalysisResult<()>;

    /// The target of a write statement.
    fn visit_write_target(&mut self, name: &ObjectName) -> AnalysisResult<()>;

    /// Called before descending into a derived table. Returning `false` skips
    /// the subquery — used by the post-rewrite verification to stop at a guard
    /// it has already recognised. The default descends.
    fn enter_derived(&mut self, subquery: &mut Query) -> AnalysisResult<bool> {
        let _ = subquery;
        Ok(true)
    }
}

/// Names of common table expressions visible at the current point in the walk.
/// Compared case-insensitively, matching MySQL's treatment of CTE names.
type CteScope = Vec<String>;

/// Whether `name` resolves to a common table expression rather than a stored
/// table.
///
/// The `is_unqualified` half is load-bearing and is the fail-open direction if
/// it is ever dropped: a *qualified* name always denotes a table, so
/// `WITH orders AS (…) SELECT * FROM sales.orders` reads the real
/// `sales.orders`, and treating it as a CTE reference would forward it with no
/// predicate at all. Pinned by
/// `a_qualified_reference_is_not_shadowed_by_a_same_named_cte`.
fn shadowed(scope: &CteScope, name: &TableName) -> bool {
    name.is_unqualified() && scope.iter().any(|c| c.eq_ignore_ascii_case(name.leaf()))
}

/// Walk a statement, visiting every table reference in it.
pub fn walk_statement<V: SiteVisitor>(statement: &mut Statement, v: &mut V) -> AnalysisResult<()> {
    let mut scope = CteScope::new();
    match statement {
        Statement::Query(query) => walk_query(query, &mut scope, v),
        Statement::Insert(insert) => walk_insert(insert, &mut scope, v),
        Statement::Update(update) => walk_update(update, &mut scope, v),
        Statement::Delete(delete) => walk_delete(delete, &mut scope, v),
        Statement::Set(set) => walk_set(set, &mut scope, v),
        // The name after `FROM`/`IN` means different things per statement, and
        // `ShowStatementOptions` does not record which: `parent_type` is a
        // Snowflake field that `MySqlDialect` always leaves `None`. So the
        // statement kind is the only thing that says whether that name is a
        // table, and it is passed in explicitly rather than sniffed.
        Statement::ShowColumns { show_options, .. } => {
            walk_show_options(show_options, NameIsTable::Yes, &mut scope, v)
        }
        Statement::ShowTables { show_options, .. }
        | Statement::ShowDatabases { show_options, .. }
        | Statement::ShowSchemas { show_options, .. }
        | Statement::ShowViews { show_options, .. } => {
            walk_show_options(show_options, NameIsTable::No, &mut scope, v)
        }
        Statement::ShowVariables { filter, .. }
        | Statement::ShowStatus { filter, .. }
        | Statement::ShowCollation { filter } => walk_show_filter(filter.as_mut(), &mut scope, v),
        Statement::ShowCharset(show) => walk_show_filter(show.filter.as_mut(), &mut scope, v),
        Statement::ShowVariable { variable } => walk_show_variable(variable),
        Statement::ShowCreate { obj_type, obj_name } => walk_show_create(obj_type, obj_name, v),
        other => unsupported(format!("{} statement", variant_name(other))),
    }
}

/// Walk a `SET`, enumerating any table its assigned expressions read.
///
/// `SET` is **not** inert, which is the whole reason it goes through the walk
/// rather than being waved through as harmless session chatter:
///
/// ```sql
/// SET @x = (SELECT total FROM sales.orders);
/// SELECT @x;   -- returns it, with no policy-bearing reference in sight
/// ```
///
/// Forwarding every `SET` because "it cannot return rows" would be a live
/// bypass. Enumerating the assigned expressions puts that statement on exactly
/// the same footing as any other read of the table.
fn walk_set<V: SiteVisitor>(
    set: &mut sqlparser::ast::Set,
    scope: &mut CteScope,
    v: &mut V,
) -> AnalysisResult<()> {
    use sqlparser::ast::Set;
    match set {
        Set::SingleAssignment { values, .. } | Set::ParenthesizedAssignments { values, .. } => {
            for value in values {
                walk_expr(value, scope, v)?;
            }
            Ok(())
        }
        Set::MultipleAssignments { assignments } => {
            for assignment in assignments {
                walk_expr(&mut assignment.value, scope, v)?;
            }
            Ok(())
        }
        Set::SetTimeZone { value, .. } => walk_expr(value, scope, v),
        // `SET NAMES utf8mb4` — a charset and an optional collation, both bare
        // identifiers. This is the statement almost every connector sends first,
        // and it reads nothing.
        Set::SetNames { .. } | Set::SetNamesDefault {} => Ok(()),
        // Transaction modes are a fixed enum; the snapshot is a literal.
        Set::SetTransaction { .. } => Ok(()),
        // `SET ROLE` changes which privileges the session holds. Policy is keyed
        // on the authenticated username, which does not change — but the rows
        // the backend will return may, and the proxy has no model of that.
        Set::SetRole { .. } => unsupported("SET ROLE"),
        Set::SetSessionAuthorization(_) => unsupported("SET SESSION AUTHORIZATION"),
        Set::SetSessionParam(_) => unsupported("SET session parameter"),
    }
}

/// Whether the name after `FROM`/`IN` in a `SHOW` names a table or a database.
///
/// `SHOW COLUMNS FROM sales.orders` names a table; `SHOW TABLES FROM sales`
/// names a database. Enumerating the latter as a table would ask the policy
/// layer the wrong question, and a policy on `sales.orders` must not be matched
/// by `SHOW TABLES FROM sales`.
#[derive(Clone, Copy, PartialEq)]
enum NameIsTable {
    Yes,
    No,
}

fn walk_show_options<V: SiteVisitor>(
    options: &mut sqlparser::ast::ShowStatementOptions,
    names_a_table: NameIsTable,
    scope: &mut CteScope,
    v: &mut V,
) -> AnalysisResult<()> {
    if let Some(show_in) = &options.show_in {
        if let Some(parent_name) = &show_in.parent_name {
            if names_a_table == NameIsTable::Yes {
                v.visit_write_target(parent_name)?;
            }
        }
    }
    if let Some(limit) = &mut options.limit {
        walk_expr(limit, scope, v)?;
    }
    match &mut options.filter_position {
        None => {}
        Some(
            sqlparser::ast::ShowStatementFilterPosition::Infix(filter)
            | sqlparser::ast::ShowStatementFilterPosition::Suffix(filter),
        ) => walk_show_filter(Some(filter), scope, v)?,
    }
    Ok(())
}

fn walk_show_filter<V: SiteVisitor>(
    filter: Option<&mut sqlparser::ast::ShowStatementFilter>,
    scope: &mut CteScope,
    v: &mut V,
) -> AnalysisResult<()> {
    use sqlparser::ast::ShowStatementFilter;
    match filter {
        None => Ok(()),
        // Patterns are literals.
        Some(ShowStatementFilter::Like(_))
        | Some(ShowStatementFilter::ILike(_))
        | Some(ShowStatementFilter::NoKeyword(_)) => Ok(()),
        Some(ShowStatementFilter::Where(expr)) => walk_expr(expr, scope, v),
    }
}

/// `Statement::ShowVariable` is `sqlparser`'s catch-all for `SHOW <words>` forms
/// it does not model, and it hands back the raw tokens uninterpreted:
///
/// ```text
/// SHOW ENGINES                  -> ["ENGINES"]
/// SHOW INDEX FROM sales.orders  -> ["INDEX", "FROM", "sales", "orders"]
/// ```
///
/// A single token cannot name anything, so there is nothing to enumerate and the
/// statement is forwarded — this is what keeps `SHOW ENGINES`, `SHOW WARNINGS`,
/// `SHOW GRANTS` and friends working.
///
/// Anything longer *does* contain a name, and recovering which token is the
/// table would mean reimplementing MySQL's `SHOW` grammar from a token list —
/// precisely the guessing D3's allowlist exists to forbid. So those refuse.
/// `SHOW INDEX FROM sales.orders` is the case that matters: it discloses
/// metadata about a policy-bearing table, and the walk cannot locate the name
/// reliably enough to ask the policy layer about it.
fn walk_show_variable(variable: &[Ident]) -> AnalysisResult<()> {
    if variable.len() > 1 {
        return unsupported("SHOW form this proxy cannot parse into named objects");
    }
    Ok(())
}

/// `SHOW CREATE TABLE sales.orders` names a table, so it is enumerated and the
/// statement is refused when that table is policy-bearing.
///
/// It discloses the table's definition rather than its rows, and metadata
/// disclosure is an accepted limitation of this control — but the spec's rule is
/// that every table reference is enumerated and the statement forwarded only if
/// none is policy-bearing, and this is a table reference. Refusing is also the
/// direction that costs nothing: `SHOW CREATE TABLE` on an unrestricted table
/// still works, and `SHOW TABLES` still lists everything.
fn walk_show_create<V: SiteVisitor>(
    obj_type: &sqlparser::ast::ShowCreateObject,
    obj_name: &ObjectName,
    v: &mut V,
) -> AnalysisResult<()> {
    use sqlparser::ast::ShowCreateObject;
    match obj_type {
        ShowCreateObject::Table | ShowCreateObject::View => v.visit_write_target(obj_name),
        // An event, function, procedure or trigger name is not a table, so
        // there is nothing here the policy layer could resolve.
        ShowCreateObject::Event
        | ShowCreateObject::Function
        | ShowCreateObject::Procedure
        | ShowCreateObject::Trigger => Ok(()),
    }
}

fn walk_query<V: SiteVisitor>(
    query: &mut Query,
    scope: &mut CteScope,
    v: &mut V,
) -> AnalysisResult<()> {
    if !query.pipe_operators.is_empty() {
        return unsupported("pipe operator");
    }
    if query.settings.is_some() {
        return unsupported("SETTINGS clause");
    }
    if query.format_clause.is_some() {
        return unsupported("FORMAT clause");
    }
    if query.for_clause.is_some() {
        return unsupported("FOR XML/JSON clause");
    }
    if !query.locks.is_empty() {
        return unsupported("FOR UPDATE/SHARE clause");
    }
    if query.fetch.is_some() {
        return unsupported("FETCH clause");
    }

    let entered = scope.len();
    if let Some(with) = &mut query.with {
        let recursive = with.recursive;
        for cte in &mut with.cte_tables {
            // A recursive CTE may refer to itself, so its own name is in scope
            // for its body; a non-recursive one sees only the CTEs before it.
            // Getting this order wrong in the permissive direction would let a
            // later CTE name mask a real table read by an earlier body.
            if recursive {
                scope.push(cte_name(cte)?);
                walk_cte(cte, scope, v)?;
            } else {
                walk_cte(cte, scope, v)?;
                scope.push(cte_name(cte)?);
            }
        }
    }

    walk_set_expr(&mut query.body, scope, v)?;
    if let Some(order_by) = &mut query.order_by {
        walk_order_by(order_by, scope, v)?;
    }
    if let Some(limit) = &mut query.limit_clause {
        walk_limit_clause(limit, scope, v)?;
    }

    scope.truncate(entered);
    Ok(())
}

fn cte_name(cte: &Cte) -> AnalysisResult<String> {
    if !cte.alias.columns.is_empty() {
        return unsupported("CTE column alias list");
    }
    if cte.alias.at.is_some() {
        return unsupported("CTE AT clause");
    }
    Ok(cte.alias.name.value.clone())
}

fn walk_cte<V: SiteVisitor>(cte: &mut Cte, scope: &mut CteScope, v: &mut V) -> AnalysisResult<()> {
    if cte.from.is_some() {
        return unsupported("CTE FROM clause");
    }
    // `materialized` is a hint only and introduces no reference site.
    walk_query(&mut cte.query, scope, v)
}

fn walk_set_expr<V: SiteVisitor>(
    body: &mut SetExpr,
    scope: &mut CteScope,
    v: &mut V,
) -> AnalysisResult<()> {
    match body {
        SetExpr::Select(select) => walk_select(select, scope, v),
        SetExpr::Query(query) => walk_query(query, scope, v),
        SetExpr::SetOperation { left, right, .. } => {
            // Every branch of a UNION / INTERSECT / EXCEPT is walked; a branch
            // that read the policy table unconstrained is the classic bypass.
            walk_set_expr(left, scope, v)?;
            walk_set_expr(right, scope, v)
        }
        SetExpr::Values(values) => {
            for row in &mut values.rows {
                for expr in &mut row.content {
                    walk_expr(expr, scope, v)?;
                }
            }
            Ok(())
        }
        SetExpr::Insert(_) => unsupported("INSERT in a query body"),
        SetExpr::Update(_) => unsupported("UPDATE in a query body"),
        SetExpr::Delete(_) => unsupported("DELETE in a query body"),
        SetExpr::Merge(_) => unsupported("MERGE in a query body"),
        SetExpr::Table(_) => unsupported("TABLE command"),
    }
}

fn walk_select<V: SiteVisitor>(
    select: &mut Select,
    scope: &mut CteScope,
    v: &mut V,
) -> AnalysisResult<()> {
    if !select.optimizer_hints.is_empty() {
        return unsupported("optimizer hint");
    }
    if select
        .select_modifiers
        .as_ref()
        .is_some_and(|m| *m != SelectModifiers::default())
    {
        return unsupported("MySQL SELECT modifier");
    }
    if select.top.is_some() {
        return unsupported("TOP clause");
    }
    if select.exclude.is_some() {
        return unsupported("EXCLUDE clause");
    }
    if select.into.is_some() {
        return unsupported("SELECT INTO clause");
    }
    if !select.lateral_views.is_empty() {
        return unsupported("LATERAL VIEW clause");
    }
    if select.prewhere.is_some() {
        return unsupported("PREWHERE clause");
    }
    if !select.connect_by.is_empty() {
        return unsupported("CONNECT BY clause");
    }
    if !select.cluster_by.is_empty() {
        return unsupported("CLUSTER BY clause");
    }
    if !select.distribute_by.is_empty() {
        return unsupported("DISTRIBUTE BY clause");
    }
    if !select.sort_by.is_empty() {
        return unsupported("SORT BY clause");
    }
    if !select.named_window.is_empty() {
        return unsupported("WINDOW clause");
    }
    if select.qualify.is_some() {
        return unsupported("QUALIFY clause");
    }
    if select.value_table_mode.is_some() {
        return unsupported("SELECT AS STRUCT/VALUE");
    }
    if select.flavor != SelectFlavor::Standard {
        return unsupported("FROM-first SELECT");
    }
    match &select.distinct {
        None | Some(Distinct::All) | Some(Distinct::Distinct) => {}
        Some(Distinct::On(_)) => return unsupported("DISTINCT ON clause"),
    }

    for item in &mut select.projection {
        walk_select_item(item, scope, v)?;
    }
    for table in &mut select.from {
        walk_table_with_joins(table, scope, v)?;
    }
    if let Some(selection) = &mut select.selection {
        walk_expr(selection, scope, v)?;
    }
    match &mut select.group_by {
        // Refused outright, not for what it does but for what it *is*.
        //
        // `sqlparser` reads `GROUP BY ALL` as this dedicated node. MySQL reads
        // `ALL` as a reserved word and rejects the statement, and Doris follows
        // MySQL. So the parser and the backend disagree about what the text even
        // is — the parser differential in its purest form, and the one risk D3's
        // allowlist exists to keep narrow.
        //
        // Refusing is free: Doris rejects these statements anyway, so nothing
        // that works today stops working, and the client gets a clearer error
        // from the proxy than it would from the backend. Accepting a shape whose
        // parse is known not to match the backend's is the thing worth avoiding,
        // whether or not this particular one can be turned into a leak.
        GroupByExpr::All(_) => return unsupported("GROUP BY ALL"),
        GroupByExpr::Expressions(exprs, modifiers) => {
            if !modifiers.is_empty() {
                return unsupported("GROUP BY modifier");
            }
            for expr in exprs {
                walk_expr(expr, scope, v)?;
            }
        }
    }
    if let Some(having) = &mut select.having {
        walk_expr(having, scope, v)?;
    }
    Ok(())
}

fn walk_select_item<V: SiteVisitor>(
    item: &mut SelectItem,
    scope: &mut CteScope,
    v: &mut V,
) -> AnalysisResult<()> {
    match item {
        SelectItem::UnnamedExpr(expr) => walk_expr(expr, scope, v),
        SelectItem::ExprWithAlias { expr, .. } => walk_expr(expr, scope, v),
        SelectItem::ExprWithAliases { .. } => unsupported("multi-alias select item"),
        SelectItem::QualifiedWildcard(_, options) | SelectItem::Wildcard(options) => {
            // Unreachable through `MySqlDialect`, and kept deliberately.
            //
            // Every one of these six options is parsed only behind a dialect
            // predicate — `supports_select_wildcard_ilike`, `_exclude`,
            // `_except`, `_replace`, `_rename`, `_with_alias`
            // (`sqlparser-0.62.0/src/parser/mod.rs:18593-18620`). All six
            // default to `false` in `dialect/mod.rs`, and `dialect/mysql.rs`
            // overrides none of them, so the parser never even attempts the
            // syntax and the fields are structurally always `None`.
            //
            // Mutation testing therefore cannot kill mutants of this guard, and
            // no test can either. **That is not a reason to delete it.** It is
            // the D3 allowlist doing its job: if a later `sqlparser` turns one
            // of those predicates on for MySQL, a wildcard modifier would start
            // reaching the walk, and without this guard it would be accepted
            // silently rather than refused.
            if options.opt_ilike.is_some()
                || options.opt_exclude.is_some()
                || options.opt_except.is_some()
                || options.opt_replace.is_some()
                || options.opt_rename.is_some()
                || options.opt_alias.is_some()
            {
                return unsupported("wildcard modifier");
            }
            Ok(())
        }
    }
}

fn walk_table_with_joins<V: SiteVisitor>(
    table: &mut TableWithJoins,
    scope: &mut CteScope,
    v: &mut V,
) -> AnalysisResult<()> {
    walk_table_factor(&mut table.relation, scope, v)?;
    for join in &mut table.joins {
        walk_join(join, scope, v)?;
    }
    Ok(())
}

fn walk_join<V: SiteVisitor>(
    join: &mut Join,
    scope: &mut CteScope,
    v: &mut V,
) -> AnalysisResult<()> {
    // Every join operand is a reference site in its own right, including the
    // preserved side of an outer join.
    walk_table_factor(&mut join.relation, scope, v)?;

    // `global` is a ClickHouse distribution hint with no reference site, but it
    // is not part of the MySQL surface this proxy accepts.
    if join.global {
        return unsupported("GLOBAL join");
    }

    let constraint = match &mut join.join_operator {
        // Design D2's table argues wrapping is correct for inner and outer join
        // operands. These are those operators.
        JoinOperator::Join(c)
        | JoinOperator::Inner(c)
        | JoinOperator::Left(c)
        | JoinOperator::LeftOuter(c)
        | JoinOperator::Right(c)
        | JoinOperator::RightOuter(c)
        | JoinOperator::FullOuter(c)
        | JoinOperator::CrossJoin(c)
        | JoinOperator::StraightJoin(c) => c,
        // Semi and anti joins are not in design D2's position table. Their
        // interaction with a filtered operand is arguable rather than proven, so
        // per the project's standing rule they are refused, not improvised.
        JoinOperator::Semi(_)
        | JoinOperator::LeftSemi(_)
        | JoinOperator::RightSemi(_)
        | JoinOperator::Anti(_)
        | JoinOperator::LeftAnti(_)
        | JoinOperator::RightAnti(_) => return unsupported("semi/anti join"),
        JoinOperator::CrossApply | JoinOperator::OuterApply => return unsupported("APPLY join"),
        JoinOperator::AsOf { .. } => return unsupported("ASOF join"),
        JoinOperator::ArrayJoin | JoinOperator::LeftArrayJoin | JoinOperator::InnerArrayJoin => {
            return unsupported("ARRAY JOIN")
        }
    };

    match constraint {
        // The `ON` expression may itself contain subqueries.
        JoinConstraint::On(expr) => walk_expr(expr, scope, v),
        // `USING` and `NATURAL` match on column names, which a `SELECT *` wrap
        // preserves; neither introduces a reference site.
        JoinConstraint::Using(_) | JoinConstraint::Natural | JoinConstraint::None => Ok(()),
    }
}

fn walk_table_factor<V: SiteVisitor>(
    factor: &mut TableFactor,
    scope: &mut CteScope,
    v: &mut V,
) -> AnalysisResult<()> {
    // Phase one validates the node and descends into any children. It must not
    // hold a borrow into `factor` past its end, because phase two hands the
    // whole factor to the visitor, which may replace it.
    let named_relation = match factor {
        TableFactor::Table {
            name,
            alias,
            args,
            with_hints,
            version,
            with_ordinality,
            partitions,
            json_path,
            sample,
            index_hints,
        } => {
            if args.is_some() {
                return unsupported("table-valued function");
            }
            if !with_hints.is_empty() {
                return unsupported("table hint");
            }
            if version.is_some() {
                return unsupported("table version clause");
            }
            if *with_ordinality {
                return unsupported("WITH ORDINALITY");
            }
            if !partitions.is_empty() {
                return unsupported("partition selection");
            }
            if json_path.is_some() {
                return unsupported("JSON path on a table");
            }
            if sample.is_some() {
                return unsupported("TABLESAMPLE clause");
            }
            if !index_hints.is_empty() {
                return unsupported("index hint");
            }
            if let Some(alias) = alias {
                // A column alias list would have to be reproduced on the wrap,
                // and `AT` is a time-travel qualifier. Neither is supported.
                if !alias.columns.is_empty() {
                    return unsupported("table alias column list");
                }
                if alias.at.is_some() {
                    return unsupported("table alias AT clause");
                }
            }
            let table_name = TableName::from_object_name(name)?;
            Some(shadowed(scope, &table_name))
        }
        TableFactor::Derived {
            lateral,
            subquery,
            alias,
            sample,
        } => {
            if *lateral {
                return unsupported("LATERAL derived table");
            }
            if sample.is_some() {
                return unsupported("TABLESAMPLE clause");
            }
            if let Some(alias) = alias {
                if !alias.columns.is_empty() {
                    return unsupported("derived table column alias list");
                }
                if alias.at.is_some() {
                    return unsupported("derived table AT clause");
                }
            }
            if v.enter_derived(subquery)? {
                walk_query(subquery, scope, v)?;
            }
            None
        }
        TableFactor::NestedJoin {
            table_with_joins,
            alias,
        } => {
            if alias.is_some() {
                return unsupported("aliased parenthesised join");
            }
            walk_table_with_joins(table_with_joins, scope, v)?;
            None
        }
        other => return unsupported(format!("{} in a relation position", variant_name(other))),
    };

    if let Some(shadowed_by_cte) = named_relation {
        v.visit_relation(factor, shadowed_by_cte)?;
    }
    Ok(())
}

fn walk_order_by<V: SiteVisitor>(
    order_by: &mut OrderBy,
    scope: &mut CteScope,
    v: &mut V,
) -> AnalysisResult<()> {
    if order_by.interpolate.is_some() {
        return unsupported("INTERPOLATE clause");
    }
    match &mut order_by.kind {
        OrderByKind::All(_) => Ok(()),
        OrderByKind::Expressions(exprs) => {
            for expr in exprs {
                walk_order_by_expr(expr, scope, v)?;
            }
            Ok(())
        }
    }
}

fn walk_order_by_expr<V: SiteVisitor>(
    order_by: &mut OrderByExpr,
    scope: &mut CteScope,
    v: &mut V,
) -> AnalysisResult<()> {
    if order_by.with_fill.is_some() {
        return unsupported("WITH FILL clause");
    }
    walk_expr(&mut order_by.expr, scope, v)
}

fn walk_limit_clause<V: SiteVisitor>(
    limit: &mut LimitClause,
    scope: &mut CteScope,
    v: &mut V,
) -> AnalysisResult<()> {
    match limit {
        LimitClause::LimitOffset {
            limit,
            offset,
            limit_by,
        } => {
            // Unreachable through `MySqlDialect`, and kept deliberately.
            //
            // `LIMIT n BY expr` is ClickHouse syntax, parsed only when
            // `supports_limit_by()` is true (`parser/mod.rs:13089`). It defaults
            // to `false` in `dialect/mod.rs:1672` and is overridden only in
            // `clickhouse.rs` and `generic.rs`, so under this dialect the
            // parser stops before the `BY` keyword and `limit_by` is always
            // empty. No test can reach this guard; it is D3 cover against a
            // future dialect change, not dead code.
            if !limit_by.is_empty() {
                return unsupported("LIMIT BY clause");
            }
            if let Some(limit) = limit {
                walk_expr(limit, scope, v)?;
            }
            if let Some(offset) = offset {
                walk_expr(&mut offset.value, scope, v)?;
            }
            Ok(())
        }
        LimitClause::OffsetCommaLimit { offset, limit } => {
            walk_expr(offset, scope, v)?;
            walk_expr(limit, scope, v)
        }
    }
}

/// Walk an expression, descending into every subquery it can contain.
///
/// Every arm is written out. The final arm refuses; it never continues, because
/// an expression kind this proxy does not understand may hide a table reference
/// — which is exactly the shape of the leak invariant 6 forbids.
fn walk_expr<V: SiteVisitor>(
    expr: &mut Expr,
    scope: &mut CteScope,
    v: &mut V,
) -> AnalysisResult<()> {
    match expr {
        // Leaves.
        Expr::Identifier(_)
        | Expr::CompoundIdentifier(_)
        | Expr::Value(_)
        | Expr::TypedString(_)
        | Expr::Wildcard(_)
        | Expr::QualifiedWildcard(..) => Ok(()),

        // Single-operand predicates and operators.
        Expr::IsFalse(inner)
        | Expr::IsNotFalse(inner)
        | Expr::IsTrue(inner)
        | Expr::IsNotTrue(inner)
        | Expr::IsNull(inner)
        | Expr::IsNotNull(inner)
        | Expr::IsUnknown(inner)
        | Expr::IsNotUnknown(inner)
        | Expr::Nested(inner)
        | Expr::UnaryOp { expr: inner, .. }
        | Expr::Cast { expr: inner, .. }
        | Expr::Extract { expr: inner, .. }
        | Expr::Collate { expr: inner, .. } => walk_expr(inner, scope, v),

        Expr::IsDistinctFrom(left, right) | Expr::IsNotDistinctFrom(left, right) => {
            walk_expr(left, scope, v)?;
            walk_expr(right, scope, v)
        }
        Expr::BinaryOp { left, right, .. } => {
            walk_expr(left, scope, v)?;
            walk_expr(right, scope, v)
        }

        Expr::Like {
            expr, pattern, any, ..
        }
        | Expr::ILike {
            expr, pattern, any, ..
        } => {
            if *any {
                return unsupported("LIKE ANY");
            }
            walk_expr(expr, scope, v)?;
            walk_expr(pattern, scope, v)
        }
        Expr::RLike { expr, pattern, .. } => {
            walk_expr(expr, scope, v)?;
            walk_expr(pattern, scope, v)
        }

        Expr::InList { expr, list, .. } => {
            walk_expr(expr, scope, v)?;
            for item in list {
                walk_expr(item, scope, v)?;
            }
            Ok(())
        }
        // `x IN (SELECT ...)` — a reference site inside a predicate. Design D2
        // wraps the relation inside the subquery like any other.
        Expr::InSubquery { expr, subquery, .. } => {
            walk_expr(expr, scope, v)?;
            walk_query(subquery, scope, v)
        }
        Expr::Between {
            expr, low, high, ..
        } => {
            walk_expr(expr, scope, v)?;
            walk_expr(low, scope, v)?;
            walk_expr(high, scope, v)
        }
        Expr::Position { expr, r#in } => {
            walk_expr(expr, scope, v)?;
            walk_expr(r#in, scope, v)
        }
        Expr::Substring {
            expr,
            substring_from,
            substring_for,
            ..
        } => {
            walk_expr(expr, scope, v)?;
            if let Some(from) = substring_from {
                walk_expr(from, scope, v)?;
            }
            if let Some(for_) = substring_for {
                walk_expr(for_, scope, v)?;
            }
            Ok(())
        }
        Expr::Trim {
            expr,
            trim_what,
            trim_characters,
            ..
        } => {
            walk_expr(expr, scope, v)?;
            if let Some(what) = trim_what {
                walk_expr(what, scope, v)?;
            }
            if let Some(characters) = trim_characters {
                for character in characters {
                    walk_expr(character, scope, v)?;
                }
            }
            Ok(())
        }
        Expr::Ceil { expr, field } | Expr::Floor { expr, field } => {
            match field {
                CeilFloorKind::DateTimeField(_) | CeilFloorKind::Scale(_) => {}
            }
            walk_expr(expr, scope, v)
        }
        Expr::Interval(interval) => walk_expr(&mut interval.value, scope, v),
        Expr::Tuple(exprs) => {
            for expr in exprs {
                walk_expr(expr, scope, v)?;
            }
            Ok(())
        }
        Expr::Case {
            operand,
            conditions,
            else_result,
            ..
        } => {
            if let Some(operand) = operand {
                walk_expr(operand, scope, v)?;
            }
            for when in conditions {
                walk_expr(&mut when.condition, scope, v)?;
                walk_expr(&mut when.result, scope, v)?;
            }
            if let Some(else_result) = else_result {
                walk_expr(else_result, scope, v)?;
            }
            Ok(())
        }
        // Correlated `EXISTS` and scalar subqueries: reference sites that an
        // outer predicate would never constrain.
        Expr::Exists { subquery, .. } => walk_query(subquery, scope, v),
        Expr::Subquery(query) => walk_query(query, scope, v),
        Expr::Function(function) => walk_function(function, scope, v),

        // Everything else refuses, named by its AST variant.
        other => unsupported(format!("{} expression", variant_name(other))),
    }
}

fn walk_function<V: SiteVisitor>(
    function: &mut sqlparser::ast::Function,
    scope: &mut CteScope,
    v: &mut V,
) -> AnalysisResult<()> {
    if function.uses_odbc_syntax {
        return unsupported("ODBC function syntax");
    }
    if !function.within_group.is_empty() {
        return unsupported("WITHIN GROUP clause");
    }
    match &mut function.over {
        None => {}
        Some(WindowType::NamedWindow(_)) => return unsupported("named window reference"),
        Some(WindowType::WindowSpec(spec)) => {
            if spec.window_name.is_some() {
                return unsupported("named window reference");
            }
            if spec.window_frame.is_some() {
                return unsupported("window frame clause");
            }
            for expr in &mut spec.partition_by {
                walk_expr(expr, scope, v)?;
            }
            for order_by in &mut spec.order_by {
                walk_order_by_expr(order_by, scope, v)?;
            }
        }
    }
    if let Some(filter) = &mut function.filter {
        walk_expr(filter, scope, v)?;
    }
    walk_function_arguments(&mut function.parameters, scope, v)?;
    walk_function_arguments(&mut function.args, scope, v)
}

fn walk_function_arguments<V: SiteVisitor>(
    arguments: &mut FunctionArguments,
    scope: &mut CteScope,
    v: &mut V,
) -> AnalysisResult<()> {
    match arguments {
        FunctionArguments::None => Ok(()),
        FunctionArguments::Subquery(query) => walk_query(query, scope, v),
        FunctionArguments::List(list) => {
            if !list.clauses.is_empty() {
                return unsupported("function argument clause");
            }
            for arg in &mut list.args {
                let arg_expr = match arg {
                    FunctionArg::Unnamed(arg_expr) => arg_expr,
                    FunctionArg::Named { arg, .. } => arg,
                    FunctionArg::ExprNamed { name, arg, .. } => {
                        walk_expr(name, scope, v)?;
                        arg
                    }
                };
                match arg_expr {
                    FunctionArgExpr::Expr(expr) => walk_expr(expr, scope, v)?,
                    FunctionArgExpr::Wildcard | FunctionArgExpr::QualifiedWildcard(_) => {}
                    FunctionArgExpr::WildcardWithOptions(_) => {
                        return unsupported("wildcard modifier in function argument")
                    }
                }
            }
            Ok(())
        }
    }
}

fn walk_insert<V: SiteVisitor>(
    insert: &mut Insert,
    scope: &mut CteScope,
    v: &mut V,
) -> AnalysisResult<()> {
    if !insert.optimizer_hints.is_empty() {
        return unsupported("optimizer hint");
    }
    if insert.or.is_some() {
        return unsupported("INSERT OR clause");
    }
    if insert.table_alias.is_some() {
        return unsupported("INSERT table alias");
    }
    if insert.partitioned.is_some() {
        return unsupported("INSERT partition clause");
    }
    if insert.on.is_some() {
        return unsupported("ON DUPLICATE KEY / ON CONFLICT clause");
    }
    if insert.returning.is_some() {
        return unsupported("RETURNING clause");
    }
    if insert.output.is_some() {
        return unsupported("OUTPUT clause");
    }
    if insert.insert_alias.is_some() {
        return unsupported("INSERT row alias");
    }
    if insert.settings.is_some() {
        return unsupported("SETTINGS clause");
    }
    if insert.format_clause.is_some() {
        return unsupported("FORMAT clause");
    }
    // Unreachable through *any* dialect in `sqlparser` 0.62, and kept
    // deliberately.
    //
    // Stronger than the usual "not under MySQL" argument: all four fields are
    // assigned in exactly one place in the entire parser
    // (`parser/mod.rs:18069-18072`), and that assignment sets every one of them
    // to `None`/empty. There is no parse path for Snowflake's `INSERT ALL` /
    // `INSERT FIRST` at all — the AST models the construct, the parser does not
    // yet build it. So no input reaches this guard and no test can kill a mutant
    // of it.
    //
    // It stays because the day `sqlparser` learns that syntax, a multi-table
    // INSERT would arrive carrying several write targets that `walk_insert`
    // enumerates only one of — exactly the incomplete enumeration invariant 6
    // forbids.
    if insert.multi_table_insert_type.is_some()
        || !insert.multi_table_into_clauses.is_empty()
        || !insert.multi_table_when_clauses.is_empty()
        || insert.multi_table_else_clause.is_some()
    {
        return unsupported("multi-table INSERT");
    }

    match &insert.table {
        TableObject::TableName(name) => v.visit_write_target(name)?,
        TableObject::TableFunction(_) => return unsupported("INSERT into a table function"),
        TableObject::TableQuery(_) => return unsupported("INSERT into a subquery"),
    }
    for assignment in &mut insert.assignments {
        walk_expr(&mut assignment.value, scope, v)?;
    }
    if let Some(source) = &mut insert.source {
        walk_query(source, scope, v)?;
    }
    Ok(())
}

fn walk_update<V: SiteVisitor>(
    update: &mut Update,
    scope: &mut CteScope,
    v: &mut V,
) -> AnalysisResult<()> {
    if !update.optimizer_hints.is_empty() {
        return unsupported("optimizer hint");
    }
    if update.or.is_some() {
        return unsupported("UPDATE OR clause");
    }
    if update.returning.is_some() {
        return unsupported("RETURNING clause");
    }
    if update.output.is_some() {
        return unsupported("OUTPUT clause");
    }

    walk_table_with_joins(&mut update.table, scope, v)?;
    match &mut update.from {
        None => {}
        Some(UpdateTableFromKind::BeforeSet(tables) | UpdateTableFromKind::AfterSet(tables)) => {
            for table in tables {
                walk_table_with_joins(table, scope, v)?;
            }
        }
    }
    for assignment in &mut update.assignments {
        walk_expr(&mut assignment.value, scope, v)?;
    }
    if let Some(selection) = &mut update.selection {
        walk_expr(selection, scope, v)?;
    }
    for order_by in &mut update.order_by {
        walk_order_by_expr(order_by, scope, v)?;
    }
    if let Some(limit) = &mut update.limit {
        walk_expr(limit, scope, v)?;
    }
    Ok(())
}

fn walk_delete<V: SiteVisitor>(
    delete: &mut sqlparser::ast::Delete,
    scope: &mut CteScope,
    v: &mut V,
) -> AnalysisResult<()> {
    if !delete.optimizer_hints.is_empty() {
        return unsupported("optimizer hint");
    }
    if delete.returning.is_some() {
        return unsupported("RETURNING clause");
    }
    if delete.output.is_some() {
        return unsupported("OUTPUT clause");
    }

    for name in &delete.tables {
        v.visit_write_target(name)?;
    }
    let from = match &mut delete.from {
        FromTable::WithFromKeyword(tables) | FromTable::WithoutKeyword(tables) => tables,
    };
    for table in from {
        walk_table_with_joins(table, scope, v)?;
    }
    if let Some(using) = &mut delete.using {
        for table in using {
            walk_table_with_joins(table, scope, v)?;
        }
    }
    if let Some(selection) = &mut delete.selection {
        walk_expr(selection, scope, v)?;
    }
    for order_by in &mut delete.order_by {
        walk_order_by_expr(order_by, scope, v)?;
    }
    if let Some(limit) = &mut delete.limit {
        walk_expr(limit, scope, v)?;
    }
    Ok(())
}

/// Build an [`Ident`] preserving the quoting of an existing one.
pub(crate) fn same_quoting(model: &Ident, value: &str) -> Ident {
    match model.quote_style {
        Some(quote) => Ident::with_quote(quote, value),
        None => Ident::new(value),
    }
}

#[cfg(test)]
mod tests {
    use super::contains_executable_comment;

    /// The executable-comment scanner's decision surface, exercised directly.
    ///
    /// The end-to-end tests in `tests/analyze_walk.rs` pin the *verdict* for a
    /// handful of statements, which leaves almost every internal transition of
    /// this state machine free to be wrong without a test noticing — mutation
    /// testing found 22 surviving mutants in it, including one deleting the `#`
    /// line-comment arm outright. A scanner that decides refusal deserves to be
    /// tested as a scanner.
    ///
    /// Each row is `(input, contains_marker)`. The interesting rows are the ones
    /// where a marker is *present in the text* but inert because of where it
    /// sits, and the mirrored ones where the same marker is live.
    #[test]
    fn executable_comment_scanner_decision_surface() {
        const CASES: &[(&str, bool)] = &[
            // -- nothing to find -------------------------------------------
            ("", false),
            ("   ", false),
            ("SELECT 1", false),
            ("SELECT * FROM sales.orders", false),
            // -- the marker itself, at each position in the input ----------
            ("/*!", true),
            ("/*!50000 SELECT 1 */", true),
            ("SELECT 1 /*!", true),
            ("/*! */ SELECT 1", true),
            ("SELECT 1 /*!99999 UNION SELECT 2 */", true),
            // A lone `/*` is an ordinary comment; only `!` makes it a gate.
            ("SELECT 1 /* */", false),
            ("SELECT 1 /*", false),
            ("SELECT /*+ hint */ 1", false),
            // A `/` that starts no comment must not consume what follows it —
            // otherwise division swallows a live marker further along the line.
            ("SELECT 1/2 /*! live */", true),
            ("SELECT a/b/c /*! live */", true),
            ("SELECT 1/2", false),
            // -- `#` line comments -----------------------------------------
            ("SELECT 1 # /*! inert */", false),
            ("SELECT 1 # inert\n", false),
            ("# /*! inert */", false),
            // ...but the comment ends at the newline.
            ("SELECT 1 # inert\n/*! live */", true),
            ("#\n/*!", true),
            // -- `--` line comments ----------------------------------------
            ("SELECT 1 -- /*! inert */", false),
            ("SELECT 1 --\n", false),
            ("SELECT 1 --", false),
            ("SELECT 1 -- inert\n/*! live */", true),
            // `--` needs whitespace after it to be a comment; `1--2` is
            // arithmetic, so the marker after it is live.
            ("SELECT 1--2 /*! live */", true),
            ("SELECT 1-2 /*! live */", true),
            ("SELECT 1 - -2 /*! live */", true),
            // -- `/* */` block comments ------------------------------------
            ("SELECT /* inert /*! */ 1", false),
            ("SELECT /* inert */ 1", false),
            // Block comments do not nest in MySQL: the first `*/` ends it, so a
            // marker after that `*/` is live even though it looks nested.
            ("SELECT /* a /*! */ */ 1", false),
            ("SELECT /* a */ /*! live */ 1", true),
            ("SELECT /* a */ 1 /*! live */", true),
            // A `/` inside a block comment must not be mistaken for its end.
            ("SELECT /* a/b /*! inert */ 1", false),
            ("SELECT /* a/b */ /*! live */ 1", true),
            // A `*` inside a block comment must not end it either.
            ("SELECT /* a*b /*! inert */ 1", false),
            // An unterminated block comment swallows the rest of the input.
            ("SELECT /* unterminated /*! inert", false),
            // The cursor must resume *after* the closing `*/`, not before it.
            //
            // These pin that specifically, because nothing else does. Both put a
            // delimiter in the last two bytes of the comment body, which is
            // exactly the window a cursor that resumed too early would re-read
            // in `Normal` state: the quote would open a string that swallows the
            // live marker following the comment, and the `/*` would re-enter the
            // comment it just left and never terminate.
            ("SELECT /* x' */ /*! live */", true),
            ("SELECT /* x\" */ /*! live */", true),
            ("SELECT /* x` */ /*! live */", true),
            ("SELECT 1 /*/**/", false),
            ("SELECT 1 /*/**/ /*! live */", true),
            // -- string literals and quoted identifiers --------------------
            ("SELECT '/*! inert */'", false),
            ("SELECT \"/*! inert */\"", false),
            ("SELECT `/*! inert */`", false),
            ("SELECT '/*!' , '/*!'", false),
            // ...and a marker after the literal closes is live.
            ("SELECT '/*! inert */' /*! live */", true),
            ("SELECT \"x\" /*! live */", true),
            ("SELECT `x` /*! live */", true),
            // Doubled delimiters are literal and do not close the string, so
            // the marker stays inside it.
            ("SELECT 'it''s /*! inert */'", false),
            ("SELECT \"say \"\"hi\"\" /*! inert */\"", false),
            ("SELECT `a``b /*! inert */`", false),
            // ...but the string does eventually close.
            ("SELECT 'it''s' /*! live */", true),
            ("SELECT 'a''' /*! live */", true),
            ("SELECT `a``b` /*! live */", true),
            // An unterminated literal swallows the rest of the input.
            ("SELECT 'unterminated /*! inert", false),
            // One quote kind does not close another.
            ("SELECT 'a \" b' /*! live */", true),
            ("SELECT \"a ' b\" /*! live */", true),
            // A comment marker inside a string does not start a comment, so the
            // later marker is still live.
            ("SELECT '-- not a comment' /*! live */", true),
            ("SELECT '# not a comment' /*! live */", true),
            ("SELECT '/* not a comment' /*! live */", true),
            // Conversely a quote inside a comment does not open a string.
            ("SELECT 1 -- it's fine\n/*! live */", true),
            ("SELECT 1 /* it's fine */ /*! live */", true),
            ("SELECT 1 # it's fine\n/*! live */", true),
        ];

        for (input, expected) in CASES {
            assert_eq!(
                contains_executable_comment(input),
                *expected,
                "scanner disagreed on {input:?}"
            );
        }
    }

    /// The scanner must terminate and must not panic on any input, including
    /// the truncated and adversarial shapes a client can send. It runs on
    /// attacker-controlled bytes, so invariant 2 applies to it directly.
    #[test]
    fn executable_comment_scanner_never_panics_on_partial_input() {
        // Every prefix of a statement that mixes all the delimiters, which
        // between them reach every state with a truncated tail.
        let sql = "SELECT 'a''b', \"c\"\"d\", `e``f` /* g/h */ -- i\n# j\n/*!99999 k */";
        for end in 0..=sql.len() {
            if sql.is_char_boundary(end) {
                let _ = contains_executable_comment(&sql[..end]);
            }
        }

        // Delimiters with nothing after them.
        for input in [
            "'", "\"", "`", "/", "/*", "/*!", "-", "--", "#", "*", "*/", "''", "``", "\"\"", "/*/",
            "/**", "--\n", "#\n",
        ] {
            let _ = contains_executable_comment(input);
        }
    }
}
