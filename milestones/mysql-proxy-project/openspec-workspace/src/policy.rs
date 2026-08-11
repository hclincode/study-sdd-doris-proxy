//! Row-filtering policy configuration.
//!
//! Owned by the `policy-config` capability. See
//! `openspec/changes/add-row-filter-proxy-mvp/specs/policy-config/spec.md`.
//!
//! # The two absences that must never be confused
//!
//! *No policy* for a `(user, table)` pair means **unrestricted** — the reference
//! is forwarded untouched. *A policy with no permitted values* would mean "this
//! user may see no rows at all", which the spec forbids as a silent outcome: it
//! is a configuration error that prevents startup. Those two are kept apart by
//! construction rather than by discipline:
//!
//! - [`PolicySet::lookup`] returns [`PolicyDecision::Unrestricted`] for the
//!   first case and [`PolicyDecision::Restricted`] for the second.
//! - A [`Policy`] cannot be built with an empty permitted set — validation
//!   rejects it before a [`PolicySet`] exists, so
//!   [`Policy::permitted_values`] is always non-empty.
//!
//! # Startup ordering
//!
//! There is no way to obtain a [`PolicySet`] except by successful validation, so
//! a partially applied configuration is unrepresentable. The binary must call
//! [`PolicySet::load_from_path`] and propagate its error *before* binding a
//! listener; that ordering lives in `main.rs`.
//!
//! # Configuration is immutable for the process lifetime
//!
//! [`PolicySet`] exposes no mutating operation and is read from disk exactly
//! once. Editing the file afterwards cannot affect a loaded set.
//!
//! # How permitted values reach the backend
//!
//! The rewriter maps [`PermittedValue`] onto a `sqlparser` `Value` node and
//! lets `sqlparser`'s renderer emit the literal. **Nothing in this module
//! renders SQL** — [`PermittedValue`]'s `Display` is the value as written, with
//! no quoting — so there is no second escaping path here to disagree with the
//! renderer.
//!
//! What that renderer does matters for task 8.6. It doubles an isolated `'`,
//! but it does **not** escape backslashes, and it deliberately leaves alone any
//! `'` it judges already-escaped. Pinned by `tests/policy_value_rendering.rs`:
//!
//! | configured value | rendered | MySQL reads (default `sql_mode`) |
//! |---|---|---|
//! | `O'Brien` | `'O''Brien'` | `O'Brien` — correct |
//! | `a\b` | `'a\b'` | `a` + backspace — **a different value** |
//! | `a''b` | `'a''b'` | `a'b` — **a different value** |
//! | `a\` | `'a\'` | literal never terminates — **statement corrupted** |
//!
//! Rows 2 and 3 can *widen* the permitted set if some row happens to hold the
//! transformed value, and row 4 changes the shape of the injected predicate.
//!
//! **Closed at load time.** Rather than depend on the backend's escaping mode,
//! a permitted value containing a backslash, a `''` sequence, or a NUL is
//! rejected when the file is read — see the "Permitted values must survive
//! transmission unaltered" requirement in `specs/policy-config`. Rows 2, 3 and 4
//! above are therefore unreachable from configuration; they are documented
//! because they are the reason the rule exists, and pinned by
//! `tests/policy_value_rendering.rs` so that a `sqlparser` change is noticed.
//! `O'Brien` and values carrying SQL punctuation still load: an isolated quote
//! renders correctly, so refusing those would cost function for nothing.
//!
//! ## Escaping in the emitter cannot replace this rule
//!
//! The recurring proposal is to escape these values on the way out instead —
//! double the backslashes and quotes before handing the value to `sqlparser` —
//! and then drop this rule as redundant. **The two are not interchangeable.
//! They differ in kind, not in placement.**
//!
//! Pre-escaping doubles a backslash, which is correct under MySQL's default
//! `sql_mode` and wrong under `NO_BACKSLASH_ESCAPES`, where `\\` matches two
//! literal characters. It is **mode-dependent**: correct only while an
//! assumption about the backend holds. Refusing the value at load time is
//! **mode-independent** — correct under every `sql_mode`, because the value
//! never reaches SQL at all. The proxy does not control the backend's
//! `sql_mode`, and a control whose correctness is conditional on a setting we
//! do not own is a weaker shape even when its failure mode is benign.
//!
//! There is a second, subtler reason, and it is the one that decided the
//! question. Pre-escaping is not independent of the renderer: doubling a quote
//! produces `''`, and that only survives because `sqlparser` declines to
//! re-double a quote it judges already-escaped — the same heuristic tabled
//! above, which its own source comment admits is guessing. So the emitter would
//! take `O'Brien`, a value that is already correct *without* help, and make it
//! contingent on that guess. Two things would have to hold instead of one. That
//! is coupling wearing the costume of redundancy.
//!
//! This rule is therefore the primary control. If you find pre-escaping in the
//! emitter, it is defence in depth at best, and it does not license removing
//! this rule: that trade needs a spec change and an end-to-end test under both
//! `sql_mode` settings (task 8.6), not a cleanup commit.
//!
//! **This has been proposed twice and rejected twice**, the second time by
//! someone who arrived at it independently, having correctly diagnosed the
//! underlying rendering defect. It is a reasonable-looking idea that survives
//! first contact with the evidence and fails on the two points above, so the
//! next person to think of it is in good company and should read this section
//! before acting on it. An emitter-side escaper has existed at least once;
//! whether one exists as you read this changes nothing in the argument.
//!
//! # Identifier comparison
//!
//! Database and table names are compared **ASCII-case-insensitively**, so a
//! policy cannot be evaded by respelling `sales.orders` as `Sales.ORDERS`. This
//! is the fail-closed direction: on a case-sensitive backend it can over-apply a
//! policy to a distinctly-named table, which costs compatibility rather than
//! confidentiality. Two policies for one user whose table names collide under
//! that comparison are a configuration error. Usernames are compared exactly,
//! matching MySQL's case-sensitive account names.
//!
//! **That argument holds only while identifiers are ASCII, so they are required
//! to be.** `database`, `table` and `column` are refused at load if they contain
//! any non-ASCII character. `to_ascii_lowercase` leaves such characters alone,
//! which would make one name match case-insensitively in its ASCII part and
//! case-sensitively in the rest — flipping the failure direction from
//! over-applying to **under**-applying, and an under-applied policy is a
//! disclosure. This was a real leak, not a hypothetical: a policy on
//! `sales.ordres_é` forwarded a query for `sales.ordres_É` with no predicate and
//! no refusal. See `required_ascii_identifier` for why `to_lowercase()` is not
//! the fix.

use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::fmt;
use std::path::Path;

use serde::Deserialize;

use crate::error::{ProxyError, Result};

/// A table named by database and table, as written in the configuration.
///
/// The spelling is preserved for diagnostics; matching goes through a
/// normalised key private to this module, so a policy cannot be matched by
/// anything a caller constructs by hand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QualifiedTable {
    database: String,
    table: String,
}

impl QualifiedTable {
    pub fn new(database: impl Into<String>, table: impl Into<String>) -> Self {
        Self {
            database: database.into(),
            table: table.into(),
        }
    }

    // No `database()` / `table()` part accessors: nothing needs the parts
    // separately, and mutation testing found them untested because they are
    // uncalled. `Display` renders `database.table`, which is what every
    // consumer actually wants. Add them back with a test if a caller appears.
}

impl fmt::Display for QualifiedTable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.database, self.table)
    }
}

/// The normalised form used for policy matching. Private: nothing outside this
/// module should be able to construct a match key by hand.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct TableKey {
    database: String,
    table: String,
}

impl TableKey {
    fn new(database: &str, table: &str) -> Self {
        Self {
            database: database.to_ascii_lowercase(),
            table: table.to_ascii_lowercase(),
        }
    }
}

/// A table as it appeared in a statement: qualified (`sales.orders`) or not
/// (`orders`), with any alias already stripped by the caller.
///
/// An alias is deliberately absent from this type. The alias names the relation
/// in the surrounding query; it never changes which table is being read, so it
/// cannot affect which policy applies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableRef {
    database: Option<String>,
    table: String,
}

impl TableRef {
    /// A reference written with an explicit database, as in `FROM sales.orders`.
    pub fn qualified(database: impl Into<String>, table: impl Into<String>) -> Self {
        Self {
            database: Some(database.into()),
            table: table.into(),
        }
    }

    /// A reference written bare, as in `FROM orders`. Resolved against the
    /// session's current database at lookup time.
    pub fn unqualified(table: impl Into<String>) -> Self {
        Self {
            database: None,
            table: table.into(),
        }
    }

    pub fn database(&self) -> Option<&str> {
        self.database.as_deref()
    }

    pub fn table(&self) -> &str {
        &self.table
    }
}

// No `Display` for `TableRef`: nothing formats one. It existed for diagnostics
// that were never written, and mutation testing found it unobserved — a `fmt`
// that wrote nothing at all passed the whole suite. A refusal message must not
// echo a user's SQL back anyway (design D5), so the most likely future caller
// is one that should not exist.

/// A permitted column value, emitted into the injected predicate as a literal.
///
/// Never as a parameter placeholder — invariant 3 requires the rewrite to leave
/// placeholder count and order untouched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermittedValue {
    Text(String),
    Integer(i64),
}

/// The value as written, for diagnostics and log lines.
///
/// **Deliberately not a SQL literal**: no quoting, no escaping, no delimiters.
/// Turning a configured value into SQL is the rewriter's job — it maps
/// [`PermittedValue`] variants onto `sqlparser` `Value` nodes and lets the
/// renderer quote them. Nothing in this module renders SQL, so nothing here can
/// be mistaken for the emitter and quietly become a second escaping path that
/// disagrees with the first.
///
/// This type once carried a `to_sql_literal` for that purpose. It was removed:
/// it escaped differently from the renderer, so using it to build an AST literal
/// would have double-escaped, and a plausible-looking method on the policy type
/// is exactly what someone reaches for.
impl fmt::Display for PermittedValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PermittedValue::Text(value) => f.write_str(value),
            PermittedValue::Integer(value) => write!(f, "{value}"),
        }
    }
}

/// One row-filtering rule: this user, reading this table, may see only rows
/// whose `column` is in `permitted_values`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Policy {
    // No `user` field: a policy is already stored under its username in
    // `PolicySet::by_user`, so carrying it here duplicated the map key and the
    // copy was never read. Mutation testing found the accessor untested because
    // nothing called it.
    table: QualifiedTable,
    column: String,
    permitted_values: Vec<PermittedValue>,
}

impl Policy {
    pub fn table(&self) -> &QualifiedTable {
        &self.table
    }

    pub fn column(&self) -> &str {
        &self.column
    }

    /// The complete set of values this user may see. Never empty: a policy that
    /// permits nothing is rejected at load time.
    pub fn permitted_values(&self) -> &[PermittedValue] {
        &self.permitted_values
    }
}

/// The outcome of asking whether a table reference is restricted for a user.
///
/// Three outcomes, not two. Collapsing `Unresolvable` into `Unrestricted` would
/// forward an unqualified reference that might name a policy table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyDecision<'a> {
    /// No policy applies: forward this reference untouched.
    Unrestricted,
    /// A policy applies: this reference must be constrained before forwarding.
    Restricted(&'a Policy),
    /// The reference is unqualified, the session has no current database, and
    /// the user has at least one policy — so it cannot be shown that this
    /// reference is *not* a policy table. The statement must be rejected.
    Unresolvable,
}

impl PolicyDecision<'_> {
    /// The policy to enforce, if any. `None` for both `Unrestricted` and
    /// `Unresolvable`, so callers that must distinguish them cannot do it
    /// through this method.
    pub fn policy(&self) -> Option<&Policy> {
        match self {
            PolicyDecision::Restricted(policy) => Some(policy),
            _ => None,
        }
    }

    pub fn is_unrestricted(&self) -> bool {
        matches!(self, PolicyDecision::Unrestricted)
    }

    pub fn is_restricted(&self) -> bool {
        matches!(self, PolicyDecision::Restricted(_))
    }

    /// Whether the reference could not be resolved to a table at all.
    ///
    /// # Why this exists with no production caller
    ///
    /// It was deleted once as dead API — nothing called it, so both `-> true`
    /// and `-> false` mutants survived — and restored deliberately. The triad
    /// is load-bearing as *shape*: an API offering only "restricted" and
    /// "unrestricted" teaches the next caller that there are two states, and
    /// the two-state reading of a three-state decision is the defect this
    /// project exists to avoid. It forces `!is_unrestricted()` as the only way
    /// to ask about the third case, which conflates `Restricted` with
    /// `Unresolvable` — correct for the rewriter's verifier, which wants both
    /// treated as unguarded, and a live bypass anywhere that must tell them
    /// apart.
    ///
    /// Pinned by `the_decision_predicates_answer_correctly_for_every_state`, so
    /// it is not itself an untested helper.
    pub fn is_unresolvable(&self) -> bool {
        matches!(self, PolicyDecision::Unresolvable)
    }
}

/// The validated policy configuration for the process.
///
/// Immutable once built. There is no constructor that skips validation.
#[derive(Debug, Clone, Default)]
pub struct PolicySet {
    by_user: HashMap<String, HashMap<TableKey, Policy>>,
    advisories: Vec<String>,
}

impl PolicySet {
    /// A configuration with no policies: every user is unrestricted.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Read and validate the policy file. Any failure leaves no policy set at
    /// all — there is no partially applied configuration.
    pub fn load_from_path(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let origin = path.display().to_string();
        let source = std::fs::read_to_string(path).map_err(|err| {
            ProxyError::Config(format!(
                "{origin}: could not read policy configuration: {err}"
            ))
        })?;
        Self::from_toml_str(&source, &origin)
    }

    /// Validate policy configuration already in memory. `origin` names the
    /// source in diagnostics.
    pub fn from_toml_str(source: &str, origin: &str) -> Result<Self> {
        let raw: RawFile = toml::from_str(source).map_err(|err| {
            ProxyError::Config(format!(
                "{origin}: could not parse policy configuration: {err}"
            ))
        })?;

        let mut by_user: HashMap<String, HashMap<TableKey, Policy>> = HashMap::new();

        for (index, entry) in raw.policy.iter().enumerate() {
            let label = format!("{origin}: {}", entry.describe(index));

            // `user` is compared byte-for-byte and never folded, so it has no
            // folding asymmetry to protect against. The other three are folded
            // for matching or emitted into SQL — see `required_ascii_identifier`.
            let user = required_identifier(entry.user.as_deref(), "user", &label)?;
            let database =
                required_ascii_identifier(entry.database.as_deref(), "database", &label)?;
            let table = required_ascii_identifier(entry.table.as_deref(), "table", &label)?;
            let column = required_ascii_identifier(entry.column.as_deref(), "column", &label)?;

            let Some(raw_values) = entry.permitted_values.as_deref() else {
                return Err(ProxyError::Config(format!(
                    "{label}: missing field `permitted_values`"
                )));
            };
            if raw_values.is_empty() {
                return Err(ProxyError::Config(format!(
                    "{label}: `permitted_values` is empty; a policy that permits no values is a \
                     configuration error, not a policy that permits no rows"
                )));
            }
            let permitted_values = raw_values
                .iter()
                .enumerate()
                .map(|(position, value)| permitted_value(value, position, &label))
                .collect::<Result<Vec<_>>>()?;

            let policy = Policy {
                table: QualifiedTable::new(database, table),
                column: column.to_string(),
                permitted_values,
            };

            match by_user
                .entry(user.to_string())
                .or_default()
                .entry(TableKey::new(database, table))
            {
                Entry::Vacant(slot) => {
                    slot.insert(policy);
                }
                Entry::Occupied(existing) => {
                    return Err(ProxyError::Config(format!(
                        "{label}: duplicate policy — user {user:?} already has a policy on `{}`",
                        existing.get().table()
                    )));
                }
            }
        }

        let advisories = username_advisories(&raw, origin);
        for advisory in &advisories {
            // Emitted here rather than left to the caller, so an operator sees
            // it without `main.rs` having to remember to ask.
            tracing::warn!("{advisory}");
        }

        Ok(Self {
            by_user,
            advisories,
        })
    }

    /// Whether this user has any policy at all.
    ///
    /// The rewriter needs this to decide what to do with a statement it could
    /// not parse: a user with no policies has nothing to protect (design D6).
    pub fn has_any_policy(&self, user: &str) -> bool {
        self.by_user.contains_key(user)
    }

    /// Non-fatal warnings raised while loading, already emitted via `tracing`.
    ///
    /// Exposed so they can be asserted on directly: a warning that only exists
    /// as log output is a warning nothing tests.
    pub fn advisories(&self) -> &[String] {
        &self.advisories
    }

    /// How many policies were loaded. For startup logging and tests.
    pub fn policy_count(&self) -> usize {
        self.by_user.values().map(HashMap::len).sum()
    }

    /// Resolve the policy governing `reference` for `user`.
    ///
    /// An unqualified reference is resolved against `current_database` first, so
    /// a policy on `sales.orders` matches `FROM orders` only while the session
    /// is in `sales`.
    pub fn lookup(
        &self,
        user: &str,
        reference: &TableRef,
        current_database: Option<&str>,
    ) -> PolicyDecision<'_> {
        // A user with no policies has no policy-bearing table, whatever this
        // reference resolves to — so resolution cannot matter.
        let Some(tables) = self.by_user.get(user) else {
            return PolicyDecision::Unrestricted;
        };

        let database = match reference.database().or(current_database) {
            Some(database) => database,
            None => return PolicyDecision::Unresolvable,
        };

        match tables.get(&TableKey::new(database, reference.table())) {
            Some(policy) => PolicyDecision::Restricted(policy),
            None => PolicyDecision::Unrestricted,
        }
    }
}

/// The file as written, before validation. Unknown keys are refused at both
/// levels: a misspelled `[[policies]]` section would otherwise load zero
/// policies and disable filtering entirely.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFile {
    #[serde(default)]
    policy: Vec<RawPolicy>,
}

/// Every field is optional here so validation can name what is missing, rather
/// than leaving the diagnostic to serde.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPolicy {
    user: Option<String>,
    database: Option<String>,
    table: Option<String>,
    column: Option<String>,
    permitted_values: Option<Vec<RawPermittedValue>>,
}

/// A permitted value as the file expressed it, before it is checked against what
/// [`PermittedValue`] can hold.
///
/// This exists instead of `toml::Value` for one reason: `toml::Value` stores
/// integers as `i64`, so a value outside that range fails *inside* the TOML
/// parser with a message like "u64 value was too large", naming neither the
/// policy nor the field. Deserialising through our own visitor keeps the
/// oversize case reachable, so the operator gets a diagnostic that says which
/// policy and why.
#[derive(Debug)]
enum RawPermittedValue {
    Text(String),
    Integer(i64),
    /// An integer the file expressed but `i64` cannot hold, kept as written.
    OversizeInteger(String),
    /// Any other TOML type, named for the diagnostic.
    Unsupported(&'static str),
}

impl<'de> serde::Deserialize<'de> for RawPermittedValue {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(RawPermittedValueVisitor)
    }
}

struct RawPermittedValueVisitor;

impl<'de> serde::de::Visitor<'de> for RawPermittedValueVisitor {
    type Value = RawPermittedValue;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a string or an integer")
    }

    fn visit_str<E>(self, value: &str) -> std::result::Result<Self::Value, E> {
        Ok(RawPermittedValue::Text(value.to_string()))
    }

    fn visit_i64<E>(self, value: i64) -> std::result::Result<Self::Value, E> {
        Ok(RawPermittedValue::Integer(value))
    }

    fn visit_u64<E>(self, value: u64) -> std::result::Result<Self::Value, E> {
        Ok(oversize_or_integer(i64::try_from(value), value))
    }

    fn visit_i128<E>(self, value: i128) -> std::result::Result<Self::Value, E> {
        Ok(oversize_or_integer(i64::try_from(value), value))
    }

    fn visit_u128<E>(self, value: u128) -> std::result::Result<Self::Value, E> {
        Ok(oversize_or_integer(i64::try_from(value), value))
    }

    fn visit_f64<E>(self, _value: f64) -> std::result::Result<Self::Value, E> {
        Ok(RawPermittedValue::Unsupported("float"))
    }

    fn visit_bool<E>(self, _value: bool) -> std::result::Result<Self::Value, E> {
        Ok(RawPermittedValue::Unsupported("boolean"))
    }

    fn visit_map<A>(self, mut map: A) -> std::result::Result<Self::Value, A::Error>
    where
        A: serde::de::MapAccess<'de>,
    {
        // A TOML datetime arrives as a map under a private key. Draining the map
        // rather than bailing keeps the error the *policy* error below, not a
        // serde error from an abandoned deserialiser.
        let mut kind = "table";
        while let Some(key) = map.next_key::<String>()? {
            map.next_value::<serde::de::IgnoredAny>()?;
            if key.starts_with("$__toml") {
                kind = "datetime";
            }
        }
        Ok(RawPermittedValue::Unsupported(kind))
    }

    fn visit_seq<A>(self, mut seq: A) -> std::result::Result<Self::Value, A::Error>
    where
        A: serde::de::SeqAccess<'de>,
    {
        while seq.next_element::<serde::de::IgnoredAny>()?.is_some() {}
        Ok(RawPermittedValue::Unsupported("array"))
    }
}

fn oversize_or_integer<T: fmt::Display, E>(
    converted: std::result::Result<i64, E>,
    original: T,
) -> RawPermittedValue {
    match converted {
        Ok(value) => RawPermittedValue::Integer(value),
        Err(_) => RawPermittedValue::OversizeInteger(original.to_string()),
    }
}

impl RawPolicy {
    /// Identify this policy in a diagnostic, using whatever it does supply.
    fn describe(&self, index: usize) -> String {
        format!(
            "policy #{} (user {:?}, table `{}.{}`)",
            index + 1,
            self.user.as_deref().unwrap_or("<missing>"),
            self.database.as_deref().unwrap_or("<missing>"),
            self.table.as_deref().unwrap_or("<missing>"),
        )
    }
}

/// A required identifier field: present, non-empty, and without surrounding
/// whitespace. Whitespace is refused rather than trimmed — silently rewriting an
/// identifier is how a policy ends up applying to a table nobody named.
fn required_identifier<'a>(value: Option<&'a str>, field: &str, label: &str) -> Result<&'a str> {
    let Some(value) = value else {
        return Err(ProxyError::Config(format!(
            "{label}: missing field `{field}`"
        )));
    };
    if value.is_empty() {
        return Err(ProxyError::Config(format!(
            "{label}: field `{field}` is empty"
        )));
    }
    if value.trim() != value {
        return Err(ProxyError::Config(format!(
            "{label}: field `{field}` has leading or trailing whitespace"
        )));
    }
    Ok(value)
}

/// Warn about usernames that could have been written more than one way.
///
/// # Why this is a warning and not a rule
///
/// A username is compared byte-for-byte and never folded or normalised, and
/// that is correct: Doris compares account names as byte strings, and no MySQL
/// collation performs Unicode normalisation. So NFC `josé` and NFD `josé` are
/// two genuinely *different* Doris accounts that render identically. Folding
/// them together would apply one account's policy to another — the same
/// disclosure shape as the ASCII-folding leak, pointing the other way.
///
/// Rejecting them is also wrong: an operator with a legitimately non-ASCII
/// account could then express no policy for that user at all, leaving them
/// unrestricted — the same disclosure again, by way of a proxy that will not
/// start.
///
/// So the matching is right and the hazard is in *authoring*: two byte
/// sequences no editor will distinguish. Detection is the only mitigation that
/// does not introduce a worse bug, and it is useless without the code points,
/// because seeing the two forms side by side is precisely what is impossible.
fn username_advisories(raw: &RawFile, origin: &str) -> Vec<String> {
    let mut seen: Vec<&str> = Vec::new();
    let mut advisories = Vec::new();

    for entry in &raw.policy {
        let Some(user) = entry.user.as_deref() else {
            continue;
        };
        if user.is_ascii() || seen.contains(&user) {
            continue;
        }
        seen.push(user);

        advisories.push(format!(
            "{origin}: user {user:?} ({}) contains non-ASCII characters. Doris compares account \
             names byte-for-byte and no MySQL collation applies Unicode normalisation, so a \
             username that renders identically in a different encoding is a DIFFERENT account, \
             not another spelling of this one. If these code points do not byte-match the \
             account Doris authenticates, this policy applies to nobody and that user is \
             unrestricted — compare them against the account the backend holds",
            code_points(user)
        ));
    }

    advisories
}

/// `josé` -> `U+006A U+006F U+0073 U+00E9`. The whole point of the advisory:
/// the two encodings are indistinguishable until spelled out this way.
fn code_points(value: &str) -> String {
    value
        .chars()
        .map(|ch| format!("U+{:04X}", ch as u32))
        .collect::<Vec<_>>()
        .join(" ")
}

/// An identifier that will be **case-folded** for matching, or emitted into SQL:
/// `database`, `table`, `column`. Must additionally be ASCII.
///
/// # Why ASCII-only, and why `to_lowercase()` is not the fix
///
/// Matching folds with `to_ascii_lowercase`, which leaves every non-ASCII
/// character untouched. That makes matching case-*insensitive* for the ASCII
/// part of a name and case-*sensitive* for the rest — so a policy on
/// `sales.ordres_é` did not match a query for `sales.ordres_É`, and the
/// reference was forwarded with no predicate and no refusal. An over-applied
/// policy costs function; an under-applied one is a disclosure. The asymmetry
/// was the defect, not the folding.
///
/// Swapping in `str::to_lowercase` moves the boundary rather than removing it.
/// Unicode simple lowercasing is not MySQL's collation-based folding either —
/// dotless `ı` under a Turkish collation, final sigma, and other locale-
/// dependent cases still disagree — so it would restore a rule that is correct
/// under some backend settings and silently wrong under others. That is the
/// shape this project has already rejected twice for permitted values.
///
/// Refusing at load is correct under **every** backend identifier-folding
/// setting, because the identifier never participates in a comparison at all.
/// Supporting non-ASCII names honestly means reading the backend's
/// `lower_case_table_names` and collation at startup and folding the way it
/// does — a feature, not a one-line change.
fn required_ascii_identifier<'a>(
    value: Option<&'a str>,
    field: &str,
    label: &str,
) -> Result<&'a str> {
    let value = required_identifier(value, field, label)?;

    if let Some(offender) = value.chars().find(|ch| !ch.is_ascii()) {
        return Err(ProxyError::Config(format!(
            "{label}: field `{field}` ({value:?}) contains the non-ASCII character {offender:?}. \
             Policy matching folds ASCII case only, so this name would match case-sensitively \
             where the rest of it matches case-insensitively, and a policy that silently fails to \
             match is a disclosure rather than an inconvenience. Refusing is correct under every \
             backend identifier-folding setting. Before reaching for Unicode lowercasing, read the \
             note on `required_ascii_identifier` in src/policy.rs: it is not MySQL's collation \
             folding either"
        )));
    }

    Ok(value)
}

fn permitted_value(
    value: &RawPermittedValue,
    position: usize,
    label: &str,
) -> Result<PermittedValue> {
    let position = position + 1;
    match value {
        RawPermittedValue::Text(text) => {
            // A permitted value only means what the operator intended if the
            // backend compares against the value that was configured. These
            // three cannot be guaranteed to survive rendering, so they are
            // refused here.
            //
            // Why refuse rather than escape them ourselves: pre-doubling a
            // backslash is correct under MySQL's default `sql_mode` and wrong
            // under `NO_BACKSLASH_ESCAPES`, where `\\` matches a literal two
            // characters. Refusing is correct under both. The proxy does not
            // control the backend's `sql_mode` and must not ship a control
            // whose correctness depends on it.
            if text.contains('\0') {
                return Err(ProxyError::Config(format!(
                    "{label}: permitted value #{position} ({text:?}) contains a NUL byte, which \
                     cannot be transmitted faithfully in a SQL literal"
                )));
            }
            if text.contains('\\') {
                return Err(ProxyError::Config(format!(
                    "{label}: permitted value #{position} ({text:?}) contains a backslash, which \
                     this proxy cannot transmit faithfully. The SQL renderer does not escape it, \
                     so the backend would read it as an escape sequence and the policy would \
                     match a value the operator never configured"
                )));
            }
            if text.contains("''") {
                return Err(ProxyError::Config(format!(
                    "{label}: permitted value #{position} ({text:?}) contains two consecutive \
                     single quotes, which this proxy cannot transmit faithfully. The SQL renderer \
                     leaves them as written and the backend reads them as one quote, so the \
                     policy would match a different value. A value containing a single quote, \
                     such as O'Brien, is fine"
                )));
            }
            Ok(PermittedValue::Text(text.clone()))
        }
        RawPermittedValue::Integer(number) => Ok(PermittedValue::Integer(*number)),

        // Worth a diagnostic of its own rather than "unsupported type". Doris
        // `LARGEINT` (128-bit) and `BIGINT UNSIGNED` both hold values outside
        // `i64`, and either is a plausible tenant key — so an operator can hit
        // this with a perfectly reasonable policy and deserves to be told that
        // the proxy is the limit, not their file.
        RawPermittedValue::OversizeInteger(written) => Err(ProxyError::Config(format!(
            "{label}: permitted value #{position} is the integer {written}, which does not fit \
             the 64-bit signed range this proxy represents numeric permitted values in \
             ({min} to {max}). Doris `LARGEINT` and `BIGINT UNSIGNED` columns can hold values \
             outside that range, and a policy keyed on one of them cannot be expressed today. \
             This is a known limitation of the proxy, not a malformed configuration file — \
             raise it rather than working around it, because quoting the value would make it a \
             text value and change what the policy matches",
            min = i64::MIN,
            max = i64::MAX,
        ))),

        RawPermittedValue::Unsupported(kind) => Err(ProxyError::Config(format!(
            "{label}: permitted value #{position} has unsupported type `{kind}`; permitted \
             values must be strings or integers"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load(source: &str) -> Result<PolicySet> {
        PolicySet::from_toml_str(source, "test.toml")
    }

    fn analyst_orders() -> PolicySet {
        load(
            r#"
            [[policy]]
            user = "analyst"
            database = "sales"
            table = "orders"
            column = "region"
            permitted_values = ["APAC", "EMEA"]
            "#,
        )
        .expect("valid configuration")
    }

    fn config_error(source: &str) -> String {
        match load(source) {
            Err(ProxyError::Config(message)) => message,
            Err(other) => panic!("expected a configuration error, got {other:?}"),
            Ok(_) => panic!("expected a configuration error, configuration loaded"),
        }
    }

    #[test]
    fn configured_policy_is_resolved_for_user_and_table() {
        let policies = analyst_orders();
        let decision = policies.lookup(
            "analyst",
            &TableRef::qualified("sales", "orders"),
            Some("sales"),
        );

        let policy = decision.policy().expect("policy applies");
        assert_eq!(policy.table().to_string(), "sales.orders");
        assert_eq!(policy.column(), "region");
        assert_eq!(
            policy.permitted_values(),
            [
                PermittedValue::Text("APAC".into()),
                PermittedValue::Text("EMEA".into())
            ]
        );
    }

    #[test]
    fn policies_are_independent_per_user() {
        let policies = load(
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
            table = "orders"
            column = "region"
            permitted_values = ["EMEA", "AMER"]
            "#,
        )
        .expect("valid configuration");

        let orders = TableRef::qualified("sales", "orders");
        let analyst = policies.lookup("analyst", &orders, None);
        let auditor = policies.lookup("auditor", &orders, None);

        assert_eq!(
            analyst.policy().unwrap().permitted_values(),
            [PermittedValue::Text("APAC".into())]
        );
        assert_eq!(
            auditor.policy().unwrap().permitted_values(),
            [
                PermittedValue::Text("EMEA".into()),
                PermittedValue::Text("AMER".into())
            ]
        );
    }

    #[test]
    fn policies_are_independent_per_table() {
        let policies = analyst_orders();

        assert!(policies
            .lookup("analyst", &TableRef::qualified("sales", "orders"), None)
            .is_restricted());
        assert!(policies
            .lookup("analyst", &TableRef::qualified("sales", "products"), None)
            .is_unrestricted());
    }

    #[test]
    fn unlisted_table_passes_through_unrestricted() {
        let policies = analyst_orders();
        let decision = policies.lookup("analyst", &TableRef::qualified("sales", "products"), None);

        assert_eq!(decision, PolicyDecision::Unrestricted);
        assert!(decision.policy().is_none());
    }

    #[test]
    fn user_with_no_policies_at_all_is_unrestricted() {
        let policies = analyst_orders();

        assert!(!policies.has_any_policy("reporting"));
        assert_eq!(
            policies.lookup("reporting", &TableRef::qualified("sales", "orders"), None),
            PolicyDecision::Unrestricted
        );
    }

    #[test]
    fn no_policy_is_distinct_from_a_policy_with_no_permitted_values() {
        // The distinction is enforced by construction: "no policy" is a variant
        // of the decision, and the other case cannot be loaded at all.
        let policies = analyst_orders();
        assert_eq!(
            policies.lookup("analyst", &TableRef::qualified("sales", "products"), None),
            PolicyDecision::Unrestricted
        );

        let message = config_error(
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

        // And every policy that does load permits at least one value.
        for policy in policies.by_user.values().flat_map(HashMap::values) {
            assert!(!policy.permitted_values().is_empty());
        }
    }

    #[test]
    fn unqualified_reference_resolves_to_qualified_policy() {
        let policies = analyst_orders();
        let decision = policies.lookup("analyst", &TableRef::unqualified("orders"), Some("sales"));

        assert!(decision.is_restricted());
    }

    #[test]
    fn same_named_table_in_another_database_is_not_matched() {
        let policies = analyst_orders();

        assert_eq!(
            policies.lookup("analyst", &TableRef::unqualified("orders"), Some("staging")),
            PolicyDecision::Unrestricted
        );
        assert_eq!(
            policies.lookup(
                "analyst",
                &TableRef::qualified("staging", "orders"),
                Some("sales")
            ),
            PolicyDecision::Unrestricted
        );
    }

    #[test]
    fn unqualified_reference_without_a_current_database_is_unresolvable() {
        let policies = analyst_orders();
        let decision = policies.lookup("analyst", &TableRef::unqualified("orders"), None);

        assert_eq!(decision, PolicyDecision::Unresolvable);
        assert!(decision.policy().is_none());
        assert!(!decision.is_unrestricted(), "must not read as unrestricted");
    }

    #[test]
    fn unresolvable_reference_is_unrestricted_for_a_user_with_no_policies() {
        let policies = analyst_orders();

        assert_eq!(
            policies.lookup("reporting", &TableRef::unqualified("orders"), None),
            PolicyDecision::Unrestricted
        );
    }

    #[test]
    fn table_identity_ignores_ascii_case() {
        let policies = analyst_orders();

        assert!(policies
            .lookup("analyst", &TableRef::qualified("SALES", "Orders"), None)
            .is_restricted());
    }

    #[test]
    fn ascii_case_folding_still_applies_after_the_non_ascii_rule() {
        // Pinned so that nobody "fixes" the non-ASCII leak by removing the
        // folding: dropping it would let `SALES.ORDERS` evade a policy on
        // `sales.orders`, which is the evasion the folding exists to close.
        let policies = analyst_orders();

        for (database, table) in [
            ("sales", "orders"),
            ("SALES", "ORDERS"),
            ("Sales", "Orders"),
            ("sALES", "oRDERS"),
        ] {
            assert!(
                policies
                    .lookup("analyst", &TableRef::qualified(database, table), None)
                    .is_restricted(),
                "{database}.{table} must match the sales.orders policy"
            );
        }
    }

    #[test]
    fn a_non_ascii_table_name_is_rejected_naming_the_policy() {
        // Regression: a policy on `ordres_é` did not match a query for
        // `ordres_É`, because `to_ascii_lowercase` folds neither. The reference
        // was forwarded unconstrained, with no refusal anywhere on the path.
        // Such a policy can no longer be configured at all.
        let message = config_error(
            r#"
            [[policy]]
            user = "analyst"
            database = "sales"
            table = "ordres_é"
            column = "region"
            permitted_values = ["APAC"]
            "#,
        );

        assert!(message.contains("non-ASCII character"), "{message}");
        assert!(message.contains("policy #1"), "{message}");
        assert!(message.contains("field `table`"), "{message}");
    }

    #[test]
    fn a_non_ascii_database_name_is_rejected() {
        let message = config_error(
            r#"
            [[policy]]
            user = "analyst"
            database = "vendës"
            table = "orders"
            column = "region"
            permitted_values = ["APAC"]
            "#,
        );
        assert!(message.contains("non-ASCII character"), "{message}");
        assert!(message.contains("field `database`"), "{message}");
    }

    #[test]
    fn a_non_ascii_column_name_is_rejected() {
        let message = config_error(
            r#"
            [[policy]]
            user = "analyst"
            database = "sales"
            table = "orders"
            column = "région"
            permitted_values = ["APAC"]
            "#,
        );
        assert!(message.contains("non-ASCII character"), "{message}");
        assert!(message.contains("field `column`"), "{message}");
    }

    #[test]
    fn the_non_ascii_diagnostic_warns_against_unicode_lowercasing() {
        // The obvious "fix" is `to_lowercase()`, which is also wrong. The
        // diagnostic has to say so, or it will be applied.
        let message = config_error(
            r#"
            [[policy]]
            user = "analyst"
            database = "sales"
            table = "Ördérs"
            column = "region"
            permitted_values = ["APAC"]
            "#,
        );
        assert!(message.contains("Unicode lowercasing"), "{message}");
    }

    #[test]
    fn a_non_ascii_username_is_still_accepted() {
        // Usernames are compared byte-for-byte and never folded, so they carry
        // no folding asymmetry. Restricting them would cost function for no
        // security gain — see the note to the lead about normalisation.
        let policies = load(
            r#"
            [[policy]]
            user = "josé"
            database = "sales"
            table = "orders"
            column = "region"
            permitted_values = ["APAC"]
            "#,
        )
        .expect("a non-ASCII username is representable");

        assert!(policies.has_any_policy("josé"));
        assert!(policies
            .lookup("josé", &TableRef::qualified("sales", "orders"), None)
            .is_restricted());
    }

    #[test]
    fn a_non_ascii_username_is_flagged_with_its_code_points() {
        let policies = load(
            r#"
            [[policy]]
            user = "josé"
            database = "sales"
            table = "orders"
            column = "region"
            permitted_values = ["APAC"]
            "#,
        )
        .expect("a non-ASCII username loads");

        let advisory = policies
            .advisories()
            .iter()
            .find(|a| a.contains("josé"))
            .expect("the username is flagged");

        // Without the code points the warning is unactionable: the whole
        // problem is that the two encodings render identically.
        assert!(
            advisory.contains("U+006A U+006F U+0073 U+00E9"),
            "{advisory}"
        );
        assert!(advisory.contains("DIFFERENT account"), "{advisory}");
        assert!(advisory.contains("applies to nobody"), "{advisory}");
    }

    #[test]
    fn the_two_encodings_of_a_username_are_flagged_distinctly() {
        // NFC josé and NFD josé, which no editor will show as different.
        let policies = load(
            "[[policy]]\nuser = \"jos\u{00e9}\"\ndatabase = \"sales\"\ntable = \"orders\"\n\
             column = \"region\"\npermitted_values = [\"APAC\"]\n\n\
             [[policy]]\nuser = \"jose\u{0301}\"\ndatabase = \"sales\"\ntable = \"orders\"\n\
             column = \"region\"\npermitted_values = [\"EMEA\"]\n",
        )
        .expect("both usernames load as separate accounts");

        // Two distinct users, not a duplicate-policy collision.
        assert_eq!(policies.policy_count(), 2);
        assert_eq!(policies.advisories().len(), 2);

        let joined = policies.advisories().join("\n");
        assert!(joined.contains("U+006A U+006F U+0073 U+00E9"), "{joined}");
        assert!(
            joined.contains("U+006A U+006F U+0073 U+0065 U+0301"),
            "{joined}"
        );
    }

    #[test]
    fn usernames_that_render_identically_are_never_matched_together() {
        // The mirror of the folding leak: applying one account's policy to
        // another would be a disclosure, so exact matching is the correct rule.
        let policies = load(
            "[[policy]]\nuser = \"jos\u{00e9}\"\ndatabase = \"sales\"\ntable = \"orders\"\n\
             column = \"region\"\npermitted_values = [\"APAC\"]\n",
        )
        .expect("valid configuration");

        assert!(policies.has_any_policy("jos\u{00e9}"));
        assert!(!policies.has_any_policy("jose\u{0301}"));
        assert_eq!(
            policies.lookup(
                "jose\u{0301}",
                &TableRef::qualified("sales", "orders"),
                None
            ),
            PolicyDecision::Unrestricted
        );
    }

    #[test]
    fn an_ascii_only_configuration_raises_no_advisories() {
        assert!(analyst_orders().advisories().is_empty());
    }

    #[test]
    fn a_username_is_flagged_once_however_many_policies_it_has() {
        let policies = load(
            "[[policy]]\nuser = \"jos\u{00e9}\"\ndatabase = \"sales\"\ntable = \"orders\"\n\
             column = \"region\"\npermitted_values = [\"APAC\"]\n\n\
             [[policy]]\nuser = \"jos\u{00e9}\"\ndatabase = \"sales\"\ntable = \"invoices\"\n\
             column = \"region\"\npermitted_values = [\"APAC\"]\n",
        )
        .expect("valid configuration");

        assert_eq!(policies.policy_count(), 2);
        assert_eq!(policies.advisories().len(), 1);
    }

    #[test]
    fn usernames_are_compared_case_sensitively() {
        let policies = analyst_orders();

        assert_eq!(
            policies.lookup("Analyst", &TableRef::qualified("sales", "orders"), None),
            PolicyDecision::Unrestricted
        );
    }

    #[test]
    fn malformed_configuration_is_rejected() {
        let message = config_error("[[policy]\nuser = \"analyst\"");
        assert!(
            message.contains("could not parse policy configuration"),
            "{message}"
        );
    }

    #[test]
    fn unknown_top_level_section_is_rejected() {
        // A misspelled section would otherwise load zero policies silently.
        let message = config_error(
            r#"
            [[policies]]
            user = "analyst"
            database = "sales"
            table = "orders"
            column = "region"
            permitted_values = ["APAC"]
            "#,
        );
        assert!(message.contains("could not parse"), "{message}");
    }

    #[test]
    fn unknown_field_in_a_policy_is_rejected() {
        let message = config_error(
            r#"
            [[policy]]
            user = "analyst"
            database = "sales"
            table = "orders"
            column = "region"
            permitted_values = ["APAC"]
            permited_values = ["AMER"]
            "#,
        );
        assert!(message.contains("could not parse"), "{message}");
    }

    #[test]
    fn policy_missing_user_is_rejected_naming_the_policy() {
        let message = config_error(
            r#"
            [[policy]]
            database = "sales"
            table = "orders"
            column = "region"
            permitted_values = ["APAC"]
            "#,
        );
        assert!(message.contains("missing field `user`"), "{message}");
        assert!(message.contains("policy #1"), "{message}");
        assert!(message.contains("sales.orders"), "{message}");
    }

    #[test]
    fn policy_missing_database_is_rejected_naming_the_policy() {
        let message = config_error(
            r#"
            [[policy]]
            user = "analyst"
            table = "orders"
            column = "region"
            permitted_values = ["APAC"]
            "#,
        );
        assert!(message.contains("missing field `database`"), "{message}");
        assert!(message.contains("\"analyst\""), "{message}");
    }

    #[test]
    fn policy_missing_table_is_rejected_naming_the_policy() {
        let message = config_error(
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
    fn policy_missing_column_is_rejected_naming_the_policy() {
        let message = config_error(
            r#"
            [[policy]]
            user = "analyst"
            database = "sales"
            table = "orders"
            permitted_values = ["APAC"]
            "#,
        );
        assert!(message.contains("missing field `column`"), "{message}");
        assert!(message.contains("sales.orders"), "{message}");
    }

    #[test]
    fn policy_missing_permitted_values_is_rejected_naming_the_policy() {
        let message = config_error(
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
    fn empty_permitted_set_is_rejected_as_configuration_error() {
        let message = config_error(
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
    fn empty_identifier_is_rejected() {
        let message = config_error(
            r#"
            [[policy]]
            user = "analyst"
            database = "sales"
            table = ""
            column = "region"
            permitted_values = ["APAC"]
            "#,
        );
        assert!(message.contains("field `table` is empty"), "{message}");
    }

    #[test]
    fn identifier_with_surrounding_whitespace_is_rejected() {
        let message = config_error(
            r#"
            [[policy]]
            user = "analyst"
            database = "sales "
            table = "orders"
            column = "region"
            permitted_values = ["APAC"]
            "#,
        );
        assert!(
            message.contains("field `database` has leading or trailing whitespace"),
            "{message}"
        );
    }

    #[test]
    fn permitted_value_of_unsupported_type_is_rejected() {
        let message = config_error(
            r#"
            [[policy]]
            user = "analyst"
            database = "sales"
            table = "orders"
            column = "region"
            permitted_values = ["APAC", true]
            "#,
        );
        assert!(message.contains("permitted value #2"), "{message}");
        assert!(message.contains("unsupported type"), "{message}");
    }

    #[test]
    fn permitted_value_too_large_for_i64_is_rejected_with_an_actionable_diagnostic() {
        // BIGINT UNSIGNED reaches 18446744073709551615. Without the custom
        // deserialiser this failed inside the TOML parser as "u64 value was too
        // large", naming neither the policy nor the reason.
        let message = config_error(
            r#"
            [[policy]]
            user = "analyst"
            database = "sales"
            table = "orders"
            column = "tenant_id"
            permitted_values = [18446744073709551615]
            "#,
        );

        assert!(message.contains("policy #1"), "{message}");
        assert!(message.contains("sales.orders"), "{message}");
        assert!(message.contains("18446744073709551615"), "{message}");
        assert!(message.contains("LARGEINT"), "{message}");
        assert!(message.contains("BIGINT UNSIGNED"), "{message}");
        assert!(
            message.contains("known limitation"),
            "the operator must be able to tell a proxy limit from a bad file: {message}"
        );
    }

    #[test]
    fn permitted_value_beyond_the_128_bit_range_is_rejected_the_same_way() {
        // LARGEINT's maximum, which fails at a different point in the parser.
        let message = config_error(
            r#"
            [[policy]]
            user = "analyst"
            database = "sales"
            table = "orders"
            column = "tenant_id"
            permitted_values = [170141183460469231731687303715884105727]
            "#,
        );

        assert!(
            message.contains("170141183460469231731687303715884105727"),
            "{message}"
        );
        assert!(message.contains("LARGEINT"), "{message}");
    }

    #[test]
    fn a_permitted_value_at_the_i64_boundary_still_loads() {
        let policies = load(
            r#"
            [[policy]]
            user = "analyst"
            database = "sales"
            table = "orders"
            column = "tenant_id"
            permitted_values = [9223372036854775807, -9223372036854775808]
            "#,
        )
        .expect("i64 bounds are representable");

        let decision = policies.lookup("analyst", &TableRef::qualified("sales", "orders"), None);
        assert_eq!(
            decision.policy().unwrap().permitted_values(),
            [
                PermittedValue::Integer(i64::MAX),
                PermittedValue::Integer(i64::MIN)
            ]
        );
    }

    #[test]
    fn permitted_value_containing_a_backslash_is_rejected() {
        let message = config_error(
            r#"
            [[policy]]
            user = "analyst"
            database = "sales"
            table = "orders"
            column = "region"
            permitted_values = ["AP\\AC"]
            "#,
        );
        assert!(message.contains("contains a backslash"), "{message}");
        assert!(message.contains("sales.orders"), "{message}");
    }

    #[test]
    fn permitted_value_containing_a_doubled_quote_is_rejected() {
        let message = config_error(
            r#"
            [[policy]]
            user = "analyst"
            database = "sales"
            table = "orders"
            column = "region"
            permitted_values = ["AP''AC"]
            "#,
        );
        assert!(
            message.contains("two consecutive single quotes"),
            "{message}"
        );
    }

    #[test]
    fn permitted_value_containing_a_single_quote_still_loads() {
        let policies = load(
            r#"
            [[policy]]
            user = "analyst"
            database = "sales"
            table = "orders"
            column = "owner"
            permitted_values = ["O'Brien"]
            "#,
        )
        .expect("an isolated quote is representable");

        let decision = policies.lookup("analyst", &TableRef::qualified("sales", "orders"), None);
        assert_eq!(
            decision.policy().unwrap().permitted_values(),
            [PermittedValue::Text("O'Brien".into())]
        );
    }

    #[test]
    fn permitted_value_of_a_datetime_is_named_as_such() {
        let message = config_error(
            r#"
            [[policy]]
            user = "analyst"
            database = "sales"
            table = "orders"
            column = "region"
            permitted_values = [1979-05-27T07:32:00Z]
            "#,
        );
        assert!(message.contains("unsupported type `datetime`"), "{message}");
    }

    #[test]
    fn permitted_value_containing_nul_is_rejected() {
        let message = config_error(
            "[[policy]]\nuser = \"analyst\"\ndatabase = \"sales\"\ntable = \"orders\"\n\
             column = \"region\"\npermitted_values = [\"AP\\u0000AC\"]\n",
        );
        assert!(message.contains("contains a NUL byte"), "{message}");
    }

    #[test]
    fn duplicate_policy_for_same_user_and_table_is_rejected() {
        let message = config_error(
            r#"
            [[policy]]
            user = "analyst"
            database = "sales"
            table = "orders"
            column = "region"
            permitted_values = ["APAC"]

            [[policy]]
            user = "analyst"
            database = "SALES"
            table = "Orders"
            column = "region"
            permitted_values = ["AMER"]
            "#,
        );
        assert!(message.contains("duplicate policy"), "{message}");
        // The message names the colliding table through `QualifiedTable`'s
        // `Display`, and nothing asserted that until now — the operator needs
        // to know *which* table collided, not merely that one did.
        assert!(message.contains("sales.orders"), "{message}");
    }

    #[test]
    fn a_single_invalid_policy_rejects_the_whole_file() {
        // No partially applied configuration: the valid first policy must not
        // survive the invalid second one.
        let result = load(
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
        assert!(result.is_err());
    }

    #[test]
    fn a_file_with_no_policies_loads_and_restricts_nobody() {
        let policies = load("").expect("an empty policy file is valid");

        assert_eq!(policies.policy_count(), 0);
        assert!(!policies.has_any_policy("analyst"));
        assert_eq!(
            policies.lookup("analyst", &TableRef::qualified("sales", "orders"), None),
            PolicyDecision::Unrestricted
        );
    }

    #[test]
    fn integer_permitted_values_load() {
        let policies = load(
            r#"
            [[policy]]
            user = "analyst"
            database = "sales"
            table = "orders"
            column = "tenant_id"
            permitted_values = [1, 2, 42]
            "#,
        )
        .expect("valid configuration");

        let decision = policies.lookup("analyst", &TableRef::qualified("sales", "orders"), None);
        assert_eq!(
            decision.policy().unwrap().permitted_values(),
            [
                PermittedValue::Integer(1),
                PermittedValue::Integer(2),
                PermittedValue::Integer(42)
            ]
        );
    }

    #[test]
    fn the_decision_predicates_answer_correctly_for_every_state() {
        // `is_restricted` and `is_unrestricted` are the assertion helpers a
        // dozen tests in this file lean on. Nothing asserted their *negative*
        // answers, so `replace is_restricted -> bool with true` survived the
        // whole suite — and with it, every one of those assertions would have
        // passed vacuously. This pins all three states against both predicates.
        let policies = analyst_orders();

        // Every predicate against every state — a 3x3 grid rather than the
        // positive answer of each, which is how the hole above opened.
        let restricted = policies.lookup("analyst", &TableRef::qualified("sales", "orders"), None);
        assert!(restricted.is_restricted());
        assert!(!restricted.is_unrestricted());
        assert!(!restricted.is_unresolvable());

        let unrestricted =
            policies.lookup("analyst", &TableRef::qualified("sales", "products"), None);
        assert!(!unrestricted.is_restricted());
        assert!(unrestricted.is_unrestricted());
        assert!(!unrestricted.is_unresolvable());

        let unresolvable = policies.lookup("analyst", &TableRef::unqualified("orders"), None);
        assert!(!unresolvable.is_restricted());
        assert!(
            !unresolvable.is_unrestricted(),
            "an unresolvable reference must never read as unrestricted"
        );
        assert!(unresolvable.is_unresolvable());
    }

    #[test]
    fn display_renders_the_value_as_written_without_sql_quoting() {
        // Guards the replacement for `to_sql_literal`: if `Display` ever grows
        // quotes or escaping again, `policy.rs` has a second SQL renderer in it.
        assert_eq!(PermittedValue::Text("APAC".into()).to_string(), "APAC");
        assert_eq!(
            PermittedValue::Text("O'Brien".into()).to_string(),
            "O'Brien"
        );
        assert_eq!(PermittedValue::Integer(-7).to_string(), "-7");
    }

    #[test]
    fn policy_count_reports_every_loaded_policy() {
        let policies = load(
            r#"
            [[policy]]
            user = "analyst"
            database = "sales"
            table = "orders"
            column = "region"
            permitted_values = ["APAC"]

            [[policy]]
            user = "analyst"
            database = "sales"
            table = "invoices"
            column = "region"
            permitted_values = ["APAC"]

            [[policy]]
            user = "auditor"
            database = "sales"
            table = "orders"
            column = "region"
            permitted_values = ["EMEA"]
            "#,
        )
        .expect("valid configuration");

        assert_eq!(policies.policy_count(), 3);
    }
}
