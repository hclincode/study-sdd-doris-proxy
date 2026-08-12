//! MySQL wire-protocol handling: framing, the connection phase, command
//! classification, and the response state machine.

pub mod capabilities;
pub mod command;
pub mod connection_phase;
pub mod framing;
pub mod response;
