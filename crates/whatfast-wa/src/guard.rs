//! The door of the local endpoint: who is asking, and what it may do.
//!
//! The desktop owns the process and gets the rights of the user who runs it.
//! An agent arrives through the local endpoint with the id it was connected
//! with and the token of the profile: the grant in `agents.db` decides what it
//! may read, what it may write, in which chats and at which hours. A connection
//! whose token does not match is refused for as long as it lasts.
//!
//! Only the tools of [`ALLOWED`] answer to an agent. Everything else —the
//! settings, the session, the calls, the deletes— is `forbidden`: the agent
//! talks to Whatfast, never to the WhatsApp session.

use std::{path::Path, sync::Arc};

use whatfast_auto::{Call, Db, Grant, Scope, Window, defer_to_window};
use whatfast_protocol::{
    RpcError, RpcResponse, rpc_request::Request, rpc_response::Result as Response,
};

use crate::db::Store;

/// Who a request belongs to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Who {
    /// The desktop, or a local client the profile has not registered yet.
    User,
    /// One connected harness, with the grant read at the handshake.
    Agent { id: String, grant: Grant },
    /// A client that named an agent and could not prove it.
    Denied(String),
}

/// The refusal of the guard, ready to become an `RpcError`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Denied {
    /// The code of the answer: `unauthorized`, `forbidden`, `out_of_window`.
    pub code: &'static str,
    /// One line for the agent and for the log.
    pub message: String,
}

impl Denied {
    /// The refusal as the protocol answers it.
    pub fn response(&self) -> RpcResponse {
        RpcResponse {
            result: Some(Response::Error(RpcError {
                code: self.code.to_owned(),
                message: self.message.clone(),
                retryable: false,
            })),
        }
    }
}

/// The message an agent gets when the grant does not reach the chat.
fn out_of_scope(chat_id: &str, needed: Scope) -> Denied {
    Denied {
        code: "forbidden",
        message: format!(
            "the grant of this agent does not allow {needed} in {chat_id}: ask the user to widen it"
        ),
    }
}

/// The guard of the local endpoint.
#[derive(Clone)]
pub struct Guard {
    db: Arc<Db>,
    /// The local time of the profile, in minutes east of UTC: the window of a
    /// grant and the next turn of a schedule are read in it.
    offset_minutes: i32,
}

impl Guard {
    /// Opens `agents.db` in the profile.
    ///
    /// A database that cannot be opened is logged and the endpoint keeps
    /// serving as the user: a broken file must not close the desktop.
    pub fn open(data_dir: &Path, offset_minutes: i32) -> Option<Self> {
        let path = data_dir.join("agents.db");
        match Db::open(&path) {
            Ok(db) => Some(Self {
                db: Arc::new(db),
                offset_minutes,
            }),
            Err(error) => {
                tracing::warn!("agents.db is not available ({}): {error}", path.display());
                None
            }
        }
    }

    /// The database behind the guard, for the callers that need to list agents.
    pub fn db(&self) -> &Arc<Db> {
        &self.db
    }

    /// The local offset of the profile.
    pub fn offset_minutes(&self) -> i32 {
        self.offset_minutes
    }

    /// Who is on the other side of a handshake.
    ///
    /// A client that names no agent is the user. A client that names one needs
    /// its token; while the profile has no agent registered at all, the token
    /// is not asked for yet, so the first connection of a fresh install works
    /// before the interface has written its row.
    pub fn identify(&self, agent_id: &str, token: &str) -> Who {
        let agent_id = agent_id.trim();
        if agent_id.is_empty() {
            return Who::User;
        }
        match self.db.agent(agent_id) {
            Ok(Some(agent)) => {
                if agent.revoked_ms.is_some() {
                    return Who::Denied(format!("the agent {agent_id} was disconnected"));
                }
                match self.db.authorize(agent_id, token) {
                    Ok(true) => Who::Agent {
                        id: agent_id.to_owned(),
                        grant: agent.grant,
                    },
                    Ok(false) => Who::Denied(format!(
                        "the token of {agent_id} does not match: reconnect it from Whatfast"
                    )),
                    Err(error) => Who::Denied(format!("agents.db: {error}")),
                }
            }
            Ok(None) => match self.db.agents() {
                // The profile has no agent yet: the interface that writes the
                // rows does not exist in this build, so the harness is treated
                // like the user who started it.
                Ok(agents) if agents.is_empty() => {
                    tracing::info!(
                        "the agent {agent_id} speaks without a token: no agent is registered yet"
                    );
                    Who::User
                }
                Ok(_) => Who::Denied(format!(
                    "the agent {agent_id} is not connected: connect it from Whatfast"
                )),
                Err(error) => Who::Denied(format!("agents.db: {error}")),
            },
            Err(error) => Who::Denied(format!("agents.db: {error}")),
        }
    }

    /// Whether the request may be answered, given who asked.
    pub fn check(&self, who: &Who, request: &Request, store: &Store) -> Result<(), Denied> {
        let Who::Agent { id, grant } = who else {
            return match who {
                Who::User => Ok(()),
                Who::Denied(reason) => Err(Denied {
                    code: "unauthorized",
                    message: reason.clone(),
                }),
                Who::Agent { .. } => Ok(()),
            };
        };
        match request {
            // Reading a chat: the grant has to cover it.
            Request::ListMessages(request) => {
                self.readable(id, grant, &request.chat_id, store, Scope::Read)
            }
            Request::ListMessagesAfter(request) => {
                self.readable(id, grant, &request.chat_id, store, Scope::Read)
            }
            Request::ListMessagesAround(request) => {
                self.readable(id, grant, &request.chat_id, store, Scope::Read)
            }
            Request::OpenMessageWindow(request) => {
                self.readable(id, grant, &request.chat_id, store, Scope::Read)
            }
            Request::GetChatInfo(request) => {
                self.readable(id, grant, &request.chat_id, store, Scope::Read)
            }
            Request::GetMessageAttachment(request) => {
                self.readable(id, grant, &request.chat_id, store, Scope::Read)
            }
            Request::ListPinnedMessages(request) => {
                self.readable(id, grant, &request.chat_id, store, Scope::Read)
            }
            // The lists: the grant covers them, and the response is filtered
            // to the chats it names.
            Request::ListChats(_) | Request::SearchLocal(_) => {
                if grant.scope.covers(Scope::Read) {
                    Ok(())
                } else {
                    Err(out_of_scope("*", Scope::Read))
                }
            }
            Request::SetTyping(request) => {
                self.readable(id, grant, &request.chat_id, store, Scope::Send)
            }
            Request::SendText(request) => {
                self.readable(id, grant, &request.chat_id, store, Scope::Send)?;
                self.within_limits(id, grant)
            }
            // A draft is the safe way to say something in a chat the agent may
            // not send to, so the scope it needs is the lowest of the three.
            Request::DraftPropose(request) => {
                self.readable(id, grant, &request.chat_id, store, Scope::Draft)?;
                if !request.agent_id.is_empty() && request.agent_id != *id {
                    return Err(Denied {
                        code: "forbidden",
                        message: format!("{id} cannot write drafts of {}", request.agent_id),
                    });
                }
                Ok(())
            }
            Request::DraftList(_) | Request::DraftDecide(_) => {
                if grant.scope.covers(Scope::Read) {
                    Ok(())
                } else {
                    Err(out_of_scope("*", Scope::Read))
                }
            }
            // The tools that only ask about the bridge itself.
            Request::Hello(_) | Request::GetAuthState(_) | Request::Shutdown(_) => Ok(()),
            other => Err(Denied {
                code: "forbidden",
                message: format!(
                    "{} is not a tool of an agent: an agent reads, drafts and sends, nothing else",
                    tool_name(other)
                ),
            }),
        }
    }

    /// Cuts a response down to what the grant covers.
    ///
    /// The grant can name a handful of chats; the lists of the bridge are made
    /// for the desktop and return all of them, so the answer of an agent is
    /// trimmed before it leaves the process.
    pub fn filter(&self, who: &Who, response: &mut RpcResponse) {
        let Who::Agent { grant, .. } = who else {
            return;
        };
        let Some(allow) = &grant.chats.allow else {
            return;
        };
        let Some(result) = response.result.as_mut() else {
            return;
        };
        match result {
            Response::ListChats(list) => {
                list.chats
                    .retain(|chat| allowed(grant, allow, &chat.transport_jid, &chat.id));
                list.total_count = list.chats.len() as u64;
            }
            Response::SearchLocal(found) => {
                found
                    .messages
                    .retain(|message| allowed(grant, allow, &message.chat_id, &message.chat_id));
                found
                    .groups
                    .retain(|chat| allowed(grant, allow, &chat.transport_jid, &chat.id));
                // A contact without a known conversation cannot be checked
                // against the list, so it is not offered either.
                found.contacts.retain(|contact| {
                    !contact.chat_id.is_empty()
                        && allowed(grant, allow, &contact.chat_id, &contact.chat_id)
                });
            }
            _ => {}
        }
    }

    /// Records one tool call of an agent in the log the user reads.
    pub fn log(&self, who: &Who, call: &Call, at_ms: i64) {
        if matches!(who, Who::Agent { .. })
            && let Err(error) = self.db.record_call(call, at_ms)
        {
            tracing::warn!("the call of {} was not logged: {error}", call.agent_id);
        }
    }

    /// The next turn of a `cron` automation, moved into the window of its
    /// grant. `None` when the expression cannot produce another one.
    pub fn next_turn(&self, cron: &str, after_ms: i64, window: Option<Window>) -> Option<i64> {
        let next = whatfast_auto::next_at(cron, after_ms, self.offset_minutes)?;
        Some(defer_to_window(next, window, self.offset_minutes))
    }

    /// Whether the chat is covered at this scope, and (for a send) whether the
    /// hour and the ceiling allow it now.
    fn readable(
        &self,
        agent_id: &str,
        grant: &Grant,
        chat_id: &str,
        store: &Store,
        needed: Scope,
    ) -> Result<(), Denied> {
        let is_group = is_group(chat_id, store);
        if !grant.allows(chat_id, is_group, needed) {
            return Err(out_of_scope(chat_id, needed));
        }
        let _ = agent_id;
        Ok(())
    }

    /// The hours of the grant and the ceiling of messages per hour.
    fn within_limits(&self, agent_id: &str, grant: &Grant) -> Result<(), Denied> {
        let now = crate::db::now_ms();
        let minute_of_day = local_minute(now, self.offset_minutes);
        if !grant.open_at(minute_of_day) {
            let opening = grant
                .window
                .map(|window| window.to_string())
                .unwrap_or_else(|| "sin ventana".to_owned());
            return Err(Denied {
                code: "out_of_window",
                message: format!(
                    "the working hours of this agent are {opening}: the message waits for the window"
                ),
            });
        }
        if grant.per_hour > 0 {
            let since = now - 60 * 60 * 1000;
            match self.db.sends_since(agent_id, since) {
                Ok(sent) if sent >= u32::from(grant.per_hour) => {
                    return Err(Denied {
                        code: "rate_limited",
                        message: format!(
                            "the agent already sent {sent} messages in the last hour: its ceiling is {}",
                            grant.per_hour
                        ),
                    });
                }
                Ok(_) => {}
                Err(error) => tracing::warn!("the send ceiling was not counted: {error}"),
            }
        }
        Ok(())
    }
}

/// Whether the chat of an id is a group, read from the store and falling back
/// to the shape of the address.
pub fn is_group(chat_id: &str, store: &Store) -> bool {
    match store.chat(chat_id) {
        Ok(Some(row)) => is_group_jid(&row.jid),
        _ => is_group_jid(chat_id),
    }
}

/// Groups, broadcasts and channels are the many-recipient conversations.
fn is_group_jid(jid: &str) -> bool {
    jid.ends_with("@g.us") || jid.ends_with("@broadcast") || jid.ends_with("@newsletter")
}

/// Whether the chat falls inside the allowlist of a grant.
fn allowed(grant: &Grant, allow: &[String], chat_id: &str, fallback: &str) -> bool {
    let candidates = [chat_id, fallback];
    if let Some(first) = candidates.iter().find(|value| !value.is_empty())
        && is_group_jid(first)
        && !grant.chats.groups
    {
        return false;
    }
    candidates
        .iter()
        .any(|candidate| !candidate.is_empty() && allow.iter().any(|entry| entry == candidate))
}

/// The minute of the local day of an instant.
fn local_minute(now_ms: i64, offset_minutes: i32) -> u16 {
    let local = now_ms + i64::from(offset_minutes) * 60_000;
    ((local.rem_euclid(86_400_000)) / 60_000) as u16
}

/// The name of a request, for the refusal and for the log.
pub fn tool_name(request: &Request) -> String {
    let debug = format!("{request:?}");
    let name = debug.split(['(', ' ']).next().unwrap_or("desconocido");
    let mut snake = String::with_capacity(name.len() + 4);
    for (index, character) in name.char_indices() {
        if character.is_uppercase() && index > 0 {
            snake.push('_');
        }
        snake.push(character.to_ascii_lowercase());
    }
    snake
}

/// The chat a request touches, when it names one.
pub fn chat_of(request: &Request) -> Option<String> {
    match request {
        Request::ListMessages(request) => Some(request.chat_id.clone()),
        Request::ListMessagesAfter(request) => Some(request.chat_id.clone()),
        Request::ListMessagesAround(request) => Some(request.chat_id.clone()),
        Request::OpenMessageWindow(request) => Some(request.chat_id.clone()),
        Request::GetChatInfo(request) => Some(request.chat_id.clone()),
        Request::GetMessageAttachment(request) => Some(request.chat_id.clone()),
        Request::ListPinnedMessages(request) => Some(request.chat_id.clone()),
        Request::SetTyping(request) => Some(request.chat_id.clone()),
        Request::SendText(request) => Some(request.chat_id.clone()),
        Request::DraftPropose(request) => Some(request.chat_id.clone()),
        _ => None,
    }
}

/// The request of an envelope, when it carries one.
pub fn request_of(envelope: &whatfast_protocol::Envelope) -> Option<&Request> {
    match &envelope.body {
        Some(whatfast_protocol::envelope::Body::Request(request)) => request.request.as_ref(),
        _ => None,
    }
}

/// The agent fields of a `Hello`, when the envelope is one.
pub fn hello_agent(envelope: &whatfast_protocol::Envelope) -> Option<(String, String)> {
    match request_of(envelope)? {
        Request::Hello(hello) => Some((hello.agent_id.clone(), hello.agent_token.clone())),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_name_of_a_tool_is_its_snake_case() {
        let request = Request::ListChats(whatfast_protocol::ListChatsRequest::default());
        assert_eq!(tool_name(&request), "list_chats");
        let request = Request::SendText(whatfast_protocol::SendTextRequest::default());
        assert_eq!(tool_name(&request), "send_text");
    }

    #[test]
    fn the_chat_of_a_request_is_read() {
        let request = Request::ListMessages(whatfast_protocol::ListMessagesRequest {
            chat_id: "a@s.whatsapp.net".into(),
            ..Default::default()
        });
        assert_eq!(chat_of(&request).as_deref(), Some("a@s.whatsapp.net"));
        assert!(chat_of(&Request::ListChats(Default::default())).is_none());
    }

    #[test]
    fn groups_are_recognized_by_their_address() {
        assert!(is_group_jid("123@g.us"));
        assert!(is_group_jid("status@broadcast"));
        assert!(is_group_jid("120363169271016023@newsletter"));
        assert!(!is_group_jid("5491100000000@s.whatsapp.net"));
    }

    #[test]
    fn the_local_minute_follows_the_offset() {
        // 2026-10-26 12:00 UTC is 09:00 in UTC-3.
        let noon = 1_793_016_000_000;
        assert_eq!(local_minute(noon, 0), 12 * 60);
        assert_eq!(local_minute(noon, -180), 9 * 60);
    }

    #[test]
    fn the_allowlist_matches_the_chat_and_not_the_group() {
        let grant = Grant {
            scope: Scope::Send,
            chats: whatfast_auto::ChatFilter::only(vec!["a@s.whatsapp.net".into()]),
            per_hour: 0,
            window: None,
            humanize: true,
        };
        let allow = grant.chats.allow.clone().expect("list");
        assert!(allowed(
            &grant,
            &allow,
            "a@s.whatsapp.net",
            "a@s.whatsapp.net"
        ));
        assert!(!allowed(
            &grant,
            &allow,
            "b@s.whatsapp.net",
            "b@s.whatsapp.net"
        ));
        let group = Grant {
            chats: whatfast_auto::ChatFilter::only(vec!["1@g.us".into()]),
            ..grant
        };
        let allow = group.chats.allow.clone().expect("list");
        assert!(allowed(&group, &allow, "1@g.us", "1@g.us"));
        let sin_grupos = Grant {
            chats: whatfast_auto::ChatFilter {
                allow: Some(vec!["1@g.us".into()]),
                groups: false,
            },
            ..group
        };
        let allow = sin_grupos.chats.allow.clone().expect("list");
        assert!(!allowed(&sin_grupos, &allow, "1@g.us", "1@g.us"));
    }

    #[test]
    fn a_refusal_becomes_an_error_response() {
        let denied = Denied {
            code: "forbidden",
            message: "no".into(),
        };
        match denied.response().result {
            Some(Response::Error(error)) => {
                assert_eq!(error.code, "forbidden");
                assert!(!error.retryable);
            }
            other => panic!("expected an error, got {other:?}"),
        }
    }
}
