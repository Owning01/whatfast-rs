//! Port of `tauri/src/state/app-helpers.ts` (SolidJS frontend, frozen oracle).
//!
//! This module owns the chat/message domain types that the frontend shares
//! between `state/app-helpers.ts` and `lib/notification-policy.ts`: the
//! original imports them from `lib/types.ts`, but `whatfast-core` has no
//! `types` module and `lib.rs` is owned by the orchestrator, so the shared
//! DTOs live here and `notification_policy` re-exports them.
//!
//! Fidelity notes (the odd bits are deliberate, they are the oracle's):
//! - `sortChats` / `compareMessages` break ties with `String.prototype.localeCompare`,
//!   which is ICU collation, not byte order. [`locale_compare`] reproduces the
//!   two levels that matter for ids (case-insensitive primary, lowercase before
//!   uppercase tertiary).
//! - `mergeChats` merges with the object-spread `{...previous, ...incoming}`:
//!   the incoming chat wins field by field and the previous one only supplies
//!   the fields the incoming chat does not carry.
//! - `trimMessages` returns the very same array when the window is already
//!   under the cap; the port returns a copy because this crate owns its data.
//! - `readRecentChats` / `writeRecentChats` / `rememberRecentChat` touch
//!   `sessionStorage`. The pure part (parse, serialize, recompute) lives here;
//!   the storage effect belongs to the UI crate.

use std::cmp::Ordering;
use std::collections::HashMap;

/// The chat list keeps at most this many messages hydrated per conversation.
pub const MAX_ACTIVE_MESSAGES: usize = 250;

/// The session history of recent chats is capped at this many entries.
///
/// port: the original hardcodes `10` inside `rememberRecentChat`
/// (`slice(0, 10)`) instead of importing a shared constant.
pub const MAX_RECENT_CHATS: usize = 10;

/// `sessionStorage` key holding the recent chat ids.
pub const RECENT_CHATS_STORAGE_KEY: &str = "whatfast-recent";

// --- domain types -----------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ChatKind {
    #[default]
    Unspecified,
    Direct,
    Group,
    Other,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MessageStatus {
    #[default]
    Unspecified,
    Pending,
    Sent,
    Delivered,
    Read,
    Failed,
}

/// A conversation of the sidebar.
#[derive(Clone, Debug, PartialEq)]
pub struct Chat {
    /// Opaque local conversation ID, not a WhatsApp JID.
    pub id: String,
    pub kind: ChatKind,
    pub title: String,
    pub avatar_path: String,
    pub last_message_preview: String,
    pub last_message_timestamp_ms: i64,
    pub unread_count: u32,
    pub muted: bool,
    pub pinned: bool,
    pub archived: bool,
    /// Local-only favorite flag (linked-device scope has no favorites RPC).
    pub favorite: Option<bool>,
    /// Local-only block flag mirrored from the browser mock / optimistic state.
    pub blocked: Option<bool>,
    /// Local-only mute expiry; `None` means muted indefinitely or not muted.
    pub mute_until_ms: Option<i64>,
    pub phone_number: String,
    pub contact_name: String,
    pub push_name: String,
    pub business_name: String,
    pub last_message_from_me: bool,
    pub transport_jid: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LinkPreview {
    pub url: String,
    pub title: String,
    pub description: String,
    pub jpeg_thumbnail: Vec<u8>,
    pub thumbnail_width: u32,
    pub thumbnail_height: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextContent {
    pub text: String,
    pub link_preview: Option<LinkPreview>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ImageContent {
    pub caption: String,
    pub mime_type: String,
    pub local_path: String,
    pub width: u32,
    pub height: u32,
    pub file_size: u64,
    pub downloadable: bool,
    pub sticker: bool,
    pub animated: bool,
    pub thumbnail_path: String,
    pub jpeg_thumbnail: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AttachmentContent {
    pub kind: String,
    pub caption: String,
    pub mime_type: String,
    pub file_name: String,
    pub local_path: String,
    pub file_size: u64,
    pub width: u32,
    pub height: u32,
    pub duration_seconds: f64,
    pub animated: bool,
    pub voice_note: bool,
    pub downloadable: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ContactContent {
    pub display_name: String,
    pub vcard: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ContactsContent {
    pub contacts: Vec<ContactContent>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LocationContent {
    pub latitude: f64,
    pub longitude: f64,
    pub name: String,
    pub address: String,
    pub url: String,
    pub live: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct UnsupportedContent {
    pub type_name: String,
    pub fallback_text: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PollVoter {
    pub user_id: String,
    pub display_name: String,
    pub avatar_path: String,
    pub from_me: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PollOption {
    pub name: String,
    pub vote_count: u32,
    pub selected_by_me: bool,
    pub voters: Vec<PollVoter>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PollContent {
    pub question: String,
    pub options: Vec<PollOption>,
    pub selectable_options_count: u32,
    pub total_voters: u32,
}

/// The oneof of the message payload: exactly one variant is present.
#[derive(Clone, Debug, PartialEq)]
pub enum MessageContent {
    Text(TextContent),
    Unsupported(UnsupportedContent),
    Image(ImageContent),
    Attachment(AttachmentContent),
    Contacts(ContactsContent),
    Location(LocationContent),
    Poll(PollContent),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Reaction {
    pub chat_id: String,
    pub message_id: String,
    pub sender_id: String,
    pub emoji: String,
    pub timestamp_ms: i64,
    pub from_me: bool,
    pub sender_name: String,
    pub sender_phone_number: String,
    pub sender_avatar_path: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Message {
    pub id: String,
    pub chat_id: String,
    pub sender_id: String,
    pub sender_name: String,
    pub from_me: bool,
    pub timestamp_ms: i64,
    pub status: MessageStatus,
    pub edited: bool,
    pub revoked: bool,
    pub expires_at_ms: i64,
    pub sender_phone_number: String,
    pub sender_avatar_path: String,
    pub reactions: Vec<Reaction>,
    pub reply_to_message_id: String,
    pub reply_to_chat_id: String,
    pub forwarding_score: u32,
    pub mention_texts: Vec<String>,
    pub content: Option<MessageContent>,
}

/// A mention of the composer (`state/app.ts`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Mention {
    pub display_name: String,
    pub jid: String,
}

/// The composer draft (`state/app.ts`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Draft {
    pub text: String,
    pub reply_to_message_id: String,
    pub reply_to_chat_id: String,
    pub reply_preview_text: String,
    pub reply_sender_name: String,
    pub editing_message_id: String,
    pub mentions: Vec<Mention>,
}

/// The result of [`encode_mentions`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EncodedMentions {
    pub text: String,
    pub jids: Vec<String>,
}

/// Which end of the window [`trim_messages`] drops.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropSide {
    Older,
    Newer,
}

// --- chat list --------------------------------------------------------------

/// Sort the sidebar: pinned first, then newest activity, then id.
///
/// port: `Number(right.pinned) - Number(left.pinned)` puts the pinned chat
/// first; the timestamps are compared descending and the id breaks the tie
/// with `localeCompare`.
pub fn sort_chats(chats: &[Chat]) -> Vec<Chat> {
    let mut sorted = chats.to_vec();
    sorted.sort_by(|left, right| {
        let pinned = u8::from(right.pinned).cmp(&u8::from(left.pinned));
        if pinned != Ordering::Equal {
            return pinned;
        }
        let timestamp = right
            .last_message_timestamp_ms
            .cmp(&left.last_message_timestamp_ms);
        if timestamp != Ordering::Equal {
            return timestamp;
        }
        locale_compare(&left.id, &right.id)
    });
    sorted
}

/// Merge incoming chats over the ones already known, keyed by id.
///
/// port: the original builds a `Map` and then spreads `{...previous, ...incoming}`
/// over the previous value, so the incoming chat wins field by field and the
/// previous one only supplies the fields the incoming chat does not carry.
/// The iteration order of the map (first insertion wins) is kept.
pub fn merge_chats(existing: &[Chat], incoming: &[Chat]) -> Vec<Chat> {
    let mut order: Vec<String> = Vec::new();
    let mut by_id: HashMap<String, Chat> = HashMap::new();
    for chat in existing {
        if !by_id.contains_key(&chat.id) {
            order.push(chat.id.clone());
        }
        by_id.insert(chat.id.clone(), chat.clone());
    }
    for chat in incoming {
        let merged = match by_id.get(&chat.id) {
            Some(previous) => merge_chat(previous, chat),
            None => chat.clone(),
        };
        if !by_id.contains_key(&chat.id) {
            order.push(chat.id.clone());
        }
        by_id.insert(chat.id.clone(), merged);
    }
    order.iter().filter_map(|id| by_id.remove(id)).collect()
}

/// The object-spread of two chats: `incoming` wins, `previous` fills the gaps.
///
/// port: the three optional fields are `Option` here, and `None` means "the
/// incoming chat does not carry this field" — the case the bridge really
/// produces, because the backend omits those keys instead of sending `null`.
fn merge_chat(previous: &Chat, incoming: &Chat) -> Chat {
    Chat {
        id: incoming.id.clone(),
        kind: incoming.kind,
        title: incoming.title.clone(),
        avatar_path: incoming.avatar_path.clone(),
        last_message_preview: incoming.last_message_preview.clone(),
        last_message_timestamp_ms: incoming.last_message_timestamp_ms,
        unread_count: incoming.unread_count,
        muted: incoming.muted,
        pinned: incoming.pinned,
        archived: incoming.archived,
        favorite: incoming.favorite.or(previous.favorite),
        blocked: incoming.blocked.or(previous.blocked),
        mute_until_ms: incoming.mute_until_ms.or(previous.mute_until_ms),
        phone_number: incoming.phone_number.clone(),
        contact_name: incoming.contact_name.clone(),
        push_name: incoming.push_name.clone(),
        business_name: incoming.business_name.clone(),
        last_message_from_me: incoming.last_message_from_me,
        transport_jid: incoming.transport_jid.clone(),
    }
}

/// Keep the local copies of the chats whose revision moved while the
/// authoritative list was being fetched.
pub fn reconcile_authoritative_chats(
    authoritative: &[Chat],
    current: &[Chat],
    revisions_before: &HashMap<String, u32>,
    revisions_after: &HashMap<String, u32>,
) -> Vec<Chat> {
    let raced: Vec<Chat> = current
        .iter()
        .filter(|chat| {
            revision_of(revisions_after, &chat.id) > revision_of(revisions_before, &chat.id)
        })
        .cloned()
        .collect();
    sort_chats(&merge_chats(authoritative, &raced))
}

fn revision_of(revisions: &HashMap<String, u32>, chat_id: &str) -> u32 {
    revisions.get(chat_id).copied().unwrap_or(0)
}

// --- messages ---------------------------------------------------------------

/// Sort messages by timestamp and then by id.
pub fn sort_messages(messages: &[Message]) -> Vec<Message> {
    let mut sorted = messages.to_vec();
    sorted.sort_by(compare_messages);
    sorted
}

/// port: `left.timestampMs - right.timestampMs || left.id.localeCompare(right.id)`
fn compare_messages(left: &Message, right: &Message) -> Ordering {
    left.timestamp_ms
        .cmp(&right.timestamp_ms)
        .then_with(|| locale_compare(&left.id, &right.id))
}

/// Merge incoming messages over the ones already known, keyed by id, and keep
/// the window sorted.
pub fn merge_messages(existing: &[Message], incoming: &[Message]) -> Vec<Message> {
    let mut order: Vec<String> = Vec::new();
    let mut by_id: HashMap<String, Message> = HashMap::new();
    for message in existing {
        if !by_id.contains_key(&message.id) {
            order.push(message.id.clone());
        }
        by_id.insert(message.id.clone(), message.clone());
    }
    for message in incoming {
        let merged = preserve_local_media(message, by_id.get(&message.id));
        if !by_id.contains_key(&message.id) {
            order.push(message.id.clone());
        }
        by_id.insert(message.id.clone(), merged);
    }
    let merged: Vec<Message> = order.iter().filter_map(|id| by_id.remove(id)).collect();
    sort_messages(&merged)
}

/// Keep the local media paths the backend does not know about.
///
/// port: the original only merges image-to-image and attachment-to-attachment;
/// a text message over an image message drops the image.
fn preserve_local_media(message: &Message, previous: Option<&Message>) -> Message {
    let (Some(previous), Some(current)) = (previous, message.content.as_ref()) else {
        return message.clone();
    };
    let Some(previous_content) = previous.content.as_ref() else {
        return message.clone();
    };
    match (previous_content, current) {
        (MessageContent::Image(previous_image), MessageContent::Image(current_image)) => {
            let mut image = current_image.clone();
            if image.local_path.is_empty() {
                image.local_path = previous_image.local_path.clone();
            }
            if image.thumbnail_path.is_empty() {
                image.thumbnail_path = previous_image.thumbnail_path.clone();
            }
            if image.jpeg_thumbnail.is_empty() {
                image.jpeg_thumbnail = previous_image.jpeg_thumbnail.clone();
            }
            Message {
                content: Some(MessageContent::Image(image)),
                ..message.clone()
            }
        }
        (
            MessageContent::Attachment(previous_attachment),
            MessageContent::Attachment(current_attachment),
        ) => {
            let mut attachment = current_attachment.clone();
            if attachment.local_path.is_empty() {
                attachment.local_path = previous_attachment.local_path.clone();
            }
            Message {
                content: Some(MessageContent::Attachment(attachment)),
                ..message.clone()
            }
        }
        _ => message.clone(),
    }
}

/// Fast path for the common one-message live event.
///
/// port: the original takes an optional `existingIndex` that defaults to
/// `existing.findIndex(...)`; [`upsert_sorted_message_with_index`] keeps that
/// parameter, `None` meaning "look it up".
pub fn upsert_sorted_message(existing: &[Message], incoming: &Message) -> Vec<Message> {
    upsert_sorted_message_with_index(existing, incoming, None)
}

/// [`upsert_sorted_message`] with an explicit slot to replace.
pub fn upsert_sorted_message_with_index(
    existing: &[Message],
    incoming: &Message,
    existing_index: Option<usize>,
) -> Vec<Message> {
    let existing_index = existing_index.or_else(|| {
        existing
            .iter()
            .position(|message| message.id == incoming.id)
    });
    let message = preserve_local_media(
        incoming,
        existing_index.and_then(|index| existing.get(index)),
    );
    if let Some(index) = existing_index {
        let mut next = existing.to_vec();
        next[index] = message.clone();
        let previous = if index > 0 { next.get(index - 1) } else { None };
        let following = next.get(index + 1);
        let out_of_order = previous
            .is_some_and(|previous| compare_messages(previous, &message).is_gt())
            || following.is_some_and(|following| compare_messages(&message, following).is_gt());
        if out_of_order {
            next.sort_by(compare_messages);
        }
        return next;
    }

    let appends = match existing.last() {
        None => true,
        Some(last) => compare_messages(last, &message).is_le(),
    };
    if appends {
        let mut next = existing.to_vec();
        next.push(message);
        return next;
    }

    let mut low = 0usize;
    let mut high = existing.len();
    while low < high {
        let middle = (low + high) / 2;
        if compare_messages(&existing[middle], &message).is_le() {
            low = middle + 1;
        } else {
            high = middle;
        }
    }
    let mut next = Vec::with_capacity(existing.len() + 1);
    next.extend_from_slice(&existing[..low]);
    next.push(message);
    next.extend_from_slice(&existing[low..]);
    next
}

/// Keep the window at most [`MAX_ACTIVE_MESSAGES`] long.
pub fn trim_messages(messages: &[Message], drop: DropSide) -> Vec<Message> {
    if messages.len() <= MAX_ACTIVE_MESSAGES {
        return messages.to_vec();
    }
    match drop {
        DropSide::Older => messages[messages.len() - MAX_ACTIVE_MESSAGES..].to_vec(),
        DropSide::Newer => messages[..MAX_ACTIVE_MESSAGES].to_vec(),
    }
}

// --- composer ---------------------------------------------------------------

/// Replace the visible mention names with the bare JID user.
pub fn encode_mentions(text: &str, mentions: &[Mention]) -> EncodedMentions {
    let mut encoded = text.to_string();
    let mut jids: Vec<String> = Vec::new();
    for mention in mentions {
        let visible = format!("@{}", mention.display_name);
        if !encoded.contains(&visible) {
            continue;
        }
        let user = mention
            .jid
            .split('@')
            .next()
            .unwrap_or(mention.jid.as_str());
        encoded = encoded.replace(&visible, &format!("@{user}"));
        jids.push(mention.jid.clone());
    }
    EncodedMentions {
        text: encoded,
        jids,
    }
}

/// The key the media cache uses for one message of one chat.
pub fn media_key(chat_id: &str, message_id: &str) -> String {
    format!("{chat_id}\u{0}{message_id}")
}

/// Clone a draft and its mentions.
pub fn clone_draft(draft: &Draft) -> Draft {
    draft.clone()
}

/// Whether the composer holds nothing worth sending.
pub fn draft_is_empty(draft: Option<&Draft>) -> bool {
    match draft {
        None => false,
        Some(draft) => {
            draft.text.is_empty()
                && draft.reply_to_message_id.is_empty()
                && draft.reply_to_chat_id.is_empty()
                && draft.reply_preview_text.is_empty()
                && draft.reply_sender_name.is_empty()
                && draft.editing_message_id.is_empty()
                && draft.mentions.is_empty()
        }
    }
}

// --- recent chats -----------------------------------------------------------

/// Read the recent chat ids out of the raw payload stored in `sessionStorage`.
///
/// port: `None` is the "key missing" case (the original parses `"[]"` then),
/// a broken payload yields an empty list and non-string entries are dropped.
pub fn parse_recent_chats(raw: Option<&str>) -> Vec<String> {
    let Some(raw) = raw else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) else {
        return Vec::new();
    };
    let Some(items) = value.as_array() else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| item.as_str().map(str::to_string))
        .collect()
}

/// Serialize the recent chat ids for `sessionStorage`.
pub fn serialize_recent_chats(ids: &[String]) -> String {
    // port: `JSON.stringify` of an array of strings never throws.
    serde_json::to_string(ids).unwrap_or_default()
}

/// Move a chat to the front of the session history.
///
/// port: the original writes the result back to `sessionStorage`; the storage
/// effect belongs to the UI crate, the decision does not.
pub fn remember_recent_chat(chat_id: &str, current: &[String]) -> Vec<String> {
    let mut next = vec![chat_id.to_string()];
    next.extend(current.iter().filter(|id| id.as_str() != chat_id).cloned());
    next.truncate(MAX_RECENT_CHATS);
    next
}

// --- helpers ----------------------------------------------------------------

/// port: `String.prototype.localeCompare` uses the ICU collation of the
/// runtime, which is not byte order: `"a"` sorts before `"A"` and before `"B"`.
/// The port reproduces the two levels that matter for ids (case-insensitive
/// primary weight, lowercase before uppercase as the tertiary weight) and
/// falls back to the byte order for the rest.
fn locale_compare(left: &str, right: &str) -> Ordering {
    let left_primary = left.to_lowercase();
    let right_primary = right.to_lowercase();
    match left_primary.cmp(&right_primary) {
        Ordering::Equal => {
            for (left_char, right_char) in left.chars().zip(right.chars()) {
                let left_lower = left_char.is_lowercase();
                let right_lower = right_char.is_lowercase();
                if left_lower != right_lower {
                    return if left_lower {
                        Ordering::Less
                    } else {
                        Ordering::Greater
                    };
                }
            }
            left.cmp(right)
        }
        other => other,
    }
}
