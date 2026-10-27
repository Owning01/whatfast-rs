//! What an agent may do, when it may do it, and how it reaches the user.
//!
//! Four pieces, each one answering a question the rest of the application asks:
//!
//! - [`grant`]: the scope of one agent (`read`, `draft`, `send`), the chats it
//!   covers and the hours it works in. The permission is per agent **and** per
//!   chat, never global.
//! - [`db`]: `agents.db`, the shared database of the profile. Agents, their
//!   token hash, the drafts waiting for the user, the schedules and the log of
//!   every call.
//! - [`humanize`]: the pace. A message that leaves the instant the model
//!   finishes looks like a bot; the plan is a list of parts and the wait before
//!   each one.
//! - [`cron`]: the five fields of a schedule, and where a task that comes due
//!   outside the window waits for the next one.
//!
//! Nothing here touches the network, the interface or the session: the sidecar
//! owns the database and asks these types what to allow.

pub mod ask;
pub mod cron;
pub mod db;
pub mod grant;
pub mod humanize;
pub mod local;
pub mod runner;

pub use ask::{QUESTION_TIMEOUT, question_prompt};
pub use cron::{Cron, defer_to_window, next_at};
pub use db::{AgentRow, Call, Db, Draft, DraftState, Kind, Output, Schedule};
pub use grant::{ChatFilter, Grant, Scope, Window};
pub use humanize::{Jitter, Pace, Plan, plan, split, typing_ms};
pub use local::{offset_minutes, offset_now};
pub use runner::{Answer, RunError};
