//! Predicate injection by derived-table wrapping (design D2).
//!
//! Owned by the `row-filter-rewrite` capability. See
//! `openspec/changes/add-row-filter-proxy-mvp/specs/row-filter-rewrite/spec.md`.
//!
//! # Why a derived table and not `AND <policy>`
//!
//! Appending the policy to the user's `WHERE` is wrong twice over:
//!
//! - `WHERE a OR b AND policy` binds as `a OR (b AND policy)`, so `a` stays
//!   unconstrained. Getting this right would mean parenthesising the user's
//!   predicate correctly in every shape.
//! - Injecting into the enclosing `WHERE` turns a `LEFT JOIN` into inner-join
//!   semantics, which changes the result set and breaks invariant 4.
//!
//! Wrapping the *relation* sidesteps both: the predicate applies before the
//! relation participates in anything, so the surrounding syntax — precedence,
//! join type, nesting depth — cannot weaken it.
//!
//! ```sql
//! -- before
//! FROM sales.orders AS o
//! -- after
//! FROM (SELECT * FROM sales.orders WHERE `region` IN ('APAC', 'EMEA')) AS o
//! ```

use sqlparser::ast::helpers::attached_token::AttachedToken;
use sqlparser::ast::{
    Expr, GroupByExpr, Ident, ObjectName, Query, Select, SelectFlavor, SelectItem, SetExpr,
    Statement, TableAlias, TableFactor, TableWithJoins, Value, WildcardAdditionalOptions,
};

use crate::analyze::{
    analyze, relation_table_ref, walk_statement, AnalysisResult, SiteVisitor, StatementKind,
    TableName, TableRef,
};
use crate::error::RefusalReason;
use crate::policy::{
    PermittedValue, Policy, PolicyDecision, PolicySet, TableRef as PolicyReference,
};

/// Render a configured value as a `sqlparser` literal.
///
/// Mapped variant by variant. [`PermittedValue::to_sql_literal`] is deliberately
/// **not** used: it returns an already-quoted literal, which as the *content* of
/// a `SingleQuotedString` would be escaped a second time by the renderer.
///
/// Invariant 3 forbids emitting these as `?` placeholders: a rewrite must not
/// change the number or order of parameter placeholders, or the client's own
/// parameter binding — computed against the statement it wrote — becomes wrong.
fn permitted_value_expr(value: &PermittedValue) -> Expr {
    let literal = match value {
        PermittedValue::Text(text) => Value::SingleQuotedString(escape_quotes(text)),
        PermittedValue::Integer(number) => Value::Number(number.to_string(), false),
    };
    Expr::Value(literal.with_empty_span())
}

/// Double the single quotes in a configured value.
///
/// The renderer's own escaping is context-sensitive: it doubles a lone `'`, but
/// passes an existing `''` pair through untouched, and never escapes `\`. That
/// ambiguity is resolved not here but in `policy-config`, which refuses to load
/// a permitted value containing a backslash or a `''` pair — so by the time a
/// value reaches this function it can only contain isolated quotes, which both
/// this function and the renderer handle identically and correctly.
///
/// Quotes are doubled here anyway, rather than left to the renderer, so that the
/// literal stays correct if that validation is ever relaxed. **Backslashes are
/// deliberately not doubled.** Doubling one is correct under MySQL's default
/// `sql_mode` and wrong under `NO_BACKSLASH_ESCAPES`, where `\\` matches two
/// literal characters; the proxy does not control the backend's `sql_mode` and
/// must not ship a control whose correctness depends on it. Refusing the value
/// at load time, as `policy-config` does, is correct under both.
fn escape_quotes(value: &str) -> String {
    debug_assert!(
        !value.contains('\\'),
        "permitted value {value:?} contains a backslash; `policy-config` rejects these at load because no rendering of one is correct under every `sql_mode`"
    );
    value.replace('\'', "''")
}

/// Who is asking, where they are, and the policy set to ask.
///
/// Threaded through every step so that no part of the rewriter can consult a
/// policy without also supplying the session's current database — which is what
/// makes [`PolicyDecision::Unresolvable`] reachable at all.
#[derive(Clone, Copy)]
struct Context<'a> {
    user: &'a str,
    current_database: Option<&'a str>,
    policies: &'a PolicySet,
}

impl<'a> Context<'a> {
    /// The decision for a table name as written.
    fn decide_name(&self, name: &TableName) -> PolicyDecision<'a> {
        let reference = match name.qualifier() {
            Some(database) => PolicyReference::qualified(database, name.leaf()),
            None => PolicyReference::unqualified(name.leaf()),
        };
        self.policies
            .lookup(self.user, &reference, self.current_database)
    }

    /// The decision at an enumerated reference site.
    ///
    /// A reference that resolves to a common table expression is not a stored
    /// table: it carries no policy of its own, and cannot be unresolvable
    /// either. Every relation its body reads was constrained at its own site.
    fn decide_site(&self, table: &TableRef) -> PolicyDecision<'a> {
        if table.is_cte_reference {
            return PolicyDecision::Unrestricted;
        }
        self.decide_name(&table.name)
    }
}

/// What the statement's table references amount to, taken together.
///
/// Three states, matching [`PolicyDecision`]. Collapsing `Unresolvable` into
/// `Unrestricted` here would be a live bypass: [`rewrite_statement`] returns the
/// original SQL when nothing is restricted, so an unresolvable reference would
/// reach Doris unconstrained *before* the wrapper ever ran.
#[derive(Debug, PartialEq, Eq)]
enum Coverage {
    /// Nothing in the statement is policy-bearing.
    Unrestricted,
    /// At least one reference must be constrained.
    Restricted,
    /// At least one reference cannot be shown *not* to be a policy table.
    Unresolvable,
}

fn coverage(tables: &[TableRef], cx: Context<'_>) -> Coverage {
    let mut coverage = Coverage::Unrestricted;
    for table in tables {
        match cx.decide_site(table) {
            // Dominates the others: nothing elsewhere in the statement can make
            // an unresolvable reference safe to forward.
            PolicyDecision::Unresolvable => return Coverage::Unresolvable,
            PolicyDecision::Restricted(_) => coverage = Coverage::Restricted,
            PolicyDecision::Unrestricted => {}
        }
    }
    coverage
}

/// The refusal for a reference that cannot be resolved to a table.
///
/// A variant of its own rather than an unsupported *shape*: the shape is fine,
/// the session state is ambiguous, and an operator seeing many of these should
/// be looking at clients connecting without a default schema rather than at a
/// parser gap. The client sees the same words either way — a refusal that named
/// the reference it declined to resolve would be an oracle for which tables
/// carry a policy (design D5).
fn unresolvable_refusal() -> RefusalReason {
    RefusalReason::UnresolvableTableReference
}

/// Parse, constrain and render one statement for forwarding.
///
/// Returns the SQL to forward, or the reason the statement is refused. There is
/// no third outcome, and in particular no path that returns SQL whose
/// policy-bearing references were not all constrained.
pub fn rewrite_statement(
    sql: &str,
    user: &str,
    current_database: Option<&str>,
    policies: &PolicySet,
) -> AnalysisResult<String> {
    let cx = Context {
        user,
        current_database,
        policies,
    };

    let mut analysis = match analyze(sql) {
        Ok(analysis) => analysis,
        Err(reason) => {
            // Design D6. A user with no policy has nothing to protect, so an
            // unanalysable statement is forwarded as written. A user with any
            // policy gets a refusal, because "does it touch a policy table?"
            // cannot be answered for SQL that did not parse.
            return if policies.has_any_policy(user) {
                Err(reason)
            } else {
                Ok(sql.to_string())
            };
        }
    };

    match coverage(&analysis.tables, cx) {
        // An unqualified reference the session cannot resolve might name a
        // policy table. Refusing is the only answer that does not depend on
        // guessing which.
        Coverage::Unresolvable => return Err(unresolvable_refusal()),
        // Nothing to constrain. Forward the original text rather than a
        // re-rendered AST, so a statement the proxy has no business changing
        // reaches Doris byte for byte.
        Coverage::Unrestricted => return Ok(sql.to_string()),
        Coverage::Restricted => {}
    }

    if analysis.kind.is_write() {
        // Constraining writes is out of scope for the MVP, and forwarding one
        // unconstrained would let a user modify rows outside their slice.
        return Err(RefusalReason::WriteToRestrictedTable);
    }
    debug_assert_eq!(analysis.kind, StatementKind::Select);

    let mut wrapper = Wrapper { cx };
    walk_statement(&mut analysis.statement, &mut wrapper)?;

    let rewritten = analysis.statement.to_string();

    // Task 5.3. An independent re-analysis of the *rendered* output, not a
    // count kept by the code that did the wrapping. If any reference to a
    // policy-bearing table is still reachable outside a guard, the statement
    // does not go out.
    //
    // NOT EXERCISED END TO END, AND DELIBERATELY SO. This branch cannot be
    // reached through `rewrite_statement` while `Wrapper` is correct: the net
    // only fires when the wrapper has already failed. Replacing this whole
    // condition with `if false` changes nothing observable in any test, which
    // mutation testing confirmed.
    //
    // The two ways to give it teeth are not equal. Testing
    // `all_policy_refs_guarded` directly tests the subject of the check rather
    // than a proxy for it, and `tests/rewrite_verifier.rs` does exactly that —
    // that file is where this behaviour is pinned, and it is where to add a
    // case when this logic changes. The alternative, a fault-injection seam
    // that makes correct wrapping fail on demand, was considered and
    // **rejected**: it could not be `#[cfg(test)]`-only if an integration test
    // were to reach it, so it would ship as a switch that disables row
    // filtering, living inside the control it disables. An untested defensive
    // branch is a smaller liability than a build that can turn the filter off.
    //
    // What is therefore not covered is the *wiring* between wrapper and net,
    // which is a much narrower claim than the net's own behaviour.
    if !all_policy_refs_guarded(&rewritten, user, current_database, policies)? {
        return Err(RefusalReason::UnsupportedShape {
            construct: "statement whose policy-bearing references could not all be constrained"
                .to_string(),
        });
    }

    Ok(rewritten)
}

/// Replaces each policy-bearing relation with a guard derived table.
struct Wrapper<'a> {
    cx: Context<'a>,
}

impl SiteVisitor for Wrapper<'_> {
    fn visit_relation(
        &mut self,
        factor: &mut TableFactor,
        shadowed_by_cte: bool,
    ) -> AnalysisResult<()> {
        let table_ref = relation_table_ref(factor, shadowed_by_cte)?;
        match self.cx.decide_site(&table_ref) {
            PolicyDecision::Restricted(policy) => {
                *factor = guard(factor, &table_ref, policy);
                Ok(())
            }
            // No policy for this user and table: left exactly as written.
            PolicyDecision::Unrestricted => Ok(()),
            // Unreachable **because `coverage` refuses first**, in
            // `rewrite_statement` above — not because the case cannot arise.
            // Its mutant is immortal for that reason: no test can drive this
            // arm while that pre-check stands. Relax or reorder the
            // `Coverage::Unresolvable` handling there and this becomes live,
            // so it is repeated here rather than left to the caller.
            PolicyDecision::Unresolvable => Err(unresolvable_refusal()),
        }
    }

    fn visit_write_target(&mut self, _name: &ObjectName) -> AnalysisResult<()> {
        // Unreachable **because `rewrite_statement` refuses writes first**: a
        // write touching a policy table returns `WriteToRestrictedTable` before
        // any wrapping is attempted, and one touching none is forwarded without
        // being walked at all. Its mutant is immortal for that reason, not
        // because returning `Ok` here would be harmless — it would forward a
        // write with an unwrapped target.
        //
        // If write statements are ever brought into scope, this is one of the
        // places that stops being dead. Do not delete it as unused code; the
        // reason it is unused lives in another function.
        Err(RefusalReason::UnsupportedShape {
            construct: "write target reached the rewriter".to_string(),
        })
    }
}

/// Build `(SELECT * FROM <table> WHERE <policy>) AS <effective name>`.
///
/// The alias is the reference's *effective name* — the user's alias if present,
/// else the bare table name — so `o.total` and `orders.total` still resolve.
/// `SELECT *` keeps the column set, order and types of the underlying table,
/// which is what invariant 4 and the "column shape is unchanged" scenario need.
fn guard(original: &TableFactor, table_ref: &TableRef, policy: &Policy) -> TableFactor {
    let mut inner = original.clone();
    let alias_model = match &mut inner {
        TableFactor::Table { name, alias, .. } => {
            let model = alias
                .as_ref()
                .map(|a| a.name.clone())
                .or_else(|| name.0.last().and_then(|p| p.as_ident().cloned()))
                .unwrap_or_else(|| Ident::new(table_ref.effective_name()));
            // The alias moves to the wrapper; the inner reference keeps the
            // table's real name so the predicate resolves against it.
            *alias = None;
            model
        }
        // `visit_relation` only ever hands us a `TableFactor::Table`.
        _ => Ident::new(table_ref.effective_name()),
    };

    TableFactor::Derived {
        lateral: false,
        subquery: Box::new(guard_query(inner, policy)),
        alias: Some(TableAlias {
            explicit: true,
            name: crate::analyze::same_quoting(&alias_model, &alias_model.value),
            columns: Vec::new(),
            at: None,
        }),
        sample: None,
    }
}

/// The body of a guard: `SELECT * FROM <relation> WHERE <policy>`, and nothing
/// else. `relation` must already have had any alias stripped.
fn guard_query(relation: TableFactor, policy: &Policy) -> Query {
    let select = Select {
        select_token: AttachedToken::empty(),
        optimizer_hints: Vec::new(),
        distinct: None,
        select_modifiers: None,
        top: None,
        top_before_distinct: false,
        projection: vec![SelectItem::Wildcard(WildcardAdditionalOptions::default())],
        exclude: None,
        into: None,
        from: vec![TableWithJoins {
            relation,
            joins: Vec::new(),
        }],
        lateral_views: Vec::new(),
        prewhere: None,
        selection: Some(policy_predicate(policy)),
        connect_by: Vec::new(),
        group_by: GroupByExpr::Expressions(Vec::new(), Vec::new()),
        cluster_by: Vec::new(),
        distribute_by: Vec::new(),
        sort_by: Vec::new(),
        having: None,
        named_window: Vec::new(),
        qualify: None,
        window_before_qualify: false,
        value_table_mode: None,
        flavor: SelectFlavor::Standard,
    };
    Query {
        with: None,
        body: Box::new(SetExpr::Select(Box::new(select))),
        order_by: None,
        limit_clause: None,
        fetch: None,
        locks: Vec::new(),
        for_clause: None,
        settings: None,
        format_clause: None,
        pipe_operators: Vec::new(),
    }
}

/// `<column> IN (<permitted values>)`.
///
/// The column is backtick-quoted so a policy column that collides with a
/// reserved word still parses.
///
/// There is deliberately no branch for an empty permitted set. `policy-config`
/// rejects one at load time — "An empty permitted set is rejected as
/// configuration error" — so [`Policy::permitted_values`] is never empty and the
/// state is unrepresentable rather than merely unexpected. Were it ever reached
/// in a release build the rendered `IN ()` is a syntax error Doris refuses, so
/// even that failure returns no rows.
fn policy_predicate(policy: &Policy) -> Expr {
    debug_assert!(
        !policy.permitted_values().is_empty(),
        "policy on {} has an empty permitted set, which validation should have rejected",
        policy.table()
    );
    Expr::InList {
        expr: Box::new(Expr::Identifier(Ident::with_quote('`', policy.column()))),
        list: policy
            .permitted_values()
            .iter()
            .map(permitted_value_expr)
            .collect(),
        negated: false,
    }
}

/// Whether every reference to a policy-bearing table in `sql` sits inside a
/// guard produced by [`rewrite_statement`].
///
/// This is the fail-closed gate of task 5.3, and is deliberately usable from
/// tests: it re-parses the rewritten text and answers the question the spec
/// actually asks — "does the statement reaching Doris constrain every
/// policy-bearing reference?" — rather than trusting a bookkeeping counter kept
/// by the code under test.
pub fn all_policy_refs_guarded(
    sql: &str,
    user: &str,
    current_database: Option<&str>,
    policies: &PolicySet,
) -> AnalysisResult<bool> {
    let mut statement = crate::analyze::parse_single(sql)?;
    let mut verifier = Verifier {
        cx: Context {
            user,
            current_database,
            policies,
        },
        unguarded: false,
    };
    walk_statement(&mut statement, &mut verifier)?;
    Ok(!verifier.unguarded)
}

struct Verifier<'a> {
    cx: Context<'a>,
    unguarded: bool,
}

impl SiteVisitor for Verifier<'_> {
    fn visit_relation(
        &mut self,
        factor: &mut TableFactor,
        shadowed_by_cte: bool,
    ) -> AnalysisResult<()> {
        let table_ref = relation_table_ref(factor, shadowed_by_cte)?;
        // Reached without having been stopped by `enter_derived`, so this
        // reference is not inside a guard. An unresolvable one counts as
        // unguarded: it might name a policy table, and the whole point of this
        // pass is that it cannot be talked out of a refusal.
        if !self.cx.decide_site(&table_ref).is_unrestricted() {
            self.unguarded = true;
        }
        Ok(())
    }

    fn visit_write_target(&mut self, name: &ObjectName) -> AnalysisResult<()> {
        let table = TableName::from_object_name(name)?;
        if !self.cx.decide_name(&table).is_unrestricted() {
            self.unguarded = true;
        }
        Ok(())
    }

    fn enter_derived(&mut self, subquery: &mut Query) -> AnalysisResult<bool> {
        // A recognised guard is proof that the relation inside it is
        // constrained, so the walk stops rather than reporting that relation as
        // an unguarded reference to the policy table.
        Ok(!is_guard(subquery, self.cx))
    }
}

/// Structural recognition of a guard: exactly
/// `SELECT * FROM <policy table> WHERE <that table's policy predicate>`.
///
/// The check is made by *rebuilding* what [`guard`] would have produced for the
/// relation found inside, rendering it, and parsing it back — so both sides of
/// the comparison have been through the same render-and-parse path. Comparing a
/// freshly constructed AST against a parsed one would be wrong: `sqlparser`'s
/// literal rendering is not idempotent (see [`escape_quotes`]), and a
/// mismatch there would silently downgrade a guard to "not recognised".
///
/// The whole `Query` is compared, so a subquery that merely resembles a guard —
/// one with a join, an extra clause, or a different predicate — is not accepted,
/// and the walk descends into it as usual.
fn is_guard(query: &Query, cx: Context<'_>) -> bool {
    // Locate the relation, which is all that is needed to look up the policy.
    // Everything else about the shape is settled by the equality check below.
    let SetExpr::Select(select) = &*query.body else {
        return false;
    };
    let [TableWithJoins { relation, joins }] = select.from.as_slice() else {
        return false;
    };
    if !joins.is_empty() {
        return false;
    }
    let TableFactor::Table { name, alias, .. } = relation else {
        return false;
    };
    // Load-bearing, and more so than it looks: this exit sits *before* the
    // whole-query comparison below, so accepting an aliased relation here would
    // wave a subquery through without its predicate ever being checked. A guard
    // built by `guard` never has an inner alias — the alias moves to the wrapper
    // — so an alias surviving inside means this was not built by us.
    //
    // Deleting the check outright would, by contrast, be **redundant rather
    // than unnecessary**: the comparison below builds its expected value from
    // the same relation, so the alias would appear on both sides and cancel,
    // and only a subquery carrying the correct predicate would still be
    // accepted. That equivalence holds *given* the comparison. Weaken the
    // comparison and this check is live again.
    if alias.is_some() {
        return false;
    }
    let Ok(table) = TableName::from_object_name(name) else {
        return false;
    };
    // Only a `Restricted` decision identifies a guard. `Unresolvable` must not
    // be mistaken for one: the walk then descends and reports the relation
    // inside as unguarded, which is the fail-closed direction.
    let PolicyDecision::Restricted(policy) = cx.decide_name(&table) else {
        return false;
    };

    let expected = guard_query(relation.clone(), policy);
    let Ok(Statement::Query(normalised)) = crate::analyze::parse_single(&expected.to_string())
    else {
        return false;
    };
    *query == *normalised
}
