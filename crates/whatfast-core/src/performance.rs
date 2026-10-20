//! Port of `tauri/src/lib/performance.ts` (SolidJS frontend, frozen oracle).
//!
//! The module keeps the three helpers the conversation view needs to stay
//! fast with a long window of messages: a bounded set that remembers what
//! was already tried, a bounded window around the row the user is reading and
//! the reply lookups of the loaded messages.
//!
//! port: the module also carries the two helpers of
//! `tauri/src/lib/participant-roster.ts` that the frozen suite of this module
//! asserts (`participantRosterRows` and `participantDomBudget`). They live
//! here because the suite that covers them is
//! `tauri/tests/performance.test.ts`, the only suite of the oracle that
//! touches them.

use std::collections::HashMap;

// port: the model the original imports from `tauri/src/lib/types.ts` lives in
// [`crate::types`], the single home of the shared DTOs of the app. This module
// re-exports the one its public API mentions.
pub use crate::types::Message;

/// The height of a row of the roster, in pixels.
pub const PARTICIPANT_ROW_HEIGHT: u32 = 54;

/// The height of the header of a section of the roster, in pixels.
pub const PARTICIPANT_HEADER_HEIGHT: u32 = 30;

/// The number of rows the roster mounts above and below the viewport.
pub const PARTICIPANT_OVERSCAN: u32 = 6;

/// Insertion ordered set with a hard size ceiling.
///
/// Re-adding a key refreshes its recency, and the evicted key is returned so
/// callers can prune any parallel reactive state at the same time.
///
/// port: the original throws a `RangeError` when the capacity is not a
/// positive integer; the port cannot throw, so [`BoundedSet::new`] returns
/// `None` for a capacity of zero. A `usize` cannot be fractional, so the
/// "not an integer" half of the original check has no representation here.
pub struct BoundedSet<T> {
    values: Vec<T>,
    capacity: usize,
}

impl<T: PartialEq> BoundedSet<T> {
    /// A set that holds at most `capacity` values.
    pub fn new(capacity: usize) -> Option<Self> {
        if capacity < 1 {
            return None;
        }
        Some(BoundedSet {
            values: Vec::new(),
            capacity,
        })
    }

    /// The number of values the set holds.
    pub fn size(&self) -> usize {
        self.values.len()
    }

    /// The ceiling of the set.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Whether the set holds the value.
    pub fn has(&self, value: &T) -> bool {
        self.values.contains(value)
    }

    /// Add a value, refreshing its recency, and return the evicted one.
    pub fn add(&mut self, value: T) -> Option<T> {
        if let Some(index) = self.values.iter().position(|item| item == &value) {
            self.values.remove(index);
        }
        self.values.push(value);
        if self.values.len() <= self.capacity {
            return None;
        }
        let oldest = self.values.remove(0);
        Some(oldest)
    }

    /// Remove a value, and report whether it was there.
    pub fn delete(&mut self, value: &T) -> bool {
        match self.values.iter().position(|item| item == value) {
            Some(index) => {
                self.values.remove(index);
                true
            }
            None => false,
        }
    }

    /// Remove every value.
    pub fn clear(&mut self) {
        self.values.clear();
    }
}

/// The reply and quote lookups of a window of messages.
#[derive(Clone, Debug, PartialEq)]
pub struct MessageIndex {
    /// Every message of the window, by id.
    pub by_id: HashMap<String, Message>,
    /// How many messages of the window reply to each message.
    pub reply_count_by_id: HashMap<String, u32>,
    /// The first message of the window that replies to each message.
    pub first_reply_id_by_id: HashMap<String, String>,
}

/// A bounded slice of a window of messages.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoundedWindow<T> {
    pub items: Vec<T>,
    pub dropped_before: bool,
    pub dropped_after: bool,
}

/// Keep a bounded slice around an important row.
///
/// Canonical conversation refetches use this instead of blindly retaining
/// every previously visited search/pin window, while still guaranteeing that
/// the requested anchor is present after trimming.
///
/// port: the original throws a `RangeError` when the capacity is not a
/// positive integer; the port returns `None` for a capacity of zero.
pub fn bound_window_around<T: Clone>(
    items: &[T],
    capacity: usize,
    anchor_index: usize,
) -> Option<BoundedWindow<T>> {
    if capacity < 1 {
        return None;
    }
    if items.len() <= capacity {
        return Some(BoundedWindow {
            items: items.to_vec(),
            dropped_before: false,
            dropped_after: false,
        });
    }

    let last = items.len() as i64 - 1;
    let anchor = (anchor_index as i64).min(last).max(0);
    let preferred_start = anchor - (capacity as i64) / 2;
    let start = preferred_start
        .min(items.len() as i64 - capacity as i64)
        .max(0);
    let end = start + capacity as i64;
    Some(BoundedWindow {
        items: items[start as usize..end as usize].to_vec(),
        dropped_before: start > 0,
        dropped_after: end < items.len() as i64,
    })
}

/// Build all reply/quote lookups in one linear pass per message-window update.
///
/// Visible bubbles can then render in O(1) instead of each scanning as many
/// as 2,000 loaded messages for a quote, a reply count, and a first reply.
pub fn index_messages(messages: &[Message]) -> MessageIndex {
    let mut by_id: HashMap<String, Message> = HashMap::new();
    let mut reply_count_by_id: HashMap<String, u32> = HashMap::new();
    let mut first_reply_id_by_id: HashMap<String, String> = HashMap::new();

    for message in messages {
        by_id.insert(message.id.clone(), message.clone());
        if message.reply_to_message_id.is_empty() {
            continue;
        }
        // A private reply quotes a message from its source group, not another
        // message in the direct conversation currently being indexed.
        if !message.reply_to_chat_id.is_empty() && message.reply_to_chat_id != message.chat_id {
            continue;
        }
        let count = reply_count_by_id
            .entry(message.reply_to_message_id.clone())
            .or_insert(0);
        *count += 1;
        first_reply_id_by_id
            .entry(message.reply_to_message_id.clone())
            .or_insert_with(|| message.id.clone());
    }

    MessageIndex {
        by_id,
        reply_count_by_id,
        first_reply_id_by_id,
    }
}

/// A member of the roster of a group.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatParticipant {
    pub participant_id: String,
    pub display_name: String,
    pub phone_number: String,
    pub is_admin: bool,
    pub is_super_admin: bool,
    pub is_me: bool,
}

/// A row of the roster of a group.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParticipantRosterRow {
    /// The header of a section of the roster.
    Header { key: String, label: String },
    /// A member of the roster.
    Participant {
        key: String,
        participant: ChatParticipant,
        position: u32,
    },
}

/// The rows of the roster of a group, with the sections when asked for.
pub fn participant_roster_rows(
    participants: &[ChatParticipant],
    sectioned: bool,
) -> Vec<ParticipantRosterRow> {
    if !sectioned {
        return participants
            .iter()
            .enumerate()
            .map(|(index, participant)| ParticipantRosterRow::Participant {
                key: participant.participant_id.clone(),
                participant: participant.clone(),
                position: index as u32 + 1,
            })
            .collect();
    }
    let admins: Vec<&ChatParticipant> = participants
        .iter()
        .filter(|participant| participant.is_admin || participant.is_super_admin)
        .collect();
    let members: Vec<&ChatParticipant> = participants
        .iter()
        .filter(|participant| !participant.is_admin && !participant.is_super_admin)
        .collect();
    let mut result: Vec<ParticipantRosterRow> = Vec::new();
    let mut position: u32 = 0;
    if !admins.is_empty() {
        result.push(ParticipantRosterRow::Header {
            key: "admins".to_string(),
            label: format!("ADMINS \u{2014} {}", admins.len()),
        });
        for participant in admins {
            position += 1;
            result.push(ParticipantRosterRow::Participant {
                key: participant.participant_id.clone(),
                participant: participant.clone(),
                position,
            });
        }
    }
    if !members.is_empty() {
        result.push(ParticipantRosterRow::Header {
            key: "members".to_string(),
            label: format!("MEMBERS \u{2014} {}", members.len()),
        });
        for participant in members {
            position += 1;
            result.push(ParticipantRosterRow::Participant {
                key: participant.participant_id.clone(),
                participant: participant.clone(),
                position,
            });
        }
    }
    result
}

/// Upper bound used by the performance regression harness.
pub fn participant_dom_budget(viewport_height: f64) -> f64 {
    participant_dom_budget_with_scale(viewport_height, 1.0)
}

/// Upper bound used by the performance regression harness, with a scale.
pub fn participant_dom_budget_with_scale(viewport_height: f64, scale: f64) -> f64 {
    (viewport_height / (f64::from(PARTICIPANT_ROW_HEIGHT) * scale)).ceil()
        + f64::from(PARTICIPANT_OVERSCAN * 2)
        + 2.0
}
