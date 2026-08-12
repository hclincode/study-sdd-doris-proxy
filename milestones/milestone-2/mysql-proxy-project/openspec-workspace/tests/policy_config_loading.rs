//! Loading policy configuration from a file.
//!
//! Covers `specs/policy-config`, requirements "Invalid configuration prevents
//! startup" and "Configuration is fixed for the lifetime of the process".

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use doris_row_filter_proxy::error::ProxyError;
use doris_row_filter_proxy::policy::{PermittedValue, PolicyDecision, PolicySet, TableRef};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// A scratch policy file that removes itself, so a failing assertion cannot
/// leave a stale file behind for the next run to load.
struct ScratchFile(PathBuf);

impl ScratchFile {
    fn with_contents(name: &str, contents: &str) -> Self {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "doris-row-filter-proxy-{name}-{}-{}.toml",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&path, contents).expect("write scratch policy file");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn overwrite(&self, contents: &str) {
        std::fs::write(&self.0, contents).expect("overwrite scratch policy file");
    }
}

impl Drop for ScratchFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

const VALID: &str = r#"
[[policy]]
user = "analyst"
database = "sales"
table = "orders"
column = "region"
permitted_values = ["APAC", "EMEA"]
"#;

fn config_error(name: &str, contents: &str) -> String {
    let file = ScratchFile::with_contents(name, contents);
    match PolicySet::load_from_path(file.path()) {
        Err(ProxyError::Config(message)) => message,
        Err(other) => panic!("expected a configuration error, got {other:?}"),
        Ok(_) => panic!("expected a configuration error, configuration loaded"),
    }
}

#[test]
fn valid_configuration_file_loads() {
    let file = ScratchFile::with_contents("valid", VALID);
    let policies = PolicySet::load_from_path(file.path()).expect("valid configuration loads");

    assert_eq!(policies.policy_count(), 1);
    assert!(policies.has_any_policy("analyst"));

    let decision = policies.lookup("analyst", &TableRef::qualified("sales", "orders"), None);
    let policy = decision.policy().expect("policy applies");
    assert_eq!(policy.column(), "region");
    assert_eq!(policy.permitted_values().len(), 2);
}

#[test]
fn the_example_policy_file_loads() {
    // The documented example is executable documentation, not decoration.
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/policy.toml");
    let policies = PolicySet::load_from_path(&path).expect("the shipped example loads");

    assert_eq!(policies.policy_count(), 3);
    assert!(policies.has_any_policy("analyst"));
    assert!(policies.has_any_policy("auditor"));
    assert!(!policies.has_any_policy("reporting"));
}

#[test]
fn a_missing_configuration_file_is_a_configuration_error() {
    let mut path = std::env::temp_dir();
    path.push("doris-row-filter-proxy-does-not-exist.toml");
    let _ = std::fs::remove_file(&path);

    match PolicySet::load_from_path(&path) {
        Err(ProxyError::Config(message)) => {
            assert!(
                message.contains("could not read policy configuration"),
                "{message}"
            );
        }
        Err(other) => panic!("expected a configuration error, got {other:?}"),
        Ok(_) => panic!("expected a configuration error, configuration loaded"),
    }
}

#[test]
fn a_malformed_file_yields_no_policy_set() {
    let message = config_error("malformed", "[[policy]\nuser = \"analyst\"\n");
    assert!(
        message.contains("could not parse policy configuration"),
        "{message}"
    );
}

#[test]
fn a_policy_missing_its_column_yields_no_policy_set_and_names_the_policy() {
    let message = config_error(
        "no-column",
        r#"
[[policy]]
user = "analyst"
database = "sales"
table = "orders"
permitted_values = ["APAC"]
"#,
    );
    assert!(message.contains("missing field `column`"), "{message}");
    assert!(message.contains("policy #1"), "{message}");
    assert!(message.contains("sales.orders"), "{message}");
}

#[test]
fn a_policy_missing_its_permitted_values_yields_no_policy_set_and_names_the_policy() {
    let message = config_error(
        "no-values",
        r#"
[[policy]]
user = "analyst"
database = "sales"
table = "orders"
column = "region"
"#,
    );
    assert!(
        message.contains("missing field `permitted_values`"),
        "{message}"
    );
    assert!(message.contains("sales.orders"), "{message}");
}

#[test]
fn a_policy_missing_its_user_yields_no_policy_set_and_names_the_policy() {
    let message = config_error(
        "no-user",
        r#"
[[policy]]
database = "sales"
table = "orders"
column = "region"
permitted_values = ["APAC"]
"#,
    );
    assert!(message.contains("missing field `user`"), "{message}");
    assert!(message.contains("sales.orders"), "{message}");
}

#[test]
fn a_policy_missing_its_table_yields_no_policy_set_and_names_the_policy() {
    let message = config_error(
        "no-table",
        r#"
[[policy]]
user = "analyst"
database = "sales"
column = "region"
permitted_values = ["APAC"]
"#,
    );
    assert!(message.contains("missing field `table`"), "{message}");
    assert!(message.contains("\"analyst\""), "{message}");
}

#[test]
fn an_empty_permitted_set_yields_no_policy_set_and_names_the_policy() {
    let message = config_error(
        "empty-values",
        r#"
[[policy]]
user = "analyst"
database = "sales"
table = "orders"
column = "region"
permitted_values = []
"#,
    );
    assert!(message.contains("`permitted_values` is empty"), "{message}");
    assert!(message.contains("sales.orders"), "{message}");
}

#[test]
fn one_invalid_policy_discards_the_valid_ones_in_the_same_file() {
    // "The proxy SHALL NOT start with partially applied configuration."
    let message = config_error(
        "one-invalid",
        r#"
[[policy]]
user = "analyst"
database = "sales"
table = "orders"
column = "region"
permitted_values = ["APAC"]

[[policy]]
user = "auditor"
database = "sales"
table = "invoices"
column = "region"
permitted_values = []
"#,
    );
    assert!(message.contains("policy #2"), "{message}");
}

#[test]
fn editing_the_file_does_not_change_a_loaded_policy_set() {
    let file = ScratchFile::with_contents("immutable", VALID);
    let policies = PolicySet::load_from_path(file.path()).expect("valid configuration loads");

    file.overwrite(
        r#"
[[policy]]
user = "analyst"
database = "sales"
table = "orders"
column = "region"
permitted_values = ["AMER"]

[[policy]]
user = "reporting"
database = "sales"
table = "orders"
column = "region"
permitted_values = ["APAC"]
"#,
    );

    // The loaded set is what the process enforces, not what is on disk now.
    let decision = policies.lookup("analyst", &TableRef::qualified("sales", "orders"), None);
    assert_eq!(
        decision
            .policy()
            .expect("policy applies")
            .permitted_values(),
        [
            PermittedValue::Text("APAC".into()),
            PermittedValue::Text("EMEA".into())
        ]
    );

    assert!(!policies.has_any_policy("reporting"));
    assert_eq!(
        policies.lookup("reporting", &TableRef::qualified("sales", "orders"), None),
        PolicyDecision::Unrestricted
    );
    assert_eq!(policies.policy_count(), 1);
}

#[test]
fn a_file_edited_to_be_invalid_does_not_invalidate_a_loaded_policy_set() {
    let file = ScratchFile::with_contents("immutable-invalid", VALID);
    let policies = PolicySet::load_from_path(file.path()).expect("valid configuration loads");

    file.overwrite("this is not toml [[[");

    assert!(policies
        .lookup("analyst", &TableRef::qualified("sales", "orders"), None)
        .is_restricted());
}
