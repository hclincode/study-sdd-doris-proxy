## Purpose

Defines how a MySQL client reaches an Apache Doris frontend through the proxy, and establishes the authenticated identity that row-filtering policy is keyed on. Without a trustworthy username there is nothing to filter by, so this capability owns the rule that policy applies only to an identity Doris itself has accepted.

## ADDED Requirements

### Requirement: Client sessions map one-to-one onto backend connections

The proxy SHALL open exactly one connection to the configured Doris frontend for each accepted client session, and SHALL use that connection exclusively for that session's statements. The proxy SHALL NOT pool, share, or multiplex backend connections across client sessions.

#### Scenario: Each client session owns a distinct backend connection

- **WHEN** two clients connect to the proxy concurrently
- **THEN** the proxy holds two separate backend connections to the Doris frontend, and statements issued on one client session are executed only on that session's backend connection

#### Scenario: Backend connection is released when the client disconnects

- **WHEN** a client closes its connection, or the connection is lost
- **THEN** the proxy closes the corresponding backend connection

#### Scenario: Backend unavailable at session start

- **WHEN** a client connects and the proxy cannot establish a backend connection to the Doris frontend
- **THEN** the proxy refuses the client session with an error, and does not accept statements it cannot forward

### Requirement: Doris is the sole authority on identity

The proxy SHALL delegate authentication to the Doris frontend and SHALL NOT maintain its own credential store. A client session SHALL be treated as authenticated only after the Doris frontend has accepted that user's credentials.

#### Scenario: Rejected credentials do not produce a session

- **WHEN** a client presents credentials that the Doris frontend rejects
- **THEN** the proxy fails the client's authentication and the session does not reach a state where statements can be issued

#### Scenario: Accepted credentials establish the session identity

- **WHEN** the Doris frontend accepts a client's credentials for user `analyst`
- **THEN** the proxy records `analyst` as the authenticated identity for that session

### Requirement: Policy is keyed on the backend-authenticated username

The proxy SHALL resolve row-filtering policy using the username the Doris frontend authenticated, and SHALL NOT apply policy based on a username claimed by a client that has not completed authentication.

#### Scenario: Policy follows the authenticated user, not the claimed one

- **WHEN** a client claims username `admin` during the handshake but the Doris frontend authenticates the session as `analyst`
- **THEN** the proxy applies the policy configured for `analyst`

#### Scenario: Statements are not accepted before authentication completes

- **WHEN** a client sends a query before authentication has completed
- **THEN** the proxy does not forward the statement to the backend

### Requirement: The proxy forwards statement results without altering their shape

For any statement the proxy forwards, the proxy SHALL relay the backend's result set to the client with the same columns, in the same order, with the same types, and SHALL relay backend errors to the client rather than substituting its own.

#### Scenario: Result columns are unchanged by proxying

- **WHEN** a client issues a statement that the proxy forwards unmodified
- **THEN** the client receives the same columns, in the same order and with the same types, as it would receive connecting to the Doris frontend directly

#### Scenario: Backend errors reach the client

- **WHEN** the Doris frontend returns an error for a forwarded statement
- **THEN** the client receives that error rather than a generic proxy failure
