//! Pure logic ported from the Whatfast SolidJS frontend.
//!
//! Each module is a 1:1 port of one TypeScript module of the same name in
//! `tauri/src/`. The TypeScript source and its tests are the frozen oracle:
//! see `docs/oracle.md` in the migration plan.
//!
//! Rules for every module here (see `RULEBOOK.md`):
//! - no `unwrap`, no `expect`, no `panic!` outside `#[cfg(test)]`
//! - public API is snake_case; no panics, errors are `Option`/`Result`
//! - the port keeps the TypeScript semantics, including the quirks

pub mod app_helpers;
pub mod backend_lifecycle;
pub mod chat_merge;
pub mod file_drop;
pub mod media_crop;
pub mod message_format;
pub mod notification_policy;
pub mod notification_routing;
pub mod pairing;
pub mod participant_avatar_queue;
pub mod polls;
pub mod scroll_restoration;
pub mod spotlight;
pub mod tab_keyboard;
pub mod theme;
pub mod types;
pub mod unread;
pub mod unsupported;
pub mod whatsapp_formatting;
pub mod workspace;
