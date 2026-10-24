//! The WhatsApp sidecar library: the bridge protocol, the session, the data
//! store, and the event ingestion. The binary in `main.rs` is a thin shell
//! over this; the integration tests exercise the library directly.

pub mod bridge;
pub mod calls;
pub mod db;
pub mod ingest;
pub mod media;
pub mod meta;
pub mod send;
pub mod server;
pub mod session;
pub mod wire;
