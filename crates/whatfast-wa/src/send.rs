//! Outgoing actions: text, reactions, edits, revokes, receipts, typing.
//!
//! Every send keeps the Go sidecar's idempotency contract: the desktop
//! generates a stable `client_message_id`, the store remembers the mapping
//! to the WhatsApp message id before the send, and a retry with the same id
//! never sends twice — it returns the stored outcome. A retry that reuses the
//! id with a different chat or payload is refused, the way the Go store's
//! `ReserveOutgoingMessage` did.
//!
//! The WhatsApp id is generated here, before the send, so the reservation can
//! carry it. The Go did the same with `GenerateMessageID` + the reserved row,
//! and that ordering is what makes a crash between the send and the store
//! update harmless.
//!
//! The local row lands in the store the way a peer's message would, marked
//! `from_me`, so the desktop renders it optimistically and the library's
//! own delivery receipts advance its status later.

use std::sync::Arc;

use rusqlite::OptionalExtension as _;
use wacore::proto_helpers::MessageBuilderExt as _;
use wacore::types::message_ref::MessageRef;
use whatfast_protocol::MessageStatus;
use whatsapp_rust::waproto::whatsapp as wa;
use whatsapp_rust::waproto::whatsapp::message::extended_text_message::PreviewType;
use whatsapp_rust::{Jid, MessageId, SendOptions, SendRequest};

use crate::db::{self, ChatRow, MessageRow, Store};
use crate::session::Session;

/// WhatsApp keeps a message editable for fifteen minutes; the oracle used
/// `whatsmeow.EditWindow` for the same bound.
const EDIT_WINDOW_MS: i64 = 15 * 60 * 1000;

/// A message can be revoked for everyone for 48 hours (`wa.RevokeWindow`).
const REVOKE_WINDOW_MS: i64 = 48 * 60 * 60 * 1000;

/// One parsed chat address, shared by every outgoing action.
pub(crate) fn chat_jid(chat_id: &str) -> Result<Jid, String> {
    chat_id
        .parse::<Jid>()
        .map_err(|error| format!("not a WhatsApp JID: {error}"))
}

/// Whether the chat is a channel: WhatsApp refuses reactions there.
fn is_newsletter_jid(chat_id: &str) -> bool {
    chat_id.ends_with("@newsletter")
}

/// The account's own address, the way `c.OwnID()` stamped every outgoing row.
pub(crate) fn own_sender_jid(session: &Session) -> String {
    session
        .client
        .pn()
        .map(|pn| crate::ingest::jid_text(&pn.to_non_ad()))
        .unwrap_or_default()
}

/// A new UUID for the `client_message_id` of a send that has no window behind
/// it: an automation fires with nobody in front of the desk to generate one.
pub fn uuid4() -> String {
    let mut bytes = [0u8; 16];
    if getrandom::getrandom(&mut bytes).is_err() {
        // A machine without entropy still needs an id: the clock keeps two
        // sends of the same task apart.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos() as u64)
            .unwrap_or_default();
        bytes[..8].copy_from_slice(&now.to_le_bytes());
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex = bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

/// The stored row of a retried `client_message_id`, or nothing when the id is
/// new. A retry that names another chat or another payload is refused with the
/// Go store's error instead of answering the old row.
pub(crate) fn stored_outgoing(
    store: &Store,
    client_message_id: &str,
    chat_id: &str,
    fingerprint: &str,
) -> Result<Option<MessageRow>, (String, String, bool)> {
    let hit: Option<(String, String, String)> = store
        .read(|conn| {
            conn.query_row(
                "SELECT chat_jid,text,message_id FROM outgoing_requests WHERE client_request_id=?1",
                [client_message_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()
        })
        .map_err(internal)?;
    let Some((stored_chat, stored_text, stored_id)) = hit else {
        return Ok(None);
    };
    if stored_chat != chat_id || stored_text != fingerprint {
        return Err((
            "invalid_argument".into(),
            "client_message_id already used with different payload".into(),
            false,
        ));
    }
    // A reservation written before the WhatsApp id was assigned (the previous
    // build stored an empty id) cannot answer anything; treat it as new so the
    // action still completes once.
    if stored_id.is_empty() {
        return Ok(None);
    }
    match store.message(chat_id, &stored_id).map_err(internal)? {
        Some(row) => Ok(Some(row)),
        None => Err((
            "internal".into(),
            "the outgoing reservation has no stored message".into(),
            false,
        )),
    }
}

/// Reserves the local row and the client-id mapping before the send, answering
/// the row and whether this call is a retry that must not send again.
///
/// The row lands as PENDING under a WhatsApp id generated up front, so a crash
/// between the reservation and the send leaves a row the retry answers instead
/// of a second copy. The Go's `ReserveOutgoingMessage` committed both in one
/// transaction; this port writes the row first and the mapping second, which
/// leaves the same end state for a retry (a pending row, no resend).
pub(crate) fn reserve_outgoing(
    session: &Session,
    store: &Store,
    client_message_id: &str,
    chat_id: &str,
    fingerprint: &str,
    mut row: MessageRow,
) -> Result<(MessageRow, bool), (String, String, bool)> {
    if let Some(stored) = stored_outgoing(store, client_message_id, chat_id, fingerprint)? {
        return Ok((stored, true));
    }
    row.id = session.client.generate_message_id().to_string();
    row.status = MessageStatus::Pending as i64;
    // La fila del mensaje y la de `outgoing_requests` van en la misma
    // transacción: un fallo de la segunda no deja un PENDING huérfano que un
    // reintento convierta en un duplicado.
    store
        .reserve_outgoing(&row, client_message_id, chat_id, fingerprint)
        .map_err(internal)?;
    Ok((row, false))
}

/// Records the outcome of a send that assigns its own id (the poll builder
/// does), so a retry answers the stored row. The fingerprint was already
/// compared by `stored_outgoing`.
pub(crate) fn remember_outgoing(
    store: &Store,
    client_message_id: &str,
    chat_id: &str,
    fingerprint: &str,
    message_id: &str,
) -> Result<(), (String, String, bool)> {
    store
        .write(|conn| {
            conn.execute(
                "INSERT INTO outgoing_requests(client_request_id,chat_jid,text,message_id,created_at)
                 VALUES(?1,?2,?3,?4,?5)
                 ON CONFLICT(client_request_id) DO UPDATE SET
                   chat_jid=excluded.chat_jid,
                   text=excluded.text,
                   message_id=excluded.message_id",
                rusqlite::params![client_message_id, chat_id, fingerprint, message_id, db::now_ms()],
            )?;
            Ok(())
        })
        .map_err(internal)?;
    Ok(())
}

/// The context a reply carries: the target's id, its original sender (groups
/// need it), its transport chat, and a copy of what the peer wrote.
///
/// Answers nothing for an ordinary message, and refuses an unknown target the
/// way `replyContext` did, so a quote never lands without its source.
pub(crate) fn reply_context(
    store: &Store,
    chat_id: &str,
    reply_to_chat_id: &str,
    reply_to_message_id: &str,
) -> Result<Option<wa::ContextInfo>, (String, String, bool)> {
    if reply_to_message_id.is_empty() {
        return Ok(None);
    }
    let reply_chat = if reply_to_chat_id.is_empty() {
        chat_id
    } else {
        reply_to_chat_id
    };
    let Some(target) = store
        .message(reply_chat, reply_to_message_id)
        .map_err(internal)?
    else {
        return Err((
            "not_found".into(),
            "the quoted message is not stored".into(),
            false,
        ));
    };
    let mut context = wa::ContextInfo::default();
    context.stanza_id = Some(target.id.clone());
    if !target.sender_jid.is_empty() {
        context.participant = Some(target.sender_jid.clone());
    }
    context.remote_jid = Some(if target.transport_jid.is_empty() {
        reply_chat.to_owned()
    } else {
        target.transport_jid.clone()
    });
    context.quoted_message =
        whatsapp_rust::waproto::buffa::MessageField::some(quoted_message(&target));
    Ok(Some(context))
}

/// The minimal copy of a quoted message the Go attached to its replies: the
/// kind and the text the receiving client renders in the quote bubble.
fn quoted_message(message: &MessageRow) -> wa::Message {
    match message.kind.as_str() {
        "image" => {
            let mut image = wa::message::ImageMessage::default();
            image.mimetype = Some(message.image_mime.clone());
            image.caption = Some(message.image_caption.clone());
            let mut quoted = wa::Message::default();
            quoted.image_message = whatsapp_rust::waproto::buffa::MessageField::some(image);
            quoted
        }
        "sticker" => {
            let mime = if message.image_mime.is_empty() {
                "image/webp".to_owned()
            } else {
                message.image_mime.clone()
            };
            let mut sticker = wa::message::StickerMessage::default();
            sticker.mimetype = Some(mime);
            let mut quoted = wa::Message::default();
            quoted.sticker_message = whatsapp_rust::waproto::buffa::MessageField::some(sticker);
            quoted
        }
        "video" => {
            let mut video = wa::message::VideoMessage::default();
            video.mimetype = Some(message.media_mime.clone());
            video.caption = Some(message.media_caption.clone());
            let mut quoted = wa::Message::default();
            quoted.video_message = whatsapp_rust::waproto::buffa::MessageField::some(video);
            quoted
        }
        "audio" => {
            let mut audio = wa::message::AudioMessage::default();
            audio.mimetype = Some(message.media_mime.clone());
            audio.ptt = Some(message.media_voice);
            let mut quoted = wa::Message::default();
            quoted.audio_message = whatsapp_rust::waproto::buffa::MessageField::some(audio);
            quoted
        }
        "document" => {
            let mut document = wa::message::DocumentMessage::default();
            document.mimetype = Some(message.media_mime.clone());
            document.file_name = Some(message.media_file_name.clone());
            document.caption = Some(message.media_caption.clone());
            let mut quoted = wa::Message::default();
            quoted.document_message = whatsapp_rust::waproto::buffa::MessageField::some(document);
            quoted
        }
        _ => wa::Message::text(message.text.clone()),
    }
}

/// The user addresses a mention list may carry: malformed entries and group or
/// newsletter ids are dropped so one bad tag cannot make WhatsApp reject the
/// whole message (`canonicalMentionJIDs`).
fn canonical_mentions(raw_jids: &[String]) -> Vec<String> {
    let mut mentions: Vec<String> = Vec::with_capacity(raw_jids.len());
    for raw in raw_jids {
        let Ok(jid) = raw.parse::<Jid>() else {
            continue;
        };
        if !jid.is_pn() && !jid.is_lid() {
            continue;
        }
        let canonical = jid.to_non_ad().to_string();
        if !mentions.contains(&canonical) {
            mentions.push(canonical);
        }
    }
    mentions
}

/// The message the phone would send for one text: a plain conversation when
/// there is nothing else, and the extended text when a quote, a mention or
/// the card of a link has to ride with it.
fn text_message(
    text: &str,
    context: Option<wa::ContextInfo>,
    mentions: &[String],
    preview: Option<&crate::link_preview::LinkPreview>,
) -> wa::Message {
    if context.is_none() && mentions.is_empty() && preview.is_none() {
        return wa::Message::text(text);
    }
    let mut extended = wa::message::ExtendedTextMessage::default();
    extended.text = Some(text.to_owned());
    let mut context = context;
    if !mentions.is_empty() {
        let mut value = context.take().unwrap_or_default();
        value.mentioned_jid = mentions.to_vec();
        context = Some(value);
    }
    if let Some(context) = context {
        extended.context_info = whatsapp_rust::waproto::buffa::MessageField::some(context);
    }
    if let Some(preview) = preview {
        extended.matched_text = Some(preview.matched_text.clone());
        extended.title = Some(preview.title.clone());
        extended.description = Some(preview.description.clone());
        if preview.thumbnail.is_empty() {
            extended.preview_type = Some(PreviewType::NONE);
        } else {
            extended.jpeg_thumbnail = Some(preview.thumbnail.clone());
            extended.thumbnail_width = Some(preview.width);
            extended.thumbnail_height = Some(preview.height);
            extended.preview_type = Some(PreviewType::IMAGE);
        }
    }
    let mut message = wa::Message::default();
    message.extended_text_message = whatsapp_rust::waproto::buffa::MessageField::some(extended);
    message
}

/// Sends text. Answers the wire message (with its WhatsApp id) or a
/// retryable failure.
pub async fn send_text(
    session: &Session,
    store: &Store,
    request: &whatfast_protocol::SendTextRequest,
) -> Result<MessageRow, (String, String, bool)> {
    let jid = chat_jid(&request.chat_id)
        .map_err(|message| ("invalid_argument".into(), message, false))?;
    // The stored reply chat is the target conversation when the desktop names
    // none, the way `SendText` defaulted it before reserving the row.
    let reply_chat = if request.reply_to_chat_id.is_empty() {
        request.chat_id.clone()
    } else {
        request.reply_to_chat_id.clone()
    };
    let context = reply_context(
        store,
        &request.chat_id,
        &reply_chat,
        &request.reply_to_message_id,
    )?;
    store
        .ensure_chat(&request.chat_id, &request.chat_id)
        .map_err(internal)?;
    // WhatsApp previews the first link of the text before sending it: the
    // card rides inside the message, so the receivers never fetch the page.
    // A page that does not answer in time leaves the text without a card.
    let preview = crate::link_preview::for_text(&request.text).await;
    let mut staged = MessageRow {
        chat_jid: request.chat_id.clone(),
        transport_jid: request.chat_id.clone(),
        sender_jid: own_sender_jid(session),
        text: request.text.clone(),
        timestamp: db::now_ms(),
        from_me: true,
        kind: "text".into(),
        reply_to_id: request.reply_to_message_id.clone(),
        reply_to_chat_id: if request.reply_to_message_id.is_empty() {
            String::new()
        } else {
            reply_chat
        },
        ..MessageRow::default()
    };
    if let Some(preview) = preview.as_ref() {
        // The row keeps the card so a forward or a history reload draws it
        // without asking the page again.
        staged.link_preview_url = preview.matched_text.clone();
        staged.link_preview_title = preview.title.clone();
        staged.link_preview_description = preview.description.clone();
        staged.link_preview_thumbnail = preview.thumbnail.clone();
        staged.link_preview_width = i64::from(preview.width);
        staged.link_preview_height = i64::from(preview.height);
    }
    let fingerprint = format!(
        "{}\0{}\0{}",
        request.text, request.reply_to_message_id, request.reply_to_chat_id
    );
    let (row, already_reserved) = reserve_outgoing(
        session,
        store,
        &request.client_message_id,
        &request.chat_id,
        &fingerprint,
        staged,
    )?;
    if already_reserved {
        return Ok(row);
    }

    // The reply context and the mentions ride on the message itself, the
    // WhatsApp way, and the preview of the first link rides with them.
    let message = text_message(
        &request.text,
        context,
        &canonical_mentions(&request.mentioned_jids),
        preview.as_ref(),
    );
    session
        .client
        .send(SendRequest::new(&jid, message).with_options(
            SendOptions::default().with_message_id(
                MessageId::new(row.id.as_str()).map_err(|error| {
                    ("internal".into(), format!("bad message id: {error}"), false)
                })?,
            ),
        ))
        .await
        .map_err(|error| {
            let _ =
                store.set_message_status(&request.chat_id, &row.id, MessageStatus::Failed as i64);
            ("send_failed".into(), format!("{error}"), true)
        })?;
    let mut row = row;
    row.status = MessageStatus::Sent as i64;
    store
        .set_message_status(&request.chat_id, &row.id, MessageStatus::Sent as i64)
        .map_err(internal)?;
    store.refresh_chat_pointer(&request.chat_id)
        .map_err(|error| ("internal".into(), format!("{error}"), false))?;
    Ok(row)
}

/// Recomputes the chat pointer after an outgoing write and answers the fresh
/// row for the wire event. It delegates to the store's version, which ignores
/// the revoked and reaction rows; the old copy in this module did not and put
/// the text of a deleted message back in the chat list.
pub fn refresh_chat_pointer(store: &Store, chat_id: &str) -> rusqlite::Result<ChatRow> {
    store.refresh_chat_pointer(chat_id)?;
    store
        .chat(chat_id)?
        .ok_or_else(|| rusqlite::Error::QueryReturnedNoRows)
}

/// The message reference the library's targeted actions take. Group targets
/// carry the original sender; direct ones leave it empty.
fn message_ref<'a>(
    chat: &'a Jid,
    message_id: &'a str,
    sender: Option<&'a Jid>,
    from_me: bool,
) -> Result<MessageRef<'a>, String> {
    MessageRef::new(
        chat,
        whatsapp_rust::MessageId::new(message_id)
            .map_err(|error| format!("bad message id: {error}"))?,
        sender,
        from_me,
    )
    .map_err(|error| format!("{error}"))
}

/// Parses a stored sender jid into the optional reference the actions take.
fn sender_ref(sender_jid: &str, from_me: bool) -> Result<Option<Jid>, String> {
    if sender_jid.is_empty() || from_me {
        return Ok(None);
    }
    sender_jid
        .parse::<Jid>()
        .map(Some)
        .map_err(|error| format!("bad sender jid: {error}"))
}

/// Sends (or clears) a reaction on a stored message.
pub async fn send_reaction(
    session: &Session,
    store: &Store,
    chat_id: &str,
    message_id: &str,
    emoji: &str,
) -> Result<(), (String, String, bool)> {
    let Some(row) = store.message(chat_id, message_id).map_err(internal)? else {
        return Err(("not_found".into(), "message is not stored".into(), false));
    };
    let jid = chat_jid(chat_id).map_err(|message| ("invalid_argument".into(), message, false))?;
    if is_newsletter_jid(chat_id) {
        return Err((
            "invalid_argument".into(),
            "newsletter reactions are not supported".into(),
            false,
        ));
    }
    let sender = sender_ref(&row.sender_jid, row.from_me)
        .map_err(|message| ("invalid_argument".into(), message, false))?;
    let reference = message_ref(&jid, message_id, sender.as_ref(), row.from_me)
        .map_err(|message| ("invalid_argument".into(), message, false))?;
    session
        .client
        .send_reaction(&reference, emoji)
        .await
        .map_err(|error| ("send_failed".into(), format!("{error}"), true))?;
    let reaction = crate::db::ReactionRow {
        chat_jid: chat_id.to_owned(),
        message_id: message_id.to_owned(),
        sender_jid: own_sender_jid(session),
        emoji: emoji.to_owned(),
        timestamp: db::now_ms(),
        from_me: true,
    };
    store.set_reaction(&reaction).map_err(internal)?;
    Ok(())
}

/// Why an edit must be refused, or nothing when it is allowed. The Go checked
/// ownership, the kind, a live row and the 15-minute window before sending.
fn edit_rejection(row: &MessageRow, now_ms: i64) -> Option<&'static str> {
    if !row.from_me
        || row.revoked
        || row.kind != "text"
        || row.timestamp == 0
        || now_ms.saturating_sub(row.timestamp) > EDIT_WINDOW_MS
    {
        return Some("this message can no longer be edited");
    }
    None
}

/// Why a revoke-for-everyone must be refused, or nothing when it is allowed.
fn revoke_rejection(row: &MessageRow, now_ms: i64) -> Option<&'static str> {
    if row.revoked
        || !row.from_me
        || row.timestamp == 0
        || now_ms.saturating_sub(row.timestamp) > REVOKE_WINDOW_MS
    {
        return Some("this message can no longer be deleted");
    }
    None
}

/// Edits a stored from-me text message.
pub async fn edit_message(
    session: &Session,
    store: &Store,
    chat_id: &str,
    message_id: &str,
    text: &str,
) -> Result<(), (String, String, bool)> {
    let Some(row) = store.message(chat_id, message_id).map_err(internal)? else {
        return Err(("not_found".into(), "message is not stored".into(), false));
    };
    if let Some(reason) = edit_rejection(&row, db::now_ms()) {
        return Err(("invalid_argument".into(), reason.into(), false));
    }
    let jid = chat_jid(chat_id).map_err(|message| ("invalid_argument".into(), message, false))?;
    let sender = sender_ref(&row.sender_jid, row.from_me)
        .map_err(|message| ("invalid_argument".into(), message, false))?;
    let reference = message_ref(&jid, message_id, sender.as_ref(), row.from_me)
        .map_err(|message| ("invalid_argument".into(), message, false))?;
    // The replacement content keeps the original quote and forwarding marker,
    // the way `EditText` rebuilt them from the stored row.
    let mut context = reply_context(store, chat_id, &row.reply_to_chat_id, &row.reply_to_id)?;
    if row.forwarding_score > 0 {
        let mut extended = context.take().unwrap_or_default();
        extended.is_forwarded = Some(true);
        extended.forwarding_score = Some(row.forwarding_score as u32);
        context = Some(extended);
    }
    let content = match context {
        Some(context) => wa::Message::text_with_context(text.to_owned(), context),
        None => wa::Message::text(text.to_owned()),
    };
    session
        .client
        .edit_message(whatsapp_rust::EditRequest::new(reference, content))
        .await
        .map_err(|error| ("send_failed".into(), format!("{error}"), true))?;
    store
        .mark_message_edited(chat_id, message_id, text)
        .map_err(internal)?;
    store.refresh_chat_pointer(chat_id).map_err(internal)?;
    Ok(())
}

/// Revokes a message for everyone (own messages, inside the 48-hour window).
pub async fn revoke_message(
    session: &Session,
    store: &Store,
    chat_id: &str,
    message_id: &str,
) -> Result<(), (String, String, bool)> {
    let Some(row) = store.message(chat_id, message_id).map_err(internal)? else {
        return Err(("not_found".into(), "message is not stored".into(), false));
    };
    if let Some(reason) = revoke_rejection(&row, db::now_ms()) {
        return Err(("invalid_argument".into(), reason.into(), false));
    }
    let jid = chat_jid(chat_id).map_err(|message| ("invalid_argument".into(), message, false))?;
    let sender = sender_ref(&row.sender_jid, row.from_me)
        .map_err(|message| ("invalid_argument".into(), message, false))?;
    let reference = message_ref(&jid, message_id, sender.as_ref(), row.from_me)
        .map_err(|message| ("invalid_argument".into(), message, false))?;
    session
        .client
        .revoke_message(&reference)
        .await
        .map_err(|error| ("send_failed".into(), format!("{error}"), true))?;
    store
        .mark_message_revoked(chat_id, message_id)
        .map_err(internal)?;
    store.refresh_chat_pointer(chat_id).map_err(internal)?;
    Ok(())
}

/// Announces (or clears) the typing indicator.
pub async fn set_typing(
    session: &Session,
    chat_id: &str,
    typing: bool,
) -> Result<(), (String, String, bool)> {
    let jid = chat_jid(chat_id).map_err(|message| ("invalid_argument".into(), message, false))?;
    let result = if typing {
        session.client.chatstate().send_composing(&jid).await
    } else {
        session.client.chatstate().send_paused(&jid).await
    };
    result.map_err(|error| ("send_failed".into(), format!("{error}"), true))
}

pub(crate) fn internal(error: rusqlite::Error) -> (String, String, bool) {
    ("internal".into(), format!("{error}"), false)
}

/// The Arc the dispatch hands around; kept here so the handlers stay one
/// signature.
pub type SharedStore = Arc<Store>;

/// Creates a poll and stores it with its options, so the conversation
/// renders the question and the desktop can vote later.
pub async fn create_poll(
    session: &Session,
    store: &Store,
    client_message_id: &str,
    chat_id: &str,
    question: &str,
    options: &[String],
    selectable_count: u32,
) -> Result<MessageRow, (String, String, bool)> {
    // The poll builder assigns its own id, so the reservation cannot precede
    // the send; the fingerprint still refuses a reused id with new content.
    let mut fingerprint = question.to_owned();
    for option in options {
        fingerprint.push('\u{0}');
        fingerprint.push_str(option);
    }
    fingerprint.push('\u{0}');
    fingerprint.push_str(&selectable_count.to_string());
    if let Some(row) = stored_outgoing(store, client_message_id, chat_id, &fingerprint)? {
        return Ok(row);
    }
    let jid = chat_jid(chat_id).map_err(|message| ("invalid_argument".into(), message, false))?;
    let created = session
        .client
        .polls()
        .create(jid, question, options, selectable_count.max(1))
        .await
        .map_err(|error| ("send_failed".into(), format!("{error}"), true))?;
    // The key material a later vote needs lives only in the creation: keep
    // the creator and the secret next to the definition row.
    let (send_result, creator, secret) = created.into_parts();
    let message_id = send_result.message_id.to_string();
    let row = MessageRow {
        id: message_id.clone(),
        chat_jid: chat_id.to_owned(),
        transport_jid: chat_id.to_owned(),
        sender_jid: own_sender_jid(session),
        text: question.to_owned(),
        timestamp: db::now_ms(),
        from_me: true,
        status: MessageStatus::Sent as i64,
        kind: "poll".into(),
        payload: Vec::new(),
        ..MessageRow::default()
    };
    store.insert_message(&row).map_err(internal)?;
    store
        .upsert_poll(chat_id, &message_id, question, options, selectable_count)
        .map_err(internal)?;
    store
        .set_poll_meta(
            chat_id,
            &message_id,
            &creator.to_non_ad_string(),
            secret.as_bytes(),
        )
        .map_err(internal)?;
    remember_outgoing(store, client_message_id, chat_id, &fingerprint, &message_id)?;
    store.refresh_chat_pointer(chat_id).map_err(internal)?;
    Ok(row)
}

/// Forwards one stored message to another conversation: the raw payload is
/// replayed through the library's forward preparation (it strips the quote
/// chain, bumps the forwarding score and re-encrypts for the target).
pub async fn forward_message(
    session: &Session,
    store: &Store,
    source_chat: &str,
    message_id: &str,
    target_chat: &str,
) -> Result<MessageRow, (String, String, bool)> {
    let source = store
        .message(source_chat, message_id)
        .map_err(internal)?
        .ok_or_else(|| ("not_found".into(), "message is not stored".into(), false))?;
    if source.revoked {
        return Err((
            "not_forwardable".into(),
            "this message type cannot be forwarded".into(),
            false,
        ));
    }
    if source.payload.is_empty() {
        return Err((
            "not_forwardable".into(),
            "the original wire bytes are not stored for this message".into(),
            false,
        ));
    }
    let original =
        whatsapp_rust::waproto::codec::message_decode(&source.payload).map_err(|error| {
            (
                "internal".into(),
                format!("decoding the stored message: {error}"),
                false,
            )
        })?;
    let target =
        chat_jid(target_chat).map_err(|message| ("invalid_argument".into(), message, false))?;
    let sent = session
        .client
        .forward_message(target, &original)
        .await
        .map_err(|error| ("send_failed".into(), format!("{error}"), true))?;
    // The forwarded copy is a new message in the target chat: same content,
    // new id, no quote chain and a forward marker, so the desktop can render
    // it before the server echoes it back.
    let mut row = source.clone();
    row.id = sent.message_id.to_string();
    row.chat_jid = target_chat.to_owned();
    row.transport_jid = target_chat.to_owned();
    row.sender_jid = own_sender_jid(session);
    row.timestamp = db::now_ms();
    row.status = MessageStatus::Sent as i64;
    row.reply_to_id = String::new();
    row.reply_to_chat_id = String::new();
    row.edited_at = 0;
    row.forwarding_score = row.forwarding_score.saturating_add(1).max(1);
    row.reactions.clear();
    store
        .ensure_chat(target_chat, target_chat)
        .map_err(internal)?;
    store.insert_message(&row).map_err(internal)?;
    store.refresh_chat_pointer(target_chat).map_err(internal)?;
    Ok(row)
}

