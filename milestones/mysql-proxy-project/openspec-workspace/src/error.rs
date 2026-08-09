//! Error types shared across the proxy.
//!
//! Design D5: a refusal reaches the client as a MySQL error packet with SQLSTATE
//! `42000`, distinguishing "unsupported shape" from "policy denial", and never
//! disclosing policy contents.

use thiserror::Error;

/// SQLSTATE returned for every statement the proxy refuses.
pub const SQLSTATE_REFUSED: &str = "42000";

pub type Result<T> = std::result::Result<T, ProxyError>;

/// Why the proxy refused to forward a statement.
///
/// Every variant is a *refusal*, not a failure to try. There is deliberately no
/// variant meaning "could not analyse, forwarded anyway".
#[derive(Debug, Error)]
pub enum RefusalReason {
    #[error("statement could not be parsed")]
    Unparseable,

    #[error("statement uses a construct this proxy cannot analyse")]
    UnsupportedShape { construct: String },

    /// A table was named without a database qualifier and the session has no
    /// current database to resolve it against, so it cannot be shown *not* to be
    /// a policy-bearing table.
    ///
    /// The client deliberately sees the same words as [`Self::UnsupportedShape`].
    /// The distinction is for operators, and it matters: a spike of these means
    /// clients are connecting without a default schema — plausibly a probing
    /// pattern — not that the proxy has a SQL compatibility gap. The shape is
    /// fine; the session state is ambiguous.
    ///
    /// The two are therefore separable only by variant, which is why refusals
    /// are logged with `Debug` rather than `Display`. Giving the client wording
    /// of its own would tell it which of its statements the proxy could resolve,
    /// and a table it could not resolve is precisely one that might carry a
    /// policy (design D5).
    #[error("statement uses a construct this proxy cannot analyse")]
    UnresolvableTableReference,

    #[error("write statements against restricted tables are not permitted")]
    WriteToRestrictedTable,

    #[error("multiple statements per request are not permitted")]
    MultiStatement,

    #[error("prepared statements are not supported")]
    PreparedStatement,
}

impl RefusalReason {
    /// Client-facing message. Must not name another user, table policy, or
    /// permitted value (design D5).
    pub fn client_message(&self) -> String {
        format!("proxy refused statement: {self}")
    }
}

#[derive(Debug, Error)]
pub enum ProxyError {
    #[error("statement refused: {0}")]
    Refused(#[from] RefusalReason),

    #[error("configuration error: {0}")]
    Config(String),

    #[error("backend error: {0}")]
    Backend(String),

    #[error("protocol error: {0}")]
    Protocol(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}
