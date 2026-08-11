use doris_row_filter_proxy::analyze::analyze;
use doris_row_filter_proxy::policy::PolicySet;
use doris_row_filter_proxy::rewrite::rewrite_statement;
const CFG: &str = "[[policy]]\nuser=\"analyst\"\ndatabase=\"sales\"\ntable=\"orders\"\ncolumn=\"region\"\npermitted_values=[\"APAC\",\"EMEA\"]\n";
#[test]
fn probe() {
    let set = PolicySet::from_toml_str(CFG, "p").unwrap();
    for sql in [
        "SHOW VARIABLES WHERE (SELECT COUNT(*) FROM sales.orders) = 5",
        "SHOW VARIABLES WHERE (SELECT COUNT(*) FROM sales.orders WHERE total > 400) = 1",
        "SHOW STATUS WHERE (SELECT MAX(total) FROM sales.orders) > 400",
        "SHOW COLUMNS FROM sales.orders",
        "SHOW CREATE TABLE sales.orders",
        "SHOW TABLES FROM sales",
    ] {
        let a = analyze(sql);
        let enum_str = match &a {
            Ok(x) => format!("kind={:?} refs={:?}", x.kind,
                x.tables.iter().map(|t| (t.name.to_string(), t.position)).collect::<Vec<_>>()),
            Err(e) => format!("analyze err {e}"),
        };
        let out = match rewrite_statement(sql, "analyst", Some("sales"), &set) {
            Ok(o) if o == sql => "FORWARDED VERBATIM".to_string(),
            Ok(o) => format!("REWRITTEN {o}"),
            Err(e) => format!("REFUSED ({e})"),
        };
        println!("{}\n   {}\n   -> {}", sql, enum_str, out);
    }
}
