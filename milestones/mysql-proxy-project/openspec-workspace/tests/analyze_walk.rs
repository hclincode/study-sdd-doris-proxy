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

#[test]
fn a_statement_kind_outside_the_allowlist_cannot_be_analysed() {
    for sql in [
        "SHOW TABLES",
        "CREATE TABLE t (a INT)",
        "SET autocommit = 1",
        "USE sales",
        "EXPLAIN SELECT * FROM sales.orders",
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
