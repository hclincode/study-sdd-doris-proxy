//! Tasks 4.5 and 4.6 — the allowlist walk enumerates every table reference, and
//! anything it does not recognise refuses instead of returning a short list.
//!
//! A missed reference and a refusal are the two possible outcomes. The failure
//! this file exists to catch is the third one: an enumeration that quietly comes
//! back incomplete, because that is what turns into an unconstrained read.

use doris_row_filter_proxy::analyze::{
    analyze, walk_statement, AnalysisResult, RefPosition, SiteVisitor, StatementKind,
};
use doris_row_filter_proxy::error::RefusalReason;
use proptest::prelude::*;
use sqlparser::ast::helpers::attached_token::AttachedToken;
use sqlparser::ast::{
    Expr, GroupByExpr, ObjectName, Query, Select, SelectFlavor, SelectItem, SetExpr, Statement,
    TableFactor, TableWithJoins, WildcardAdditionalOptions,
};

/// Table names the walk found, qualified as written, in walk order.
fn enumerated(sql: &str) -> Vec<String> {
    analyze(sql)
        .unwrap_or_else(|reason| panic!("expected {sql:?} to analyse, but: {reason}"))
        .tables
        .iter()
        .map(|table| table.name.to_string())
        .collect()
}

fn refusal(sql: &str) -> RefusalReason {
    match analyze(sql) {
        Ok(analysis) => panic!(
            "expected {sql:?} to be refused, but it enumerated {:?}",
            analysis
                .tables
                .iter()
                .map(|t| t.name.to_string())
                .collect::<Vec<_>>()
        ),
        Err(reason) => reason,
    }
}

#[test]
fn walk_finds_every_reference_in_a_nested_statement() {
    assert_eq!(
        enumerated(
            "SELECT * FROM (SELECT * FROM (SELECT * FROM sales.orders) inner_t) outer_t \
             WHERE id IN (SELECT id FROM sales.returns)"
        ),
        ["sales.orders", "sales.returns"]
    );
}

#[test]
fn walk_finds_every_reference_in_a_joined_statement() {
    assert_eq!(
        enumerated(
            "SELECT * FROM sales.orders o \
             INNER JOIN sales.products p ON o.pid = p.id \
             LEFT JOIN sales.regions r ON r.code = o.region \
             CROSS JOIN sales.calendar c"
        ),
        [
            "sales.orders",
            "sales.products",
            "sales.regions",
            "sales.calendar"
        ]
    );
}

#[test]
fn walk_finds_every_reference_in_a_cte_statement() {
    assert_eq!(
        enumerated(
            "WITH a AS (SELECT * FROM sales.orders), \
                  b AS (SELECT * FROM sales.products) \
             SELECT * FROM a JOIN b ON a.pid = b.id"
        ),
        ["sales.orders", "sales.products", "a", "b"]
    );
}

#[test]
fn walk_finds_every_reference_in_a_union_statement() {
    assert_eq!(
        enumerated(
            "SELECT id FROM sales.orders \
             UNION ALL SELECT id FROM sales.archived_orders \
             UNION ALL SELECT id FROM sales.pending_orders"
        ),
        [
            "sales.orders",
            "sales.archived_orders",
            "sales.pending_orders"
        ]
    );
}

#[test]
fn walk_finds_references_inside_predicates_and_projections() {
    assert_eq!(
        enumerated(
            "SELECT (SELECT MAX(total) FROM sales.orders) AS m, \
                    CASE WHEN EXISTS (SELECT 1 FROM sales.holds) THEN 1 ELSE 0 END AS h \
             FROM sales.products \
             WHERE id IN (SELECT pid FROM sales.returns) \
             GROUP BY id \
             HAVING COUNT(*) > (SELECT COUNT(*) FROM sales.limits)"
        ),
        [
            "sales.orders",
            "sales.holds",
            "sales.products",
            "sales.returns",
            "sales.limits"
        ]
    );
}

#[test]
fn a_cte_reference_is_distinguished_from_a_table_reference() {
    let analysis = analyze("WITH c AS (SELECT * FROM sales.orders) SELECT * FROM c").unwrap();
    let by_name: Vec<_> = analysis
        .tables
        .iter()
        .map(|t| (t.name.to_string(), t.is_cte_reference))
        .collect();
    assert_eq!(
        by_name,
        [("sales.orders".to_string(), false), ("c".to_string(), true)]
    );
}

/// A later CTE name must not mask a real table read by an earlier CTE body.
/// MySQL makes only the *preceding* CTEs visible inside a non-recursive body, so
/// `orders` inside `x` is the stored table, not the CTE declared after it.
/// Getting this scope rule wrong in the permissive direction would drop the
/// reference from enumeration and leave the table unconstrained.
#[test]
fn a_later_cte_name_does_not_mask_a_table_read_by_an_earlier_body() {
    let analysis = analyze(
        "WITH x AS (SELECT * FROM orders), orders AS (SELECT 1 AS a) SELECT * FROM x JOIN orders",
    )
    .unwrap();
    let by_name: Vec<_> = analysis
        .tables
        .iter()
        .map(|t| (t.name.to_string(), t.is_cte_reference))
        .collect();
    assert_eq!(
        by_name,
        [
            // Inside `x`'s body: the stored table.
            ("orders".to_string(), false),
            ("x".to_string(), true),
            // In the final SELECT: the CTE.
            ("orders".to_string(), true),
        ]
    );
}

/// A *qualified* name always denotes a stored table, so a CTE cannot mask one by
/// sharing its bare name. `WITH orders AS (…) SELECT * FROM sales.orders` reads
/// the real `sales.orders`.
///
/// This is the fail-open direction if the qualification check in `shadowed` is
/// ever lost: the reference would be classified as a CTE, carry no policy, and
/// be forwarded with no predicate. Found by mutation testing — the surviving
/// mutant replaced `TableName::is_unqualified` with `true`, and nothing caught
/// it because every other CTE test uses a CTE name that matches no policy table.
#[test]
fn a_qualified_reference_is_not_shadowed_by_a_same_named_cte() {
    let analysis =
        analyze("WITH orders AS (SELECT 1 AS region) SELECT * FROM sales.orders JOIN orders")
            .unwrap();
    assert_eq!(
        analysis
            .tables
            .iter()
            .map(|t| (t.name.to_string(), t.is_cte_reference))
            .collect::<Vec<_>>(),
        [
            // Qualified: the stored table, whatever CTEs are in scope.
            ("sales.orders".to_string(), false),
            // Unqualified and matching the CTE: the CTE.
            ("orders".to_string(), true),
        ]
    );
}

#[test]
fn a_recursive_cte_may_refer_to_itself() {
    let analysis = analyze(
        "WITH RECURSIVE c AS (SELECT * FROM sales.orders UNION ALL SELECT * FROM c) \
         SELECT * FROM c",
    )
    .unwrap();
    let cte_flags: Vec<_> = analysis
        .tables
        .iter()
        .map(|t| (t.name.to_string(), t.is_cte_reference))
        .collect();
    assert_eq!(
        cte_flags,
        [
            ("sales.orders".to_string(), false),
            ("c".to_string(), true),
            ("c".to_string(), true),
        ]
    );
}

#[test]
fn write_targets_are_enumerated_alongside_the_tables_a_write_reads() {
    let analysis = analyze("INSERT INTO scratch.copy SELECT * FROM sales.orders").unwrap();
    assert_eq!(analysis.kind, StatementKind::Insert);
    assert_eq!(
        analysis
            .tables
            .iter()
            .map(|t| (t.name.to_string(), t.position))
            .collect::<Vec<_>>(),
        [
            ("scratch.copy".to_string(), RefPosition::WriteTarget),
            ("sales.orders".to_string(), RefPosition::Relation),
        ]
    );
}

#[test]
fn an_alias_is_recorded_as_the_effective_name() {
    let analysis = analyze("SELECT * FROM sales.orders AS o").unwrap();
    assert_eq!(analysis.tables[0].effective_name(), "o");
    let analysis = analyze("SELECT * FROM sales.orders").unwrap();
    assert_eq!(analysis.tables[0].effective_name(), "orders");
}

/// Task 4.5, second half. A node kind that is not on the allowlist must produce
/// "cannot analyse" — not an empty reference list, which is what a
/// default-continue branch would produce and what would let the statement
/// through as if it touched nothing.
///
/// The node is constructed directly rather than parsed, because the point is the
/// walk's behaviour on a shape it does not know, independent of whether today's
/// `sqlparser` can produce it from MySQL text.
#[test]
fn an_unlisted_node_kind_cannot_analyse_rather_than_enumerating_nothing() {
    #[derive(Default)]
    struct Counting {
        seen: usize,
    }
    impl SiteVisitor for Counting {
        fn visit_relation(&mut self, _: &mut TableFactor, _: bool) -> AnalysisResult<()> {
            self.seen += 1;
            Ok(())
        }
        fn visit_write_target(&mut self, _: &ObjectName) -> AnalysisResult<()> {
            self.seen += 1;
            Ok(())
        }
        fn visit_named(&mut self, _: &ObjectName) -> AnalysisResult<()> {
            self.seen += 1;
            Ok(())
        }
    }

    let mut statement = statement_selecting_from(TableFactor::UNNEST {
        alias: None,
        array_exprs: Vec::new(),
        with_offset: false,
        with_offset_alias: None,
        with_ordinality: false,
    });
    let mut visitor = Counting::default();
    let outcome = walk_statement(&mut statement, &mut visitor);

    assert!(
        matches!(outcome, Err(RefusalReason::UnsupportedShape { .. })),
        "expected an unsupported-shape refusal, got {outcome:?}"
    );
    assert_eq!(
        visitor.seen, 0,
        "the walk must not report a complete enumeration for a shape it refused"
    );
}

// ---------------------------------------------------------------------------
// Task 6.10 — one request carries one statement
// ---------------------------------------------------------------------------

/// A request that parses into more than one statement is refused, because the
/// proxy must reason about each statement in its own right.
///
/// This is the *only* place multi-statement smuggling is closed. Design D4 said
/// capability negotiation closed it at the protocol level; that turned out to be
/// wrong — `opensrv-mysql` hands the whole `COM_QUERY` payload to the query
/// handler whatever the client negotiated, and the advertised flag is advisory
/// anyway. So this check is load-bearing, not a backstop.
#[test]
fn a_request_carrying_more_than_one_statement_is_refused() {
    for sql in [
        "SELECT * FROM sales.orders; SELECT 1",
        "SELECT 1; SELECT * FROM sales.orders",
        "SELECT 1;;SELECT 2",
    ] {
        assert!(
            matches!(refusal(sql), RefusalReason::MultiStatement),
            "{sql:?} should be refused as a multi-statement request"
        );
    }
}

/// A comment cannot hide the second statement: the parser tokenises through it,
/// so the statement is counted rather than skipped.
#[test]
fn a_comment_cannot_conceal_a_second_statement() {
    for sql in [
        "SELECT * FROM sales.orders /* harmless */; SELECT 1",
        "SELECT * FROM sales.orders -- trailing\n; SELECT 1",
        "SELECT * FROM sales.orders; /* second */ SELECT 1",
        "SELECT * FROM sales.orders;/*x*/SELECT 1",
    ] {
        assert!(
            matches!(refusal(sql), RefusalReason::MultiStatement),
            "{sql:?} should be refused as a multi-statement request"
        );
    }
}

/// The other half of task 6.10, and the one that is easy to get wrong: a
/// trailing semicolon is punctuation, not a second statement. Refusing
/// `SELECT 1;` would be a false rejection of the single most ordinary thing a
/// client sends.
///
/// `sqlparser`'s behaviour here was verified rather than assumed — including
/// that a *comment* after the semicolon, and even a second bare semicolon, still
/// count as one statement.
#[test]
fn a_trailing_semicolon_is_not_a_second_statement() {
    for sql in [
        "SELECT * FROM sales.orders;",
        "SELECT * FROM sales.orders;   ",
        "SELECT * FROM sales.orders;\n",
        "SELECT * FROM sales.orders; ;",
        "SELECT * FROM sales.orders /* x */;",
        "SELECT * FROM sales.orders; -- trailing comment",
        "SELECT * FROM sales.orders; /* only a comment */",
    ] {
        let analysis = analyze(sql)
            .unwrap_or_else(|reason| panic!("{sql:?} should analyse as one statement: {reason}"));
        assert_eq!(
            analysis
                .tables
                .iter()
                .map(|t| t.name.to_string())
                .collect::<Vec<_>>(),
            ["sales.orders"]
        );
    }
}

// ---------------------------------------------------------------------------
// Executable comments
// ---------------------------------------------------------------------------

/// Spec: "A statement whose execution depends on the backend's identity is
/// refused".
///
/// `sqlparser` parses the contents of `/*!NNNNN ... */` as ordinary SQL and
/// throws the version gate away. The fragment is therefore enumerated and
/// constrained — but re-rendering the AST emits it *ungated*, so the statement
/// reaching Doris runs unconditionally what the original might never have run.
/// Measured before this refusal existed:
///
/// ```text
/// in   SELECT /*!99999 id, */ region FROM sales.orders
/// out  SELECT id, region FROM (SELECT * FROM sales.orders WHERE `region` IN (…)) AS orders
/// ```
///
/// One column became two, which is the "Column shape is unchanged by rewriting"
/// scenario violated, and a gated `UNION` branch became unconditional, which is
/// invariant 4 violated.
#[test]
fn a_conditionally_executed_comment_is_refused() {
    for sql in [
        "SELECT /*!99999 id, */ region FROM sales.orders",
        "SELECT /*!50000 1, */ * FROM sales.orders",
        "SELECT * FROM sales.products /*! UNION SELECT * FROM sales.orders */",
        "SELECT * FROM sales.products /*!99999 UNION SELECT * FROM sales.orders */",
        "/*!50000 SELECT * FROM sales.orders */",
        "SELECT * FROM /*! sales.orders */",
    ] {
        let reason = refusal(sql);
        let RefusalReason::UnsupportedShape { construct } = &reason else {
            panic!("{sql:?} should refuse as an unsupported shape, got {reason:?}");
        };
        assert_eq!(construct, "conditionally-executed comment", "for {sql:?}");
    }
}

/// The refusal is about the *gate*, not about comments. An ordinary comment is
/// discarded identically by `sqlparser` and by Doris, so it stays supported —
/// otherwise this check would look like superstition and get deleted.
#[test]
fn an_ordinary_comment_is_not_refused() {
    for sql in [
        "SELECT * FROM sales.orders /* plain */",
        "SELECT * FROM sales.orders -- plain",
        "SELECT * FROM sales.orders # plain",
        "/* leading */ SELECT * FROM sales.orders",
        "SELECT /* inline */ * FROM sales.orders",
        // A marker that only appears *inside* an ordinary comment is inert:
        // MySQL does not nest block comments, so this one ends at the first `*/`.
        "SELECT * FROM sales.orders /* not really /*! */",
    ] {
        assert!(
            analyze(sql).is_ok(),
            "{sql:?} should still analyse: {:?}",
            analyze(sql).err()
        );
    }
}

/// A marker inside a string literal or a quoted identifier is data, not a gate,
/// and must not trigger a refusal — otherwise any query mentioning the sequence
/// becomes unrunnable.
#[test]
fn a_marker_inside_a_literal_is_not_a_conditionally_executed_comment() {
    for sql in [
        "SELECT * FROM sales.orders WHERE note = '/*!99999 x */'",
        "SELECT '/*!' FROM sales.orders",
        "SELECT * FROM sales.orders WHERE note = 'it''s /*! fine'",
        "SELECT `/*!weird`.x FROM sales.orders",
        "SELECT * FROM sales.orders WHERE note = \"/*! also fine\"",
    ] {
        assert!(
            analyze(sql).is_ok(),
            "{sql:?} should still analyse: {:?}",
            analyze(sql).err()
        );
    }
}

/// `--` begins a comment only when whitespace follows, which is MySQL's rule.
/// Treating it as a comment unconditionally would skip the rest of the line and
/// step straight over the marker in the first case here.
#[test]
fn a_marker_after_a_double_minus_operator_is_still_detected() {
    for sql in [
        "SELECT 1--2 /*!99999 UNION SELECT * FROM sales.orders */",
        "SELECT 1-- comment\n/*!99999 UNION SELECT * FROM sales.orders */",
    ] {
        assert!(
            matches!(refusal(sql), RefusalReason::UnsupportedShape { .. }),
            "{sql:?} should be refused"
        );
    }
}

/// A request with no statement in it at all — empty, whitespace, or nothing but
/// a comment — is refused rather than forwarded. MySQL itself errors on an empty
/// query, so this costs no compatibility.
#[test]
fn a_request_with_no_statement_is_refused() {
    for sql in ["", "   ", ";", "-- just a comment", "/* just a comment */"] {
        assert!(
            matches!(refusal(sql), RefusalReason::Unparseable),
            "{sql:?} should be refused as unparseable"
        );
    }
}

#[test]
fn a_cannot_analyse_refusal_names_the_offending_construct() {
    let RefusalReason::UnsupportedShape { construct } =
        refusal("SELECT * FROM sales.orders o WHERE MATCH (o.note) AGAINST ('x')")
    else {
        panic!("expected an unsupported-shape refusal");
    };
    assert_eq!(construct, "MatchAgainst expression");

    let RefusalReason::UnsupportedShape { construct } =
        refusal("SELECT * FROM sales.orders LATERAL VIEW explode(a) t AS c")
    else {
        panic!("expected an unsupported-shape refusal");
    };
    assert_eq!(construct, "LATERAL VIEW clause");
}

/// MySQL's own `SELECT` modifiers are not on the allowlist and are refused.
///
/// Reachable, unlike most of the exotic clauses the walk guards against:
/// `MySqlDialect` populates `Select::select_modifiers` for `SQL_CALC_FOUND_ROWS`,
/// `HIGH_PRIORITY`, `STRAIGHT_JOIN` and `SQL_NO_CACHE`. Found by mutation
/// testing — inverting the comparison in that guard stopped it refusing and no
/// test noticed.
///
/// `SQL_CALC_FOUND_ROWS` in particular has an effect the rewrite cannot preserve:
/// it makes `FOUND_ROWS()` report the row count *before* `LIMIT`, computed over
/// the guarded relation rather than the table.
#[test]
fn a_mysql_select_modifier_is_refused() {
    for sql in [
        "SELECT SQL_CALC_FOUND_ROWS * FROM sales.orders",
        "SELECT HIGH_PRIORITY * FROM sales.orders",
        "SELECT STRAIGHT_JOIN * FROM sales.orders",
        "SELECT SQL_NO_CACHE * FROM sales.orders",
    ] {
        let RefusalReason::UnsupportedShape { construct } = refusal(sql) else {
            panic!("{sql:?} should refuse as an unsupported shape");
        };
        assert_eq!(construct, "MySQL SELECT modifier", "for {sql:?}");
    }
}

/// `GROUP BY ALL` is refused, because `sqlparser` and the backend disagree about
/// what the text is rather than about what it does.
///
/// `MySqlDialect` parses it into a dedicated `GroupByExpr::All` node. MySQL
/// treats `ALL` as a reserved word and rejects the statement, and Doris follows
/// MySQL. Refusing costs nothing that works today — Doris would reject it
/// anyway — and it keeps the allowlist from accepting a shape whose parse is
/// known not to match the backend's, which is the parser differential D3 exists
/// to keep narrow.
///
/// Found by mutation testing, which showed nothing exercised `GROUP BY ALL` in
/// either direction.
#[test]
fn group_by_all_is_refused() {
    let RefusalReason::UnsupportedShape { construct } =
        refusal("SELECT region FROM sales.orders GROUP BY ALL")
    else {
        panic!("GROUP BY ALL should refuse as an unsupported shape");
    };
    assert_eq!(construct, "GROUP BY ALL");
}

/// `WITH ROLLUP` is ordinary MySQL and `sqlparser`'s `MySqlDialect` cannot parse
/// it, so a policy-bearing user is refused a common reporting query.
///
/// Recorded here rather than only in a note because it is a *compatibility* gap
/// of exactly the kind design D6 accepts, and the test will start failing if a
/// later `sqlparser` learns the syntax — at which point the allowlist needs a
/// decision about `GroupByWithModifier` rather than silently accepting it.
#[test]
fn group_by_with_rollup_is_not_parseable_by_this_dialect() {
    assert!(matches!(
        refusal("SELECT region FROM sales.orders GROUP BY region WITH ROLLUP"),
        RefusalReason::Unparseable
    ));
}

/// `SHOW TABLES` and `SET autocommit = 1` used to be in this list, and their
/// removal is the point of the session-management requirement rather than a
/// relaxation of the allowlist: they are now *analysed* — enumerated like
/// anything else and forwarded because they read no table — not waved through
/// for being of their kind. See `a_session_statement_that_reads_no_table_…` and
/// `a_metadata_statement_listing_objects_…` below.
///
/// Refusing them was D6 behaving exactly as specified, and it made the proxy
/// unable to serve any ordinary MySQL connector, every one of which sends
/// `SET NAMES` before its first query.
#[test]
fn a_statement_kind_outside_the_allowlist_cannot_be_analysed() {
    for sql in [
        "CREATE TABLE t (a INT)",
        "DROP TABLE sales.orders",
        "USE sales",
        "EXPLAIN SELECT * FROM sales.orders",
        "ANALYZE TABLE sales.orders",
    ] {
        assert!(
            matches!(refusal(sql), RefusalReason::UnsupportedShape { .. }),
            "{sql:?} should refuse as an unsupported shape"
        );
    }
}

fn statement_selecting_from(relation: TableFactor) -> Statement {
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
        selection: None,
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
    let _: Option<Expr> = None;
    Statement::Query(Box::new(Query {
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
    }))
}

// ---------------------------------------------------------------------------
// Task 4.6 — property test
// ---------------------------------------------------------------------------

/// A generated statement together with the table references it was built from,
/// in the order they appear in the text.
#[derive(Clone, Debug)]
struct Generated {
    sql: String,
    tables: Vec<String>,
}

const TABLES: [&str; 4] = [
    "sales.orders",
    "sales.products",
    "scratch.tmp",
    "sales.returns",
];

fn leaf_relation() -> impl Strategy<Value = Generated> {
    (0usize..TABLES.len(), 0u8..3).prop_map(|(index, alias)| {
        let name = TABLES[index];
        let sql = match alias {
            0 => name.to_string(),
            1 => format!("{name} AS a{index}"),
            _ => format!("{name} a{index}"),
        };
        Generated {
            sql,
            tables: vec![name.to_string()],
        }
    })
}

/// Statements built from the shapes design D2 claims to cover: nesting, joins,
/// set operations, CTE bodies and subqueries in predicates.
fn generated_statement() -> impl Strategy<Value = Generated> {
    leaf_relation()
        .prop_map(|relation| Generated {
            sql: format!("SELECT * FROM {}", relation.sql),
            tables: relation.tables,
        })
        .prop_recursive(4, 32, 2, |inner| {
            prop_oneof![
                // Derived table.
                (inner.clone(), 0usize..8).prop_map(|(q, n)| Generated {
                    sql: format!("SELECT * FROM ({}) d{n}", q.sql),
                    tables: q.tables,
                }),
                // Join against a leaf relation, inner and outer.
                (inner.clone(), leaf_relation(), 0u8..3).prop_map(|(q, r, kind)| {
                    let keyword = match kind {
                        0 => "JOIN",
                        1 => "LEFT JOIN",
                        _ => "RIGHT JOIN",
                    };
                    let mut tables = q.tables;
                    tables.extend(r.tables);
                    Generated {
                        sql: format!("SELECT * FROM ({}) j {keyword} {} ON 1 = 1", q.sql, r.sql),
                        tables,
                    }
                }),
                // Set operation.
                (inner.clone(), inner.clone()).prop_map(|(left, right)| {
                    let mut tables = left.tables;
                    tables.extend(right.tables);
                    Generated {
                        sql: format!("{} UNION ALL {}", left.sql, right.sql),
                        tables,
                    }
                }),
                // CTE.
                (inner.clone(), 0usize..8).prop_map(|(q, n)| Generated {
                    sql: format!("WITH c{n} AS ({}) SELECT * FROM c{n}", q.sql),
                    tables: q.tables,
                }),
                // Correlated existence test.
                (inner.clone(), leaf_relation()).prop_map(|(q, r)| {
                    let mut tables = r.tables;
                    tables.extend(q.tables);
                    Generated {
                        sql: format!("SELECT * FROM {} WHERE EXISTS ({})", r.sql, q.sql),
                        tables,
                    }
                }),
                // Subquery in a predicate.
                (inner, leaf_relation()).prop_map(|(q, r)| {
                    let mut tables = r.tables;
                    tables.extend(q.tables);
                    Generated {
                        sql: format!("SELECT * FROM {} WHERE id IN ({})", r.sql, q.sql),
                        tables,
                    }
                }),
            ]
        })
}

proptest! {
    /// Task 4.6. Every table name in the generated statement is either
    /// enumerated by the walk or the statement is refused. There is no third
    /// outcome, and in particular no statement that analyses successfully while
    /// leaving a reference out of the list.
    #[test]
    fn every_generated_table_reference_is_enumerated_or_the_statement_is_refused(
        generated in generated_statement()
    ) {
        match analyze(&generated.sql) {
            Err(_) => {}
            Ok(analysis) => {
                let found: Vec<String> = analysis
                    .tables
                    .iter()
                    .filter(|table| !table.is_cte_reference)
                    .map(|table| table.name.to_string())
                    .collect();
                prop_assert_eq!(
                    found,
                    generated.tables,
                    "walk missed a reference in {}",
                    generated.sql
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Session-management and metadata statements
// ---------------------------------------------------------------------------

/// Table names enumerated for `sql`, or the panic message if it was refused.
fn tables_of(sql: &str) -> Vec<String> {
    analyze(sql)
        .unwrap_or_else(|reason| panic!("{sql:?} should analyse: {reason}"))
        .tables
        .iter()
        .map(|t| t.name.to_string())
        .collect()
}

/// Spec: "A session statement that reads no table is forwarded".
///
/// These are the statements a MySQL connector sends before its first query.
/// Refusing them — which D6's rule did, correctly by its own terms — meant the
/// proxy could not serve a normal client at all: JDBC, ODBC, Python and Go all
/// issue `SET NAMES` on connect. They enumerate nothing, so there is nothing to
/// constrain and nothing to refuse.
#[test]
fn a_session_statement_that_reads_no_table_enumerates_nothing() {
    for sql in [
        "SET NAMES utf8mb4",
        "SET NAMES utf8mb4 COLLATE utf8mb4_general_ci",
        "SET autocommit=1",
        "SET SESSION sql_mode=''",
        "SET @@session.time_zone='+00:00'",
        "SET character_set_results=NULL",
        "SET SESSION TRANSACTION ISOLATION LEVEL READ COMMITTED",
        "SET @x = 1",
    ] {
        let analysis = analyze(sql).unwrap_or_else(|e| panic!("{sql:?} should analyse: {e}"));
        assert_eq!(analysis.kind, StatementKind::Session, "for {sql:?}");
        assert!(analysis.tables.is_empty(), "{sql:?} enumerated a table");
    }
}

/// Spec: "A session statement that reads a policy table is refused" — the
/// enumeration half, which is what makes the refusal possible.
///
/// **`SET` is not inert.** `SET @x = (SELECT total FROM sales.orders)` reads the
/// table, and `SELECT @x` afterwards returns what it read, with no
/// policy-bearing reference anywhere in that second statement. Forwarding every
/// `SET` because "it cannot return rows" would be a live bypass, which is why
/// these go through the same walk as everything else rather than being
/// special-cased as session chatter.
#[test]
fn a_session_statement_reading_a_table_enumerates_it() {
    assert_eq!(
        tables_of("SET @x = (SELECT total FROM sales.orders)"),
        ["sales.orders"]
    );
    // Not only the first assignment.
    assert_eq!(
        tables_of("SET @x = 1, @y = (SELECT total FROM sales.orders)"),
        ["sales.orders"]
    );
    // Nested as deeply as anywhere else.
    assert_eq!(
        tables_of("SET @x = (SELECT SUM(t.total) FROM (SELECT * FROM sales.orders) t)"),
        ["sales.orders"]
    );
    // And through a predicate rather than the projection.
    assert_eq!(
        tables_of(
            "SET @x = (SELECT 1 FROM sales.products WHERE id IN (SELECT pid FROM sales.orders))"
        ),
        ["sales.products", "sales.orders"]
    );
}

/// Spec: "A metadata statement is forwarded". Listing tables or databases
/// enumerates nothing — metadata disclosure is a documented limitation of this
/// control, not something it claims to prevent.
///
/// `SHOW TABLES FROM sales` names a *database*, and must not be enumerated as a
/// table: a policy on `sales.orders` must not be matched by it. `sqlparser`'s
/// `ShowStatementIn::parent_type` looks like the field that would say so, but it
/// is a Snowflake field that `MySqlDialect` always leaves `None` — so the
/// distinction comes from the statement kind instead.
#[test]
fn a_metadata_statement_listing_objects_enumerates_nothing() {
    for sql in [
        "SHOW TABLES",
        "SHOW TABLES FROM sales",
        "SHOW FULL TABLES FROM sales",
        "SHOW DATABASES",
        "SHOW VARIABLES LIKE 'x'",
        "SHOW STATUS",
        "SHOW COLLATION",
        "SHOW CHARACTER SET",
        "SHOW ENGINES",
        "SHOW WARNINGS",
        "SHOW GRANTS",
    ] {
        let analysis = analyze(sql).unwrap_or_else(|e| panic!("{sql:?} should analyse: {e}"));
        assert_eq!(analysis.kind, StatementKind::Metadata, "for {sql:?}");
        assert!(analysis.tables.is_empty(), "{sql:?} enumerated a table");
    }
}

/// Spec: "A metadata statement naming a policy table is forwarded" — and the
/// provenance that makes it safe to forward.
///
/// The name **is** enumerated, tagged [`RefPosition::Named`]. That matters more
/// than the forwarding does. An earlier version expressed "names are forwarded"
/// by not enumerating the name at all, which put a disposition decision inside
/// the analyser and broke invariant 6 — and, because the two cases then looked
/// identical downstream, left `SHOW VARIABLES WHERE (SELECT COUNT(*) FROM
/// sales.orders) = 5` forwarded as though it merely named the table. That is a
/// bit-at-a-time oracle over restricted rows.
///
/// Enumerate everything; tag why it is there; let the disposition layer decide.
#[test]
fn a_metadata_statement_naming_a_policy_table_enumerates_it_as_named() {
    for sql in [
        "SHOW COLUMNS FROM sales.orders",
        "SHOW FULL COLUMNS FROM sales.orders",
        "SHOW CREATE TABLE sales.orders",
        "SHOW CREATE VIEW sales.orders",
    ] {
        let analysis = analyze(sql).unwrap_or_else(|e| panic!("{sql:?} should analyse: {e}"));
        assert_eq!(analysis.kind, StatementKind::Metadata, "for {sql:?}");
        assert_eq!(
            analysis
                .tables
                .iter()
                .map(|t| (t.name.to_string(), t.position))
                .collect::<Vec<_>>(),
            [("sales.orders".to_string(), RefPosition::Named)],
            "for {sql:?}"
        );
    }

    // A database after `FROM` is not a table, and must not be enumerated as one
    // — a policy on `sales.orders` must not be matched by `SHOW TABLES FROM
    // sales`.
    for sql in ["SHOW TABLES FROM sales", "SHOW DATABASES"] {
        let analysis = analyze(sql).unwrap();
        assert!(analysis.tables.is_empty(), "{sql:?} enumerated a table");
    }
}

/// The oracle, pinned at the layer where it was lost.
///
/// `SHOW VARIABLES WHERE (SELECT COUNT(*) FROM sales.orders) = 5` returns rows
/// only when the count is 5, so a restricted user can test any predicate about
/// rows they cannot see, one bit at a time. The reference must be enumerated as
/// a **`Relation`** — a read — not as a name, or the disposition layer has no
/// way to tell it from `SHOW COLUMNS`.
#[test]
fn a_metadata_statement_reading_a_policy_table_enumerates_it_as_a_relation() {
    for sql in [
        "SHOW VARIABLES WHERE (SELECT COUNT(*) FROM sales.orders) = 5",
        "SHOW VARIABLES WHERE (SELECT COUNT(*) FROM sales.orders WHERE total > 400) = 1",
        "SHOW STATUS WHERE (SELECT MAX(total) FROM sales.orders) > 400",
        "SHOW VARIABLES WHERE Variable_name IN (SELECT region FROM sales.orders)",
    ] {
        let analysis = analyze(sql).unwrap_or_else(|e| panic!("{sql:?} should analyse: {e}"));
        assert_eq!(
            analysis
                .tables
                .iter()
                .map(|t| (t.name.to_string(), t.position))
                .collect::<Vec<_>>(),
            [("sales.orders".to_string(), RefPosition::Relation)],
            "for {sql:?} — a read must not be enumerated as a name"
        );
    }
}

/// `sqlparser` parks the `SHOW` forms it does not model in `ShowVariable` as raw
/// tokens — `SHOW INDEX FROM sales.orders` becomes
/// `["INDEX", "FROM", "sales", "orders"]`. All of them forward.
///
/// The reason is structural, not a judgement about how sensitive index metadata
/// is: that variant carries `Vec<Ident>` and nothing else, so there is no
/// expression in it, so there is no subquery in it, so it cannot read a row.
/// Under "reads versus names" a name forwards, and the proxy never needs to work
/// out which token is the table.
#[test]
fn an_unmodelled_show_form_is_forwarded() {
    for sql in [
        "SHOW ENGINES",
        "SHOW WARNINGS",
        "SHOW GRANTS",
        "SHOW INDEX FROM sales.orders",
        "SHOW KEYS FROM sales.orders",
        "SHOW TRIGGERS FROM sales",
    ] {
        let analysis = analyze(sql).unwrap_or_else(|e| panic!("{sql:?} should analyse: {e}"));
        assert!(analysis.tables.is_empty(), "{sql:?} enumerated a table");
    }
}

/// `SET ROLE` changes which privileges the session holds. Policy is keyed on the
/// authenticated username, which does not change — but the rows the backend
/// returns may, and the proxy has no model of that.
#[test]
fn set_role_is_refused() {
    assert!(matches!(
        refusal("SET ROLE admin"),
        RefusalReason::UnsupportedShape { .. }
    ));
}

/// Spec: "A transaction-control statement naming no table is forwarded", and
/// "A client can complete a transaction it was allowed to begin".
///
/// Every one of these `sqlparser` variants carries only enums and identifiers —
/// isolation levels, access modes, a savepoint name — and no `Expr`, so none of
/// them can reference a table at all.
///
/// The second scenario is why they are on the allowlist rather than left to
/// refuse: `SET autocommit=0` is forwarded, so refusing `COMMIT` would strand
/// the client holding a transaction it cannot close. Forwarding one half of a
/// pair while refusing the other is worse than refusing both. They share
/// `StatementKind::Session` for the same reason — the pairing is structural
/// rather than something two arms have to agree about.
#[test]
fn a_transaction_control_statement_is_forwarded() {
    for sql in [
        "BEGIN",
        "BEGIN WORK",
        "START TRANSACTION",
        "START TRANSACTION READ ONLY",
        "COMMIT",
        "COMMIT WORK",
        "COMMIT AND CHAIN",
        "ROLLBACK",
        "ROLLBACK WORK",
        "SAVEPOINT sp1",
        "ROLLBACK TO SAVEPOINT sp1",
        "RELEASE SAVEPOINT sp1",
        "SET TRANSACTION ISOLATION LEVEL READ COMMITTED",
    ] {
        let analysis = analyze(sql).unwrap_or_else(|e| panic!("{sql:?} should analyse: {e}"));
        assert_eq!(analysis.kind, StatementKind::Session, "for {sql:?}");
        assert!(analysis.tables.is_empty(), "{sql:?} enumerated a table");
    }
}

/// The pairing the spec demands, stated as one test so it cannot drift apart:
/// every statement of an autocommit-off transaction sequence is analysable, so a
/// client that is allowed to begin one is always allowed to close it.
#[test]
fn a_client_can_close_every_transaction_it_can_open() {
    for sql in [
        "SET autocommit=0",
        "START TRANSACTION",
        "SELECT * FROM sales.products",
        "COMMIT",
        "SET autocommit=1",
    ] {
        assert!(
            analyze(sql).is_ok(),
            "{sql:?} must be analysable, or a client can enter a state it cannot leave"
        );
    }
}

/// Transaction forms `MySqlDialect` cannot parse, recorded so the compatibility
/// cost is visible and so a later `sqlparser` forces a decision rather than
/// silently accepting them.
///
/// `START TRANSACTION WITH CONSISTENT SNAPSHOT` is the one that matters:
/// `mysqldump --single-transaction` opens with exactly that, so a policy-bearing
/// user cannot take a consistent dump through the proxy. `COMMIT RELEASE` and
/// `ROLLBACK RELEASE` are ordinary MySQL too.
///
/// These refuse as `Unparseable`, not as an unsupported shape — the parser never
/// produces a statement, so classification never sees them.
#[test]
fn transaction_forms_this_dialect_cannot_parse_are_recorded() {
    for sql in [
        "START TRANSACTION WITH CONSISTENT SNAPSHOT",
        "COMMIT RELEASE",
        "ROLLBACK RELEASE",
    ] {
        assert!(
            matches!(refusal(sql), RefusalReason::Unparseable),
            "{sql:?} should refuse as unparseable"
        );
    }
}
