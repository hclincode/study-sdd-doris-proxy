//! Proxy configuration.
//!
//! Configuration is read once at startup. There is no reload: a restart drops
//! connections, which is acceptable at this stage and avoids having to reason
//! about a live connection whose configuration changed underneath it.

use std::collections::{HashMap, HashSet};
use std::net::ToSocketAddrs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// Default depth of the bounded channel feeding the log writer.
fn default_log_capacity() -> usize {
    8192
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ListenerConfig {
    /// Identifies the listener in log records.
    pub name: String,
    /// Address the proxy accepts client connections on.
    pub bind: String,
    /// Address of the backend MySQL server for this listener.
    pub backend: String,
    /// File that log records for this listener are appended to.
    pub log_file: PathBuf,
    /// How many records may be queued before new ones are discarded.
    #[serde(default = "default_log_capacity")]
    pub log_channel_capacity: usize,
    /// Row-filter rules: table name to a predicate appended to reads of that
    /// table. A table with no entry is not filtered, and a listener with no
    /// entries behaves exactly as one that predates this feature.
    #[serde(default)]
    pub row_filters: HashMap<String, String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(rename = "listener")]
    pub listeners: Vec<ListenerConfig>,
}

#[derive(Debug)]
pub enum ConfigError {
    Read { path: PathBuf, source: std::io::Error },
    Parse { path: PathBuf, source: toml::de::Error },
    Invalid(String),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::Read { path, source } => {
                write!(f, "cannot read config file {}: {source}", path.display())
            }
            ConfigError::Parse { path, source } => {
                write!(f, "cannot parse config file {}: {source}", path.display())
            }
            ConfigError::Invalid(m) => write!(f, "invalid configuration: {m}"),
        }
    }
}

impl std::error::Error for ConfigError {}

impl Config {
    pub fn load(path: &Path) -> Result<Config, ConfigError> {
        let text = std::fs::read_to_string(path).map_err(|source| ConfigError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        let config: Config = toml::from_str(&text).map_err(|source| ConfigError::Parse {
            path: path.to_path_buf(),
            source,
        })?;
        config.validate()?;
        Ok(config)
    }

    /// Rejects configurations that would fail confusingly later.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.listeners.is_empty() {
            return Err(ConfigError::Invalid(
                "at least one [[listener]] is required".into(),
            ));
        }

        let mut names = HashSet::new();
        let mut binds = HashSet::new();

        for l in &self.listeners {
            if l.name.trim().is_empty() {
                return Err(ConfigError::Invalid("listener name must not be empty".into()));
            }
            if !names.insert(l.name.as_str()) {
                return Err(ConfigError::Invalid(format!(
                    "duplicate listener name '{}'",
                    l.name
                )));
            }
            if l.log_file.as_os_str().is_empty() {
                return Err(ConfigError::Invalid(format!(
                    "listener '{}' has an empty log_file",
                    l.name
                )));
            }
            if l.log_channel_capacity == 0 {
                return Err(ConfigError::Invalid(format!(
                    "listener '{}' has log_channel_capacity 0, which would discard every record",
                    l.name
                )));
            }

            // Resolve both endpoints now so a typo is a startup failure rather
            // than a per-connection one. The backend is re-resolved when
            // connecting, so DNS may still change at runtime.
            let bind_addr = resolve_one(&l.bind).map_err(|e| {
                ConfigError::Invalid(format!("listener '{}' bind {}: {e}", l.name, l.bind))
            })?;
            if !binds.insert(bind_addr) {
                return Err(ConfigError::Invalid(format!(
                    "listener '{}' binds {} which another listener already uses",
                    l.name, l.bind
                )));
            }
            resolve_one(&l.backend).map_err(|e| {
                ConfigError::Invalid(format!("listener '{}' backend {}: {e}", l.name, l.backend))
            })?;

            // Predicates are spliced into statements verbatim, so a malformed
            // one must stop the proxy from starting rather than surface as
            // broken SQL on every query against that table.
            for (table, predicate) in &l.row_filters {
                if table.trim().is_empty() {
                    return Err(ConfigError::Invalid(format!(
                        "listener '{}' has a row filter with an empty table name",
                        l.name
                    )));
                }
                crate::row_filter::validate_predicate(predicate).map_err(|e| {
                    ConfigError::Invalid(format!(
                        "listener '{}' row filter for table '{}': {e}",
                        l.name, table
                    ))
                })?;
            }
        }

        Ok(())
    }
}

fn resolve_one(addr: &str) -> Result<std::net::SocketAddr, String> {
    addr.to_socket_addrs()
        .map_err(|e| format!("cannot resolve: {e}"))?
        .next()
        .ok_or_else(|| "resolved to no addresses".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Result<Config, ConfigError> {
        let config: Config = toml::from_str(text).map_err(|source| ConfigError::Parse {
            path: PathBuf::from("<test>"),
            source,
        })?;
        config.validate()?;
        Ok(config)
    }

    #[test]
    fn parses_a_listener() {
        let c = parse(
            r#"
            [[listener]]
            name = "primary"
            bind = "127.0.0.1:13307"
            backend = "127.0.0.1:3306"
            log_file = "/tmp/proxy.jsonl"
            "#,
        )
        .unwrap();
        assert_eq!(c.listeners.len(), 1);
        assert_eq!(c.listeners[0].name, "primary");
        assert_eq!(c.listeners[0].log_channel_capacity, default_log_capacity());
    }

    #[test]
    fn parses_multiple_listeners_with_explicit_capacity() {
        let c = parse(
            r#"
            [[listener]]
            name = "a"
            bind = "127.0.0.1:13307"
            backend = "127.0.0.1:3306"
            log_file = "/tmp/a.jsonl"
            log_channel_capacity = 16

            [[listener]]
            name = "b"
            bind = "127.0.0.1:13308"
            backend = "127.0.0.1:3306"
            log_file = "/tmp/b.jsonl"
            "#,
        )
        .unwrap();
        assert_eq!(c.listeners.len(), 2);
        assert_eq!(c.listeners[0].log_channel_capacity, 16);
    }

    #[test]
    fn rejects_an_empty_config() {
        assert!(parse("").is_err());
    }

    #[test]
    fn rejects_duplicate_bind_addresses() {
        let err = parse(
            r#"
            [[listener]]
            name = "a"
            bind = "127.0.0.1:13307"
            backend = "127.0.0.1:3306"
            log_file = "/tmp/a.jsonl"

            [[listener]]
            name = "b"
            bind = "127.0.0.1:13307"
            backend = "127.0.0.1:3306"
            log_file = "/tmp/b.jsonl"
            "#,
        )
        .unwrap_err();
        assert!(format!("{err}").contains("already uses"), "{err}");
    }

    #[test]
    fn rejects_duplicate_names() {
        assert!(parse(
            r#"
            [[listener]]
            name = "same"
            bind = "127.0.0.1:13307"
            backend = "127.0.0.1:3306"
            log_file = "/tmp/a.jsonl"

            [[listener]]
            name = "same"
            bind = "127.0.0.1:13308"
            backend = "127.0.0.1:3306"
            log_file = "/tmp/b.jsonl"
            "#
        )
        .is_err());
    }

    #[test]
    fn rejects_unresolvable_addresses() {
        assert!(parse(
            r#"
            [[listener]]
            name = "a"
            bind = "not a host at all"
            backend = "127.0.0.1:3306"
            log_file = "/tmp/a.jsonl"
            "#
        )
        .is_err());
    }

    #[test]
    fn rejects_zero_capacity_and_empty_log_path() {
        assert!(parse(
            r#"
            [[listener]]
            name = "a"
            bind = "127.0.0.1:13307"
            backend = "127.0.0.1:3306"
            log_file = "/tmp/a.jsonl"
            log_channel_capacity = 0
            "#
        )
        .is_err());

        assert!(parse(
            r#"
            [[listener]]
            name = "a"
            bind = "127.0.0.1:13307"
            backend = "127.0.0.1:3306"
            log_file = ""
            "#
        )
        .is_err());
    }

    #[test]
    fn rejects_unknown_keys() {
        assert!(parse(
            r#"
            [[listener]]
            name = "a"
            bind = "127.0.0.1:13307"
            backend = "127.0.0.1:3306"
            log_file = "/tmp/a.jsonl"
            typo_key = true
            "#
        )
        .is_err());
    }

    #[test]
    fn parses_row_filter_rules() {
        let c = parse(
            r#"
            [[listener]]
            name = "primary"
            bind = "127.0.0.1:13307"
            backend = "127.0.0.1:3306"
            log_file = "/tmp/a.jsonl"
            row_filters = { orders = "tenant_id = 7", "shop.invoices" = "org = 7" }
            "#,
        )
        .unwrap();
        assert_eq!(c.listeners[0].row_filters.len(), 2);
        assert_eq!(
            c.listeners[0].row_filters.get("orders").map(String::as_str),
            Some("tenant_id = 7")
        );
    }

    #[test]
    fn row_filters_default_to_empty_so_existing_configs_are_unchanged() {
        let c = parse(
            r#"
            [[listener]]
            name = "primary"
            bind = "127.0.0.1:13307"
            backend = "127.0.0.1:3306"
            log_file = "/tmp/a.jsonl"
            "#,
        )
        .unwrap();
        assert!(c.listeners[0].row_filters.is_empty());
    }

    #[test]
    fn an_invalid_predicate_fails_startup_and_names_listener_and_table() {
        let err = parse(
            r#"
            [[listener]]
            name = "primary"
            bind = "127.0.0.1:13307"
            backend = "127.0.0.1:3306"
            log_file = "/tmp/a.jsonl"
            row_filters = { orders = "1=1; DROP TABLE orders" }
            "#,
        )
        .unwrap_err();
        let text = format!("{err}");
        assert!(text.contains("primary"), "{text}");
        assert!(text.contains("orders"), "{text}");
        assert!(text.contains("';'"), "{text}");
    }

    #[test]
    fn rejects_a_predicate_with_a_trailing_comment() {
        assert!(parse(
            r#"
            [[listener]]
            name = "primary"
            bind = "127.0.0.1:13307"
            backend = "127.0.0.1:3306"
            log_file = "/tmp/a.jsonl"
            row_filters = { orders = "tenant_id = 7 -- note" }
            "#
        )
        .is_err());
    }

    #[test]
    fn rejects_an_empty_table_name() {
        assert!(parse(
            r#"
            [[listener]]
            name = "primary"
            bind = "127.0.0.1:13307"
            backend = "127.0.0.1:3306"
            log_file = "/tmp/a.jsonl"
            row_filters = { "" = "tenant_id = 7" }
            "#
        )
        .is_err());
    }

    #[test]
    fn missing_file_is_a_read_error() {
        let err = Config::load(Path::new("/nonexistent/nope.toml")).unwrap_err();
        assert!(matches!(err, ConfigError::Read { .. }));
    }
}
