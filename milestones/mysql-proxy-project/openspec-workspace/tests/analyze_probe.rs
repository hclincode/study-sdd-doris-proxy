use sqlparser::dialect::MySqlDialect;
use sqlparser::parser::Parser;
#[test]
fn probe() {
    for sql in ["BEGIN", "START TRANSACTION", "COMMIT", "ROLLBACK",
                "BEGIN WORK", "COMMIT WORK", "ROLLBACK WORK",
                "START TRANSACTION READ ONLY", "SAVEPOINT sp1",
                "ROLLBACK TO SAVEPOINT sp1", "RELEASE SAVEPOINT sp1",
                "SET TRANSACTION ISOLATION LEVEL READ COMMITTED",
                "COMMIT AND CHAIN", "ROLLBACK AND NO CHAIN"] {
        match Parser::parse_sql(&MySqlDialect {}, sql) {
            Ok(st) => {
                let d = format!("{:?}", st[0]);
                println!("OK  {:<44} {}", sql, d.chars().take(76).collect::<String>());
            }
            Err(e) => println!("ERR {:<44} {e}", sql),
        }
    }
}
