## Purpose

Defines how the proxy accepts MySQL client connections, relays them to a backend MySQL server without participating in authentication, and stays synchronized with the wire protocol so that every command and its response can be observed.

## Requirements

### Requirement: Listener accepts clients and pairs each with one backend connection

The proxy SHALL listen on each configured TCP listener and, for every accepted client connection, open exactly one connection to that listener's configured backend MySQL server. A client connection and its backend connection SHALL remain paired for their entire lifetime; the proxy SHALL NOT share, pool, or reuse backend connections across clients.

#### Scenario: Client connects successfully

- **WHEN** a client opens a TCP connection to a configured listener
- **THEN** the proxy opens a dedicated TCP connection to that listener's configured backend
- **AND** all subsequent traffic for that client is carried on that one backend connection

#### Scenario: Backend is unreachable

- **WHEN** a client connects but the proxy cannot establish a backend connection
- **THEN** the proxy sends the client a MySQL ERR packet describing the connection failure
- **AND** closes the client connection without leaving a backend connection open

#### Scenario: Two clients connect concurrently

- **WHEN** two clients are connected to the same listener at the same time
- **THEN** each has its own backend connection
- **AND** session state set by one client (current database, session variables, temporary tables, prepared statements) is never visible to the other

### Requirement: Authentication is relayed end-to-end

The proxy SHALL NOT authenticate clients and SHALL NOT store, derive, or require any client credential. It SHALL relay the connection-phase packets between client and backend so that authentication is negotiated end-to-end, with the backend's salt and the backend's authentication plugin governing the exchange.

#### Scenario: Client authenticates with valid credentials

- **WHEN** a client completes the handshake against the backend's salt
- **THEN** the proxy forwards the client's authentication response to the backend unmodified
- **AND** forwards the backend's OK packet to the client
- **AND** the connection proceeds to the command phase

#### Scenario: Backend rejects the credentials

- **WHEN** the backend responds to authentication with an ERR packet
- **THEN** the proxy forwards that ERR packet to the client unmodified
- **AND** closes both connections

#### Scenario: Authentication plugin switch

- **WHEN** the backend requests an authentication method switch during the connection phase
- **THEN** the proxy forwards the switch request and the client's reply without interpreting the plugin-specific payloads

#### Scenario: Public-key exchange for full authentication

- **WHEN** the backend requests full authentication and the client requests the server's public key
- **THEN** the proxy relays the additional authentication packets in both directions
- **AND** the proxy does not require the ability to decrypt or interpret their contents

### Requirement: Unsupported capabilities are masked from the negotiation

The proxy SHALL clear the `CLIENT_SSL`, `CLIENT_COMPRESS`, and `CLIENT_LOCAL_FILES` capability bits from the backend's advertised capabilities before forwarding the initial handshake to the client, so that no connection negotiates encryption, compression, or client-side file loading. It SHALL also clear those bits from the client's handshake response before forwarding it to the backend, so that neither side believes a masked capability is in force. The proxy SHALL record the capability flags actually negotiated for each connection, because response framing depends on them.

#### Scenario: Backend advertises TLS support

- **WHEN** the backend's initial handshake advertises `CLIENT_SSL`
- **THEN** the proxy forwards a handshake to the client with `CLIENT_SSL` cleared
- **AND** a client configured to prefer TLS proceeds over an unencrypted connection

#### Scenario: Client advertises a masked capability

- **WHEN** a client's handshake response sets a masked capability such as `CLIENT_LOCAL_FILES`, which clients advertise according to what they support rather than what the server offered
- **THEN** the proxy clears that bit before forwarding the response to the backend
- **AND** the connection proceeds normally
- **AND** the backend does not treat the capability as negotiated

#### Scenario: Client attempts a TLS upgrade

- **WHEN** a client's handshake response sets `CLIENT_SSL`, meaning it will begin a TLS handshake immediately
- **THEN** the proxy closes the connection rather than forwarding the response to the backend

#### Scenario: Client requires TLS

- **WHEN** a client is configured to require TLS
- **THEN** the connection fails at the client
- **AND** the proxy does not attempt any protocol upgrade

### Requirement: Forwarded traffic is byte-faithful

Apart from the capability bits cleared during the connection phase, the proxy SHALL forward every packet payload to its peer unmodified, preserving packet boundaries and sequence identifiers. A command payload that the client split across multiple packets because it exceeds the maximum packet size SHALL be forwarded as the same number of packets with the same sequence identifiers.

#### Scenario: Ordinary query round trip

- **WHEN** a client sends a statement and the backend returns a result set
- **THEN** the bytes the client receives are identical to the bytes the backend produced

#### Scenario: Oversized command payload

- **WHEN** a client sends a command payload larger than the maximum single-packet size, split across multiple packets
- **THEN** the proxy forwards every packet of that payload to the backend with sequence identifiers unchanged
- **AND** the backend's response is returned to the client unaltered

#### Scenario: Binary result data

- **WHEN** a response contains binary column values, including bytes that resemble protocol markers
- **THEN** the proxy forwards them unaltered and does not misinterpret them as packet boundaries

### Requirement: The proxy tracks command and response boundaries

The proxy SHALL determine, for every command it forwards, where the corresponding response begins and ends, using the capability flags negotiated for that connection. It SHALL correctly handle OK responses, ERR responses, result sets, and responses consisting of multiple sequential result sets.

#### Scenario: Statement returning a result set

- **WHEN** the backend responds with a column count, column definitions, and rows
- **THEN** the proxy recognizes the terminating packet of the result set
- **AND** treats the next client packet as a new command

#### Scenario: Statement returning no rows

- **WHEN** the backend responds with a single OK packet
- **THEN** the proxy recognizes the response as complete

#### Scenario: Statement returning an error

- **WHEN** the backend responds with an ERR packet
- **THEN** the proxy recognizes the response as complete
- **AND** the connection remains usable for subsequent commands

#### Scenario: Response containing multiple result sets

- **WHEN** the backend signals that more results follow after a result set terminates
- **THEN** the proxy continues reading result sets until none remain before considering the response complete

#### Scenario: Unexpected local-file request

- **WHEN** the backend sends a local-file request despite `CLIENT_LOCAL_FILES` having been masked
- **THEN** the proxy closes both connections rather than forwarding the request to the client

### Requirement: Commands with untrackable responses are refused

The proxy SHALL refuse commands whose response framing it cannot track, rather than forwarding them and risking loss of protocol synchronization. Refusal SHALL take the form of a MySQL ERR packet returned to the client, leaving the connection usable.

#### Scenario: Replication command

- **WHEN** a client sends a command that would place the connection into a replication stream
- **THEN** the proxy returns an ERR packet to the client stating the command is not supported
- **AND** does not forward the command to the backend
- **AND** the connection remains open for subsequent commands

### Requirement: Connection teardown is paired

The proxy SHALL close a connection pair together: when either side closes, errors, or the client issues a quit command, the proxy SHALL close the other side and release both.

#### Scenario: Client quits

- **WHEN** a client sends the quit command
- **THEN** the proxy forwards it to the backend and closes both connections

#### Scenario: Client disconnects abruptly

- **WHEN** a client's TCP connection drops without a quit command
- **THEN** the proxy closes the paired backend connection

#### Scenario: Backend closes the connection

- **WHEN** the backend closes the connection, for example on idle timeout
- **THEN** the proxy closes the paired client connection
