//! Port of `tauri/src/lib/send-idempotency.ts` (SolidJS frontend, frozen oracle).
//!
//! The original keeps the client generated id of the last unresolved send of
//! every chat. An exact retry reuses that id; a changed payload, or a send
//! that already succeeded, is a new logical operation and gets a fresh id.
//!
//! port: three things the original takes from the runtime cannot be ported
//! as they are, and the port injects them instead:
//!
//! - `createId` defaults to `crypto.randomUUID()`. The crate has no random
//!   source (a new dependency is not allowed), so the factory is mandatory:
//!   [`SendIdempotency::new`] takes it.
//! - `run` is `async` and the `send` callback returns a promise. The crate is
//!   synchronous (RULEBOOK 1.4), so the callback returns a `Result` and the
//!   error travels out of [`SendIdempotency::run`].
//! - the payload is a positional tuple and the fingerprint is
//!   `JSON.stringify(payload)`. The port keeps the tuple as an enum of named
//!   fields and builds the very same JSON text, so two payloads share a
//!   fingerprint exactly when the original shares one.

use std::collections::HashMap;

/// The payload of a send: the positional tuple the bridge sends.
///
/// port: the original declares the payloads as readonly tuples. The fields
/// below are the ones of those tuples, in the same order:
///
/// - `["text", text, replyToMessageId, replyToChatId, mentionedJids]`
/// - `["image", imagePath, caption, replyToMessageId, replyToChatId]`
/// - `["sticker", imagePath, replyToMessageId, replyToChatId]`
/// - `["attachment", filePath, kind, caption, replyToMessageId, replyToChatId, voiceNote]`
/// - `["poll", question, options, selectableOptionsCount]`
/// - `["forward", sourceChatId, messageId, targetChatId]`
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SendPayload {
    /// `["text", text, reply_to_message_id, reply_to_chat_id, mentioned_jids]`
    Text {
        text: String,
        reply_to_message_id: String,
        reply_to_chat_id: String,
        mentioned_jids: Vec<String>,
    },
    /// `["image", image_path, caption, reply_to_message_id, reply_to_chat_id]`
    Image {
        image_path: String,
        caption: String,
        reply_to_message_id: String,
        reply_to_chat_id: String,
    },
    /// `["sticker", image_path, reply_to_message_id, reply_to_chat_id]`
    Sticker {
        image_path: String,
        reply_to_message_id: String,
        reply_to_chat_id: String,
    },
    /// `["attachment", file_path, kind, caption, reply_to_message_id,
    /// reply_to_chat_id, voice_note]`
    Attachment {
        file_path: String,
        /// port: the wire value of `AttachmentKind` (`0` unspecified,
        /// `1` document, `2` video, `3` audio).
        kind: u32,
        caption: String,
        reply_to_message_id: String,
        reply_to_chat_id: String,
        voice_note: bool,
    },
    /// `["poll", question, options, selectable_options_count]`
    Poll {
        question: String,
        options: Vec<String>,
        selectable_options_count: u32,
    },
    /// `["forward", source_chat_id, message_id, target_chat_id]`
    Forward {
        source_chat_id: String,
        message_id: String,
        target_chat_id: String,
    },
}

/// The id of a send that is still unresolved.
#[derive(Clone, Debug, PartialEq, Eq)]
struct SendAttempt {
    client_message_id: String,
    fingerprint: String,
}

/// Keeps the client generated id of the latest unresolved send of every chat.
pub struct SendIdempotency {
    pending_by_chat: HashMap<String, SendAttempt>,
    create_id: Box<dyn FnMut() -> String>,
}

impl SendIdempotency {
    /// A registry that mints the ids with the given factory.
    ///
    /// port: the original defaults the factory to `crypto.randomUUID()`.
    pub fn new(create_id: impl FnMut() -> String + 'static) -> Self {
        SendIdempotency {
            pending_by_chat: HashMap::new(),
            create_id: Box::new(create_id),
        }
    }

    /// Send the payload of a chat, reusing the id of the last unresolved send
    /// when the payload did not change.
    ///
    /// The error of the send travels out untouched, exactly like the original
    /// rethrows it.
    pub fn run<T, E>(
        &mut self,
        chat_id: &str,
        payload: &SendPayload,
        send: impl FnOnce(&str) -> Result<T, E>,
    ) -> Result<T, E> {
        let fingerprint = payload.fingerprint();
        let pending = self
            .pending_by_chat
            .get(chat_id)
            .filter(|attempt| attempt.fingerprint == fingerprint)
            .cloned();
        let attempt = match pending {
            Some(attempt) => attempt,
            None => SendAttempt {
                client_message_id: (self.create_id)(),
                fingerprint,
            },
        };

        // Mark the operation before invoking native IPC. This also makes two
        // overlapping calls for the same logical payload share one request ID.
        self.pending_by_chat
            .insert(chat_id.to_string(), attempt.clone());
        match send(&attempt.client_message_id) {
            Ok(result) => {
                if self.pending_by_chat.get(chat_id) == Some(&attempt) {
                    self.pending_by_chat.remove(chat_id);
                }
                Ok(result)
            }
            Err(error) => {
                // Keep the attempt only while it is still the latest operation
                // for the chat. A later, changed send must not be replaced by
                // an older failure.
                if self.pending_by_chat.get(chat_id) == Some(&attempt) {
                    self.pending_by_chat.insert(chat_id.to_string(), attempt);
                }
                Err(error)
            }
        }
    }
}

impl SendPayload {
    /// The fingerprint of the payload: `JSON.stringify(payload)`.
    ///
    /// port: the original serializes the tuple, so the text of the fingerprint
    /// is the JSON text of the tuple. The port writes the same text.
    pub fn fingerprint(&self) -> String {
        match self {
            SendPayload::Text {
                text,
                reply_to_message_id,
                reply_to_chat_id,
                mentioned_jids,
            } => format!(
                "[\"text\",{},{},{},[{}]]",
                json_string(text),
                json_string(reply_to_message_id),
                json_string(reply_to_chat_id),
                json_string_array(mentioned_jids),
            ),
            SendPayload::Image {
                image_path,
                caption,
                reply_to_message_id,
                reply_to_chat_id,
            } => format!(
                "[\"image\",{},{},{},{}]",
                json_string(image_path),
                json_string(caption),
                json_string(reply_to_message_id),
                json_string(reply_to_chat_id),
            ),
            SendPayload::Sticker {
                image_path,
                reply_to_message_id,
                reply_to_chat_id,
            } => format!(
                "[\"sticker\",{},{},{}]",
                json_string(image_path),
                json_string(reply_to_message_id),
                json_string(reply_to_chat_id),
            ),
            SendPayload::Attachment {
                file_path,
                kind,
                caption,
                reply_to_message_id,
                reply_to_chat_id,
                voice_note,
            } => format!(
                "[\"attachment\",{},{},{},{},{},{}]",
                json_string(file_path),
                kind,
                json_string(caption),
                json_string(reply_to_message_id),
                json_string(reply_to_chat_id),
                voice_note,
            ),
            SendPayload::Poll {
                question,
                options,
                selectable_options_count,
            } => format!(
                "[\"poll\",{},{},{}]",
                json_string(question),
                json_string_array(options),
                selectable_options_count,
            ),
            SendPayload::Forward {
                source_chat_id,
                message_id,
                target_chat_id,
            } => format!(
                "[\"forward\",{},{},{}]",
                json_string(source_chat_id),
                json_string(message_id),
                json_string(target_chat_id),
            ),
        }
    }
}

/// port: `JSON.stringify` of a string.
fn json_string(value: &str) -> String {
    let mut text = String::with_capacity(value.len() + 2);
    text.push('"');
    for character in value.chars() {
        match character {
            '"' => text.push_str("\\\""),
            '\\' => text.push_str("\\\\"),
            '\n' => text.push_str("\\n"),
            '\r' => text.push_str("\\r"),
            '\t' => text.push_str("\\t"),
            '\u{0008}' => text.push_str("\\b"),
            '\u{000C}' => text.push_str("\\f"),
            character if (character as u32) < 0x20 => {
                text.push_str(&format!("\\u{:04x}", character as u32));
            }
            character => text.push(character),
        }
    }
    text.push('"');
    text
}

/// port: `JSON.stringify` of an array of strings.
fn json_string_array(values: &[String]) -> String {
    let items: Vec<String> = values.iter().map(|value| json_string(value)).collect();
    format!("[{}]", items.join(","))
}
