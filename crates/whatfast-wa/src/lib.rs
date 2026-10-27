//! The WhatsApp sidecar library: the bridge protocol, the session, the data
//! store, and the event ingestion. The binary in `main.rs` is a thin shell
//! over this; the integration tests exercise the library directly.

pub mod bridge;
pub mod calls;
pub mod db;
pub mod drafts;
pub mod guard;
pub mod guides;
pub mod identity;
pub mod ingest;
pub mod media;
pub mod meta;
pub mod reconcile;
pub mod scheduler;
pub mod schedules;
pub mod send;
pub mod server;
pub mod session;
pub mod socket;
pub mod wire;
