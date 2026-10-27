//! The seven tools and their translation to bridge requests.
//!
//! The names, descriptions and schemas are the contract with the model. The
//! JSON that comes back is small on purpose: an agent reads a chat, not the
//! whole profile, and every list has a limit.

use std::time::{Duration, Instant};

use serde_json::{Value, json};
use whatfast_protocol::{
    Chat, ChatKind, GetChatInfoRequest, GetMessageAttachmentRequest, ListChatsRequest,
    ListMessagesRequest, Message, MessageStatus, SearchLocalRequest, SendTextRequest, message,
    rpc_request, rpc_response::Result as RpcResult,
};

use crate::{
    client::{Client, Error},
    time,
};

/// The deadline of a read: the same the desktop gives it.
const READ_TIMEOUT: Duration = Duration::from_secs(30);

/// The deadline of a send: it reaches WhatsApp before the answer.
const WRITE_TIMEOUT: Duration = Duration::from_secs(60);

/// The deadline of an attachment: the file may have to be downloaded.
const ATTACHMENT_TIMEOUT: Duration = Duration::from_secs(120);

/// The most characters of one message body a tool prints.
const MAX_TEXT: usize = 4000;

/// What `tools/list` answers.
pub fn definitions() -> Vec<Value> {
    vec![
        json!({
            "name": "chats_list",
            "title": "List chats",
            "description": "The conversations of this WhatsApp profile, newest first, with their title, unread count and the preview of the last message. Use it to find the chat_id that every other tool needs.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "limit": {"type": "integer", "minimum": 1, "maximum": 200, "default": 50},
                    "cursor": {"type": "string", "description": "The next_cursor of a previous page."},
                    "include_archived": {"type": "boolean", "default": true}
                }
            }
        }),
        json!({
            "name": "messages_read",
            "title": "Read messages",
            "description": "The messages of one chat, oldest to newest. Without a cursor it returns the newest page.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "chat_id": {"type": "string"},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 200, "default": 40},
                    "before_timestamp_ms": {"type": "integer", "description": "Exclusive cursor: the at_ms of the oldest message already read."},
                    "before_message_id": {"type": "string", "description": "The id of that same message."}
                },
                "required": ["chat_id"]
            }
        }),
        json!({
            "name": "messages_search",
            "title": "Search the local cache",
            "description": "Searches the messages and contacts this profile already has. It never queries WhatsApp.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {"type": "string"},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 100, "default": 20}
                },
                "required": ["query"]
            }
        }),
        json!({
            "name": "chat_info",
            "title": "Chat details",
            "description": "The details of one conversation: address, about text, group description and members.",
            "inputSchema": {
                "type": "object",
                "properties": {"chat_id": {"type": "string"}},
                "required": ["chat_id"]
            }
        }),
        json!({
            "name": "attachment_get",
            "title": "Download an attachment",
            "description": "Makes sure the file of a message is in the local cache and returns its path. The agent reads the file itself.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "chat_id": {"type": "string"},
                    "message_id": {"type": "string"}
                },
                "required": ["chat_id", "message_id"]
            }
        }),
        json!({
            "name": "messages_wait",
            "title": "Wait for a message",
            "description": "Blocks until a new message arrives, in one chat or in any chat, or until the timeout. Poll a chat with this instead of calling messages_read in a loop.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "chat_id": {"type": "string", "description": "Omit to watch every chat."},
                    "timeout_ms": {"type": "integer", "minimum": 1000, "maximum": 50000, "default": 25000},
                    "after_timestamp_ms": {"type": "integer", "description": "Start after this instant instead of after the newest message."},
                    "after_message_id": {"type": "string"}
                }
            }
        }),
        json!({
            "name": "message_send",
            "title": "Send a message",
            "description": "Sends one text message to one chat, as the linked device. Never broadcasts: one chat per call.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "chat_id": {"type": "string"},
                    "text": {"type": "string"},
                    "reply_to_message_id": {"type": "string", "description": "Sends a native reply to that message of the same chat."},
                    "client_message_id": {"type": "string", "description": "A UUID reused across retries so a message is never sent twice."}
                },
                "required": ["chat_id", "text"]
            }
        }),
        json!({
            "name": "draft_propose",
            "title": "Propose a message",
            "description": "Leaves a draft for the user to approve, in a chat where this agent may not send directly. The user sees it in Whatfast and decides: sending it is theirs.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "chat_id": {"type": "string"},
                    "text": {"type": "string"},
                    "reply_to_message_id": {"type": "string"},
                    "reason": {"type": "string", "description": "Why this message, in one line."},
                    "confidence": {"type": "number", "minimum": 0, "maximum": 1}
                },
                "required": ["chat_id", "text"]
            }
        }),
        json!({
            "name": "drafts_list",
            "title": "List drafts",
            "description": "The drafts waiting for the user: the ones this agent wrote, and the ones of the other agents.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "state": {"type": "string", "description": "proposed, approved, failed, sent or discarded. Empty answers the open ones."},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 200, "default": 50}
                }
            }
        }),
        json!({
            "name": "draft_decide",
            "title": "Approve or discard a draft",
            "description": "Approves the draft for the next turn of the user, or throws it away. Approving does not send: the send is the user's turn.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "draft_id": {"type": "integer"},
                    "send": {"type": "boolean", "description": "true approves it, false discards it."}
                },
                "required": ["draft_id", "send"]
            }
        }),
    ]
}

/// Runs one tool call.
pub fn call(client: &mut Client, name: &str, arguments: &Value) -> Result<Value, String> {
    let started = Instant::now();
    let result = match name {
        "chats_list" => chats_list(client, arguments),
        "messages_read" => messages_read(client, arguments),
        "messages_search" => messages_search(client, arguments),
        "chat_info" => chat_info(client, arguments),
        "attachment_get" => attachment_get(client, arguments),
        "messages_wait" => messages_wait(client, arguments),
        "message_send" => message_send(client, arguments),
        "draft_propose" => draft_propose(client, arguments),
        "drafts_list" => drafts_list(client, arguments),
        "draft_decide" => draft_decide(client, arguments),
        other => return Err(format!("unknown tool {other}")),
    };
    match &result {
        Ok(_) => eprintln!("tool {name} ok in {} ms", started.elapsed().as_millis()),
        Err(message) => eprintln!(
            "tool {name} failed in {} ms: {message}",
            started.elapsed().as_millis()
        ),
    }
    result
}

fn chats_list(client: &mut Client, arguments: &Value) -> Result<Value, String> {
    let limit = count(arguments, "limit", 50, 1, 200);
    let cursor = text(arguments, "cursor").unwrap_or_default();
    let include_archived = flag(arguments, "include_archived", true);
    let answer = request(
        client,
        rpc_request::Request::ListChats(ListChatsRequest { cursor, limit }),
        READ_TIMEOUT,
    )?;
    let RpcResult::ListChats(page) = answer else {
        return Err("the backend answered with another result".into());
    };
    let chats = page
        .chats
        .iter()
        .filter(|chat| include_archived || !chat.archived)
        .map(chat_json)
        .collect::<Vec<_>>();
    Ok(json!({
        "chats": chats,
        "total": page.total_count,
        "next_cursor": page.next_cursor,
    }))
}

fn messages_read(client: &mut Client, arguments: &Value) -> Result<Value, String> {
    let chat_id = required(arguments, "chat_id")?;
    let limit = count(arguments, "limit", 40, 1, 200);
    let before_timestamp_ms = number(arguments, "before_timestamp_ms").unwrap_or(0);
    let before_message_id = text(arguments, "before_message_id").unwrap_or_default();
    let answer = request(
        client,
        rpc_request::Request::ListMessages(ListMessagesRequest {
            chat_id: chat_id.clone(),
            before_timestamp_ms,
            before_message_id,
            limit,
        }),
        READ_TIMEOUT,
    )?;
    let RpcResult::ListMessages(page) = answer else {
        return Err("the backend answered with another result".into());
    };
    Ok(json!({
        "chat_id": chat_id,
        "messages": page.messages.iter().map(message_json).collect::<Vec<_>>(),
        "has_more": page.has_more,
    }))
}

fn messages_search(client: &mut Client, arguments: &Value) -> Result<Value, String> {
    let query = required(arguments, "query")?;
    let limit = count(arguments, "limit", 20, 1, 100) as usize;
    let answer = request(
        client,
        rpc_request::Request::SearchLocal(SearchLocalRequest { query }),
        READ_TIMEOUT,
    )?;
    let RpcResult::SearchLocal(found) = answer else {
        return Err("the backend answered with another result".into());
    };
    let messages = found
        .messages
        .iter()
        .take(limit)
        .map(|hit| {
            let (snippet, _) = clip(&hit.snippet);
            json!({
                "chat_id": hit.chat_id,
                "chat_title": hit.chat_title,
                "message_id": hit.message_id,
                "sender": hit.sender_name,
                "at_ms": hit.timestamp_ms,
                "at": time::iso(hit.timestamp_ms),
                "snippet": snippet,
            })
        })
        .collect::<Vec<_>>();
    let chats = found
        .groups
        .iter()
        .take(20)
        .map(chat_json)
        .collect::<Vec<_>>();
    let contacts = found
        .contacts
        .iter()
        .take(20)
        .map(|contact| {
            json!({
                "name": contact.display_name,
                "phone": contact.phone_number,
                "chat_id": contact.chat_id,
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({"messages": messages, "chats": chats, "contacts": contacts}))
}

fn chat_info(client: &mut Client, arguments: &Value) -> Result<Value, String> {
    let chat_id = required(arguments, "chat_id")?;
    let answer = request(
        client,
        rpc_request::Request::GetChatInfo(GetChatInfoRequest { chat_id }),
        READ_TIMEOUT,
    )?;
    let RpcResult::GetChatInfo(info) = answer else {
        return Err("the backend answered with another result".into());
    };
    let participants = info
        .participants
        .iter()
        .take(50)
        .map(|member| {
            json!({
                "name": member.display_name,
                "phone": member.phone_number,
                "admin": member.is_admin || member.is_super_admin,
                "me": member.is_me,
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({
        "chat": info.chat.as_ref().map(chat_json),
        "address": info.address,
        "about": info.about,
        "verified_name": info.verified_name,
        "description": info.description,
        "created_at": time::iso(info.created_at_ms),
        "created_by": info.created_by,
        "participant_count": info.participant_count,
        "participants": participants,
        "announce_only": info.announce_only,
    }))
}

fn attachment_get(client: &mut Client, arguments: &Value) -> Result<Value, String> {
    let chat_id = required(arguments, "chat_id")?;
    let message_id = required(arguments, "message_id")?;
    let answer = request(
        client,
        rpc_request::Request::GetMessageAttachment(GetMessageAttachmentRequest {
            chat_id,
            message_id,
        }),
        ATTACHMENT_TIMEOUT,
    )?;
    let RpcResult::GetMessageAttachment(file) = answer else {
        return Err("the backend answered with another result".into());
    };
    Ok(json!({
        "chat_id": file.chat_id,
        "message_id": file.message_id,
        "path": file.local_path,
    }))
}

fn messages_wait(client: &mut Client, arguments: &Value) -> Result<Value, String> {
    let started = Instant::now();
    let chat_id = text(arguments, "chat_id");
    let timeout_ms = number(arguments, "timeout_ms")
        .unwrap_or(25_000)
        .clamp(1_000, 50_000) as u64;
    let (mut cursor_at, mut cursor_id) = match number(arguments, "after_timestamp_ms") {
        Some(at) if at > 0 => (at, text(arguments, "after_message_id").unwrap_or_default()),
        // No cursor: wait for what arrives after this instant, not for a
        // message that was already there.
        _ => latest(client, chat_id.as_deref())?,
    };
    let deadline = started + Duration::from_millis(timeout_ms);
    loop {
        let fresh = fresh_messages(client, chat_id.as_deref(), cursor_at, &cursor_id)?;
        if let Some(last) = fresh.last() {
            cursor_at = last.timestamp_ms;
            cursor_id = last.id.clone();
            return Ok(json!({
                "waited_ms": started.elapsed().as_millis(),
                "messages": fresh.iter().map(message_json).collect::<Vec<_>>(),
                "cursor": {"at_ms": cursor_at, "message_id": cursor_id},
            }));
        }
        if Instant::now() >= deadline {
            return Ok(json!({
                "waited_ms": started.elapsed().as_millis(),
                "messages": [],
                "reason": "timeout",
                "cursor": {"at_ms": cursor_at, "message_id": cursor_id},
            }));
        }
        std::thread::sleep(jitter(1200, 1800));
    }
}

fn message_send(client: &mut Client, arguments: &Value) -> Result<Value, String> {
    let chat_id = required(arguments, "chat_id")?;
    let body = required(arguments, "text")?;
    let client_message_id = text(arguments, "client_message_id").unwrap_or_else(uuid4);
    let reply_to = text(arguments, "reply_to_message_id").unwrap_or_default();
    // The pace of a person: the text is cut where a person would cut it and
    // each part waits its typing time. Without the pace the message would
    // leave the instant the model finished, which is the tell the user asked
    // to avoid.
    let pace = client.pace();
    let plan = whatfast_auto::plan(&body, &pace, seed(&client_message_id));
    if plan.is_empty() {
        return Err("the text is empty".into());
    }
    let parts = plan.parts.len();
    if parts > 1 {
        typing(client, &chat_id, true);
    }
    let mut sent = None;
    for (index, part) in plan.parts.iter().enumerate() {
        let wait = plan.waits_ms.get(index).copied().unwrap_or_default();
        if wait > 0 {
            std::thread::sleep(Duration::from_millis(wait));
        }
        let answer = match client.call(
            rpc_request::Request::SendText(SendTextRequest {
                client_message_id: if index == 0 {
                    client_message_id.clone()
                } else {
                    uuid4()
                },
                chat_id: chat_id.clone(),
                text: part.clone(),
                // The reply quotes the message of the user: only the first part
                // carries it.
                reply_to_message_id: if index == 0 {
                    reply_to.clone()
                } else {
                    String::new()
                },
                mentioned_jids: Vec::new(),
                reply_to_chat_id: String::new(),
            }),
            WRITE_TIMEOUT,
        ) {
            Ok(answer) => answer,
            // Nothing went out yet and the chat is not enabled for a direct
            // send: the message waits as a draft instead of being lost.
            Err(Error::Remote { code, .. }) if code == "forbidden" && index == 0 => {
                if parts > 1 {
                    typing(client, &chat_id, false);
                }
                return draft_propose(
                    client,
                    &json!({
                        "chat_id": chat_id,
                        "text": body,
                        "reply_to_message_id": reply_to,
                        "reason": "el chat no está habilitado para enviar: queda como borrador",
                    }),
                );
            }
            Err(error) => return Err(error.to_string()),
        };
        let RpcResult::SendText(answer) = answer else {
            return Err("the backend answered with another result".into());
        };
        sent = Some(answer);
    }
    if parts > 1 {
        typing(client, &chat_id, false);
    }
    let Some(sent) = sent else {
        return Err("the backend answered nothing".into());
    };
    let message = sent.message;
    Ok(json!({
        "chat_id": chat_id,
        "id": message.as_ref().map(|message| message.id.clone()).unwrap_or_default(),
        "at_ms": message.as_ref().map(|message| message.timestamp_ms).unwrap_or(0),
        "at": time::iso(message.as_ref().map(|message| message.timestamp_ms).unwrap_or(0)),
        "status": message.as_ref().map(|message| status_name(message.status())).unwrap_or("pending"),
        "parts": parts,
    }))
}

/// Starts or clears the typing indicator of a chat. A failure is not the
/// message's: the send goes ahead.
fn typing(client: &mut Client, chat_id: &str, typing: bool) {
    let _ = request(
        client,
        rpc_request::Request::SetTyping(whatfast_protocol::SetTypingRequest {
            chat_id: chat_id.to_owned(),
            typing,
        }),
        READ_TIMEOUT,
    );
}

/// The seed of the pace: the id the desktop generated makes two sends of the
/// same text still differ, never the same one twice.
fn seed(client_message_id: &str) -> u64 {
    let mut value: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in client_message_id.as_bytes() {
        value ^= u64::from(*byte);
        value = value.wrapping_mul(0x0000_0100_0000_01b3);
    }
    value | 1
}

/// Leaves a draft for the user to approve.
fn draft_propose(client: &mut Client, arguments: &Value) -> Result<Value, String> {
    let chat_id = required(arguments, "chat_id")?;
    let body = required(arguments, "text")?;
    let confidence = arguments
        .get("confidence")
        .and_then(Value::as_f64)
        .unwrap_or(0.5)
        .clamp(0.0, 1.0);
    let answer = request(
        client,
        rpc_request::Request::DraftPropose(whatfast_protocol::DraftProposeRequest {
            chat_id: chat_id.clone(),
            text: body,
            reply_to_message_id: text(arguments, "reply_to_message_id").unwrap_or_default(),
            reason: text(arguments, "reason").unwrap_or_default(),
            confidence,
            agent_id: String::new(),
        }),
        WRITE_TIMEOUT,
    )?;
    let RpcResult::DraftPropose(draft) = answer else {
        return Err("the backend answered with another result".into());
    };
    Ok(json!({
        "chat_id": chat_id,
        "draft_id": draft.draft_id,
        "state": draft.state,
        "note": "el usuario lo ve en Whatfast y decide si sale",
    }))
}

/// The drafts waiting for the user.
fn drafts_list(client: &mut Client, arguments: &Value) -> Result<Value, String> {
    let state = text(arguments, "state").unwrap_or_default();
    let limit = count(arguments, "limit", 50, 1, 200);
    let answer = request(
        client,
        rpc_request::Request::DraftList(whatfast_protocol::DraftListRequest { state, limit }),
        READ_TIMEOUT,
    )?;
    let RpcResult::DraftList(list) = answer else {
        return Err("the backend answered with another result".into());
    };
    let drafts: Vec<Value> = list
        .drafts
        .iter()
        .map(|draft| {
            json!({
                "draft_id": draft.id,
                "agent": draft.agent_id,
                "chat_id": draft.chat_id,
                "chat": draft.chat_title,
                "text": clip(&draft.text).0,
                "reply_to_message_id": draft.reply_to_message_id,
                "reason": draft.reason,
                "confidence": draft.confidence,
                "state": draft.state,
                "at_ms": draft.at_ms,
                "at": time::iso(draft.at_ms),
            })
        })
        .collect();
    Ok(json!({ "count": drafts.len(), "drafts": drafts }))
}

/// Approves or discards one draft.
fn draft_decide(client: &mut Client, arguments: &Value) -> Result<Value, String> {
    let draft_id =
        number(arguments, "draft_id").ok_or_else(|| "draft_id is required".to_string())?;
    let send = flag(arguments, "send", true);
    let answer = request(
        client,
        rpc_request::Request::DraftDecide(whatfast_protocol::DraftDecideRequest { draft_id, send }),
        WRITE_TIMEOUT,
    )?;
    let RpcResult::DraftDecide(decided) = answer else {
        return Err("the backend answered with another result".into());
    };
    if !decided.error.is_empty() {
        return Err(decided.error);
    }
    Ok(json!({
        "draft_id": draft_id,
        "state": decided.draft.as_ref().map(|draft| draft.state.clone()).unwrap_or_default(),
    }))
}

/// The instant and the id of the newest message the profile has.
fn latest(client: &mut Client, chat_id: Option<&str>) -> Result<(i64, String), String> {
    match chat_id {
        Some(chat_id) => {
            let answer = request(
                client,
                rpc_request::Request::ListMessages(ListMessagesRequest {
                    chat_id: chat_id.to_owned(),
                    before_timestamp_ms: 0,
                    before_message_id: String::new(),
                    limit: 1,
                }),
                READ_TIMEOUT,
            )?;
            let RpcResult::ListMessages(page) = answer else {
                return Err("the backend answered with another result".into());
            };
            Ok(page
                .messages
                .last()
                .map(|message| (message.timestamp_ms, message.id.clone()))
                .unwrap_or((0, String::new())))
        }
        None => {
            let answer = request(
                client,
                rpc_request::Request::ListChats(ListChatsRequest {
                    cursor: String::new(),
                    limit: 200,
                }),
                READ_TIMEOUT,
            )?;
            let RpcResult::ListChats(page) = answer else {
                return Err("the backend answered with another result".into());
            };
            let newest = page
                .chats
                .iter()
                .map(|chat| chat.last_message_timestamp_ms)
                .max()
                .unwrap_or(0);
            // The id of that message is not in the chat row; the timestamp is
            // the cursor and the caller keeps the instant for the next call.
            Ok((newest, String::new()))
        }
    }
}

/// The messages that arrived after the cursor, oldest first.
fn fresh_messages(
    client: &mut Client,
    chat_id: Option<&str>,
    cursor_at: i64,
    cursor_id: &str,
) -> Result<Vec<Message>, String> {
    let mut fresh = Vec::new();
    match chat_id {
        Some(chat_id) => {
            let answer = request(
                client,
                rpc_request::Request::ListMessages(ListMessagesRequest {
                    chat_id: chat_id.to_owned(),
                    before_timestamp_ms: 0,
                    before_message_id: String::new(),
                    limit: 30,
                }),
                READ_TIMEOUT,
            )?;
            let RpcResult::ListMessages(page) = answer else {
                return Err("the backend answered with another result".into());
            };
            fresh.extend(
                page.messages
                    .into_iter()
                    .filter(|message| after(message, cursor_at, cursor_id)),
            );
        }
        None => {
            let answer = request(
                client,
                rpc_request::Request::ListChats(ListChatsRequest {
                    cursor: String::new(),
                    limit: 200,
                }),
                READ_TIMEOUT,
            )?;
            let RpcResult::ListChats(page) = answer else {
                return Err("the backend answered with another result".into());
            };
            let busy = page
                .chats
                .iter()
                .filter(|chat| chat.last_message_timestamp_ms > cursor_at)
                .take(4)
                .map(|chat| chat.id.clone())
                .collect::<Vec<_>>();
            for chat_id in busy {
                let answer = request(
                    client,
                    rpc_request::Request::ListMessages(ListMessagesRequest {
                        chat_id,
                        before_timestamp_ms: 0,
                        before_message_id: String::new(),
                        limit: 10,
                    }),
                    READ_TIMEOUT,
                )?;
                let RpcResult::ListMessages(page) = answer else {
                    return Err("the backend answered with another result".into());
                };
                fresh.extend(
                    page.messages
                        .into_iter()
                        .filter(|message| after(message, cursor_at, cursor_id)),
                );
            }
        }
    }
    fresh.sort_by(|left, right| {
        (left.timestamp_ms, left.id.as_str()).cmp(&(right.timestamp_ms, right.id.as_str()))
    });
    Ok(fresh)
}

/// Whether one message is newer than the cursor, ordered the way the store
/// orders: timestamp first, then the id bytewise.
fn after(message: &Message, cursor_at: i64, cursor_id: &str) -> bool {
    message.timestamp_ms > cursor_at
        || (message.timestamp_ms == cursor_at && message.id.as_str() > cursor_id)
}

fn chat_json(chat: &Chat) -> Value {
    json!({
        "chat_id": chat.id,
        "title": chat.title,
        "kind": match chat.kind() {
            ChatKind::Direct => "direct",
            ChatKind::Group => "group",
            _ => "other",
        },
        "unread": chat.unread_count,
        "muted": chat.muted,
        "pinned": chat.pinned,
        "archived": chat.archived,
        "last_message": chat.last_message_preview,
        "last_message_at_ms": chat.last_message_timestamp_ms,
        "last_message_at": time::iso(chat.last_message_timestamp_ms),
        "last_from_me": chat.last_message_from_me,
        "phone": chat.phone_number,
        "business": chat.business_name,
    })
}

fn message_json(message: &Message) -> Value {
    let (body, kind) = body_of(message);
    let (body, clipped) = clip(&body);
    let mut value = json!({
        "id": message.id,
        "chat_id": message.chat_id,
        "at_ms": message.timestamp_ms,
        "at": time::iso(message.timestamp_ms),
        "from_me": message.from_me,
        "sender": message.sender_name,
        "kind": kind,
        "text": body,
        "status": status_name(message.status()),
    });
    if !message.reply_to_message_id.is_empty() {
        value["reply_to"] = json!(message.reply_to_message_id);
    }
    if message.edited {
        value["edited"] = json!(true);
    }
    if message.revoked {
        value["revoked"] = json!(true);
    }
    if clipped {
        value["clipped"] = json!(true);
    }
    value
}

/// The text an agent reads for one message, and what kind of message it is.
fn body_of(message: &Message) -> (String, &'static str) {
    match &message.content {
        Some(message::Content::Text(text)) => (text.text.clone(), "text"),
        Some(message::Content::Image(image)) => (labelled("[photo]", &image.caption), "image"),
        Some(message::Content::Attachment(file)) => {
            let prefix = if file.voice_note {
                "[voice note]".to_owned()
            } else if file.kind.is_empty() {
                "[file]".to_owned()
            } else {
                format!("[{} {}]", file.kind, file.file_name)
            };
            (labelled(&prefix, &file.caption), "attachment")
        }
        Some(message::Content::Contacts(contacts)) => {
            let names = contacts
                .contacts
                .iter()
                .map(|contact| contact.display_name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            (format!("[contacts] {names}"), "contacts")
        }
        Some(message::Content::Location(place)) => (
            format!("[location] {} {}", place.name, place.address)
                .trim()
                .to_owned(),
            "location",
        ),
        Some(message::Content::Poll(poll)) => (format!("[poll] {}", poll.question), "poll"),
        Some(message::Content::Unsupported(_)) | None => (String::new(), "other"),
    }
}

fn labelled(prefix: &str, caption: &str) -> String {
    if caption.is_empty() {
        prefix.to_owned()
    } else {
        format!("{prefix} {caption}")
    }
}

/// Cuts a body to what a model can afford to read, and says it was cut.
fn clip(text: &str) -> (String, bool) {
    if text.chars().count() <= MAX_TEXT {
        return (text.to_owned(), false);
    }
    let clipped = text.chars().take(MAX_TEXT).collect::<String>();
    (format!("{clipped}…"), true)
}

fn status_name(status: MessageStatus) -> &'static str {
    match status {
        MessageStatus::Unspecified => "unknown",
        MessageStatus::Pending => "pending",
        MessageStatus::Sent => "sent",
        MessageStatus::Delivered => "delivered",
        MessageStatus::Read => "read",
        MessageStatus::Failed => "failed",
    }
}

fn request(
    client: &mut Client,
    request: rpc_request::Request,
    timeout: Duration,
) -> Result<RpcResult, String> {
    client.call(request, timeout).map_err(|error| match error {
        Error::Missing => "no backend is listening on this profile: open Whatfast".to_owned(),
        // An older backend drops the requests it does not know and answers an
        // empty one: the message says so instead of blaming the tool.
        Error::Remote { ref message, .. } if message.contains("empty request") => {
            "el backend de Whatfast es más viejo que este servidor: actualizá la aplicación"
                .to_owned()
        }
        error => error.to_string(),
    })
}

fn text(arguments: &Value, key: &str) -> Option<String> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .filter(|value| !value.is_empty())
}

fn required(arguments: &Value, key: &str) -> Result<String, String> {
    text(arguments, key).ok_or_else(|| format!("{key} is required"))
}

fn number(arguments: &Value, key: &str) -> Option<i64> {
    arguments.get(key).and_then(|value| {
        value
            .as_i64()
            .or_else(|| value.as_f64().map(|number| number as i64))
    })
}

fn count(arguments: &Value, key: &str, default: i64, min: i64, max: i64) -> u32 {
    number(arguments, key).unwrap_or(default).clamp(min, max) as u32
}

fn flag(arguments: &Value, key: &str, default: bool) -> bool {
    arguments
        .get(key)
        .and_then(Value::as_bool)
        .unwrap_or(default)
}

/// A pause that is never the same twice, so two waits do not line up.
fn jitter(min: u64, max: u64) -> Duration {
    let mut bytes = [0u8; 8];
    let value = if getrandom::getrandom(&mut bytes).is_ok() {
        u64::from_le_bytes(bytes)
    } else {
        min
    };
    Duration::from_millis(min + value % (max.saturating_sub(min) + 1))
}

/// A version 4 UUID, the id the desktop reuses across the retries of a send.
fn uuid4() -> String {
    let mut bytes = [0u8; 16];
    if getrandom::getrandom(&mut bytes).is_err() {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_uuid_has_the_shape_of_a_version_four() {
        let id = uuid4();
        assert_eq!(id.len(), 36);
        assert_eq!(id.as_bytes()[14], b'4');
        assert!(matches!(id.as_bytes()[19], b'8' | b'9' | b'a' | b'b'));
        assert_ne!(id, uuid4());
    }

    #[test]
    fn a_long_body_is_cut_and_says_so() {
        let (text, clipped) = clip(&"a".repeat(MAX_TEXT + 10));
        assert!(clipped);
        assert_eq!(text.chars().count(), MAX_TEXT + 1);
        let (text, clipped) = clip("hola");
        assert!(!clipped);
        assert_eq!(text, "hola");
    }

    #[test]
    fn the_cursor_is_exclusive() {
        let message = Message {
            id: "b".into(),
            timestamp_ms: 10,
            ..Default::default()
        };
        assert!(after(&message, 9, "z"));
        assert!(!after(&message, 10, "b"));
        assert!(after(&message, 10, "a"));
    }

    #[test]
    fn a_jitter_stays_inside_its_window() {
        for _ in 0..64 {
            let pause = jitter(1200, 1800).as_millis() as u64;
            assert!((1200..=1800).contains(&pause), "{pause}");
        }
    }
}
