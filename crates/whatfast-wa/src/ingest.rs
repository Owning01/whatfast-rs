//! Turns the library's events into store rows and bridge events.
//!
//! The classification table mirrors the Go sidecar's `domainMessage` /
//! `extractMessageContent`: same kinds, same fallback texts, same handling of
//! edits and revokes inside `ProtocolMessage` wrappers, same labels for the
//! content the contract has no bespoke rendering for. The history walk mirrors
//! the Go's history-sync ingestion: every conversation becomes a chat, every
//! visible message a row, the aggregate reactions land on their parent, and the
//! desktop hears about both.

use std::sync::Arc;

use wacore::types::events::{ChatPresenceUpdate, Event, InboundMessage, Receipt};
use wacore::types::message::{MessageInfo, StanzaMessageType};
use wacore::types::presence::{ChatActivity, ReceiptType};
use whatfast_protocol::{
    BackendEvent, ChatUpserted, MessageStatus, MessageUpserted, TypingChanged,
};
use whatsapp_rust::waproto::whatsapp as wa;

use crate::db::{ChatFlag, ChatRow, MessageRow, ReactionRow, Store};

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

/// The Go's `firstNonEmpty`: the first candidate, or the fallback.
fn first_non_empty<'a>(candidates: &[&'a str], fallback: &'a str) -> &'a str {
    candidates
        .iter()
        .copied()
        .find(|candidate| !candidate.is_empty())
        .unwrap_or(fallback)
}

/// The poll payload regardless of which versioned field WhatsApp used, the
/// way the Go's `messagePoll` walks them.
fn message_poll(message: &wa::Message) -> Option<&wa::message::PollCreationMessage> {
    let direct = message
        .poll_creation_message
        .as_option()
        .or_else(|| message.poll_creation_message_v2.as_option())
        .or_else(|| message.poll_creation_message_v3.as_option())
        .or_else(|| message.poll_creation_message_v5.as_option())
        .or_else(|| message.poll_creation_message_v6.as_option());
    if let Some(poll) = direct {
        return Some(poll);
    }
    let wrapped = message
        .poll_creation_message_v4
        .as_option()
        .and_then(|wrapper| wrapper.message.as_option())?;
    message_poll(wrapped)
}

/// The poll-result snapshot, whichever of the two fields carries it.
fn message_poll_snapshot(message: &wa::Message) -> Option<&wa::message::PollResultSnapshotMessage> {
    message
        .poll_result_snapshot_message
        .as_option()
        .or_else(|| message.poll_result_snapshot_message_v3.as_option())
}

/// A PTV (round video note) is a video attachment the UI plays round.
fn message_video(message: &wa::Message) -> Option<&wa::message::VideoMessage> {
    message
        .video_message
        .as_option()
        .or_else(|| message.ptv_message.as_option())
}

fn message_document(message: &wa::Message) -> Option<&wa::message::DocumentMessage> {
    message.document_message.as_option().or_else(|| {
        message
            .document_with_caption_message
            .as_option()
            .and_then(|wrapper| wrapper.message.as_option())
            .and_then(|inner| inner.document_message.as_option())
    })
}

/// The Go's `pollText`: the question and one bullet per named option.
fn poll_text(poll: &wa::message::PollCreationMessage) -> String {
    let mut text = join_non_empty(": ", &["📊 Poll", &s(&poll.name)]);
    for option in &poll.options {
        let name = s(&option.option_name);
        if !name.is_empty() {
            text.push_str("\n• ");
            text.push_str(&name);
        }
    }
    text
}

/// The Go's `callLogText`, duration suffix included.
fn call_log_text(call_log: &wa::message::CallLogMessage) -> String {
    let video = call_log.is_video.unwrap_or(false);
    let outcome = call_log.call_outcome;
    let mut text = match outcome {
        Some(wa::message::call_log_message::CallOutcome::Missed) => {
            if video {
                "Missed video call".to_owned()
            } else {
                "Missed call".to_owned()
            }
        }
        _ => {
            let mut text = if video {
                "📹 Video call".to_owned()
            } else {
                "📞 Call".to_owned()
            };
            if outcome == Some(wa::message::call_log_message::CallOutcome::Rejected) {
                text.push_str(" (declined)");
            }
            text
        }
    };
    let seconds = call_log.duration_secs.unwrap_or(0);
    if seconds > 0 {
        text.push_str(&format!(" ({})", go_duration_string(seconds as u64)));
    }
    text
}

/// The Go's `time.Duration.String()` for a whole number of seconds, so the
/// call labels match character for character.
fn go_duration_string(seconds: u64) -> String {
    if seconds < 60 {
        return format!("{seconds}s");
    }
    let minutes = seconds / 60;
    if minutes < 60 {
        return format!("{minutes}m{}s", seconds % 60);
    }
    format!("{}h{}m{}s", minutes / 60, minutes % 60, seconds % 60)
}

fn album_text(album: &wa::message::AlbumMessage) -> String {
    let total = album.expected_image_count.unwrap_or(0) + album.expected_video_count.unwrap_or(0);
    if total > 0 {
        format!("🖼️ Album ({total} items)")
    } else {
        "🖼️ Album".to_owned()
    }
}

/// The Go's `ephemeralSettingText`: off, or days/hours/minutes.
fn ephemeral_setting_text(seconds: u32) -> String {
    if seconds == 0 {
        return "⏳ Disappearing messages turned off".to_owned();
    }
    let duration = u64::from(seconds);
    if duration >= 24 * 60 * 60 {
        let days = duration / (24 * 60 * 60);
        if days == 1 {
            "⏳ Disappearing messages set to 1 day".to_owned()
        } else {
            format!("⏳ Disappearing messages set to {days} days")
        }
    } else if duration >= 60 * 60 {
        format!(
            "⏳ Disappearing messages set to {} hours",
            duration / (60 * 60)
        )
    } else {
        format!("⏳ Disappearing messages set to {} minutes", duration / 60)
    }
}

/// The Go's `messageInlineText`: the plain body without decoding anything
/// richer.
fn inline_text(message: &wa::Message) -> String {
    if let Some(conversation) = &message.conversation
        && !conversation.is_empty()
    {
        return conversation.clone();
    }
    message
        .extended_text_message
        .as_option()
        .map(|extended| s(&extended.text))
        .unwrap_or_default()
}

/// The `ContextInfo` of the first populated content field, in the Go's
/// `messageContextInfo` order.
fn message_context_info(message: &wa::Message) -> Option<&wa::ContextInfo> {
    if let Some(extended) = message.extended_text_message.as_option() {
        return extended.context_info.as_option();
    }
    if let Some(image) = message.image_message.as_option() {
        return image.context_info.as_option();
    }
    if let Some(sticker) = message.sticker_message.as_option() {
        return sticker.context_info.as_option();
    }
    if let Some(video) = message_video(message) {
        return video.context_info.as_option();
    }
    if let Some(audio) = message.audio_message.as_option() {
        return audio.context_info.as_option();
    }
    if let Some(document) = message_document(message) {
        return document.context_info.as_option();
    }
    [
        message
            .contact_message
            .as_option()
            .and_then(|contact| contact.context_info.as_option()),
        message
            .contacts_array_message
            .as_option()
            .and_then(|contacts| contacts.context_info.as_option()),
        message
            .location_message
            .as_option()
            .and_then(|location| location.context_info.as_option()),
        message
            .live_location_message
            .as_option()
            .and_then(|live| live.context_info.as_option()),
        message_poll(message).and_then(|poll| poll.context_info.as_option()),
        message_poll_snapshot(message).and_then(|snapshot| snapshot.context_info.as_option()),
        message
            .group_invite_message
            .as_option()
            .and_then(|invite| invite.context_info.as_option()),
        message
            .event_message
            .as_option()
            .and_then(|event| event.context_info.as_option()),
        message
            .event_invite_message
            .as_option()
            .and_then(|invite| invite.context_info.as_option()),
        message
            .buttons_message
            .as_option()
            .and_then(|buttons| buttons.context_info.as_option()),
        message
            .buttons_response_message
            .as_option()
            .and_then(|response| response.context_info.as_option()),
        message
            .list_message
            .as_option()
            .and_then(|list| list.context_info.as_option()),
        message
            .list_response_message
            .as_option()
            .and_then(|response| response.context_info.as_option()),
        message
            .template_message
            .as_option()
            .and_then(|template| template.context_info.as_option()),
        message
            .template_button_reply_message
            .as_option()
            .and_then(|reply| reply.context_info.as_option()),
        message
            .interactive_message
            .as_option()
            .and_then(|interactive| interactive.context_info.as_option()),
        message
            .interactive_response_message
            .as_option()
            .and_then(|response| response.context_info.as_option()),
        message
            .product_message
            .as_option()
            .and_then(|product| product.context_info.as_option()),
        message
            .order_message
            .as_option()
            .and_then(|order| order.context_info.as_option()),
        message
            .newsletter_admin_invite_message
            .as_option()
            .and_then(|invite| invite.context_info.as_option()),
        message
            .album_message
            .as_option()
            .and_then(|album| album.context_info.as_option()),
        message
            .sticker_pack_message
            .as_option()
            .and_then(|pack| pack.context_info.as_option()),
    ]
    .into_iter()
    .flatten()
    .next()
}

/// The proto field name of the first populated content payload, skipping the
/// fields that only carry signal/keying data. Mirrors the Go's
/// `messageContentFieldName` walk, in field-number order.
fn message_content_field_name(message: &wa::Message) -> &'static str {
    macro_rules! pick {
        ($($field:ident => $name:literal),* $(,)?) => {
            $(
                if message.$field.is_set() {
                    return $name;
                }
            )*
        };
    }
    pick!(
        image_message => "imageMessage",
        contact_message => "contactMessage",
        location_message => "locationMessage",
        extended_text_message => "extendedTextMessage",
        document_message => "documentMessage",
        audio_message => "audioMessage",
        video_message => "videoMessage",
        call => "call",
        chat => "chat",
        protocol_message => "protocolMessage",
        contacts_array_message => "contactsArrayMessage",
        highly_structured_message => "highlyStructuredMessage",
        send_payment_message => "sendPaymentMessage",
        live_location_message => "liveLocationMessage",
        request_payment_message => "requestPaymentMessage",
        decline_payment_request_message => "declinePaymentRequestMessage",
        cancel_payment_request_message => "cancelPaymentRequestMessage",
        template_message => "templateMessage",
        sticker_message => "stickerMessage",
        group_invite_message => "groupInviteMessage",
        template_button_reply_message => "templateButtonReplyMessage",
        product_message => "productMessage",
        list_message => "listMessage",
        view_once_message => "viewOnceMessage",
        order_message => "orderMessage",
        list_response_message => "listResponseMessage",
        ephemeral_message => "ephemeralMessage",
        invoice_message => "invoiceMessage",
        buttons_message => "buttonsMessage",
        buttons_response_message => "buttonsResponseMessage",
        payment_invite_message => "paymentInviteMessage",
        interactive_message => "interactiveMessage",
        reaction_message => "reactionMessage",
        sticker_sync_rmr_message => "stickerSyncRmrMessage",
        interactive_response_message => "interactiveResponseMessage",
        poll_creation_message => "pollCreationMessage",
        poll_update_message => "pollUpdateMessage",
        keep_in_chat_message => "keepInChatMessage",
        document_with_caption_message => "documentWithCaptionMessage",
        request_phone_number_message => "requestPhoneNumberMessage",
        view_once_message_v2 => "viewOnceMessageV2",
        enc_reaction_message => "encReactionMessage",
        edited_message => "editedMessage",
        view_once_message_v2_extension => "viewOnceMessageV2Extension",
        poll_creation_message_v2 => "pollCreationMessageV2",
        scheduled_call_creation_message => "scheduledCallCreationMessage",
        group_mentioned_message => "groupMentionedMessage",
        pin_in_chat_message => "pinInChatMessage",
        poll_creation_message_v3 => "pollCreationMessageV3",
        scheduled_call_edit_message => "scheduledCallEditMessage",
        ptv_message => "ptvMessage",
        bot_invoke_message => "botInvokeMessage",
        call_log_messsage => "callLogMesssage",
        message_history_bundle => "messageHistoryBundle",
        enc_comment_message => "encCommentMessage",
        bcall_message => "bcallMessage",
        lottie_sticker_message => "lottieStickerMessage",
        event_message => "eventMessage",
        enc_event_response_message => "encEventResponseMessage",
        comment_message => "commentMessage",
        newsletter_admin_invite_message => "newsletterAdminInviteMessage",
        placeholder_message => "placeholderMessage",
        secret_encrypted_message => "secretEncryptedMessage",
        album_message => "albumMessage",
        event_cover_image => "eventCoverImage",
        sticker_pack_message => "stickerPackMessage",
        status_mention_message => "statusMentionMessage",
        poll_result_snapshot_message => "pollResultSnapshotMessage",
        poll_creation_option_image_message => "pollCreationOptionImageMessage",
        associated_child_message => "associatedChildMessage",
        group_status_mention_message => "groupStatusMentionMessage",
        poll_creation_message_v4 => "pollCreationMessageV4",
        status_add_yours => "statusAddYours",
        group_status_message => "groupStatusMessage",
        rich_response_message => "richResponseMessage",
        status_notification_message => "statusNotificationMessage",
        limit_sharing_message => "limitSharingMessage",
        bot_task_message => "botTaskMessage",
        question_message => "questionMessage",
        message_history_notice => "messageHistoryNotice",
        group_status_message_v2 => "groupStatusMessageV2",
        bot_forwarded_message => "botForwardedMessage",
        status_question_answer_message => "statusQuestionAnswerMessage",
        question_reply_message => "questionReplyMessage",
        question_response_message => "questionResponseMessage",
        status_quoted_message => "statusQuotedMessage",
        status_sticker_interaction_message => "statusStickerInteractionMessage",
        poll_creation_message_v5 => "pollCreationMessageV5",
        newsletter_follower_invite_message_v2 => "newsletterFollowerInviteMessageV2",
        poll_result_snapshot_message_v3 => "pollResultSnapshotMessageV3",
        newsletter_admin_profile_message => "newsletterAdminProfileMessage",
        spoiler_message => "spoilerMessage",
        poll_creation_message_v6 => "pollCreationMessageV6",
        conditional_reveal_message => "conditionalRevealMessage",
        poll_add_option_message => "pollAddOptionMessage",
        event_invite_message => "eventInviteMessage",
        group_root_key_share => "groupRootKeyShare",
        payment_reminder_message => "paymentReminderMessage",
        split_payment_message => "splitPaymentMessage",
        newsletter_admin_profile_status_message => "newsletterAdminProfileStatusMessage",
        root_secret_distribute_message => "rootSecretDistributeMessage",
        split_payment_update_message => "splitPaymentUpdateMessage",
        music_message => "musicMessage",
        status_link_preview_metadata => "statusLinkPreviewMetadata",
        bot_platform_registration_success_message => "botPlatformRegistrationSuccessMessage",
        newsletter_scheduled_message => "newsletterScheduledMessage",
        acp2_setting_message => "acp2SettingMessage",
        audio_sticker_message => "audioStickerMessage",
    );
    ""
}

/// The Go's `humanizeMessageField`: `pollResultSnapshotMessageV3` becomes
/// `Poll result snapshot`.
fn humanize_message_field(name: &str) -> String {
    let trimmed = name.trim_end_matches(|character: char| character.is_ascii_digit());
    let trimmed = trimmed.strip_suffix('V').unwrap_or(trimmed);
    let trimmed = trimmed.strip_suffix("Message").unwrap_or(trimmed);
    let trimmed = if trimmed.is_empty() { name } else { trimmed };
    let mut out = String::with_capacity(trimmed.len() + 4);
    for (index, character) in trimmed.chars().enumerate() {
        if index == 0 {
            out.push(character.to_ascii_uppercase());
        } else if character.is_ascii_uppercase() {
            out.push(' ');
            out.push(character.to_ascii_lowercase());
        } else {
            out.push(character);
        }
    }
    out
}

/// The Go's `fallbackMessageContent`: a label derived from whichever payload
/// field is populated, so message types with no bespoke rendering still show
/// what they are.
fn fallback_message_content(message: &wa::Message, stanza_type: &str) -> (String, String) {
    let name = message_content_field_name(message);
    if name.is_empty() {
        let kind = if stanza_type.is_empty() {
            "unknown"
        } else {
            stanza_type
        };
        return (kind.to_owned(), "Message".to_owned());
    }
    let label = humanize_message_field(name);
    (label.to_lowercase().replace(' ', "_"), label)
}

/// The Go's `passiveMessage`: payloads that only carry protocol or state
/// signals WhatsApp itself never renders as a chat bubble.
///
/// The poll option update is listed here because this port does not apply it
/// to the polls table yet: dropping it leaves no bubble, like the oracle, and
/// the option returns with the next full sync.
fn passive_message(message: &wa::Message) -> bool {
    if let Some(protocol) = message.protocol_message.as_option() {
        let kind = protocol.r#type;
        return !(kind == Some(wa::message::protocol_message::Type::Revoke)
            || kind == Some(wa::message::protocol_message::Type::MessageEdit)
            || kind == Some(wa::message::protocol_message::Type::EphemeralSetting));
    }
    if message.poll_update_message.is_set()
        || message.poll_add_option_message.is_set()
        || message.enc_reaction_message.is_set()
        || message.enc_comment_message.is_set()
        || message.enc_event_response_message.is_set()
        || message.sticker_sync_rmr_message.is_set()
        || message.secret_encrypted_message.is_set()
    {
        return true;
    }
    inline_text(message).is_empty() && message_content_field_name(message).is_empty()
}

/// Classifies one (possibly wrapped) content message into the store's shape,
/// mirroring the Go's `extractMessageContent` order and fallback texts.
fn classify(content: &wa::Message, stanza_type: &str) -> Content {
    let mut out = Content::default();

    if let Some(image) = content.image_message.as_option() {
        out.kind = "image".into();
        out.text = first_non_empty(&[&s(&image.caption)], "📷 Photo").to_owned();
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
    if let Some(video) = message_video(content) {
        out.kind = "video".into();
        out.text = first_non_empty(&[&s(&video.caption)], "🎬 Video").to_owned();
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
    if let Some(document) = message_document(content) {
        out.kind = "document".into();
        let file_name = s(&document.file_name);
        let title = s(&document.title);
        out.text = [
            s(&document.caption),
            if file_name.is_empty() {
                title
            } else {
                file_name
            },
        ]
        .into_iter()
        .find(|candidate| !candidate.is_empty())
        .unwrap_or_else(|| "📄 Document".into());
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
        out.text = first_non_empty(&[&s(&contact.display_name)], "Contact").to_owned();
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
        out.text = first_non_empty(&[&s(&live.caption)], "📍 Live location").to_owned();
        out.location = Some(LocationParts {
            lat: live.degrees_latitude.unwrap_or(0.0),
            lng: live.degrees_longitude.unwrap_or(0.0),
            name: s(&live.caption),
            live: true,
            ..Default::default()
        });
        return out;
    }
    if let Some(poll) = message_poll(content) {
        out.kind = "poll".into();
        out.text = poll_text(poll);
        let question = s(&poll.name);
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
    if let Some(snapshot) = message_poll_snapshot(content) {
        out.kind = "poll".into();
        out.text = join_non_empty(": ", &["📊 Poll results", &s(&snapshot.name)]);
        return out;
    }
    if let Some(invite) = content.group_invite_message.as_option() {
        out.kind = "group_invite".into();
        let first = join_non_empty(": ", &["👥 Group invite", &s(&invite.group_name)]);
        out.text = join_non_empty("\n", &[first.as_str(), s(&invite.caption).as_str()]);
        return out;
    }
    if let Some(event) = content.event_message.as_option() {
        out.kind = "event".into();
        let mut text = join_non_empty(": ", &["📅 Event", &s(&event.name)]);
        if event.is_canceled.unwrap_or(false) {
            text.push_str(" (canceled)");
        }
        out.text = join_non_empty("\n", &[text.as_str(), s(&event.description).as_str()]);
        return out;
    }
    if let Some(invite) = content.event_invite_message.as_option() {
        out.kind = "event".into();
        let first = join_non_empty(": ", &["📅 Event invite", &s(&invite.event_title)]);
        out.text = join_non_empty("\n", &[first.as_str(), s(&invite.caption).as_str()]);
        return out;
    }
    if let Some(buttons) = content.buttons_message.as_option() {
        out.kind = "interactive".into();
        out.text = first_non_empty(
            &[&s(&buttons.content_text), &s(&buttons.footer_text)],
            "Interactive message",
        )
        .to_owned();
        return out;
    }
    if let Some(response) = content.buttons_response_message.as_option() {
        out.kind = "interactive".into();
        let selected = match response.response.as_ref() {
            Some(wa::message::buttons_response_message::Response::SelectedDisplayText(text)) => {
                text.clone()
            }
            _ => String::new(),
        };
        out.text = first_non_empty(&[&selected], "Button reply").to_owned();
        return out;
    }
    if let Some(list) = content.list_message.as_option() {
        out.kind = "interactive".into();
        let joined = join_non_empty("\n", &[&s(&list.title), &s(&list.description)]);
        out.text = first_non_empty(&[&joined], "List message").to_owned();
        return out;
    }
    if let Some(response) = content.list_response_message.as_option() {
        out.kind = "interactive".into();
        out.text = first_non_empty(&[&s(&response.title)], "List reply").to_owned();
        return out;
    }
    if let Some(template) = content.template_message.as_option() {
        out.kind = "interactive".into();
        let hydrated = template.hydrated_template.as_option();
        let content_text = hydrated.map(|inner| s(&inner.hydrated_content_text));
        let footer_text = hydrated.map(|inner| s(&inner.hydrated_footer_text));
        let content_text = content_text.unwrap_or_default();
        let footer_text = footer_text.unwrap_or_default();
        out.text = first_non_empty(&[&content_text, &footer_text], "Template message").to_owned();
        return out;
    }
    if let Some(reply) = content.template_button_reply_message.as_option() {
        out.kind = "interactive".into();
        out.text = first_non_empty(&[&s(&reply.selected_display_text)], "Button reply").to_owned();
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
        let joined = join_non_empty("\n", &[&header_title, &body]);
        out.text = first_non_empty(&[&joined], "Interactive message").to_owned();
        return out;
    }
    if let Some(response) = content.interactive_response_message.as_option() {
        out.kind = "interactive".into();
        let body = response
            .body
            .as_option()
            .and_then(|body| body.text.clone())
            .unwrap_or_default();
        out.text = first_non_empty(&[&body], "Interactive reply").to_owned();
        return out;
    }
    if let Some(product) = content.product_message.as_option() {
        out.kind = "product".into();
        let title = product
            .product
            .as_option()
            .map(|snapshot| s(&snapshot.title))
            .unwrap_or_default();
        let first = join_non_empty(": ", &["🛍️ Product", &title]);
        out.text = join_non_empty("\n", &[first.as_str(), s(&product.body).as_str()]);
        return out;
    }
    if let Some(order) = content.order_message.as_option() {
        out.kind = "order".into();
        let mut text = join_non_empty(": ", &["🛒 Order", &s(&order.order_title)]);
        let count = order.item_count.unwrap_or(0);
        if count > 0 {
            text.push_str(&format!(" ({count} items)"));
        }
        out.text = join_non_empty("\n", &[text.as_str(), s(&order.message).as_str()]);
        return out;
    }
    if let Some(invoice) = content.invoice_message.as_option() {
        out.kind = "payment".into();
        out.text = join_non_empty(": ", &["🧾 Invoice", &s(&invoice.note)]);
        return out;
    }
    if let Some(payment) = content.send_payment_message.as_option() {
        out.kind = "payment".into();
        let note = payment
            .note_message
            .as_option()
            .map(inline_text)
            .unwrap_or_default();
        out.text = join_non_empty("\n", &["💸 Payment", note.as_str()]);
        return out;
    }
    if let Some(request) = content.request_payment_message.as_option() {
        out.kind = "payment".into();
        let mut text = "💸 Payment request".to_owned();
        let amount = request.amount1000.unwrap_or(0);
        if amount > 0 {
            text = format!(
                "{text}: {} {:.2}",
                s(&request.currency_code_iso4217),
                amount as f64 / 1000.0
            );
        }
        let note = request
            .note_message
            .as_option()
            .map(inline_text)
            .unwrap_or_default();
        out.text = join_non_empty("\n", &[text.as_str(), note.as_str()]);
        return out;
    }
    if content.decline_payment_request_message.is_set() {
        out.kind = "payment".into();
        out.text = "💸 Payment request declined".into();
        return out;
    }
    if content.cancel_payment_request_message.is_set() {
        out.kind = "payment".into();
        out.text = "💸 Payment request cancelled".into();
        return out;
    }
    if content.payment_invite_message.is_set() {
        out.kind = "payment".into();
        out.text = "💸 Payment invite".into();
        return out;
    }
    if content.payment_reminder_message.is_set() {
        out.kind = "payment".into();
        out.text = "💸 Payment reminder".into();
        return out;
    }
    if content.split_payment_message.is_set() {
        out.kind = "payment".into();
        out.text = "💸 Split payment".into();
        return out;
    }
    if content.split_payment_update_message.is_set() {
        out.kind = "payment".into();
        out.text = "💸 Split payment update".into();
        return out;
    }
    if let Some(call_log) = content.call_log_messsage.as_option() {
        out.kind = "call".into();
        out.text = call_log_text(call_log);
        return out;
    }
    if let Some(scheduled) = content.scheduled_call_creation_message.as_option() {
        out.kind = "call".into();
        out.text = join_non_empty(": ", &["📞 Call scheduled", &s(&scheduled.title)]);
        return out;
    }
    if content.scheduled_call_edit_message.is_set() {
        out.kind = "call".into();
        out.text = "📞 Scheduled call updated".into();
        return out;
    }
    if content.bcall_message.is_set() {
        out.kind = "call".into();
        out.text = "📞 Call".into();
        return out;
    }
    if let Some(pin) = content.pin_in_chat_message.as_option() {
        out.kind = "pin".into();
        out.text = if pin.r#type == Some(wa::message::pin_in_chat_message::Type::UnpinForAll) {
            "📌 Unpinned a message".into()
        } else {
            "📌 Pinned a message".into()
        };
        return out;
    }
    if let Some(keep) = content.keep_in_chat_message.as_option() {
        out.kind = "pin".into();
        out.text = if keep.keep_type == Some(wa::KeepType::UndoKeepForAll) {
            "Removed a message from kept messages".into()
        } else {
            "Kept a message in chat".into()
        };
        return out;
    }
    if let Some(comment) = content.comment_message.as_option() {
        let mut inner = comment
            .message
            .as_option()
            .map(|message| classify(message, stanza_type))
            .unwrap_or_else(|| Content {
                text: "Message".into(),
                ..Default::default()
            });
        inner.kind = "comment".into();
        return inner;
    }
    if let Some(invite) = content.newsletter_admin_invite_message.as_option() {
        out.kind = "newsletter".into();
        let first = join_non_empty(
            ": ",
            &["📰 Newsletter admin invite", &s(&invite.newsletter_name)],
        );
        out.text = join_non_empty("\n", &[first.as_str(), s(&invite.caption).as_str()]);
        return out;
    }
    if content.newsletter_follower_invite_message_v2.is_set() {
        out.kind = "newsletter".into();
        out.text = "📰 Newsletter invite".into();
        return out;
    }
    if let Some(album) = content.album_message.as_option() {
        out.kind = "album".into();
        out.text = album_text(album);
        return out;
    }
    if let Some(pack) = content.sticker_pack_message.as_option() {
        out.kind = "sticker_pack".into();
        out.text = join_non_empty(": ", &["🎨 Sticker pack", &s(&pack.name)]);
        return out;
    }
    if content.request_phone_number_message.is_set() {
        out.kind = "text".into();
        out.text = "📱 Requested a phone number".into();
        return out;
    }
    if let Some(protocol) = content.protocol_message.as_option()
        && protocol.r#type == Some(wa::message::protocol_message::Type::EphemeralSetting)
    {
        out.kind = "ephemeral_setting".into();
        out.text = ephemeral_setting_text(protocol.ephemeral_expiration.unwrap_or(0));
        return out;
    }
    // Everything else: the plain body, then the derived label. The link
    // preview rides on the extended text's matched URL, like the Go.
    out.text = inline_text(content);
    if let Some(extended) = content.extended_text_message.as_option()
        && !s(&extended.matched_text).is_empty()
    {
        out.link_preview = Some(LinkPreviewParts {
            url: s(&extended.matched_text),
            title: s(&extended.title),
            description: s(&extended.description),
            thumbnail: extended.jpeg_thumbnail.clone().unwrap_or_default(),
            width: extended.thumbnail_width.unwrap_or(0),
            height: extended.thumbnail_height.unwrap_or(0),
        });
    }
    if out.text.is_empty() {
        let (kind, text) = fallback_message_content(content, stanza_type);
        out.kind = kind;
        out.text = text;
    } else {
        out.kind = "text".into();
    }
    out
}

fn is_edit(info: &MessageInfo) -> bool {
    matches!(
        info.edit,
        wacore::types::message::EditAttribute::MessageEdit
            | wacore::types::message::EditAttribute::AdminEdit
    )
}

/// The stanza `type` attribute the Go passed to `extractMessageContent`; it
/// only reaches the row when no content field is populated.
fn stanza_type(info: &MessageInfo) -> String {
    info.r#type
        .as_ref()
        .map(StanzaMessageType::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// Turns one inbound message into the rows the store takes, applying the
/// edit/revoke unwrapping of the Go's `domainMessage`.
pub fn inbound_to_row(inbound: &InboundMessage, mine: &str) -> Option<Ingested> {
    let info = &inbound.info;
    let raw_content: &wa::Message = &inbound.message;
    let protocol = raw_content.protocol_message.as_option();
    let mut content: &wa::Message = raw_content;
    if is_edit(info)
        && let Some(protocol_message) = protocol
        && let Some(edited) = protocol_message.edited_message.as_option()
    {
        content = edited;
    }
    let payload = whatsapp_rust::waproto::codec::message_to_vec(content);
    let classified = classify(content, &stanza_type(info));
    let timestamp = info.timestamp.timestamp_millis();
    let mut message = MessageRow {
        id: info.id.to_string(),
        chat_jid: jid_text(&info.source.chat),
        transport_jid: jid_text(&info.source.chat),
        sender_jid: jid_text(&info.source.sender.to_non_ad()),
        timestamp,
        from_me: info.source.is_from_me,
        status: INBOUND_STATUS,
        // The live path is the Go's `incrementUnread=true`: a new incoming
        // row grows the chat counter once, and the store owns the `!existed`
        // half of the rule.
        unread: !info.source.is_from_me,
        ..MessageRow::default()
    };
    // The quoted message the author answered: the Go copied it off the first
    // populated content field's ContextInfo.
    if let Some(context) = message_context_info(content) {
        message.reply_to_id = s(&context.stanza_id);
        message.reply_to_chat_id = s(&context.remote_jid);
        let score = i64::from(context.forwarding_score.unwrap_or(0));
        message.forwarding_score = if context.is_forwarded.unwrap_or(false) && score == 0 {
            1
        } else {
            score
        };
    }
    // Pin events point at the pinned message through their own key.
    if let Some(pin) = content.pin_in_chat_message.as_option() {
        let target = pin.key.as_option().and_then(|key| key.id.clone());
        if let Some(target) = target {
            message.reply_to_id = target;
            message.reply_to_chat_id = message.chat_jid.clone();
        }
    }
    // An edit carries its replacement under the original message's id.
    if is_edit(info)
        && let Some(protocol_message) = protocol
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
    // A revoke marks the original message gone and stamps the event time, the
    // way the Go's reducer recorded the mutation.
    if let Some(protocol_message) = protocol
        && protocol_message.r#type == Some(wa::message::protocol_message::Type::Revoke)
    {
        let target = protocol_message
            .key
            .as_option()
            .and_then(|key| key.id.clone());
        if let Some(target) = target {
            message.id = target;
            message.kind = "revoked".into();
            message.text = "Message deleted".into();
            message.revoked = true;
            message.edited_at = timestamp;
        }
    }
    if message.kind.is_empty() {
        message.kind = classified.kind.clone();
        message.text = classified.text.clone();
    }
    message.payload = payload;
    apply_content(&mut message, &classified);

    let chat = ChatRow {
        jid: message.chat_jid.clone(),
        preferred_jid: message.transport_jid.clone(),
        ..ChatRow::default()
    };
    let _ = mine;
    Some(Ingested { chat, message })
}

/// Persists the definition of one stored poll message — question, options,
/// creator and key — so a later vote finds what the Go rebuilt off its
/// stored creation. Snapshots carry no definition and are skipped; a row
/// without key material keeps whatever the sends wave stored, so an echo
/// never clobbers the real secret with an empty one.
pub(crate) fn persist_poll_definition(store: &Store, row: &MessageRow) {
    if row.kind != "poll" || row.payload.is_empty() {
        return;
    }
    let Ok(message) = whatsapp_rust::waproto::codec::message_decode(&row.payload) else {
        return;
    };
    let Some(poll) = message_poll(&message) else {
        return;
    };
    let question = s(&poll.name);
    if question.is_empty() {
        return;
    }
    let options = poll
        .options
        .iter()
        .map(|option| s(&option.option_name))
        .collect::<Vec<_>>();
    if options.len() < 2 {
        return;
    }
    let selectable = poll.selectable_options_count.unwrap_or(1).max(1);
    store
        .upsert_poll(&row.chat_jid, &row.id, &question, &options, selectable)
        .ok();
    // The key lives on the message envelope, not on the poll content: the
    // phone's `MessageContextInfo` carries the secret the votes encrypt with.
    let secret = message
        .message_context_info
        .as_option()
        .and_then(|context| context.message_secret.clone())
        .unwrap_or_default();
    if !secret.is_empty() || !row.sender_jid.is_empty() {
        store
            .set_poll_meta(&row.chat_jid, &row.id, &row.sender_jid, &secret)
            .ok();
    }
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
///
/// The field list is pinned by `tests/identity_reconcile.rs` (another
/// partition's file), so the conversation-level metadata and the aggregate
/// reactions travel through [`learn_history_contacts`], which already receives
/// the whole chunk.
pub struct HistoryConversation {
    pub jid: String,
    pub unread: u32,
    pub end_of_history_transfer: bool,
    pub messages: Vec<MessageRow>,
}

/// Learns the push names the history sync carries in bulk, plus the metadata
/// only the whole chunk knows: the group title, the archive flag and the
/// aggregate reactions.
///
/// The Go applied those per conversation (`UpsertChatMetadata`,
/// `ApplyReactions`). This pass covers the conversations whose id is already
/// canonical — groups and newsletters. A direct chat keyed by the hidden
/// identity needs the identity map to fold into its phone-number twin, and
/// that map only reaches [`store_conversation`]: those fields stay pending
/// there.
pub fn learn_history_contacts(store: &Store, parsed: &wa::HistorySync) {
    for pushname in &parsed.pushnames {
        let jid = pushname.id.clone().unwrap_or_default();
        let name = pushname.pushname.clone().unwrap_or_default();
        if jid.is_empty() || name.is_empty() || name == "-" || name == "username" {
            continue;
        }
        store.learn_contact(&jid, "", &name, "").ok();
    }
    for conversation in &parsed.conversations {
        let chat_jid = conversation.id.clone();
        if !chat_jid.ends_with("@g.us") && !chat_jid.ends_with("@newsletter") {
            continue;
        }
        let name = conversation.name.clone().unwrap_or_default();
        if !name.is_empty() {
            store.set_chat_name(&chat_jid, &name).ok();
        }
        if let Some(archived) = conversation.archived {
            store
                .set_chat_flag(&chat_jid, ChatFlag::Archived, archived)
                .ok();
        }
        for entry in &conversation.messages {
            let Some(web_message) = entry.message.as_option() else {
                continue;
            };
            let key = web_message.key.as_option();
            let Some(message_id) = key.and_then(|key| key.id.clone()) else {
                continue;
            };
            let message_participant = key.and_then(|key| key.participant.clone());
            for raw in &web_message.reactions {
                let reaction_key = raw.key.as_option();
                let reacted_by_me = reaction_key.and_then(|key| key.from_me).unwrap_or(false);
                let reactor = if reacted_by_me {
                    reaction_key
                        .and_then(|key| key.participant.clone())
                        .or_else(|| message_participant.clone())
                        .unwrap_or_else(|| chat_jid.clone())
                } else {
                    reaction_key
                        .and_then(|key| key.participant.clone())
                        .unwrap_or_default()
                };
                if reactor.is_empty() {
                    continue;
                }
                let reaction_at = match raw.sender_timestamp_ms.unwrap_or(0) {
                    at if at > 0 => at,
                    _ => web_message.message_timestamp.unwrap_or(0) as i64 * 1000,
                };
                store
                    .set_reaction(&ReactionRow {
                        chat_jid: chat_jid.clone(),
                        message_id: message_id.clone(),
                        sender_jid: reactor,
                        emoji: s(&raw.text),
                        timestamp: reaction_at,
                        from_me: reacted_by_me,
                    })
                    .ok();
            }
        }
    }
}

/// Walks a parsed history sync and returns every conversation with rows.
/// Signal-only payloads are skipped, like the Go skipped them.
pub fn history_conversations(parsed: &wa::HistorySync) -> Vec<HistoryConversation> {
    let mut out = Vec::new();
    for conversation in &parsed.conversations {
        let chat_jid = conversation.id.clone();
        if chat_jid.is_empty() {
            continue;
        }
        let mut messages = Vec::new();
        for entry in &conversation.messages {
            let Some(web_message) = entry.message.as_option() else {
                continue;
            };
            let Some(message_proto) = web_message.message.as_option() else {
                continue;
            };
            let key = web_message.key.as_option();
            let Some(message_id) = key.and_then(|key| key.id.clone()) else {
                continue;
            };
            let from_me = key.and_then(|key| key.from_me).unwrap_or(false);
            let sender = if from_me {
                String::new()
            } else {
                key.and_then(|key| key.participant.clone())
                    .unwrap_or_else(|| chat_jid.clone())
            };
            if passive_message(message_proto) {
                continue;
            }
            let classified = classify(message_proto, "");
            let mut row = MessageRow {
                id: message_id,
                chat_jid: chat_jid.clone(),
                transport_jid: chat_jid.clone(),
                sender_jid: sender,
                timestamp: web_message.message_timestamp.unwrap_or(0) as i64 * 1000,
                from_me,
                status: INBOUND_STATUS,
                kind: classified.kind.clone(),
                text: classified.text.clone(),
                // History never grows the unread counter: the Go applied the
                // whole batch with `incrementUnread=false`.
                unread: false,
                ..MessageRow::default()
            };
            row.payload = whatsapp_rust::waproto::codec::message_to_vec(message_proto);
            apply_content(&mut row, &classified);
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
    for message in &conversation.messages {
        let mut row = message.clone();
        row.chat_jid = canonical.clone();
        row.reply_to_chat_id = reply_chat_id(store, identity, &canonical, &row);
        store.insert_message(&row)?;
        persist_poll_definition(store, &row);
    }
    let chat = store.chat(&canonical)?.unwrap_or_default();
    // The oracle ignores the conversation's `unreadCount`: the history rows
    // land with `incrementUnread=false`, so the counter only grows with live
    // traffic. The count stays on the struct for the sync report.
    Ok((chat_upserted(chat), conversation.messages.len() as u64))
}

/// The Go's `resolveMessageReplyChat`: no target clears the pair, a target
/// with no source chat points at the message's own chat, and a source chat is
/// resolved to the conversation the store keys.
fn reply_chat_id(
    store: &Store,
    identity: &crate::identity::Identity,
    chat_jid: &str,
    message: &MessageRow,
) -> String {
    if message.reply_to_id.is_empty() {
        return String::new();
    }
    if message.reply_to_chat_id.is_empty() {
        return chat_jid.to_owned();
    }
    let (canonical, _folds) =
        crate::reconcile::canonical_chat(store, identity, &message.reply_to_chat_id);
    canonical
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
///
/// The mapping follows the Go's `reduceReceipt`: only `read` (and the
/// self-read a peer would never send) is a read; every other receipt,
/// `played` included, reports a delivery. `Sent` is the LID-incapable
/// downgrade, which the oracle also promoted to `Delivered`.
pub fn receipt_status(kind: &ReceiptType) -> Option<MessageStatus> {
    match kind {
        ReceiptType::Read | ReceiptType::ReadSelf => Some(MessageStatus::Read),
        ReceiptType::Delivered
        | ReceiptType::Sent
        | ReceiptType::Sender
        | ReceiptType::Played
        | ReceiptType::PlayedSelf => Some(MessageStatus::Delivered),
        _ => None,
    }
}

/// Applies a receipt to the store and answers with the wire event.
///
/// The Go emitted one `receipt` event per referenced id after a successful
/// update, whether or not the status moved, so the desktop re-reads the row.
pub fn store_receipt(store: &Store, receipt: &Receipt) -> rusqlite::Result<Vec<BackendEvent>> {
    let Some(status) = receipt_status(&receipt.r#type) else {
        return Ok(Vec::new());
    };
    // A read this account produced on another device is a local read marker,
    // not a peer status: the Go cleared the row's unread flag and announced
    // the chat instead of touching the status. That writer lives in the
    // mark-read wave; until it lands these receipts move nothing.
    if receipt.source.is_from_me
        && matches!(receipt.r#type, ReceiptType::Read | ReceiptType::ReadSelf)
    {
        return Ok(Vec::new());
    }
    let chat_jid = store
        .resolve_chat_id(&jid_text(&receipt.source.chat))
        .unwrap_or_else(|_| jid_text(&receipt.source.chat));
    let mut events = Vec::new();
    for message_id in &receipt.message_ids {
        let id = message_id.to_string();
        store.set_message_status(&chat_jid, &id, status as i64)?;
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
    }
    Ok(events)
}

/// Applies a typing state and answers with the wire event.
///
/// Nothing is persisted: the indicators are transient and expire on the
/// desktop side, like the Go's `reduceChatPresence`.
pub fn store_chat_presence(store: &Store, presence: &ChatPresenceUpdate) -> Option<BackendEvent> {
    // The Go dropped our own presence: another device of this account typing
    // is not a peer state the desktop shows.
    if presence.source.is_from_me {
        return None;
    }
    let chat_jid = store
        .resolve_chat_id(&jid_text(&presence.source.chat))
        .unwrap_or_else(|_| jid_text(&presence.source.chat));
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
                // A poll vote, a pin notification or a key-distribution
                // envelope never becomes a bubble.
                if passive_message(&inbound.message) {
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
                let chat = item.chat;
                let mut message = item.message;
                // One person, one chat: the hidden-identity jid of the
                // envelope folds into the phone-number chat when the
                // identity map knows the pair.
                let (canonical, _folds) =
                    crate::reconcile::canonical_chat(store, identity, &chat.jid);
                message.chat_jid = canonical.clone();
                message.reply_to_chat_id = reply_chat_id(store, identity, &canonical, &message);
                store.ensure_chat(&canonical, &chat.preferred_jid).ok();
                // The store advances the chat pointer forward-only and grows
                // the unread counter from the row's own flag; the reducer must
                // not write the pointer itself or an out-of-order message
                // drags the preview backwards.
                if store.insert_message(&message).is_err() {
                    continue;
                }
                persist_poll_definition(store, &message);
                if let Ok(Some(fresh)) = store.chat(&message.chat_jid) {
                    events.push(chat_upserted(fresh));
                }
                // An edit or a revoke merges into an existing row and the
                // desktop still has to hear about it, like the Go's reducer
                // announced every applied message.
                events.push(message_upserted(&message));
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
    // The reaction carries its own event time; the envelope's is the fallback
    // the Go used when the sender stamped none.
    let envelope_at = info.timestamp.timestamp_millis();

    if let Some(reaction) = content.reaction_message.as_option() {
        let target = reaction
            .key
            .as_option()
            .and_then(|key| key.id.clone())
            .unwrap_or_default();
        let emoji = reaction.text.clone().unwrap_or_default();
        let at = match reaction.sender_timestamp_ms.unwrap_or(0) {
            stamp if stamp > 0 => stamp,
            _ => envelope_at,
        };
        // The Go's `ApplyReactionIfNewer` resolved-or-created the conversation:
        // a reaction can arrive before any message of that chat.
        if !target.is_empty() {
            store.ensure_chat(&canonical, &canonical).ok();
        }
        apply_reaction(
            store, events, canonical, target, sender_jid, emoji, from_me, at,
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

/// Resolves the encrypted reactions of one batch against the client's
/// message-secret store, in a spawned task (the lookup is async and the
/// event handler is sync). The results land in the same event queue and the
/// same store the live path uses.
pub fn spawn_enc_reactions(
    store: Arc<Store>,
    client_slot: Arc<std::sync::OnceLock<Arc<whatsapp_rust::client::Client>>>,
    identity: Arc<crate::identity::Identity>,
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
    let at = match reaction.sender_timestamp_ms.unwrap_or(0) {
        stamp if stamp > 0 => stamp,
        _ => info.timestamp.timestamp_millis(),
    };
    // Same resolve-or-create the plain reaction path performs.
    store.ensure_chat(&canonical, &canonical).ok();
    let mut out = Vec::new();
    apply_reaction(
        store, &mut out, canonical, target, reactor, emoji, from_me, at,
    );
    for event in out {
        let _ = events.send_blocking(event);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use wacore::types::events::InboundMessage;
    use wacore::types::message::{MessageInfo, MessageSource};
    use whatsapp_rust::waproto::buffa::MessageField;
    use whatsapp_rust::waproto::whatsapp as wa;

    use super::*;

    fn info(id: &str, from_me: bool, timestamp_ms: i64) -> MessageInfo {
        MessageInfo {
            source: MessageSource {
                chat: "120363@g.us".parse().unwrap(),
                sender: "5491100000000@s.whatsapp.net".parse().unwrap(),
                is_from_me: from_me,
                is_group: true,
                ..Default::default()
            },
            id: id.into(),
            timestamp: whatsapp_rust::chrono::DateTime::from_timestamp_millis(timestamp_ms)
                .unwrap(),
            ..Default::default()
        }
    }

    fn classify_one(content: &wa::Message) -> Content {
        classify(content, "text")
    }

    fn inbound(content: wa::Message, from_me: bool, timestamp_ms: i64) -> InboundMessage {
        InboundMessage::builder()
            .message(Arc::new(content))
            .info(Arc::new(info("MSG1", from_me, timestamp_ms)))
            .build()
    }

    fn row_of(content: wa::Message) -> MessageRow {
        inbound_to_row(&inbound(content, false, 1_700_000_000_000), "")
            .expect("a row")
            .message
    }

    #[test]
    fn a_poll_carries_the_question_and_every_named_option() {
        let mut first = wa::message::poll_creation_message::Option::default();
        first.option_name = Some("Pizza".into());
        let mut second = wa::message::poll_creation_message::Option::default();
        second.option_name = Some("Sushi".into());
        let mut poll = wa::message::PollCreationMessage::default();
        poll.name = Some("Lunch".into());
        poll.options = vec![first, second];
        let mut content = wa::Message::default();
        content.poll_creation_message_v3 = MessageField::some(poll);
        let classified = classify_one(&content);
        assert_eq!(classified.kind, "poll");
        assert_eq!(
            classified.text,
            "\u{1F4CA} Poll: Lunch\n\u{2022} Pizza\n\u{2022} Sushi"
        );
    }

    #[test]
    fn a_stored_poll_keeps_its_definition_and_key() {
        let mut first = wa::message::poll_creation_message::Option::default();
        first.option_name = Some("Pizza".into());
        let mut second = wa::message::poll_creation_message::Option::default();
        second.option_name = Some("Sushi".into());
        let mut poll = wa::message::PollCreationMessage::default();
        poll.name = Some("Lunch".into());
        poll.options = vec![first, second];
        poll.selectable_options_count = Some(1);
        let mut envelope = wa::Message::default();
        envelope.poll_creation_message_v3 = MessageField::some(poll);
        let mut envelope_info = wa::MessageContextInfo::default();
        envelope_info.message_secret = Some(vec![9u8; 32]);
        envelope.message_context_info = MessageField::some(envelope_info);
        let mut row = row_of(envelope);
        row.chat_jid = "g@g.us".into();
        row.id = "p9".into();
        row.sender_jid = "peer@s.whatsapp.net".into();
        let dir = std::env::temp_dir().join(format!(
            "whatfast-wa-poll-key-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::open(&dir.join("chats.db")).unwrap();
        store.ensure_chat("g@g.us", "g@g.us").unwrap();
        store.insert_message(&row).unwrap();
        persist_poll_definition(&store, &row);
        let snapshots = store.poll_snapshots("g@g.us", &["p9".into()]).unwrap();
        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].question, "Lunch");
        let meta = store.poll_meta("g@g.us", "p9").unwrap().unwrap();
        assert_eq!(meta.creator_jid, "peer@s.whatsapp.net");
        assert_eq!(meta.message_secret, vec![9u8; 32]);
    }

    #[test]
    fn an_unknown_payload_gets_the_humanized_field_label() {
        let mut content = wa::Message::default();
        content.placeholder_message =
            MessageField::some(wa::message::PlaceholderMessage::default());
        let classified = classify_one(&content);
        assert_eq!(classified.kind, "placeholder");
        assert_eq!(classified.text, "Placeholder");
    }

    #[test]
    fn the_interactive_family_keeps_the_label_of_the_oracle() {
        let mut buttons = wa::message::ButtonsMessage::default();
        buttons.content_text = Some("Pick one".into());
        let mut response = wa::message::ButtonsResponseMessage::default();
        response.response = Some(
            wa::message::buttons_response_message::Response::SelectedDisplayText("Yes".into()),
        );
        let mut list = wa::message::ListMessage::default();
        list.title = Some("Menu".into());
        list.description = Some("Pick one".into());
        let mut body = wa::message::interactive_response_message::Body::default();
        body.text = Some("Done".into());
        let mut interactive_response = wa::message::InteractiveResponseMessage::default();
        interactive_response.body = MessageField::some(body);

        let mut buttons_content = wa::Message::default();
        buttons_content.buttons_message = MessageField::some(buttons);
        let mut response_content = wa::Message::default();
        response_content.buttons_response_message = MessageField::some(response);
        let mut list_content = wa::Message::default();
        list_content.list_message = MessageField::some(list);
        let mut interactive_content = wa::Message::default();
        interactive_content.interactive_response_message = MessageField::some(interactive_response);

        for (content, text) in [
            (buttons_content, "Pick one"),
            (response_content, "Yes"),
            (list_content, "Menu\nPick one"),
            (interactive_content, "Done"),
        ] {
            let classified = classify_one(&content);
            assert_eq!(classified.kind, "interactive");
            assert_eq!(classified.text, text);
        }
    }

    #[test]
    fn the_long_tail_kinds_match_the_oracle_labels() {
        let mut missed_call = wa::message::CallLogMessage::default();
        missed_call.is_video = Some(true);
        missed_call.call_outcome = Some(wa::message::call_log_message::CallOutcome::Missed);
        let mut timed_call = wa::message::CallLogMessage::default();
        timed_call.duration_secs = Some(90);
        timed_call.call_outcome = Some(wa::message::call_log_message::CallOutcome::Connected);
        let mut invoice = wa::message::InvoiceMessage::default();
        invoice.note = Some("table 4".into());
        let mut album = wa::message::AlbumMessage::default();
        album.expected_image_count = Some(4);
        let mut pack = wa::message::StickerPackMessage::default();
        pack.name = Some("Cats".into());
        let mut pin = wa::message::PinInChatMessage::default();
        pin.r#type = Some(wa::message::pin_in_chat_message::Type::UnpinForAll);
        let mut keep = wa::message::KeepInChatMessage::default();
        keep.keep_type = Some(wa::KeepType::UndoKeepForAll);
        let mut timer = wa::message::ProtocolMessage::default();
        timer.r#type = Some(wa::message::protocol_message::Type::EphemeralSetting);
        timer.ephemeral_expiration = Some(604_800);

        let mut missed = wa::Message::default();
        missed.call_log_messsage = MessageField::some(missed_call);
        let mut timed = wa::Message::default();
        timed.call_log_messsage = MessageField::some(timed_call);
        let mut invoice_content = wa::Message::default();
        invoice_content.invoice_message = MessageField::some(invoice);
        let mut album_content = wa::Message::default();
        album_content.album_message = MessageField::some(album);
        let mut pack_content = wa::Message::default();
        pack_content.sticker_pack_message = MessageField::some(pack);
        let mut pin_content = wa::Message::default();
        pin_content.pin_in_chat_message = MessageField::some(pin);
        let mut keep_content = wa::Message::default();
        keep_content.keep_in_chat_message = MessageField::some(keep);
        let mut timer_content = wa::Message::default();
        timer_content.protocol_message = MessageField::some(timer);

        for (content, kind, text) in [
            (missed, "call", "Missed video call"),
            (timed, "call", "\u{1F4DE} Call (1m30s)"),
            (invoice_content, "payment", "\u{1F9FE} Invoice: table 4"),
            (album_content, "album", "\u{1F5BC}\u{FE0F} Album (4 items)"),
            (pack_content, "sticker_pack", "\u{1F3A8} Sticker pack: Cats"),
            (pin_content, "pin", "\u{1F4CC} Unpinned a message"),
            (keep_content, "pin", "Removed a message from kept messages"),
            (
                timer_content,
                "ephemeral_setting",
                "\u{23F3} Disappearing messages set to 7 days",
            ),
        ] {
            let classified = classify_one(&content);
            assert_eq!(classified.kind, kind);
            assert_eq!(classified.text, text);
        }
    }

    #[test]
    fn signal_only_payloads_never_become_bubbles() {
        let mut poll_update = wa::Message::default();
        poll_update.poll_update_message = MessageField::some(Default::default());
        let mut enc_reaction = wa::Message::default();
        enc_reaction.enc_reaction_message = MessageField::some(Default::default());
        let mut sender_key = wa::Message::default();
        sender_key.sender_key_distribution_message = MessageField::some(Default::default());
        let mut app_state = wa::message::ProtocolMessage::default();
        app_state.r#type = Some(wa::message::protocol_message::Type::AppStateSyncKeyShare);
        let mut app_state_content = wa::Message::default();
        app_state_content.protocol_message = MessageField::some(app_state);
        for content in [poll_update, enc_reaction, sender_key, app_state_content] {
            assert!(passive_message(&content));
        }

        let mut conversation = wa::Message::default();
        conversation.conversation = Some("hi".into());
        let mut image = wa::Message::default();
        image.image_message = MessageField::some(wa::message::ImageMessage::default());
        let mut key = wa::MessageKey::default();
        key.id = Some("target".into());
        let mut revoke = wa::message::ProtocolMessage::default();
        revoke.r#type = Some(wa::message::protocol_message::Type::Revoke);
        revoke.key = MessageField::some(key);
        let mut revoke_content = wa::Message::default();
        revoke_content.protocol_message = MessageField::some(revoke);
        for content in [conversation, image, revoke_content] {
            assert!(!passive_message(&content));
        }
    }

    #[test]
    fn the_reply_context_and_the_forwarding_score_ride_on_the_row() {
        let mut context = wa::ContextInfo::default();
        context.stanza_id = Some("quoted".into());
        context.remote_jid = Some("5491100000001@s.whatsapp.net".into());
        context.is_forwarded = Some(true);
        let mut image = wa::message::ImageMessage::default();
        image.caption = Some("look".into());
        image.mimetype = Some("image/jpeg".into());
        image.file_length = Some(2048);
        image.width = Some(640);
        image.height = Some(480);
        image.context_info = MessageField::some(context);
        let mut content = wa::Message::default();
        content.image_message = MessageField::some(image);

        let row = row_of(content);
        assert_eq!(row.kind, "image");
        assert_eq!(row.text, "look");
        assert_eq!(row.reply_to_id, "quoted");
        assert_eq!(row.reply_to_chat_id, "5491100000001@s.whatsapp.net");
        assert_eq!(row.forwarding_score, 1);
        assert_eq!(row.timestamp, 1_700_000_000_000);
        assert_eq!(row.image_mime, "image/jpeg");
        assert_eq!(row.image_size, 2048);
        assert_eq!(row.image_width, 640);
        assert_eq!(row.image_height, 480);
        assert!(row.unread);
        assert!(!row.from_me);
    }

    #[test]
    fn a_revoke_keeps_the_target_id_and_the_event_time() {
        let mut key = wa::MessageKey::default();
        key.id = Some("original".into());
        let mut protocol = wa::message::ProtocolMessage::default();
        protocol.r#type = Some(wa::message::protocol_message::Type::Revoke);
        protocol.key = MessageField::some(key);
        let mut content = wa::Message::default();
        content.protocol_message = MessageField::some(protocol);

        let row = inbound_to_row(&inbound(content, false, 1_700_000_000_500), "")
            .expect("a row")
            .message;
        assert_eq!(row.id, "original");
        assert_eq!(row.kind, "revoked");
        assert_eq!(row.text, "Message deleted");
        assert!(row.revoked);
        assert_eq!(row.edited_at, 1_700_000_000_500);
    }

    #[test]
    fn a_photo_without_a_caption_keeps_the_emoji_fallback() {
        let mut image = wa::message::ImageMessage::default();
        image.mimetype = Some("image/webp".into());
        let mut content = wa::Message::default();
        content.image_message = MessageField::some(image);
        assert_eq!(row_of(content).text, "\u{1F4F7} Photo");
    }

    #[test]
    fn an_edit_rewrites_the_original_row_under_its_own_id() {
        let mut edited = wa::Message::default();
        edited.conversation = Some("corrected text".into());
        let mut key = wa::MessageKey::default();
        key.id = Some("original".into());
        let mut protocol = wa::message::ProtocolMessage::default();
        protocol.r#type = Some(wa::message::protocol_message::Type::MessageEdit);
        protocol.key = MessageField::some(key);
        protocol.timestamp_ms = Some(9999);
        protocol.edited_message = MessageField::some(edited);
        let mut wrapper = wa::Message::default();
        wrapper.protocol_message = MessageField::some(protocol);

        let mut information = info("edit-event", false, 1_700_000_000_000);
        information.edit = wacore::types::message::EditAttribute::MessageEdit;
        let envelope = InboundMessage::builder()
            .message(Arc::new(wrapper))
            .info(Arc::new(information))
            .build();
        let row = inbound_to_row(&envelope, "").expect("a row").message;
        assert_eq!(row.id, "original");
        assert_eq!(row.kind, "text");
        assert_eq!(row.text, "corrected text");
        assert_eq!(row.edited_at, 9999);
        // The edited envelope is the original message's time, not the event's.
        assert_eq!(row.timestamp, 1_700_000_000_000);
    }
}
