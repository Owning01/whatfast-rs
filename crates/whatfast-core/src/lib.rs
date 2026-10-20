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

pub mod whatsapp_formatting;
pub mod media_crop;
pub mod message_format;
pub mod polls;
pub mod chat_merge;
pub mod unsupported;
pub mod file_drop;
pub mod unread;
pub mod pairing;
pub mod workspace;
pub mod app_helpers;
pub mod tab_keyboard;
pub mod notification_policy;
pub mod backend_lifecycle;
pub mod scroll_restoration;
