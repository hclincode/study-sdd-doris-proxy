## MODIFIED Requirements

### Requirement: Forwarded traffic is byte-faithful

Apart from the capability bits cleared during the connection phase and command payloads the proxy has deliberately rewritten, the proxy SHALL forward every packet payload to its peer unmodified, preserving packet boundaries and sequence identifiers. Responses SHALL always be forwarded unmodified. A command payload that the client split across multiple packets because it exceeds the maximum packet size SHALL be forwarded as the same number of packets with the same sequence identifiers, and a rewritten command SHALL occupy the same number of packets as the command the client sent.

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

#### Scenario: Command the proxy did not rewrite

- **WHEN** the proxy forwards a command it chose not to rewrite
- **THEN** the payload the backend receives is byte-identical to the one the client sent

#### Scenario: Command the proxy rewrote

- **WHEN** the proxy forwards a command whose statement it rewrote
- **THEN** the payload differs from the one the client sent only by the inserted text
- **AND** it occupies the same number of packets with the same sequence identifiers
- **AND** the response is returned to the client unmodified
