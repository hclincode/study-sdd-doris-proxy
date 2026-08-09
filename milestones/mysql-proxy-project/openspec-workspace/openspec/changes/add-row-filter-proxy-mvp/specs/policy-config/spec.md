## Purpose

Defines the row-filtering policy that the proxy enforces: which user, querying which table, may see rows whose column value falls in which permitted set. This configuration lives outside Doris so it can be reviewed, diffed and deployed as a versioned artifact, and it is the sole source of the predicates the proxy injects.

## ADDED Requirements

### Requirement: A policy binds a user and table to a permitted set of column values

The configuration SHALL express, for a given username and a given qualified table, one column name and a set of permitted values for that column. The proxy SHALL treat that set as the complete definition of which rows that user may see in that table.

#### Scenario: A configured policy is resolved for a user and table

- **WHEN** the configuration declares that user `analyst` querying table `sales.orders` is restricted to column `region` with permitted values `["APAC", "EMEA"]`
- **THEN** the proxy resolves that policy when `analyst` queries `sales.orders`, restricting results to rows whose `region` is `APAC` or `EMEA`

#### Scenario: Policies are independent per user

- **WHEN** the configuration declares different permitted values on `sales.orders` for `analyst` and for `auditor`
- **THEN** each user's queries are restricted using only their own permitted values

#### Scenario: Policies are independent per table

- **WHEN** user `analyst` has a policy on `sales.orders` and no policy on `sales.products`
- **THEN** queries against `sales.orders` are restricted and queries against `sales.products` are not

### Requirement: A table with no policy for the requesting user is unrestricted

The proxy SHALL forward references to tables for which the requesting user has no configured policy without injecting a predicate. Absence of a policy SHALL mean "no restriction", not "no access".

#### Scenario: Unlisted table passes through unmodified

- **WHEN** user `analyst` queries table `sales.products`, for which no policy is configured for `analyst`
- **THEN** the statement's reference to `sales.products` is forwarded without an injected predicate

#### Scenario: A user with no policies at all is unrestricted

- **WHEN** user `reporting` appears nowhere in the configuration and issues a parseable `SELECT`
- **THEN** the statement is forwarded without any injected predicate

### Requirement: Table identity in configuration is unambiguous

The configuration SHALL identify tables by qualified name so that a policy cannot be evaded or misapplied through a different spelling of the same table. The proxy SHALL resolve an unqualified table reference against the session's current database before matching it to a policy.

#### Scenario: Unqualified reference resolves to the qualified policy

- **WHEN** a policy is configured for `sales.orders`, the session's current database is `sales`, and the user queries `FROM orders`
- **THEN** the proxy applies the `sales.orders` policy to that reference

#### Scenario: Same-named table in another database is not matched

- **WHEN** a policy is configured for `sales.orders`, the session's current database is `staging`, and the user queries `FROM orders`
- **THEN** the proxy does not apply the `sales.orders` policy to that reference

#### Scenario: Aliased reference is still matched

- **WHEN** a policy is configured for `sales.orders` and the user queries `FROM sales.orders AS o`
- **THEN** the proxy applies the policy to that reference

#### Scenario: A respelled table name is still matched

- **WHEN** a policy is configured for `sales.orders` and the user queries `FROM SALES.Orders`
- **THEN** the proxy applies the policy to that reference

#### Scenario: An unqualified reference with no current database is refused

- **WHEN** a policy is configured for `sales.orders`, the session has no current database, and a user with at least one configured policy queries `FROM orders`
- **THEN** the proxy rejects the statement rather than forwarding an unconstrained reference

#### Scenario: A user with no policies is unaffected by an unresolvable reference

- **WHEN** a user with no configured policy for any table queries `FROM orders` while the session has no current database
- **THEN** the statement is forwarded without an injected predicate

### Requirement: Table names are matched case-insensitively

The proxy SHALL compare database and table names ASCII-case-insensitively when matching a reference to a policy, so that a policy cannot be evaded by respelling the name. The proxy SHALL compare usernames case-sensitively, matching the behaviour of MySQL account names. Two policies for the same user whose table names differ only by case SHALL be rejected as a configuration error.

#### Scenario: Case-respelled reference does not evade a policy

- **WHEN** a policy is configured for `sales.orders` and a restricted user queries `FROM SALES.ORDERS`
- **THEN** the policy is applied

#### Scenario: Usernames differing by case are different accounts

- **WHEN** a policy is configured for user `analyst` and user `Analyst` connects
- **THEN** `Analyst` is not subject to `analyst`'s policy

#### Scenario: Case-colliding policies for one user are a configuration error

- **WHEN** the configuration declares policies for one user on both `sales.orders` and `sales.ORDERS`
- **THEN** the proxy exits with a diagnostic rather than starting with an ambiguous match

### Requirement: Unrecognised configuration is rejected

The proxy SHALL reject configuration containing any key or section it does not recognise, rather than ignoring it. A configuration whose policies cannot be read because they were misspelled SHALL prevent startup, not yield an empty policy set.

#### Scenario: A misspelled section does not silently disable filtering

- **WHEN** the configuration declares its policies under a misspelled section name, so that no policy is recognised
- **THEN** the proxy exits with a diagnostic, rather than starting with zero policies and filtering nothing

#### Scenario: An unrecognised key within a policy is rejected

- **WHEN** a configured policy carries a key the proxy does not recognise
- **THEN** the proxy exits with a diagnostic naming the offending policy

### Requirement: Invalid configuration prevents startup

The proxy SHALL validate the configuration before accepting client connections and SHALL refuse to start when it is malformed, when a policy names an empty permitted-value set, or when a policy is missing a user, table, or column. The proxy SHALL NOT start with partially applied configuration.

#### Scenario: Malformed configuration aborts startup

- **WHEN** the proxy is started with a configuration file that cannot be parsed
- **THEN** the proxy exits with a diagnostic identifying the problem and accepts no client connections

#### Scenario: Incomplete policy aborts startup

- **WHEN** a configured policy omits the column name or the permitted-value set
- **THEN** the proxy exits with a diagnostic naming the offending policy and accepts no client connections

#### Scenario: An empty permitted set is rejected as configuration error

- **WHEN** a configured policy declares an empty set of permitted values
- **THEN** the proxy exits with a diagnostic rather than starting with a policy that silently permits no rows

#### Scenario: A permitted value of an unsupported type is rejected at load

- **WHEN** a configured policy declares a permitted value that is neither text nor an integer the proxy can represent
- **THEN** the proxy exits with a diagnostic that names the unsupported value and states which types are supported, rather than coercing it or starting with a policy it cannot faithfully enforce

#### Scenario: A permitted value the proxy cannot represent fails loudly, not quietly

- **WHEN** a configured policy declares a numeric permitted value too large for the proxy's integer representation, as a Doris `LARGEINT` or `BIGINT UNSIGNED` key may be
- **THEN** the proxy exits with a diagnostic rather than truncating the value or matching a different row set than the operator intended

### Requirement: Permitted values must survive transmission unaltered

A permitted value only means what the operator intended if the value the backend compares against is the value that was configured. The proxy SHALL reject at load time any permitted value it cannot guarantee to transmit unaltered, and SHALL NOT rely on the backend's escaping mode to make a value safe. The diagnostic SHALL name the offending policy and value and explain that the value cannot be transmitted faithfully.

#### Scenario: A value containing a backslash is rejected

- **WHEN** a configured policy declares a permitted value containing a backslash
- **THEN** the proxy exits with a diagnostic, rather than emitting a value the backend may read as an escape sequence and so permit a row the operator never permitted

#### Scenario: A value containing a doubled quote is rejected

- **WHEN** a configured policy declares a permitted value containing two consecutive single-quote characters
- **THEN** the proxy exits with a diagnostic, rather than emitting a value the backend reads as a single quote and so matching a different row set

#### Scenario: Ordinary values containing a quote still load

- **WHEN** a configured policy declares a permitted value containing a single quote, such as `O'Brien`
- **THEN** the configuration loads and queries constrained by that policy match rows whose value is exactly `O'Brien`

#### Scenario: A value cannot alter the structure of the injected predicate

- **WHEN** a configured policy declares a permitted value containing SQL punctuation, such as a quote followed by a closing parenthesis and a comment marker
- **THEN** either the configuration is rejected or the emitted predicate treats the value strictly as a value, and in no case does the injected predicate change shape

### Requirement: Configuration is fixed for the lifetime of the process

The proxy SHALL load configuration once at startup and SHALL apply the same policy set for the lifetime of the process. Policy SHALL NOT change underneath an established session.

#### Scenario: Editing the file does not change a running proxy

- **WHEN** the configuration file is modified while the proxy is running
- **THEN** established and new sessions continue to be filtered using the configuration loaded at startup
