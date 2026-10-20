//! Port of the `reconcileAuthoritativeChats` logic of
//! `tauri/src/state/app-helpers.ts` (SolidJS frontend, frozen oracle).
//!
//! The authoritative list of the chats of the backend replaces the local one
//! when the account resyncs, but a chat whose revision moved while the list
//! was being fetched keeps the copy the live events delivered.
//!
//! port: the function is already ported in
//! [`crate::app_helpers::reconcile_authoritative_chats`], the module that
//! ports `state/app-helpers.ts`. This module does not repeat the logic: it
//! re-exports it, so the public path of the frozen suite
//! (`tauri/tests/authoritative-resync.test.ts`) is the one of the port.

pub use crate::app_helpers::{Chat, ChatKind, reconcile_authoritative_chats};
