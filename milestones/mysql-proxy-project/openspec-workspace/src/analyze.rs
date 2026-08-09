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

    /// The parts as written, outermost qualifier first.
    pub fn parts(&self) -> &[String] {
        &self.parts
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
}

impl StatementKind {
    /// `INSERT`, `REPLACE`, `UPDATE` and `DELETE` are writes. The MVP refuses
    /// them against policy-bearing tables rather than constraining them.
    pub fn is_write(self) -> bool {
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
        other => unsupported(format!("{} statement", variant_name(other))),
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
        GroupByExpr::All(modifiers) => {
            if !modifiers.is_empty() {
                return unsupported("GROUP BY ALL modifier");
            }
        }
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
