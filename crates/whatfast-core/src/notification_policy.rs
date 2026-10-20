//! Port of `tauri/src/lib/notification-policy.ts` (SolidJS frontend, frozen oracle).
//!
//! The domain types come from [`crate::app_helpers`]: the original imports
//! them from `lib/types.ts`, but `whatfast-core` has no `types` module, so the
//! shared DTOs live in `app_helpers` and are re-exported here.
//!
//! Fidelity notes (the odd bits are deliberate, they are the oracle's):
//! - `notificationTitle` chains `||`, which is *falsy* based: an empty title
//!   falls through to the sender name, an empty sender name falls through to
//!   the phone number. The port reproduces that explicitly.
//! - `notificationBody` truncates at 180 characters and slices at 179. The
//!   strings of JS are UTF-16, so the length and the slice are counted in
//!   UTF-16 code units here; the slice never cuts a surrogate pair in half
//!   (Rust strings cannot hold a lone surrogate).

pub use crate::app_helpers::{Chat, Message, MessageContent};
use crate::workspace::Pane;

/// The facts the notification policy decides on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NotificationPolicyInput {
    pub enabled: bool,
    pub visible: bool,
    pub chat_visible: bool,
    pub muted: bool,
    pub incoming: bool,
}

/// Whether an incoming message deserves a notification.
pub fn should_notify(input: &NotificationPolicyInput) -> bool {
    input.enabled && input.incoming && !input.muted && !(input.visible && input.chat_visible)
}

/// At compact widths CSS renders only the focused split pane. Treating the
/// hidden pane as visible would silently suppress its notifications and mark
/// incoming messages read even though the user cannot see them.
pub fn is_chat_actually_visible(
    panes: &[Pane],
    focused_pane_id: &str,
    chat_id: &str,
    compact_split: bool,
) -> bool {
    panes.iter().any(|pane| {
        pane.active_chat_id == chat_id && (!compact_split || pane.id == focused_pane_id)
    })
}

/// The title of the notification.
///
/// port: the original chains `chat?.title || message.senderName ||
/// chat?.phoneNumber || "New WhatsApp message"`. The `||` operator is falsy
/// based, so every empty string falls through to the next candidate.
pub fn notification_title(chat: Option<&Chat>, message: &Message) -> String {
    chat.and_then(|chat| non_empty(&chat.title))
        .or_else(|| non_empty(&message.sender_name))
        .or_else(|| chat.and_then(|chat| non_empty(&chat.phone_number)))
        .unwrap_or_else(|| "New WhatsApp message".to_string())
}

/// The body of the notification.
pub fn notification_body(message: &Message, show_preview: bool) -> String {
    if !show_preview {
        return "New message".to_string();
    }
    let mut preview = "Message".to_string();
    if message.revoked {
        preview = "This message was deleted".to_string();
    } else {
        match message.content.as_ref() {
            Some(MessageContent::Text(text)) => preview = text.text.clone(),
            Some(MessageContent::Image(image)) => {
                preview = non_empty(&image.caption).unwrap_or_else(|| {
                    if image.sticker {
                        "Sticker".to_string()
                    } else {
                        "Photo".to_string()
                    }
                });
            }
            Some(MessageContent::Attachment(attachment)) => {
                preview = non_empty(&attachment.caption)
                    .or_else(|| non_empty(&attachment.file_name))
                    .or_else(|| non_empty(&attachment.kind))
                    .unwrap_or_else(|| "Attachment".to_string());
            }
            Some(MessageContent::Contacts(contacts)) => {
                preview = if contacts.contacts.len() == 1 {
                    contacts
                        .contacts
                        .first()
                        .and_then(|contact| non_empty(&contact.display_name))
                        .unwrap_or_else(|| "Contact".to_string())
                } else {
                    format!("{} contacts", contacts.contacts.len())
                };
            }
            Some(MessageContent::Location(location)) => {
                preview = non_empty(&location.name)
                    .or_else(|| non_empty(&location.address))
                    .unwrap_or_else(|| "Location".to_string());
            }
            Some(MessageContent::Unsupported(unsupported)) => {
                preview = non_empty(&unsupported.fallback_text)
                    .or_else(|| non_empty(&unsupported.type_name))
                    .unwrap_or_else(|| "Message".to_string());
            }
            // port: the original has no branch for polls, so the preview stays
            // the generic "Message".
            Some(MessageContent::Poll(_)) | None => {}
        }
    }
    let text = collapse_whitespace(&preview);
    if js_utf16_len(&text) > 180 {
        format!("{}…", js_slice_utf16(&text, 179))
    } else if text.is_empty() {
        "New message".to_string()
    } else {
        text
    }
}

/// port: the truthiness of JS — an empty string is falsy.
fn non_empty(value: &str) -> Option<String> {
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

/// port: `preview.replace(/\s+/g, " ").trim()` — every run of whitespace
/// becomes a single space and the ends are trimmed.
fn collapse_whitespace(value: &str) -> String {
    let mut collapsed = String::with_capacity(value.len());
    let mut pending_space = false;
    for character in value.chars() {
        if character.is_whitespace() {
            pending_space = !collapsed.is_empty();
        } else {
            if pending_space {
                collapsed.push(' ');
                pending_space = false;
            }
            collapsed.push(character);
        }
    }
    collapsed
}

/// port: the strings of JS are UTF-16, so `String.prototype.length` counts
/// UTF-16 code units.
fn js_utf16_len(value: &str) -> usize {
    value.encode_utf16().count()
}

/// port: `String.prototype.slice(0, end)` counted in UTF-16 code units. The
/// port never cuts a surrogate pair in half: the character that would be split
/// is left out.
fn js_slice_utf16(value: &str, end: usize) -> String {
    let mut sliced = String::new();
    let mut units = 0usize;
    for character in value.chars() {
        let width = character.len_utf16();
        if units + width > end {
            break;
        }
        sliced.push(character);
        units += width;
    }
    sliced
}
