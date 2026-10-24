//! Store rows → bridge protos.
//!
//! The mapping mirrors the Go's `wireMessage` / `wireChat`: same fields, same
//! fallbacks. Identity enrichment (display names, phone numbers, avatars)
//! lands with the contacts wave; today the store row is the whole truth.

use whatfast_protocol::{
    AttachmentContent, Chat as WireChat, ChatKind, ContactContent, ContactsContent, ImageContent,
    LocationContent, Message as WireMessage, PollContent, Reaction as WireReaction, TextContent,
    UnsupportedContent,
};

use crate::db::{ChatRow, MessageRow, ReactionRow};

/// The conversation kind of a chat, by the transport JID's server.
fn chat_kind(jid: &str) -> ChatKind {
    if jid.ends_with("@g.us") || jid.ends_with("@broadcast") {
        ChatKind::Group
    } else if jid.contains('@') {
        ChatKind::Direct
    } else {
        ChatKind::Other
    }
}

/// The digits of a jid's user half, the phone number a direct chat shows.
fn phone_digits(jid: &str) -> String {
    jid.split('@').next().unwrap_or("").to_owned()
}

/// The title of a chat, resolved with the Go's rule: for direct chats the
/// saved contact name wins, then the business name, then the push name the
/// contact chose, then the stored name, then the bare phone digits.
fn chat_title(chat: &ChatRow) -> String {
    if chat.address_jid.ends_with("@g.us") {
        return chat.name.clone();
    }
    [
        chat.contact_name.as_str(),
        chat.business_name.as_str(),
        chat.push_name.as_str(),
        chat.name.as_str(),
        chat.phone_number.as_str(),
    ]
    .into_iter()
    .find(|candidate| !candidate.is_empty())
    .map(str::to_owned)
    .unwrap_or_else(|| phone_digits(&chat.address_jid))
}

/// The wire chat of one store row. `muted` is true while the mute deadline
/// is in the future, matching the Go's rule.
pub fn wire_chat(chat: &ChatRow) -> WireChat {
    let now = crate::db::now_ms();
    WireChat {
        id: chat.jid.clone(),
        kind: chat_kind(&chat.address_jid) as i32,
        title: chat_title(chat),
        avatar_path: String::new(),
        last_message_preview: chat.last_message_text.clone(),
        last_message_timestamp_ms: chat.last_message_at,
        unread_count: chat.unread_count.max(0) as u32,
        muted: chat.muted_until > now,
        pinned: chat.pinned,
        archived: chat.archived,
        phone_number: if chat.phone_number.is_empty() {
            phone_digits(&chat.address_jid)
        } else {
            chat.phone_number.clone()
        },
        contact_name: chat.contact_name.clone(),
        push_name: chat.push_name.clone(),
        business_name: chat.business_name.clone(),
        last_message_from_me: chat.last_message_from_me,
        transport_jid: chat.address_jid.clone(),
        blocked: chat.blocked,
        mute_until_ms: chat.muted_until,
    }
}

/// The wire status number of a stored status. Unknown numbers collapse to
/// unspecified rather than lying.
fn wire_status(status: i64) -> i32 {
    match status {
        1..=5 => status as i32,
        _ => 0,
    }
}

/// The wire message of one store row, content oneof included.
pub fn wire_message(message: &MessageRow) -> WireMessage {
    let status = wire_status(message.status);
    let reactions = message.reactions.iter().map(wire_reaction).collect();
    let content = wire_content(message);
    WireMessage {
        id: message.id.clone(),
        chat_id: message.chat_jid.clone(),
        sender_id: message.sender_jid.clone(),
        sender_name: String::new(),
        from_me: message.from_me,
        timestamp_ms: message.timestamp,
        status,
        edited: message.edited_at > 0,
        revoked: message.revoked,
        expires_at_ms: 0,
        sender_phone_number: String::new(),
        sender_avatar_path: String::new(),
        reactions,
        reply_to_message_id: message.reply_to_id.clone(),
        forwarding_score: message.forwarding_score.max(0) as u32,
        mention_texts: Vec::new(),
        reply_to_chat_id: message.reply_to_chat_id.clone(),
        content: Some(content),
    }
}

fn wire_content(message: &MessageRow) -> whatfast_protocol::message::Content {
    use whatfast_protocol::message::Content as C;
    if message.revoked || message.kind == "revoked" {
        return C::Unsupported(UnsupportedContent {
            type_name: "revoked".into(),
            fallback_text: message.text.clone(),
        });
    }
    match message.kind.as_str() {
        "text" => C::Text(TextContent {
            text: message.text.clone(),
            link_preview: wire_link_preview(message),
        }),
        "image" | "sticker" => C::Image(ImageContent {
            caption: message.image_caption.clone(),
            mime_type: message.image_mime.clone(),
            local_path: message.image_local_path.clone(),
            width: message.image_width.max(0) as u32,
            height: message.image_height.max(0) as u32,
            file_size: message.image_size.max(0) as u64,
            downloadable: message.image_size > 0 && message.image_local_path.is_empty(),
            sticker: message.kind == "sticker",
            animated: message.image_animated,
            thumbnail_path: String::new(),
            jpeg_thumbnail: message.image_thumbnail.clone(),
        }),
        "video" | "audio" | "document" => C::Attachment(AttachmentContent {
            kind: message.kind.clone(),
            caption: if message.media_caption.is_empty() {
                message.text.clone()
            } else {
                message.media_caption.clone()
            },
            mime_type: message.media_mime.clone(),
            file_name: message.media_file_name.clone(),
            local_path: message.media_local_path.clone(),
            file_size: message.media_size.max(0) as u64,
            width: message.media_width.max(0) as u32,
            height: message.media_height.max(0) as u32,
            duration_seconds: message.media_duration.max(0) as u32,
            animated: message.media_animated,
            voice_note: message.media_voice,
            downloadable: message.media_size > 0 && message.media_local_path.is_empty(),
        }),
        "contacts" => C::Contacts(ContactsContent {
            contacts: message
                .contacts_json
                .lines()
                .map(|_| ContactContent::default())
                .collect(),
        }),
        "location" => C::Location(LocationContent {
            latitude: message.location_lat,
            longitude: message.location_lng,
            name: message.location_name.clone(),
            address: message.location_address.clone(),
            url: message.location_url.clone(),
            live: message.location_live,
        }),
        "poll" => C::Poll(PollContent {
            question: message.text.clone(),
            options: Vec::new(),
            selectable_options_count: 1,
            total_voters: 0,
        }),
        _ => C::Unsupported(UnsupportedContent {
            type_name: message.kind.clone(),
            fallback_text: message.text.clone(),
        }),
    }
}

/// The text message's link preview, when the row carries one.
fn wire_link_preview(message: &MessageRow) -> Option<whatfast_protocol::LinkPreview> {
    if message.link_preview_url.is_empty() {
        return None;
    }
    Some(whatfast_protocol::LinkPreview {
        url: message.link_preview_url.clone(),
        title: message.link_preview_title.clone(),
        description: message.link_preview_description.clone(),
        jpeg_thumbnail: message.link_preview_thumbnail.clone(),
        thumbnail_width: message.link_preview_width.max(0) as u32,
        thumbnail_height: message.link_preview_height.max(0) as u32,
    })
}

pub fn wire_reaction(reaction: &ReactionRow) -> WireReaction {
    WireReaction {
        chat_id: reaction.chat_jid.clone(),
        message_id: reaction.message_id.clone(),
        sender_id: reaction.sender_jid.clone(),
        emoji: reaction.emoji.clone(),
        timestamp_ms: reaction.timestamp,
        from_me: reaction.from_me,
        sender_name: String::new(),
        sender_phone_number: String::new(),
        sender_avatar_path: String::new(),
    }
}
