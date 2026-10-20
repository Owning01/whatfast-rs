//! Port of `tauri/src/state/scroll-restoration.ts` (SolidJS frontend, frozen oracle).
//!
//! A raw scrollTop is not stable when older rows are prepended or media changes
//! height. Persist the first visible message plus its offset from the viewport
//! instead, and resolve that anchor against the next bounded message window.
//!
//! Fidelity notes (the odd bits are deliberate, they are the oracle's):
//! - the viewport geometry is fractional (CSS pixels), so it is `f64` here.
//! - an anchor with an empty message id is dropped: the original checks the
//!   id with the truthiness of JS, where `""` is falsy.

/// The default tolerance that decides whether the viewport follows the latest
/// message.
pub const DEFAULT_LATEST_TOLERANCE: f64 = 80.0;

/// What was visible when the snapshot was taken.
#[derive(Clone, Debug, PartialEq)]
pub struct ScrollSnapshot {
    pub anchor_message_id: String,
    pub anchor_offset: f64,
    pub at_latest: bool,
}

/// One laid out row of the virtualized list.
#[derive(Clone, Debug, PartialEq)]
pub struct VirtualRow {
    pub index: usize,
    pub start: f64,
    pub end: f64,
}

/// Where the viewport has to go.
#[derive(Clone, Debug, PartialEq)]
pub enum ScrollRestoreTarget {
    Anchor { index: usize, offset: f64 },
    Unread { index: usize },
    Latest { index: usize },
    Empty,
}

/// Capture the first visible message and its offset instead of a raw offset.
pub fn capture_scroll_snapshot(
    message_ids: &[String],
    virtual_rows: &[VirtualRow],
    scroll_top: f64,
    scroll_height: f64,
    client_height: f64,
    latest_tolerance: f64,
) -> Option<ScrollSnapshot> {
    if message_ids.is_empty() || virtual_rows.is_empty() {
        return None;
    }
    let first_visible = virtual_rows
        .iter()
        .find(|row| row.end > scroll_top)
        .or_else(|| virtual_rows.first())?;
    let anchor_message_id = message_ids.get(first_visible.index)?;
    // port: `!anchorMessageId` is the truthiness of JS, where `""` is falsy.
    if anchor_message_id.is_empty() {
        return None;
    }
    Some(ScrollSnapshot {
        anchor_message_id: anchor_message_id.clone(),
        anchor_offset: first_visible.start - scroll_top,
        at_latest: scroll_height - scroll_top - client_height <= latest_tolerance,
    })
}

/// Resolve the snapshot against the next bounded message window.
pub fn resolve_scroll_restore(
    message_ids: &[String],
    snapshot: Option<&ScrollSnapshot>,
    first_unread_message_id: &str,
) -> ScrollRestoreTarget {
    if message_ids.is_empty() {
        return ScrollRestoreTarget::Empty;
    }
    if let Some(snapshot) = snapshot {
        if snapshot.at_latest {
            return ScrollRestoreTarget::Latest {
                index: message_ids.len() - 1,
            };
        }
        if let Some(index) = message_ids
            .iter()
            .position(|id| id == &snapshot.anchor_message_id)
        {
            return ScrollRestoreTarget::Anchor {
                index,
                offset: snapshot.anchor_offset,
            };
        }
    }
    // port: `firstUnreadMessageId ? ... : -1` — an empty id is falsy in JS.
    let unread_index = if first_unread_message_id.is_empty() {
        None
    } else {
        message_ids
            .iter()
            .position(|id| id == first_unread_message_id)
    };
    if let Some(index) = unread_index {
        return ScrollRestoreTarget::Unread { index };
    }
    ScrollRestoreTarget::Latest {
        index: message_ids.len() - 1,
    }
}
