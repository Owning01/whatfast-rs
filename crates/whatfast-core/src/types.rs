//! The domain DTOs the whole crate shares.
//!
//! port: the oracle keeps them in `tauri/src/lib/types.ts`, and every module
//! that needs them imports them from there: `lib/format.ts`, `lib/polls.ts`,
//! `lib/notification-policy.ts` and `state/app-helpers.ts` all import the
//! same `Chat` and the same `Message`. Two batches of the migration each
//! wrote their own copy of those types, so the canonical ones live here and
//! the modules import them.
//!
//! The file also holds the two projections the pure formatters work with,
//! [`ChatSummary`] and [`MessageSummary`]. Their documentation says why they
//! are not the canonical DTOs.

/// The kind of conversation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(i32)]
pub enum ChatKind {
    #[default]
    Unspecified = 0,
    Direct = 1,
    Group = 2,
    Other = 3,
}

/// The delivery state of a message.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(i32)]
pub enum MessageStatus {
    #[default]
    Unspecified = 0,
    Pending = 1,
    Sent = 2,
    Delivered = 3,
    Read = 4,
    Failed = 5,
}

impl MessageStatus {
    /// The raw wire value of the status: the number the bridge sends.
    pub const fn as_wire(self) -> i32 {
        self as i32
    }

    /// The status of a raw wire value.
    ///
    /// port: the bridge crosses the enum as its numeric value, so a value the
    /// crate does not know is `Unspecified`.
    pub const fn from_wire(value: i32) -> Self {
        match value {
            1 => Self::Pending,
            2 => Self::Sent,
            3 => Self::Delivered,
            4 => Self::Read,
            5 => Self::Failed,
            _ => Self::Unspecified,
        }
    }
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

/// A message of the conversation.
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

/// A reaction to a message.
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

/// The content of a message: the externally tagged oneof of the bridge.
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
pub struct TextContent {
    pub text: String,
    pub link_preview: Option<LinkPreview>,
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

/// The fields of a chat that the pure formatters read.
///
/// port: the oracle's `lib/format.ts` takes the whole `Chat` DTO, but the
/// frozen suite of that module builds the chat field by field, and a Rust
/// struct literal names every field: a projection that drops a field cannot
/// be the same type as [`Chat`]. The canonical DTO of the app stays [`Chat`];
/// this is the projection the formatters and the poll reducer work with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatSummary {
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
    pub phone_number: String,
    pub contact_name: String,
    pub push_name: String,
    pub business_name: String,
    pub last_message_from_me: bool,
    pub transport_jid: String,
}

/// The fields of a message that the pure formatters and the poll reducer read.
///
/// port: the same reason as [`ChatSummary`]: the frozen suites build the
/// message field by field. The `status` keeps the raw wire value of
/// [`MessageStatus`] because no ported function reads the status.
#[derive(Debug, Clone, PartialEq)]
pub struct MessageSummary {
    pub id: String,
    pub chat_id: String,
    pub sender_id: String,
    pub sender_name: String,
    pub from_me: bool,
    pub timestamp_ms: i64,
    /// The raw wire value of [`MessageStatus`]; no ported function reads it.
    pub status: i32,
    pub edited: bool,
    pub revoked: bool,
    pub expires_at_ms: i64,
    pub sender_phone_number: String,
    pub sender_avatar_path: String,
    pub reactions: Vec<Reaction>,
    pub reply_to_message_id: String,
    pub reply_to_chat_id: String,
    pub content: Option<MessageContent>,
}

impl From<&Chat> for ChatSummary {
    fn from(chat: &Chat) -> Self {
        ChatSummary {
            id: chat.id.clone(),
            kind: chat.kind,
            title: chat.title.clone(),
            avatar_path: chat.avatar_path.clone(),
            last_message_preview: chat.last_message_preview.clone(),
            last_message_timestamp_ms: chat.last_message_timestamp_ms,
            unread_count: chat.unread_count,
            muted: chat.muted,
            pinned: chat.pinned,
            archived: chat.archived,
            phone_number: chat.phone_number.clone(),
            contact_name: chat.contact_name.clone(),
            push_name: chat.push_name.clone(),
            business_name: chat.business_name.clone(),
            last_message_from_me: chat.last_message_from_me,
            transport_jid: chat.transport_jid.clone(),
        }
    }
}

impl From<&Message> for MessageSummary {
    fn from(message: &Message) -> Self {
        MessageSummary {
            id: message.id.clone(),
            chat_id: message.chat_id.clone(),
            sender_id: message.sender_id.clone(),
            sender_name: message.sender_name.clone(),
            from_me: message.from_me,
            timestamp_ms: message.timestamp_ms,
            status: message.status.as_wire(),
            edited: message.edited,
            revoked: message.revoked,
            expires_at_ms: message.expires_at_ms,
            sender_phone_number: message.sender_phone_number.clone(),
            sender_avatar_path: message.sender_avatar_path.clone(),
            reactions: message.reactions.clone(),
            reply_to_message_id: message.reply_to_message_id.clone(),
            reply_to_chat_id: message.reply_to_chat_id.clone(),
            content: message.content.clone(),
        }
    }
}
