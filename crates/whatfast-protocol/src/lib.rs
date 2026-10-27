//! Protobuf contract of the Whatfast bridge.
//!
//! The message types are generated from `proto/bridge.proto`. The envelope
//! framing (4-byte big-endian length + payload) and the request/response
//! lifecycle live in `transport.rs`, which is still a TODO(port).

pub mod fake;
pub mod local;
pub mod transport;

include!(concat!(env!("OUT_DIR"), "/whatfast.bridge.v1.rs"));
