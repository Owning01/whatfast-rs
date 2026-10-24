//! Turns the library's events into store rows and bridge events.
//!
//! The classification table mirrors the Go sidecar's `domainMessage` /
//! `extractMessageContent`: same kinds, same fallback texts, same handling of
//! edits and revokes inside `ProtocolMessage` wrappers. The history walk
//! mirrors the Go's history-sync ingestion: every conversation becomes a
//! chat, every message a row, and the desktop hears about both.

use wacore::types::events::{ChatPresenceUpdate, Event, InboundMessage, Receipt};
use wacore::types::message::MessageInfo;
use wacore::types::presence::{ChatActivity, ReceiptType};
use whatfast_protocol::{
    BackendEvent, ChatUpserted, MessageStatus, MessageUpserted, TypingChanged,
};
use whatsapp_rust::waproto::whatsapp as wa;

use crate::db::{ChatRow, MessageRow, ReactionRow, Store};
/// The status a freshly stored inbound message carries until a receipt or
/// its own send record says otherwise. The Go stored `Delivered` for peers'
/// messages; the value is not shown for them, so this is only about parity.
const INBOUND_STATUS: i64 = MessageStatus::Delivered as i64;

/// The JID shape every id in the store uses: non-AD, canonical text.
pub fn jid_text(jid: &whatsapp_rust::Jid) -> String {
    jid.to_non_ad_string()
}

/// One ingested inbound message: the rows to write and the wire events to
/// emit. The chat row is returned too because a live message can create its
/// conversation.
pub struct Ingested {
    pub chat: ChatRow,
    pub message: MessageRow,
}

/// The classification of one message's content: everything `classify` reads
/// off the proto, in the exact shape the store row takes.
#[derive(Default)]
struct Content {
    kind: String,
    text: String,
    image: Option<(ImageParts, bool)>, // parts + sticker flag
    attachment: Option<AttachmentParts>,
    contacts: Vec<(String, String)>,
    location: Option<LocationParts>,
    link_preview: Option<LinkPreviewParts>,
    poll: Option<PollParts>,
    reply_to_id: String,
    reply_to_chat_id: String,
    is_forwarded: bool,
    forwarding_score: i64,
}

#[derive(Default)]
struct ImageParts {
    mime: String,
    caption: String,
    direct_path: String,
    media_key: Vec<u8>,
    file_sha256: Vec<u8>,
    file_enc_sha256: Vec<u8>,
    width: u32,
    height: u32,
    file_size: u64,
    animated: bool,
    thumbnail: Vec<u8>,
}

#[derive(Default)]
#[expect(dead_code, reason = "local_path is written by the media wave")]
struct AttachmentParts {
    mime: String,
    caption: String,
    file_name: String,
    direct_path: String,
    media_key: Vec<u8>,
    file_sha256: Vec<u8>,
    file_enc_sha256: Vec<u8>,
    width: u32,
    height: u32,
    file_size: u64,
    duration: u32,
    animated: bool,
    voice: bool,
    local_path: String,
}

#[derive(Default)]
struct LocationParts {
    lat: f64,
    lng: f64,
    name: String,
    address: String,
    url: String,
    live: bool,
}

#[derive(Default)]
struct LinkPreviewParts {
    url: String,
    title: String,
    description: String,
    thumbnail: Vec<u8>,
    width: u32,
    height: u32,
}

#[derive(Default)]
#[expect(dead_code, reason = "the polls table fills with the sends wave")]
struct PollParts {
    question: String,
    options: Vec<String>,
    selectable: u32,
}

fn s(field: &Option<String>) -> String {
    field.clone().unwrap_or_default()
}

fn join_non_empty(sep: &str, parts: &[&str]) -> String {
    parts
        .iter()
        .filter(|part| !part.is_empty())
        .cloned()
        .collect::<Vec<_>>()
        .join(sep)
}

/// Classifies one (possibly wrapped) content message into the store's shape,
/// mirroring the Go's `extractMessageContent` order and fallback texts.
fn classify(content: &wa::Message) -> Content {
    let mut out = Content::default();

    if let Some(image) = content.image_message.as_option() {
        out.kind = "image".into();
        out.text = match s(&image.caption) {
            empty if empty.is_empty() => "📷 Photo".into(),
            caption => caption,
        };
        out.image = Some((
            ImageParts {
                mime: s(&image.mimetype),
                caption: s(&image.caption),
                direct_path: s(&image.direct_path),
                media_key: image.media_key.clone().unwrap_or_default(),
                file_sha256: image.file_sha256.clone().unwrap_or_default(),
                file_enc_sha256: image.file_enc_sha256.clone().unwrap_or_default(),
                width: image.width.unwrap_or(0),
                height: image.height.unwrap_or(0),
                file_size: image.file_length.unwrap_or(0),
                thumbnail: image.jpeg_thumbnail.clone().unwrap_or_default(),
                animated: false,
            },
            false,
        ));
        if let Some(context) = image.context_info.as_option() {
            out.is_forwarded = context_forwarded(context, &mut out.forwarding_score);
        }
        return out;
    }
    if let Some(sticker) = content.sticker_message.as_option() {
        out.kind = "sticker".into();
        out.text = "Sticker".into();
        out.image = Some((
            ImageParts {
                mime: s(&sticker.mimetype),
                direct_path: s(&sticker.direct_path),
                media_key: sticker.media_key.clone().unwrap_or_default(),
                file_sha256: sticker.file_sha256.clone().unwrap_or_default(),
                file_enc_sha256: sticker.file_enc_sha256.clone().unwrap_or_default(),
                width: sticker.width.unwrap_or(0),
                height: sticker.height.unwrap_or(0),
                file_size: sticker.file_length.unwrap_or(0),
                animated: sticker.is_animated.unwrap_or(false)
                    || sticker.is_lottie.unwrap_or(false),
                ..Default::default()
            },
            true,
        ));
        return out;
    }
    if let Some(video) = content.video_message.as_option() {
        out.kind = "video".into();
        out.text = match s(&video.caption) {
            empty if empty.is_empty() => "🎬 Video".into(),
            caption => caption,
        };
        out.attachment = Some(AttachmentParts {
            mime: s(&video.mimetype),
            caption: s(&video.caption),
            direct_path: s(&video.direct_path),
            media_key: video.media_key.clone().unwrap_or_default(),
            file_sha256: video.file_sha256.clone().unwrap_or_default(),
            file_enc_sha256: video.file_enc_sha256.clone().unwrap_or_default(),
            width: video.width.unwrap_or(0),
            height: video.height.unwrap_or(0),
            file_size: video.file_length.unwrap_or(0),
            duration: video.seconds.unwrap_or(0),
            animated: video.gif_playback.unwrap_or(false),
            ..Default::default()
        });
        return out;
    }
    if let Some(note) = content.ptv_message.as_option() {
        // A PTV (round video note) is a video attachment the UI plays round.
        out.kind = "video".into();
        out.text = "🎬 Video".into();
        out.attachment = Some(AttachmentParts {
            mime: s(&note.mimetype),
            direct_path: s(&note.direct_path),
            media_key: note.media_key.clone().unwrap_or_default(),
            file_sha256: note.file_sha256.clone().unwrap_or_default(),
            file_enc_sha256: note.file_enc_sha256.clone().unwrap_or_default(),
            width: note.width.unwrap_or(0),
            height: note.height.unwrap_or(0),
            file_size: note.file_length.unwrap_or(0),
            duration: note.seconds.unwrap_or(0),
            ..Default::default()
        });
        return out;
    }
    if let Some(audio) = content.audio_message.as_option() {
        out.kind = "audio".into();
        out.text = if audio.ptt.unwrap_or(false) {
            "🎤 Voice message".into()
        } else {
            "🎵 Audio".into()
        };
        out.attachment = Some(AttachmentParts {
            mime: s(&audio.mimetype),
            direct_path: s(&audio.direct_path),
            media_key: audio.media_key.clone().unwrap_or_default(),
            file_sha256: audio.file_sha256.clone().unwrap_or_default(),
            file_enc_sha256: audio.file_enc_sha256.clone().unwrap_or_default(),
            file_size: audio.file_length.unwrap_or(0),
            duration: audio.seconds.unwrap_or(0),
            voice: audio.ptt.unwrap_or(false),
            ..Default::default()
        });
        return out;
    }
    if let Some(document) = content.document_message.as_option().or_else(|| {
        content
            .document_with_caption_message
            .as_option()
            .and_then(|d| d.message.as_option())
            .and_then(|m| m.document_message.as_option())
    }) {
        out.kind = "document".into();
        out.text = {
            let file_name = s(&document.file_name);
            let title = s(&document.title);
            [
                s(&document.caption),
                if file_name.is_empty() {
                    title
                } else {
                    file_name
                },
            ]
            .into_iter()
            .find(|candidate| !candidate.is_empty())
            .unwrap_or_else(|| "📄 Document".into())
        };
        out.attachment = Some(AttachmentParts {
            mime: s(&document.mimetype),
            caption: s(&document.caption),
            file_name: s(&document.file_name),
            direct_path: s(&document.direct_path),
            media_key: document.media_key.clone().unwrap_or_default(),
            file_sha256: document.file_sha256.clone().unwrap_or_default(),
            file_enc_sha256: document.file_enc_sha256.clone().unwrap_or_default(),
            file_size: document.file_length.unwrap_or(0),
            ..Default::default()
        });
        return out;
    }
    if let Some(contact) = content.contact_message.as_option() {
        out.kind = "contact".into();
        out.text = match s(&contact.display_name) {
            empty if empty.is_empty() => "Contact".into(),
            name => name,
        };
        out.contacts
            .push((s(&contact.display_name), s(&contact.vcard)));
        return out;
    }
    if let Some(array) = content.contacts_array_message.as_option() {
        out.kind = "contacts".into();
        out.text = match s(&array.display_name) {
            empty if empty.is_empty() => format!("{} contacts", array.contacts.len()),
            name => name,
        };
        for contact in &array.contacts {
            out.contacts
                .push((s(&contact.display_name), s(&contact.vcard)));
        }
        return out;
    }
    if let Some(location) = content.location_message.as_option() {
        out.kind = "location".into();
        out.text = {
            let name = s(&location.name);
            let address = s(&location.address);
            [
                name.clone(),
                if name.is_empty() {
                    address
                } else {
                    String::new()
                },
            ]
            .into_iter()
            .find(|candidate| !candidate.is_empty())
            .unwrap_or_else(|| "📍 Location".into())
        };
        out.location = Some(LocationParts {
            lat: location.degrees_latitude.unwrap_or(0.0),
            lng: location.degrees_longitude.unwrap_or(0.0),
            name: s(&location.name),
            address: s(&location.address),
            url: s(&location.url),
            live: location.is_live.unwrap_or(false),
        });
        return out;
    }
    if let Some(live) = content.live_location_message.as_option() {
        out.kind = "location".into();
        out.text = match s(&live.caption) {
            empty if empty.is_empty() => "📍 Live location".into(),
            caption => caption,
        };
        out.location = Some(LocationParts {
            lat: live.degrees_latitude.unwrap_or(0.0),
            lng: live.degrees_longitude.unwrap_or(0.0),
            name: s(&live.caption),
            live: true,
            ..Default::default()
        });
        return out;
    }
    if let Some(poll) = content
        .poll_creation_message
        .as_option()
        .or_else(|| content.poll_creation_message_v2.as_option())
        .or_else(|| content.poll_creation_message_v3.as_option())
    {
        out.kind = "poll".into();
        let question = s(&poll.name);
        out.text = question.clone();
        let options = poll
            .options
            .iter()
            .map(|option| s(&option.option_name))
            .collect::<Vec<_>>();
        out.poll = Some(PollParts {
            question,
            selectable: poll.selectable_options_count.unwrap_or(1),
            options,
        });
        return out;
    }
    if let Some(snapshot) = content.poll_result_snapshot_message.as_option() {
        out.kind = "poll".into();
        let label = "📊 Poll results".to_owned();
        let name = s(&snapshot.name);
        out.text = join_non_empty(": ", &[label.as_str(), name.as_str()]);
        return out;
    }
    if let Some(invite) = content.group_invite_message.as_option() {
        out.kind = "group_invite".into();
        let label = "👥 Group invite".to_owned();
        let name = s(&invite.group_name);
        let caption = s(&invite.caption);
        let first = join_non_empty(": ", &[label.as_str(), name.as_str()]);
        out.text = join_non_empty(
            "
",
            &[first.as_str(), caption.as_str()],
        );
        return out;
    }
    if let Some(event) = content.event_message.as_option() {
        out.kind = "event".into();
        let label = "📅 Event".to_owned();
        let name = s(&event.name);
        let mut text = join_non_empty(": ", &[label.as_str(), name.as_str()]);
        if event.is_canceled.unwrap_or(false) {
            text.push_str(" (canceled)");
        }
        {
            let description = s(&event.description);
            text = join_non_empty(
                "
",
                &[text.as_str(), description.as_str()],
            );
        }
        out.text = text;
        return out;
    }
    if let Some(interactive) = content.interactive_message.as_option() {
        out.kind = "interactive".into();
        let header_title = interactive
            .header
            .as_option()
            .and_then(|header| header.title.clone())
            .unwrap_or_default();
        let body = interactive
            .body
            .as_option()
            .and_then(|body| body.text.clone())
            .unwrap_or_default();
        out.text = join_non_empty(
            "\n",
            [header_title, body]
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
                .as_slice(),
        );
        if out.text.is_empty() {
            out.text = "Interactive message".into();
        }
        return out;
    }
    if let Some(response) = content.interactive_response_message.as_option() {
        out.kind = "interactive".into();
        out.text = response
            .interactive_response_message
            .clone()
            .map(|inner| format!("{inner:?}"))
            .unwrap_or_else(|| "Interactive reply".into());
        return out;
    }
    if let Some(template) = content.template_message.as_option() {
        out.kind = "interactive".into();
        out.text = template
            .hydrated_template
            .as_option()
            .and_then(|hydrated| {
                hydrated
                    .hydrated_content_text
                    .clone()
                    .filter(|text| !text.is_empty())
                    .or_else(|| hydrated.hydrated_footer_text.clone())
            })
            .unwrap_or_else(|| "Template message".into());
        return out;
    }
    if let Some(buttons) = content.buttons_message.as_option() {
        out.kind = "interactive".into();
        out.text = s(&buttons.content_text);
        if out.text.is_empty() {
            out.text = "Interactive message".into();
        }
        return out;
    }
    if let Some(conversation) = &content.conversation {
        out.kind = "text".into();
        out.text = conversation.clone();
        return out;
    }
    if let Some(extended) = content.extended_text_message.as_option() {
        out.kind = "text".into();
        out.text = s(&extended.text);
        if extended.matched_text.is_some() {
            out.link_preview = Some(LinkPreviewParts {
                url: s(&extended.matched_text),
                title: s(&extended.title),
                description: s(&extended.description),
                thumbnail: extended.jpeg_thumbnail.clone().unwrap_or_default(),
                ..Default::default()
            });
        }
        return out;
    }
    // Everything else the contract carries as UnsupportedContent; the kind
    // names it for the preview.
    out.kind = "unsupported".into();
    out.text = String::new();
    out
}

fn context_forwarded(context: &wa::ContextInfo, score: &mut i64) -> bool {
    if !context.is_forwarded.unwrap_or(false) {
        return false;
    }
    *score = context.forwarding_score.unwrap_or(0) as i64;
    if *score == 0 {
        *score = 1;
    }
    true
}

/// Turns one inbound message into the rows the store takes, applying the
/// edit/revoke unwrapping of the Go's `domainMessage`.
pub fn inbound_to_row(inbound: &InboundMessage, mine: &str) -> Option<Ingested> {
    let info = &inbound.info;
    let raw_content: &wa::Message = &inbound.message;
    let mut protocol = raw_content
        .protocol_message
        .as_option()
        .cloned()
        .or_else(|| info_edit_protocol(info));
    let mut content: &wa::Message = raw_content;
    if is_edit(info)
        && let Some(protocol_message) = &protocol
        && let Some(edited) = protocol_message.edited_message.as_option()
    {
        content = edited;
    }
    let payload = whatsapp_rust::waproto::codec::message_to_vec(content);
    let mut classified = classify(content);
    let mut message = MessageRow {
        id: info.id.to_string(),
        chat_jid: jid_text(&info.source.chat),
        transport_jid: jid_text(&info.source.chat),
        sender_jid: jid_text(&info.source.sender.to_non_ad()),
        timestamp: info.timestamp.timestamp_millis(),
        from_me: info.source.is_from_me,
        status: INBOUND_STATUS,
        ..MessageRow::default()
    };
    // Pin events point at the pinned message through their own key.
    if let Some(pin) = content.pin_in_chat_message.as_option() {
        let target = pin.key.as_option().and_then(|key| key.id.clone());
        if let Some(target) = target {
            classified.reply_to_id = target;
            classified.reply_to_chat_id = message.chat_jid.clone();
        }
    }
    // An edit carries its replacement under the original message's id.
    if is_edit(info)
        && let Some(protocol_message) = &protocol
    {
        let target = protocol_message
            .key
            .as_option()
            .and_then(|key| key.id.clone());
        if let Some(target) = target {
            message.id = target;
            message.edited_at = protocol_message.timestamp_ms.unwrap_or(0);
        }
    }
    // A revoke marks the original message gone.
    if let Some(protocol_message) = &protocol
        && protocol_message
            .r#type
            .is_some_and(|kind| kind == wa::message::protocol_message::Type::Revoke)
    {
        let target = protocol_message
            .key
            .as_option()
            .and_then(|key| key.id.clone());
        if let Some(target) = target {
            message.id = target;
            message.text = "Message deleted".into();
            message.kind = "revoked".into();
            message.revoked = true;
        }
    }
    let _ = (&mut protocol, mine);
    message.payload = payload;
    message.kind = classified.kind.clone();
    message.text = classified.text.clone();
    message.reply_to_id = classified.reply_to_id.clone();
    message.reply_to_chat_id = if classified.reply_to_chat_id.is_empty() {
        String::new()
    } else {
        classified.reply_to_chat_id.clone()
    };
    message.forwarding_score = classified.forwarding_score;
    apply_content(&mut message, &classified);

    let mut chat = ChatRow {
        jid: message.chat_jid.clone(),
        preferred_jid: message.transport_jid.clone(),
        ..ChatRow::default()
    };
    if info.source.is_group {
        chat.name = String::new();
    }
    if message.kind == "revoked" {
        // A revoke of the chat's last message must not blank the preview to
        // its text; the Go previewed "Message deleted".
        chat.last_message_text = message.text.clone();
    }
    Some(Ingested { chat, message })
}

fn is_edit(info: &MessageInfo) -> bool {
    matches!(
        info.edit,
        wacore::types::message::EditAttribute::MessageEdit
            | wacore::types::message::EditAttribute::AdminEdit
    )
}

fn info_edit_protocol(_info: &MessageInfo) -> Option<wa::message::ProtocolMessage> {
    None
}

/// Writes the classified content fields into the row.
fn apply_content(message: &mut MessageRow, classified: &Content) {
    if let Some((image, sticker)) = &classified.image {
        message.image_mime = image.mime.clone();
        message.image_caption = image.caption.clone();
        message.image_thumbnail = image.thumbnail.clone();
        message.image_direct_path = image.direct_path.clone();
        message.image_media_key = image.media_key.clone();
        message.image_file_sha256 = image.file_sha256.clone();
        message.image_file_enc_sha256 = image.file_enc_sha256.clone();
        message.image_width = image.width as i64;
        message.image_height = image.height as i64;
        message.image_size = image.file_size as i64;
        message.image_animated = image.animated;
        if *sticker {
            message.kind = "sticker".into();
        }
    }
    if let Some(attachment) = &classified.attachment {
        message.media_mime = attachment.mime.clone();
        message.media_caption = attachment.caption.clone();
        message.media_file_name = attachment.file_name.clone();
        message.media_direct_path = attachment.direct_path.clone();
        message.media_media_key = attachment.media_key.clone();
        message.media_file_sha256 = attachment.file_sha256.clone();
        message.media_file_enc_sha256 = attachment.file_enc_sha256.clone();
        message.media_width = attachment.width as i64;
        message.media_height = attachment.height as i64;
        message.media_size = attachment.file_size as i64;
        message.media_duration = attachment.duration as i64;
        message.media_voice = attachment.voice;
        message.media_animated = attachment.animated;
    }
    if !classified.contacts.is_empty() {
        let contacts = classified
            .contacts
            .iter()
            .map(|(display_name, vcard)| {
                format!(
                    "{{\"display_name\":{},\"vcard\":{}}}",
                    json_string(display_name),
                    json_string(vcard)
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        message.contacts_json = format!("[{contacts}]");
    }
    if let Some(location) = &classified.location {
        message.location_lat = location.lat;
        message.location_lng = location.lng;
        message.location_name = location.name.clone();
        message.location_address = location.address.clone();
        message.location_url = location.url.clone();
        message.location_live = location.live;
    }
    if let Some(preview) = &classified.link_preview {
        message.link_preview_url = preview.url.clone();
        message.link_preview_title = preview.title.clone();
        message.link_preview_description = preview.description.clone();
        message.link_preview_thumbnail = preview.thumbnail.clone();
        message.link_preview_width = preview.width as i64;
        message.link_preview_height = preview.height as i64;
    }
}

/// Minimal JSON string escaping for the contacts array; vCards carry quotes
/// and backslashes routinely.
fn json_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for character in value.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

/// One history-sync conversation, pre-digested for the store.
pub struct HistoryConversation {
    pub jid: String,
    pub unread: u32,
    pub end_of_history_transfer: bool,
    pub messages: Vec<MessageRow>,
}

/// Learns the push names the history sync carries in bulk.
pub fn learn_history_contacts(store: &Store, parsed: &wa::HistorySync) {
    for pushname in &parsed.pushnames {
        let jid = pushname.id.clone().unwrap_or_default();
        let name = pushname.pushname.clone().unwrap_or_default();
        if jid.is_empty() || name.is_empty() {
            continue;
        }
        store.learn_contact(&jid, "", &name, "").ok();
    }
}

/// Walks a parsed history sync and returns every conversation with rows.
/// Message keys without content are skipped, the way the Go skipped them.
pub fn history_conversations(parsed: &wa::HistorySync) -> Vec<HistoryConversation> {
    let mut out = Vec::new();
    for conversation in &parsed.conversations {
        let chat_jid = conversation.id.clone();
        if chat_jid.is_empty() {
            continue;
        }
        let mut messages = Vec::new();
        for entry in &conversation.messages {
            let Some(key) = entry.message.as_option() else {
                continue;
            };
            let Some(message_proto) = key.message.as_option() else {
                continue;
            };
            let Some(message_id) = key.key.as_option().and_then(|key| key.id.clone()) else {
                continue;
            };
            let from_me = key
                .key
                .as_option()
                .and_then(|key| key.from_me)
                .unwrap_or(false);
            let sender = if from_me {
                String::new()
            } else {
                key.key
                    .as_option()
                    .and_then(|key| key.participant.clone())
                    .unwrap_or_else(|| chat_jid.clone())
            };
            let classified = classify(message_proto);
            let mut row = MessageRow {
                id: message_id,
                chat_jid: chat_jid.clone(),
                transport_jid: chat_jid.clone(),
                sender_jid: sender,
                timestamp: key.message_timestamp.unwrap_or(0) as i64 * 1000,
                from_me,
                status: INBOUND_STATUS,
                ..MessageRow::default()
            };
            row.payload = whatsapp_rust::waproto::codec::message_to_vec(message_proto);
            row.kind = classified.kind.clone();
            row.text = classified.text.clone();
            row.reply_to_id = classified.reply_to_id.clone();
            row.forwarding_score = classified.forwarding_score;
            apply_content(&mut row, &classified);
            if row.timestamp == 0 && row.text.is_empty() && row.kind == "unsupported" {
                continue;
            }
            messages.push(row);
        }
        out.push(HistoryConversation {
            jid: chat_jid,
            unread: conversation.unread_count.unwrap_or(0),
            end_of_history_transfer: conversation.end_of_history_transfer.unwrap_or(false),
            messages,
        });
    }
    out
}

/// Persists one conversation and answers with the wire events for it.
pub fn store_conversation(
    store: &Store,
    identity: &crate::identity::Identity,
    conversation: &HistoryConversation,
) -> rusqlite::Result<(BackendEvent, u64)> {
    // The history may key one person by the hidden identity while the live
    // messages arrive by phone number: both fold into the one chat.
    let (canonical, _folds) = crate::reconcile::canonical_chat(store, identity, &conversation.jid);
    store.ensure_chat(&canonical, &conversation.jid)?;
    let mut stored = 0u64;
    let mut newest = 0i64;
    for message in &conversation.messages {
        newest = newest.max(message.timestamp);
        let mut row = message.clone();
        row.chat_jid = canonical.clone();
        if store.insert_message(&row)? {
            stored += 1;
        }
    }
    if conversation.unread > 0 {
        store.write(|conn| {
            conn.execute(
                "UPDATE chats SET unread_count=MAX(unread_count,?1) WHERE jid=?2",
                rusqlite::params![conversation.unread as i64, canonical],
            )?;
            Ok(())
        })?;
    }
    let chat = store.chat(&canonical)?.unwrap_or_default();
    Ok((chat_upserted(chat), stored))
}

/// The wire event for one chat row.
pub fn chat_upserted(chat: ChatRow) -> BackendEvent {
    BackendEvent {
        sequence: 0,
        event: Some(whatfast_protocol::backend_event::Event::ChatUpserted(
            ChatUpserted {
                chat: Some(crate::wire::wire_chat(&chat)),
            },
        )),
    }
}

/// The wire event for one stored message (with its reactions).
pub fn message_upserted(message: &MessageRow) -> BackendEvent {
    BackendEvent {
        sequence: 0,
        event: Some(whatfast_protocol::backend_event::Event::MessageUpserted(
            MessageUpserted {
                message: Some(crate::wire::wire_message(message)),
            },
        )),
    }
}

/// The wire event for a reaction change.
pub fn reaction_updated(reaction: &ReactionRow, removed: bool) -> BackendEvent {
    BackendEvent {
        sequence: 0,
        event: Some(whatfast_protocol::backend_event::Event::ReactionUpdated(
            whatfast_protocol::ReactionUpdated {
                reaction: Some(crate::wire::wire_reaction(reaction)),
                removed,
            },
        )),
    }
}

/// Maps a receipt to the wire status it advances, `None` for kinds the
/// bridge does not carry.
pub fn receipt_status(kind: &ReceiptType) -> Option<MessageStatus> {
    match kind {
        ReceiptType::Delivered => Some(MessageStatus::Delivered),
        ReceiptType::Read => Some(MessageStatus::Read),
        ReceiptType::Sender | ReceiptType::Sent => Some(MessageStatus::Sent),
        _ => None,
    }
}

/// Applies a receipt to the store and answers with the wire event.
pub fn store_receipt(store: &Store, receipt: &Receipt) -> rusqlite::Result<Vec<BackendEvent>> {
    let Some(status) = receipt_status(&receipt.r#type) else {
        return Ok(Vec::new());
    };
    let chat_jid = store
        .resolve_chat_id(&jid_text(&receipt.source.chat))
        .unwrap_or_else(|_| jid_text(&receipt.source.chat));
    let mut events = Vec::new();
    for message_id in &receipt.message_ids {
        let id = message_id.to_string();
        if store.set_message_status(&chat_jid, &id, status as i64)?
            && let Some(message) = store.message(&chat_jid, &id)?
        {
            events.push(BackendEvent {
                sequence: 0,
                event: Some(whatfast_protocol::backend_event::Event::ReceiptUpdated(
                    whatfast_protocol::ReceiptUpdated {
                        chat_id: chat_jid.clone(),
                        message_id: id,
                        status: status as i32,
                        timestamp_ms: receipt.timestamp.timestamp_millis(),
                    },
                )),
            });
            let _ = message;
        }
    }
    Ok(events)
}

/// Applies a typing state and answers with the wire event.
pub fn store_chat_presence(store: &Store, presence: &ChatPresenceUpdate) -> Option<BackendEvent> {
    let chat_jid = store
        .resolve_chat_id(&jid_text(&presence.source.chat))
        .unwrap_or_else(|_| jid_text(&presence.source.chat));
    let _ = store; // transient state: nothing to persist
    let (typing, recording) = match presence.state {
        ChatActivity::Typing => (true, false),
        ChatActivity::RecordingAudio => (true, true),
        ChatActivity::Idle => (false, false),
        _ => (false, false),
    };
    Some(BackendEvent {
        sequence: 0,
        event: Some(whatfast_protocol::backend_event::Event::TypingChanged(
            TypingChanged {
                chat_id: chat_jid,
                sender_id: jid_text(&presence.source.sender.to_non_ad()),
                sender_name: presence.source.sender.to_non_ad_string(),
                typing,
                recording,
            },
        )),
    })
}

/// One event off the library, as the pump sees it: the store writes and the
/// bridge events that follow it. Connection-level events stay in
/// `session.rs`; everything that touches the database lives here.
///
/// Reactions arrive inside message envelopes: `reaction_message` in plain
/// form, `enc_reaction_message` encrypted with the parent message's secret.
/// An encrypted reaction whose parent is unknown is logged and dropped; the
/// repair wave will replay those from the journal.
pub fn apply_event(
    store: &Store,
    identity: &crate::identity::Identity,
    event: &Event,
) -> (Vec<BackendEvent>, Vec<Ingested>) {
    match event {
        Event::Messages(batch) => {
            let mut events = Vec::new();
            for inbound in batch.iter() {
                // Reactions first: they are envelopes, not content, and must
                // not land in the messages table as empty rows.
                if reaction_from_inbound(store, identity, inbound, &mut events) {
                    continue;
                }
                // Learn the sender's push name from the envelope: this is
                // where contact names come from between syncs.
                if !inbound.info.push_name.is_empty() && !inbound.info.source.is_from_me {
                    let sender = jid_text(&inbound.info.source.sender.to_non_ad());
                    let notify = inbound.info.push_name.to_string();
                    if !sender.is_empty() {
                        store.learn_contact(&sender, "", &notify, "").ok();
                    }
                }
                let Some(item) = inbound_to_row(inbound, "") else {
                    continue;
                };
                let mut chat = item.chat.clone();
                let mut message = item.message.clone();
                // One person, one chat: the hidden-identity jid of the
                // envelope folds into the phone-number chat when the
                // identity map knows the pair.
                let (canonical, _folds) =
                    crate::reconcile::canonical_chat(store, identity, &chat.jid);
                message.chat_jid = canonical.clone();
                chat.jid = canonical;
                store.ensure_chat(&chat.jid, &chat.preferred_jid).ok();
                if store.insert_message(&message).unwrap_or(false) {
                    let mut fresh = store
                        .chat(&message.chat_jid)
                        .unwrap_or_default()
                        .unwrap_or(chat);
                    fresh.last_message_text = preview_of(&message);
                    store
                        .write(|conn| {
                            conn.execute(
                                "UPDATE chats SET last_message_id=?1,last_message_text=?2,last_message_at=?3 WHERE jid=?4",
                                rusqlite::params![
                                    message.id,
                                    fresh.last_message_text,
                                    message.timestamp,
                                    message.chat_jid
                                ],
                            )?;
                            Ok(())
                        })
                        .ok();
                    events.push(chat_upserted(fresh));
                    events.push(message_upserted(&message));
                }
            }
            (events, Vec::new())
        }
        Event::Receipt(receipt) => (
            store_receipt(store, receipt).unwrap_or_default(),
            Vec::new(),
        ),
        Event::ChatPresence(presence) => (
            store_chat_presence(store, presence).into_iter().collect(),
            Vec::new(),
        ),
        _ => (Vec::new(), Vec::new()),
    }
}

/// Extracts a reaction from one inbound envelope into the store. Answers
/// true when the envelope was a reaction (so the caller skips storing it).
/// Encrypted reactions are handled asynchronously by the caller: they need
/// the parent message's secret, which the store answers asynchronously.
fn reaction_from_inbound(
    store: &Store,
    identity: &crate::identity::Identity,
    inbound: &InboundMessage,
    events: &mut Vec<BackendEvent>,
) -> bool {
    let content = &inbound.message;
    let info = &inbound.info;
    let (canonical, _folds) =
        crate::reconcile::canonical_chat(store, identity, &jid_text(&info.source.chat));
    let sender_jid = jid_text(&info.source.sender.to_non_ad());
    let from_me = info.source.is_from_me;
    let now = crate::db::now_ms();

    if let Some(reaction) = content.reaction_message.as_option() {
        let target = reaction
            .key
            .as_option()
            .and_then(|key| key.id.clone())
            .unwrap_or_default();
        let emoji = reaction.text.clone().unwrap_or_default();
        apply_reaction(
            store, events, canonical, target, sender_jid, emoji, from_me, now,
        );
        return true;
    }
    // Encrypted reactions are resolved by the caller with the client's
    // store: this sync pass only recognizes them.
    content.enc_reaction_message.is_set()
}

/// The shared write+emit of one reaction change.
#[expect(clippy::too_many_arguments, reason = "one argument per reaction field")]
pub(crate) fn apply_reaction(
    store: &Store,
    events: &mut Vec<BackendEvent>,
    chat_jid: String,
    target: String,
    sender_jid: String,
    emoji: String,
    from_me: bool,
    timestamp: i64,
) {
    if target.is_empty() {
        return;
    }
    let reaction = ReactionRow {
        chat_jid: chat_jid.clone(),
        message_id: target,
        sender_jid,
        emoji: emoji.clone(),
        timestamp,
        from_me,
    };
    store.set_reaction(&reaction).ok();
    events.push(reaction_updated(&reaction, emoji.is_empty()));
}

/// The chat-list preview of a message, same rule as the store's.
fn preview_of(message: &MessageRow) -> String {
    if !message.text.is_empty() {
        return message.text.clone();
    }
    if !message.image_caption.is_empty() {
        return message.image_caption.clone();
    }
    if !message.media_caption.is_empty() {
        return message.media_caption.clone();
    }
    match message.kind.as_str() {
        "image" => "📷 Photo".into(),
        "sticker" => "Sticker".into(),
        "video" => "🎬 Video".into(),
        "audio" | "voice" => "🎤 Voice message".into(),
        "document" => "📄 Document".into(),
        "location" => "📍 Location".into(),
        "contacts" => "Contacts".into(),
        "poll" => "📊 Poll".into(),
        "revoked" => "Message deleted".into(),
        _ => String::new(),
    }
}

/// Resolves the encrypted reactions of one batch against the client's
/// message-secret store, in a spawned task (the lookup is async and the
/// event handler is sync). The results land in the same event queue and the
/// same store the live path uses.
pub fn spawn_enc_reactions(
    store: std::sync::Arc<Store>,
    client_slot: std::sync::Arc<std::sync::OnceLock<std::sync::Arc<whatsapp_rust::client::Client>>>,
    identity: std::sync::Arc<crate::identity::Identity>,
    events: async_channel::Sender<BackendEvent>,
    batch: &wacore::types::events::MessageBatch,
) {
    let pending: Vec<wacore::types::events::InboundMessage> = batch
        .messages
        .iter()
        .filter(|inbound| inbound.message.enc_reaction_message.is_set())
        .cloned()
        .collect();
    if pending.is_empty() {
        return;
    }
    let Some(client) = client_slot.get().cloned() else {
        tracing::debug!(
            "{} encrypted reaction(s) dropped: no client yet",
            pending.len()
        );
        return;
    };
    tokio::spawn(async move {
        for inbound in pending {
            if let Err(error) =
                apply_enc_reaction(&store, &client, &identity, &events, &inbound).await
            {
                tracing::warn!("an encrypted reaction failed to decrypt: {error}");
            }
        }
    });
}

async fn apply_enc_reaction(
    store: &Store,
    client: &whatsapp_rust::client::Client,
    identity: &crate::identity::Identity,
    events: &async_channel::Sender<BackendEvent>,
    inbound: &InboundMessage,
) -> anyhow::Result<()> {
    let content = &inbound.message;
    let Some(enc) = content.enc_reaction_message.as_option() else {
        return Ok(());
    };
    let info = &inbound.info;
    // The secrets live under the jid the envelope carried; the reaction
    // itself belongs to the canonical chat the conversation lives in.
    let chat_jid = jid_text(&info.source.chat);
    let (canonical, _folds) = crate::reconcile::canonical_chat(store, identity, &chat_jid);
    let reactor = jid_text(&info.source.sender.to_non_ad());
    let key = enc.target_message_key.as_option();
    let target = key
        .and_then(|target_key| target_key.id.clone())
        .unwrap_or_default();
    if target.is_empty() {
        return Ok(());
    }
    let participant = key
        .and_then(|target_key| target_key.participant.clone())
        .unwrap_or_default();
    let payload = enc.enc_payload.clone().unwrap_or_default();
    let iv = enc.enc_iv.clone().unwrap_or_default();
    let backend = client.persistence_manager().backend();
    // The secret lives under the parent message's sender key; the store
    // convention for 1:1 parents is the chat jid itself.
    let secret = match backend.get_msg_secret(&chat_jid, &chat_jid, &target).await {
        Ok(Some(secret)) => Some(secret),
        _ => match backend
            .get_msg_secret(&chat_jid, &participant, &target)
            .await
        {
            Ok(Some(secret)) => Some(secret),
            _ => backend
                .get_msg_secret(&chat_jid, &reactor, &target)
                .await
                .ok()
                .flatten(),
        },
    };
    let Some(secret) = secret else {
        anyhow::bail!("no secret for {target}");
    };
    let parent_sender = if participant.is_empty() {
        chat_jid.clone()
    } else {
        participant
    };
    let reaction = wacore::reaction::decrypt_reaction_with_secret(
        &payload,
        &iv,
        &secret,
        &target,
        &parent_sender,
        &reactor,
    )?;
    let emoji = reaction.text.clone().unwrap_or_default();
    let from_me = info.source.is_from_me;
    let mut out = Vec::new();
    apply_reaction(
        store,
        &mut out,
        canonical,
        target,
        reactor,
        emoji,
        from_me,
        crate::db::now_ms(),
    );
    for event in out {
        let _ = events.send_blocking(event);
    }
    Ok(())
}
