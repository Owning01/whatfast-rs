//! Store rows → bridge protos.
//!
//! The mapping mirrors the Go's `wireMessage` / `wireChat`: same fields, same
//! fallbacks. Identity enrichment (display names, phone numbers, avatars)
//! lands with the contacts wave; today the store row is the whole truth.

use whatfast_protocol::{
    AttachmentContent, Chat as WireChat, ChatKind, ContactContent, ContactsContent, ImageContent,
    LocationContent, Message as WireMessage, PollContent, PollOption, PollVoter,
    Reaction as WireReaction, TextContent, UnsupportedContent,
};

use crate::db::{ChatRow, MessageRow, PollSnapshot, ReactionRow};

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
pub(crate) fn phone_digits(jid: &str) -> String {
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

/// The wire message of one store row, content oneof included. A poll row is
/// served without its stored definition, which the oracle renders as an
/// unsupported payload; callers that can read the `polls` table use
/// [`wire_message_polled`].
pub fn wire_message(message: &MessageRow) -> WireMessage {
    wire_message_polled(message, None)
}

/// The wire message of one store row, with the poll definition when the
/// caller looked it up: the Go `wireMessage` rendered a poll only while its
/// `polls` row existed.
pub fn wire_message_polled(message: &MessageRow, poll: Option<&PollSnapshot>) -> WireMessage {
    let status = wire_status(message.status);
    let reactions = message.reactions.iter().map(wire_reaction).collect();
    let content = wire_content(message, poll);
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

fn wire_content(
    message: &MessageRow,
    poll: Option<&PollSnapshot>,
) -> whatfast_protocol::message::Content {
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
            // The Go advertised the download affordance whenever the message
            // was live, whether or not the cache already held the file.
            downloadable: !message.revoked,
            sticker: message.kind == "sticker",
            animated: message.image_animated,
            thumbnail_path: String::new(),
            jpeg_thumbnail: message.image_thumbnail.clone(),
        }),
        "video" | "audio" | "document" => {
            // The Go schema stored every media kind in the `image_*` columns;
            // a migrated row still names its file there. The same rule the
            // download path uses decides which column group is authoritative.
            let descriptor_in_image_columns = message.media_direct_path.is_empty()
                && message.media_media_key.is_empty()
                && !message.image_direct_path.is_empty();
            let mime = if descriptor_in_image_columns && !message.image_mime.is_empty() {
                message.image_mime.clone()
            } else {
                message.media_mime.clone()
            };
            let local_path = if descriptor_in_image_columns && message.media_local_path.is_empty() {
                message.image_local_path.clone()
            } else {
                message.media_local_path.clone()
            };
            let file_size = if descriptor_in_image_columns {
                message.image_size
            } else {
                message.media_size
            };
            let width = if descriptor_in_image_columns {
                message.image_width
            } else {
                message.media_width
            };
            let height = if descriptor_in_image_columns {
                message.image_height
            } else {
                message.media_height
            };
            let animated = if descriptor_in_image_columns {
                message.image_animated
            } else {
                message.media_animated
            };
            C::Attachment(AttachmentContent {
                kind: message.kind.clone(),
                caption: if message.media_caption.is_empty() {
                    message.text.clone()
                } else {
                    message.media_caption.clone()
                },
                mime_type: mime,
                file_name: message.media_file_name.clone(),
                local_path,
                file_size: file_size.max(0) as u64,
                width: width.max(0) as u32,
                height: height.max(0) as u32,
                duration_seconds: message.media_duration.max(0) as u32,
                animated,
                voice_note: message.media_voice,
                downloadable: !message.revoked,
            })
        }
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
        // A poll whose definition is gone renders as an unsupported payload,
        // exactly the Go fallback: the content lived in `polls`, not in the
        // message row.
        "poll" => match poll {
            Some(poll) => C::Poll(PollContent {
                question: poll.question.clone(),
                options: poll
                    .options
                    .iter()
                    .map(|option| PollOption {
                        name: option.name.clone(),
                        vote_count: option.votes,
                        selected_by_me: option.selected_by_me,
                        voters: option
                            .voters
                            .iter()
                            .map(|voter| PollVoter {
                                user_id: voter.jid.clone(),
                                display_name: String::new(),
                                avatar_path: String::new(),
                                from_me: voter.from_me,
                            })
                            .collect(),
                    })
                    .collect(),
                selectable_options_count: poll.selectable_count,
                total_voters: poll.total_voters,
            }),
            None => C::Unsupported(UnsupportedContent {
                type_name: message.kind.clone(),
                fallback_text: message.text.clone(),
            }),
        },
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

#[cfg(test)]
mod tests {
    use super::{wire_message, wire_message_polled};
    use crate::db::{MessageRow, PollOptionRow, PollSnapshot, PollVoterRow};

    fn row(kind: &str) -> MessageRow {
        MessageRow {
            id: "m1".into(),
            chat_jid: "a@s.whatsapp.net".into(),
            transport_jid: "a@s.whatsapp.net".into(),
            kind: kind.into(),
            status: 2,
            ..Default::default()
        }
    }

    /// The Go schema stored every media kind in the `image_*` columns: a
    /// migrated attachment row must still name its file and its size.
    #[test]
    fn a_legacy_attachment_reads_its_descriptor_from_the_image_columns() {
        let mut message = row("video");
        message.image_mime = "video/mp4".into();
        message.image_local_path = "/cache/video.mp4".into();
        message.image_direct_path = "/v/t62.7118-24".into();
        message.image_media_key = vec![1, 2, 3];
        message.image_width = 640;
        message.image_height = 480;
        message.image_size = 2048;
        message.media_mime = "stale/mime".into();

        let wired = wire_message(&message);
        let Some(whatfast_protocol::message::Content::Attachment(attachment)) = wired.content
        else {
            panic!("expected attachment content");
        };
        assert_eq!(attachment.kind, "video");
        assert_eq!(attachment.mime_type, "video/mp4");
        assert_eq!(attachment.local_path, "/cache/video.mp4");
        assert_eq!(attachment.file_size, 2048);
        assert_eq!(attachment.width, 640);
        assert_eq!(attachment.height, 480);
        assert!(attachment.downloadable);
    }

    /// The download chip follows the message, not the cache: the Go wired
    /// `Downloadable: !m.Revoked` for every live media message.
    #[test]
    fn a_cached_image_is_still_downloadable_and_a_revoked_one_is_not() {
        let mut message = row("image");
        message.image_size = 10;
        message.image_local_path = "/cache/image.jpg".into();
        let Some(whatfast_protocol::message::Content::Image(image)) =
            wire_message(&message).content
        else {
            panic!("expected image content");
        };
        assert!(image.downloadable);

        message.revoked = true;
        let Some(whatfast_protocol::message::Content::Unsupported(unsupported)) =
            wire_message(&message).content
        else {
            panic!("expected unsupported content");
        };
        assert_eq!(unsupported.type_name, "revoked");
    }

    /// A poll renders from the `polls` table; without a stored definition the
    /// Go fell back to an unsupported payload.
    #[test]
    fn a_poll_renders_its_stored_definition() {
        let mut message = row("poll");
        message.text = "Lunch?".into();
        let wired = wire_message(&message);
        let Some(whatfast_protocol::message::Content::Unsupported(unsupported)) = wired.content
        else {
            panic!("expected the unsupported fallback");
        };
        assert_eq!(unsupported.type_name, "poll");

        let snapshot = PollSnapshot {
            message_id: "m1".into(),
            question: "Lunch?".into(),
            selectable_count: 2,
            total_voters: 1,
            options: vec![
                PollOptionRow {
                    name: "Pizza".into(),
                    votes: 0,
                    selected_by_me: false,
                    voters: Vec::new(),
                },
                PollOptionRow {
                    name: "Sushi".into(),
                    votes: 1,
                    selected_by_me: true,
                    voters: vec![PollVoterRow {
                        jid: "me@s.whatsapp.net".into(),
                        from_me: true,
                    }],
                },
            ],
        };
        let Some(whatfast_protocol::message::Content::Poll(poll)) =
            wire_message_polled(&message, Some(&snapshot)).content
        else {
            panic!("expected poll content");
        };
        assert_eq!(poll.selectable_options_count, 2);
        assert_eq!(poll.total_voters, 1);
        assert_eq!(poll.options[1].name, "Sushi");
        assert_eq!(poll.options[1].vote_count, 1);
        assert!(poll.options[1].selected_by_me);
        assert!(poll.options[1].voters[0].from_me);
    }
}
