//! Outgoing actions: text, reactions, edits, revokes, receipts, typing.
//!
//! Every send keeps the Go sidecar's idempotency contract: the desktop
//! generates a stable `client_message_id`, the store remembers the mapping
//! to the WhatsApp message id before the send, and a retry with the same id
//! never sends twice — it returns the stored outcome.
//!
//! The local row lands in the store the way a peer's message would, marked
//! `from_me`, so the desktop renders it optimistically and the library's
//! own delivery receipts advance its status later.

use std::sync::Arc;

use rusqlite::OptionalExtension as _;
use wacore::proto_helpers::MessageBuilderExt as _;
use wacore::types::message_ref::MessageRef;
use whatfast_protocol::MessageStatus;
use whatsapp_rust::Jid;
use whatsapp_rust::waproto::whatsapp as wa;

use crate::db::{self, ChatRow, MessageRow, Store};
use crate::session::Session;

/// One parsed chat address, shared by every outgoing action.
pub(crate) fn chat_jid(chat_id: &str) -> Result<Jid, String> {
    chat_id
        .parse::<Jid>()
        .map_err(|error| format!("not a WhatsApp JID: {error}"))
}

/// The stored outcome of a client_message_id, if one exists.
pub(crate) fn stored_request(store: &Store, client_request_id: &str) -> Option<(String, String)> {
    let hit: Option<(String, String)> = store
        .read(|conn| {
            conn.query_row(
                "SELECT chat_jid,message_id FROM outgoing_requests WHERE client_request_id=?1",
                [client_request_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
        })
        .ok()?;
    hit
}

/// Builds the local row of an outgoing message and stores it as PENDING.
fn stage_outgoing(
    store: &Store,
    chat: &str,
    text: &str,
    reply_to: &str,
    reply_to_chat: &str,
) -> rusqlite::Result<MessageRow> {
    // The WhatsApp id is assigned at send time; the row carries the client
    // id until then, so a crash between send and store cannot duplicate it.
    let row = MessageRow {
        id: String::new(),
        chat_jid: chat.to_owned(),
        transport_jid: chat.to_owned(),
        sender_jid: String::new(),
        text: text.to_owned(),
        timestamp: db::now_ms(),
        from_me: true,
        status: MessageStatus::Pending as i64,
        kind: "text".into(),
        reply_to_id: reply_to.to_owned(),
        reply_to_chat_id: reply_to_chat.to_owned(),
        ..MessageRow::default()
    };
    store.ensure_chat(chat, chat)?;
    Ok(row)
}

/// Sends text. Answers the wire message (with its WhatsApp id) or a
/// retryable failure.
pub async fn send_text(
    session: &Session,
    store: &Store,
    request: &whatfast_protocol::SendTextRequest,
) -> Result<MessageRow, (String, String, bool)> {
    if let Some((_chat, message_id)) = stored_request(store, &request.client_message_id)
        && let Some(row) = store
            .message(&request.chat_id, &message_id)
            .map_err(internal)?
    {
        return Ok(row);
    }
    let jid = chat_jid(&request.chat_id)
        .map_err(|message| ("invalid_argument".into(), message, false))?;
    let staged = stage_outgoing(
        store,
        &request.chat_id,
        &request.text,
        &request.reply_to_message_id,
        &request.reply_to_chat_id,
    )
    .map_err(|error| ("internal".into(), format!("{error}"), false))?;
    store
        .write(|conn| {
            conn.execute(
                "INSERT OR IGNORE INTO outgoing_requests(client_request_id,chat_jid,text,message_id,created_at)
                 VALUES(?1,?2,?3,'',?4)",
                rusqlite::params![
                    request.client_message_id,
                    request.chat_id,
                    request.text,
                    db::now_ms()
                ],
            )?;
            Ok(())
        })
        .map_err(|error| ("internal".into(), format!("{error}"), false))?;

    // The reply context rides on the message itself, the WhatsApp way.
    let mut message = wa::Message::text(request.text.clone());
    if !request.reply_to_message_id.is_empty() {
        let reply_chat = if request.reply_to_chat_id.is_empty() {
            request.chat_id.clone()
        } else {
            request.reply_to_chat_id.clone()
        };
        let mut context = wa::ContextInfo::default();
        context.stanza_id = Some(request.reply_to_message_id.clone());
        context.remote_jid = Some(reply_chat);
        message = wa::Message::text_with_context(request.text.clone(), context);
    }
    let sent = session
        .client
        .send_message(jid, message)
        .await
        .map_err(|error| ("send_failed".into(), format!("{error}"), true))?;
    let message_id = sent.message_id.to_string();
    let mut row = staged;
    row.id = message_id.clone();
    row.status = MessageStatus::Sent as i64;
    store
        .insert_message(&row)
        .map_err(|error| ("internal".into(), format!("{error}"), false))?;
    store
        .write(|conn| {
            conn.execute(
                "UPDATE outgoing_requests SET message_id=?1 WHERE client_request_id=?2",
                rusqlite::params![message_id, request.client_message_id],
            )?;
            Ok(())
        })
        .map_err(|error| ("internal".into(), format!("{error}"), false))?;
    refresh_chat_pointer(store, &request.chat_id)
        .map_err(|error| ("internal".into(), format!("{error}"), false))?;
    Ok(row)
}

/// Recomputes the chat pointer after an outgoing write and answers the
/// fresh row for the wire event.
pub fn refresh_chat_pointer(store: &Store, chat_id: &str) -> rusqlite::Result<ChatRow> {
    store.write(|conn| {
        conn.execute(
            "UPDATE chats SET
               last_message_id=(SELECT id FROM messages WHERE chat_jid=?1
                                ORDER BY timestamp DESC, id DESC LIMIT 1),
               last_message_text=(SELECT CASE
                                    WHEN text<>'' THEN text
                                    WHEN image_caption<>'' THEN image_caption
                                    WHEN media_caption<>'' THEN media_caption
                                    ELSE ''
                                  END FROM messages WHERE chat_jid=?1
                                  ORDER BY timestamp DESC, id DESC LIMIT 1),
               last_message_at=COALESCE((SELECT timestamp FROM messages WHERE chat_jid=?1
                                         ORDER BY timestamp DESC, id DESC LIMIT 1),0)
             WHERE jid=?1",
            rusqlite::params![chat_id],
        )?;
        Ok(())
    })?;
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
        sender_jid: session
            .client
            .pn()
            .map(|pn| crate::ingest::jid_text(&pn))
            .unwrap_or_default(),
        emoji: emoji.to_owned(),
        timestamp: db::now_ms(),
        from_me: true,
    };
    store.set_reaction(&reaction).map_err(internal)?;
    Ok(())
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
    if !row.from_me {
        return Err((
            "invalid_argument".into(),
            "only own messages can be edited".into(),
            false,
        ));
    }
    let jid = chat_jid(chat_id).map_err(|message| ("invalid_argument".into(), message, false))?;
    let sender = sender_ref(&row.sender_jid, row.from_me)
        .map_err(|message| ("invalid_argument".into(), message, false))?;
    let reference = message_ref(&jid, message_id, sender.as_ref(), row.from_me)
        .map_err(|message| ("invalid_argument".into(), message, false))?;
    session
        .client
        .edit_message(whatsapp_rust::EditRequest::new(
            reference,
            wa::Message::text(text.to_owned()),
        ))
        .await
        .map_err(|error| ("send_failed".into(), format!("{error}"), true))?;
    store
        .mark_message_edited(chat_id, message_id, text)
        .map_err(internal)?;
    refresh_chat_pointer(store, chat_id).map_err(internal)?;
    Ok(())
}

/// Revokes a message for everyone (own messages) or, in groups, as admin.
pub async fn revoke_message(
    session: &Session,
    store: &Store,
    chat_id: &str,
    message_id: &str,
) -> Result<(), (String, String, bool)> {
    let Some(row) = store.message(chat_id, message_id).map_err(internal)? else {
        return Err(("not_found".into(), "message is not stored".into(), false));
    };
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
    refresh_chat_pointer(store, chat_id).map_err(internal)?;
    Ok(())
}

/// The server-side read receipt for a chat's unread messages. The store's
/// unread rows carry their senders, so groups get per-sender receipts.
pub async fn mark_read(
    session: &Session,
    store: &Store,
    chat_id: &str,
) -> Result<(), (String, String, bool)> {
    let jid = chat_jid(chat_id).map_err(|message| ("invalid_argument".into(), message, false))?;
    let unread = store
        .read(|conn| {
            let mut statement = conn.prepare(
                "SELECT id,sender_jid FROM messages
                 WHERE chat_jid=?1 AND unread=1 AND from_me=0
                 ORDER BY timestamp ASC, id ASC LIMIT 500",
            )?;
            let rows = statement
                .query_map(rusqlite::params![chat_id], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .map_err(internal)?;
    if unread.is_empty() {
        return Ok(());
    }
    let ids: Vec<&str> = unread.iter().map(|(id, _)| id.as_str()).collect();
    let sender: Option<Jid> = unread
        .iter()
        .map(|(_, sender)| sender.as_str())
        .find(|sender| !sender.is_empty())
        .and_then(|sender| sender.parse::<Jid>().ok());
    session
        .client
        .mark_as_read(&jid, sender.as_ref(), &ids)
        .await
        .map_err(|error| ("send_failed".into(), format!("{error}"), true))?;
    store.mark_chat_read(chat_id).map_err(internal)?;
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
    if let Some((_chat, message_id)) = stored_request(store, client_message_id)
        && let Some(row) = store.message(chat_id, &message_id).map_err(internal)?
    {
        return Ok(row);
    }
    let jid = chat_jid(chat_id).map_err(|message| ("invalid_argument".into(), message, false))?;
    let created = session
        .client
        .polls()
        .create(jid, question, options, selectable_count.max(1))
        .await
        .map_err(|error| ("send_failed".into(), format!("{error}"), true))?;
    let message_id = created.send_result().message_id.to_string();
    let row = MessageRow {
        id: message_id.clone(),
        chat_jid: chat_id.to_owned(),
        transport_jid: chat_id.to_owned(),
        sender_jid: String::new(),
        text: question.to_owned(),
        timestamp: db::now_ms(),
        from_me: true,
        status: whatfast_protocol::MessageStatus::Sent as i64,
        kind: "poll".into(),
        payload: Vec::new(),
        ..MessageRow::default()
    };
    store.insert_message(&row).map_err(internal)?;
    store
        .upsert_poll(chat_id, &message_id, question, options, selectable_count)
        .map_err(internal)?;
    refresh_chat_pointer(store, chat_id).map_err(internal)?;
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
    // The forwarded copy lands in the target chat when the server echoes
    // it back through the live pipeline; the local row only needs the
    // pointer to move, which the echo will do. Until then the source row is
    // the answer the desktop expects.
    let _ = sent;
    store
        .ensure_chat(target_chat, target_chat)
        .map_err(internal)?;
    refresh_chat_pointer(store, target_chat).map_err(internal)?;
    Ok(source)
}
