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

    /// A session-configuring statement would have read a restricted table's rows
    /// into session state.
    ///
    /// `SET @x = (SELECT total FROM sales.orders)` puts rows somewhere the proxy
    /// does not track, and `SELECT @x` returns them afterwards — so wrapping the
    /// relation would not bound what the client can read. Refusing is the only
    /// honest answer.
    ///
    /// Unlike [`Self::UnresolvableTableReference`], this one gets wording of its
    /// own. That refusal fires for tables that only *might* be policy-bearing,
    /// so distinct wording would confirm which; this one fires only for tables
    /// that *are*, and [`Self::WriteToRestrictedTable`] already discloses exactly
    /// that much in the same situation. Saying so plainly costs nothing and tells
    /// an operator which control refused.
    #[error("statements that read a restricted table into session state are not permitted")]
    RestrictedTableIntoSessionState,

    /// Rewriting the statement would have discarded a `/*!NNNNN … */` version
    /// gate, so it is refused rather than rewritten.
    ///
    /// MySQL runs a gate's contents only when the server is at least version
    /// `NNNNN` — a question about the backend that only the backend can answer.
    /// `sqlparser` parses the contents as ordinary SQL and drops the gate, so
    /// re-rendering emits the fragment unconditionally: the proxy would have
    /// answered that question on the backend's behalf, always "yes".
    ///
    /// Named for the mechanism, not the syntax. The hazard is **rewriting**, not
    /// executable comments — the identical statement is forwarded byte for byte
    /// when it needs no rewrite, which is what keeps `mysqldump` working. A name
    /// like "executable comment not supported" would invite someone to widen
    /// this into a blanket refusal and break that.
    ///
    /// Wording of its own is deliberate, and the reasoning differs from
    /// [`Self::UnresolvableTableReference`]: the client wrote the comment, so
    /// naming it discloses nothing the client does not already know, and the
    /// same policy-bearing inference is already available from
    /// [`Self::WriteToRestrictedTable`]. Against that, a `mysqldump` user who is
    /// told only "unsupported construct" will spend an afternoon on their SQL
    /// rather than on the comment.
    #[error(
        "this statement cannot be rewritten without discarding a version-gated \
         comment, so it was refused rather than altered"
    )]
    RewriteWouldDiscardVersionGate,

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
