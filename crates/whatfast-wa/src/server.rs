//! Handshake, request dispatch, and event emission over the bridge.
//!
//! The protocol rules mirror the Go sidecar this replaces, so the desktop
//! observes the same behavior during and after the migration:
//!
//! - The first request must be `Hello`; anything earlier answers
//!   `handshake_required` and the process keeps serving.
//! - `Hello` negotiates the envelope version. Without overlap it answers
//!   `unsupported_protocol` and exits; with overlap every later envelope must
//!   carry the agreed version or the answer is `unsupported_protocol` and the
//!   process exits.
//! - RPCs that the Rust sidecar has not ported yet answer `not_implemented`
//!   instead of stalling, so the desktop fails visibly on exactly the
//!   feature that is still missing.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use whatfast_protocol::{
    AuthStateResponse, BackendEvent, ConnectionState, Envelope, HelloResponse, ReconnectResponse,
    RpcError, RpcResponse, ShutdownResponse, StartPairingResponse, envelope, rpc_request,
    rpc_response, transport::PROTOCOL_VERSION,
};

use crate::db::Store;
use crate::session::Session;
use rusqlite::OptionalExtension as _;

/// Where handling one envelope leaves the process.
pub enum Outcome {
    /// The response must be written and the loop continues.
    Respond(Envelope),
    /// The response must be written, then the process exits: the desktop
    /// asked for `Shutdown` or the protocol version is beyond repair.
    RespondThenExit(Envelope),
}

/// What a dispatched request produced: the successful payload, plus whether
/// the process exits after the response is written.
struct Handled {
    result: rpc_response::Result,
    exit_after: bool,
}

/// The conversation state of one served process.
pub struct Server {
    handshaken: bool,
    /// Monotonic event sequence for the lifetime of the process; resets with
    /// the process, as the contract states.
    sequence: AtomicU64,
    /// The profile directory: the device store lives here, keyed by process.
    data_dir: PathBuf,
    /// The chats.db every data event writes into, shared with the pump.
    store: Arc<Store>,
    /// Where avatars and media caches live: `<data-dir>/media`.
    media_dir: std::path::PathBuf,
    /// Where the event pump leaves the backend events it normalizes.
    events: async_channel::Sender<BackendEvent>,
    session: Option<Arc<Session>>,
}

impl Server {
    pub fn new(
        data_dir: PathBuf,
        store: Arc<Store>,
        events: async_channel::Sender<BackendEvent>,
    ) -> Self {
        Self {
            handshaken: false,
            sequence: AtomicU64::new(0),
            media_dir: data_dir.join("media"),
            data_dir,
            store,
            events,
            session: None,
        }
    }

    /// Handles one request envelope end to end and produces the exact
    /// envelope to write back.
    pub async fn handle(&mut self, incoming: Envelope) -> Outcome {
        let request_id = incoming.request_id;
        let Some(envelope::Body::Request(request)) = incoming.body else {
            // `bridge::read_frame` rejects non-request frames before they get
            // here; reaching this arm means an envelope with no body at all.
            return self.error_response(request_id, "invalid_argument", "empty request", false);
        };
        let is_hello = matches!(request.request, Some(rpc_request::Request::Hello(_)));
        if !self.handshaken && !is_hello {
            return self.respond(
                request_id,
                error(
                    "handshake_required",
                    "Hello must be the first request",
                    false,
                ),
            );
        }
        if self.handshaken && incoming.protocol_version != PROTOCOL_VERSION {
            return self.respond_then_exit(
                request_id,
                error(
                    "unsupported_protocol",
                    "envelope protocol version mismatch",
                    false,
                ),
            );
        }
        match self.dispatch(request.request).await {
            Ok(handled) => {
                let response = RpcResponse {
                    result: Some(handled.result),
                };
                if handled.exit_after {
                    self.respond_then_exit(request_id, response)
                } else {
                    self.respond(request_id, response)
                }
            }
            Err(failure) if is_hello && failure.code == "unsupported_protocol" => {
                self.respond_then_exit(request_id, error(&failure.code, &failure.message, false))
            }
            Err(failure) => self.respond(
                request_id,
                error(&failure.code, &failure.message, failure.retryable),
            ),
        }
    }

    async fn dispatch(
        &mut self,
        request: Option<rpc_request::Request>,
    ) -> Result<Handled, Failure> {
        match request {
            Some(rpc_request::Request::Hello(hello)) => {
                if hello.minimum_protocol_version > PROTOCOL_VERSION
                    || hello.maximum_protocol_version < PROTOCOL_VERSION
                {
                    return Err(Failure::fatal(
                        "unsupported_protocol",
                        format!("protocol v{PROTOCOL_VERSION} is not supported by desktop"),
                    ));
                }
                self.handshaken = true;
                Ok(Handled {
                    result: rpc_response::Result::Hello(HelloResponse {
                        backend_version: env!("CARGO_PKG_VERSION").to_owned(),
                        protocol_version: PROTOCOL_VERSION,
                    }),
                    exit_after: false,
                })
            }
            Some(rpc_request::Request::GetAuthState(_)) => {
                let auth = match &self.session {
                    Some(session) => session.auth_state(),
                    None => AuthStateResponse {
                        paired: false,
                        logged_in: false,
                        own_user_id: String::new(),
                        connection_state: ConnectionState::Offline as i32,
                    },
                };
                Ok(Handled {
                    result: rpc_response::Result::AuthState(auth),
                    exit_after: false,
                })
            }
            Some(rpc_request::Request::StartPairing(_)) => {
                // A paired session has nothing to pair; a running unpaired
                // session is already streaming QR codes; an absent session
                // starts here. The QR codes themselves leave as events.
                if let Some(session) = &self.session {
                    let started = !session.state.is_paired();
                    return Ok(Handled {
                        result: rpc_response::Result::StartPairing(StartPairingResponse {
                            started,
                        }),
                        exit_after: false,
                    });
                }
                let session =
                    Session::start(&self.data_dir, Arc::clone(&self.store), self.events.clone())
                        .await?;
                session.state.set_connection(ConnectionState::Connecting);
                self.push_event(connection_changed(
                    ConnectionState::Connecting,
                    String::new(),
                ));
                self.session = Some(session);
                Ok(Handled {
                    result: rpc_response::Result::StartPairing(StartPairingResponse {
                        started: true,
                    }),
                    exit_after: false,
                })
            }
            Some(rpc_request::Request::Reconnect(_)) => {
                let started = match &self.session {
                    Some(session) => session.reconnect().await,
                    None => {
                        // A linked device connects here; an unpaired session
                        // has nothing to reconnect to.
                        if Session::has_linked_device(&self.data_dir).await? {
                            let session = Session::start(
                                &self.data_dir,
                                Arc::clone(&self.store),
                                self.events.clone(),
                            )
                            .await?;
                            self.session = Some(session);
                            true
                        } else {
                            false
                        }
                    }
                };
                Ok(Handled {
                    result: rpc_response::Result::Reconnect(ReconnectResponse { started }),
                    exit_after: false,
                })
            }
            Some(rpc_request::Request::Logout(_)) => {
                if let Some(session) = self.session.take() {
                    session.logout().await;
                    session.shutdown().await;
                }
                Ok(Handled {
                    result: rpc_response::Result::Logout(whatfast_protocol::LogoutResponse {}),
                    exit_after: false,
                })
            }
            Some(rpc_request::Request::ListChats(request)) => {
                let (before_at, before_jid) = parse_chat_cursor(&request.cursor);
                let (chats, total, next_cursor) =
                    self.store
                        .chats_page(before_at, &before_jid, request.limit.clamp(1, 200))?;
                let page = chats.iter().map(crate::wire::wire_chat).collect::<Vec<_>>();
                Ok(Handled {
                    result: rpc_response::Result::ListChats(whatfast_protocol::ListChatsResponse {
                        chats: page,
                        total_count: total,
                        next_cursor,
                    }),
                    exit_after: false,
                })
            }
            Some(rpc_request::Request::ListMessages(request)) => {
                let rows = self.store.messages_before(
                    &request.chat_id,
                    request.before_timestamp_ms,
                    &request.before_message_id,
                    request.limit.clamp(1, 200),
                )?;
                let has_more = rows.len() as u32 == request.limit.clamp(1, 200);
                let messages = rows.iter().map(crate::wire::wire_message).collect();
                Ok(Handled {
                    result: rpc_response::Result::ListMessages(
                        whatfast_protocol::ListMessagesResponse { messages, has_more },
                    ),
                    exit_after: false,
                })
            }
            Some(rpc_request::Request::ListMessagesAfter(request)) => {
                let (rows, has_more) = self.store.messages_after(
                    &request.chat_id,
                    request.after_timestamp_ms,
                    &request.after_message_id,
                    request.limit.clamp(1, 200),
                )?;
                let messages = rows.iter().map(crate::wire::wire_message).collect();
                Ok(Handled {
                    result: rpc_response::Result::ListMessagesAfter(
                        whatfast_protocol::ListMessagesAfterResponse { messages, has_more },
                    ),
                    exit_after: false,
                })
            }
            Some(rpc_request::Request::ListMessagesAround(request)) => {
                let Some((rows, has_older, has_newer)) =
                    self.store
                        .messages_around(&request.chat_id, &request.message_id, 60)?
                else {
                    return Err(Failure::fatal(
                        "not_found",
                        "the anchor message is not stored",
                    ));
                };
                let messages = rows.iter().map(crate::wire::wire_message).collect();
                Ok(Handled {
                    result: rpc_response::Result::ListMessagesAround(
                        whatfast_protocol::ListMessagesAroundResponse {
                            messages,
                            has_older,
                            has_newer,
                            anchor_message_id: request.message_id,
                        },
                    ),
                    exit_after: false,
                })
            }
            Some(rpc_request::Request::OpenMessageWindow(request)) => {
                let (rows, has_older, has_newer, first_unread) =
                    open_window(&self.store, &request.chat_id)?;
                let messages = rows.iter().map(crate::wire::wire_message).collect();
                Ok(Handled {
                    result: rpc_response::Result::OpenMessageWindow(
                        whatfast_protocol::OpenMessageWindowResponse {
                            messages,
                            has_older,
                            has_newer,
                            first_unread_message_id: first_unread,
                        },
                    ),
                    exit_after: false,
                })
            }
            Some(rpc_request::Request::SearchLocal(request)) => {
                let query = request.query.trim().to_owned();
                let groups = self
                    .store
                    .search_groups(&query, 6)?
                    .iter()
                    .map(crate::wire::wire_chat)
                    .collect::<Vec<_>>();
                let messages = self
                    .store
                    .search_messages(&query, 20)?
                    .into_iter()
                    .map(|(chat, message)| {
                        let title = crate::wire::wire_chat(&chat);
                        whatfast_protocol::MessageSearchResult {
                            chat_id: message.chat_jid.clone(),
                            message_id: message.id.clone(),
                            chat_title: if title.title.is_empty() {
                                chat.jid.clone()
                            } else {
                                title.title
                            },
                            sender_name: if message.from_me {
                                "You".into()
                            } else {
                                String::new()
                            },
                            timestamp_ms: message.timestamp,
                            snippet: truncate_runes(&message.text, 180),
                            kind: message.kind.clone(),
                            archived: chat.archived,
                            chat: Some(crate::wire::wire_chat(&chat)),
                        }
                    })
                    .collect::<Vec<_>>();
                Ok(Handled {
                    result: rpc_response::Result::SearchLocal(
                        whatfast_protocol::SearchLocalResponse {
                            contacts: Vec::new(),
                            groups,
                            messages,
                        },
                    ),
                    exit_after: false,
                })
            }
            Some(rpc_request::Request::GetSyncStatus(_)) => {
                let status = match &self.session {
                    Some(session) => session.state.sync_status(),
                    None => whatfast_protocol::SyncStatus {
                        phase: whatfast_protocol::SyncPhase::NotStarted as i32,
                        ..Default::default()
                    },
                };
                Ok(Handled {
                    result: rpc_response::Result::SyncStatus(
                        whatfast_protocol::SyncStatusResponse {
                            status: Some(status),
                        },
                    ),
                    exit_after: false,
                })
            }
            Some(rpc_request::Request::SendText(request)) => {
                let session = self
                    .session
                    .clone()
                    .ok_or_else(|| Failure::fatal("not_connected", "WhatsApp is not linked yet"))?;
                let row = crate::send::send_text(&session, &self.store, &request)
                    .await
                    .map_err(|(code, message, retryable)| Failure {
                        code,
                        message,
                        retryable,
                    })?;
                let chat = crate::send::refresh_chat_pointer(&self.store, &row.chat_jid)
                    .map_err(Failure::from)?;
                self.push_event(crate::ingest::message_upserted(&row));
                self.push_event(crate::ingest::chat_upserted(chat));
                Ok(Handled {
                    result: rpc_response::Result::SendText(whatfast_protocol::SendTextResponse {
                        message: Some(crate::wire::wire_message(&row)),
                    }),
                    exit_after: false,
                })
            }
            Some(rpc_request::Request::SendReaction(request)) => {
                let session = self
                    .session
                    .clone()
                    .ok_or_else(|| Failure::fatal("not_connected", "WhatsApp is not linked yet"))?;
                crate::send::send_reaction(
                    &session,
                    &self.store,
                    &request.chat_id,
                    &request.message_id,
                    &request.emoji,
                )
                .await
                .map_err(|(code, message, retryable)| Failure {
                    code,
                    message,
                    retryable,
                })?;
                let reaction = crate::db::ReactionRow {
                    chat_jid: request.chat_id.clone(),
                    message_id: request.message_id.clone(),
                    sender_jid: String::new(),
                    emoji: request.emoji.clone(),
                    timestamp: crate::db::now_ms(),
                    from_me: true,
                };
                let removed = request.emoji.is_empty();
                self.push_event(crate::ingest::reaction_updated(&reaction, removed));
                Ok(Handled {
                    result: rpc_response::Result::SendReaction(
                        whatfast_protocol::SendReactionResponse {
                            reaction: Some(crate::wire::wire_reaction(&reaction)),
                            removed,
                        },
                    ),
                    exit_after: false,
                })
            }
            Some(rpc_request::Request::EditMessage(request)) => {
                let session = self
                    .session
                    .clone()
                    .ok_or_else(|| Failure::fatal("not_connected", "WhatsApp is not linked yet"))?;
                crate::send::edit_message(
                    &session,
                    &self.store,
                    &request.chat_id,
                    &request.message_id,
                    &request.text,
                )
                .await
                .map_err(|(code, message, retryable)| Failure {
                    code,
                    message,
                    retryable,
                })?;
                let row = self
                    .store
                    .message(&request.chat_id, &request.message_id)?
                    .ok_or_else(|| Failure::fatal("not_found", "message is not stored"))?;
                self.push_event(crate::ingest::message_upserted(&row));
                Ok(Handled {
                    result: rpc_response::Result::EditMessage(
                        whatfast_protocol::EditMessageResponse {
                            message: Some(crate::wire::wire_message(&row)),
                        },
                    ),
                    exit_after: false,
                })
            }
            Some(rpc_request::Request::DeleteMessage(request)) => {
                let session = self
                    .session
                    .clone()
                    .ok_or_else(|| Failure::fatal("not_connected", "WhatsApp is not linked yet"))?;
                if !request.delete_for_everyone {
                    // Local-only delete: the row flips to revoked, like the Go.
                    self.store
                        .mark_message_revoked(&request.chat_id, &request.message_id)?;
                } else {
                    crate::send::revoke_message(
                        &session,
                        &self.store,
                        &request.chat_id,
                        &request.message_id,
                    )
                    .await
                    .map_err(|(code, message, retryable)| Failure {
                        code,
                        message,
                        retryable,
                    })?;
                }
                let row = self
                    .store
                    .message(&request.chat_id, &request.message_id)?
                    .ok_or_else(|| Failure::fatal("not_found", "message is not stored"))?;
                self.push_event(crate::ingest::message_upserted(&row));
                Ok(Handled {
                    result: rpc_response::Result::DeleteMessage(
                        whatfast_protocol::DeleteMessageResponse {
                            message: Some(crate::wire::wire_message(&row)),
                        },
                    ),
                    exit_after: false,
                })
            }
            Some(rpc_request::Request::MarkRead(request)) => {
                if request.chat_id.is_empty() {
                    return Err(Failure::fatal("invalid_argument", "chat_id is required"));
                }
                let session = self
                    .session
                    .clone()
                    .ok_or_else(|| Failure::fatal("not_connected", "WhatsApp is not linked yet"))?;
                crate::send::mark_read(&session, &self.store, &request.chat_id)
                    .await
                    .map_err(|(code, message, retryable)| Failure {
                        code,
                        message,
                        retryable,
                    })?;
                Ok(Handled {
                    result: rpc_response::Result::MarkRead(whatfast_protocol::MarkReadResponse {}),
                    exit_after: false,
                })
            }
            Some(rpc_request::Request::SetTyping(request)) => {
                if let Some(session) = &self.session {
                    crate::send::set_typing(session, &request.chat_id, request.typing)
                        .await
                        .map_err(|(code, message, retryable)| Failure {
                            code,
                            message,
                            retryable,
                        })?;
                }
                Ok(Handled {
                    result: rpc_response::Result::SetTyping(
                        whatfast_protocol::SetTypingResponse {},
                    ),
                    exit_after: false,
                })
            }
            Some(rpc_request::Request::SendImage(request)) => {
                let session = self
                    .session
                    .clone()
                    .ok_or_else(|| Failure::fatal("not_connected", "WhatsApp is not linked yet"))?;
                let row = crate::media::send_image(
                    &session,
                    &self.store,
                    &request.client_message_id,
                    &request.chat_id,
                    &request.image_path,
                    &request.caption,
                    &request.reply_to_message_id,
                    &request.reply_to_chat_id,
                )
                .await
                .map_err(|(code, message, retryable)| Failure {
                    code,
                    message,
                    retryable,
                })?;
                let chat = crate::send::refresh_chat_pointer(&self.store, &row.chat_jid)
                    .map_err(Failure::from)?;
                self.push_event(crate::ingest::message_upserted(&row));
                self.push_event(crate::ingest::chat_upserted(chat));
                Ok(Handled {
                    result: rpc_response::Result::SendImage(whatfast_protocol::SendImageResponse {
                        message: Some(crate::wire::wire_message(&row)),
                    }),
                    exit_after: false,
                })
            }
            Some(rpc_request::Request::CallPlace(request)) => {
                let session = self
                    .session
                    .clone()
                    .ok_or_else(|| Failure::fatal("not_connected", "WhatsApp is not linked yet"))?;
                if request.video {
                    return Err(Failure::fatal(
                        "video_unsupported",
                        "video calls are not wired yet; place a voice call",
                    ));
                }
                let call_id =
                    calls_call_helper_place(&session, &self.events, &request.chat_id).await?;
                Ok(Handled {
                    result: rpc_response::Result::CallPlace(whatfast_protocol::CallPlaceResponse {
                        call_id,
                    }),
                    exit_after: false,
                })
            }
            Some(rpc_request::Request::CallAccept(request)) => {
                let session = self
                    .session
                    .clone()
                    .ok_or_else(|| Failure::fatal("not_connected", "WhatsApp is not linked yet"))?;
                calls_call_helper_accept(&session, &self.events, &request.call_id).await?;
                Ok(Handled {
                    result: rpc_response::Result::CallAccept(
                        whatfast_protocol::CallAcceptResponse {},
                    ),
                    exit_after: false,
                })
            }
            Some(rpc_request::Request::CallReject(request)) => {
                let session = self
                    .session
                    .clone()
                    .ok_or_else(|| Failure::fatal("not_connected", "WhatsApp is not linked yet"))?;
                calls_call_helper_reject(&session, &request.call_id).await?;
                Ok(Handled {
                    result: rpc_response::Result::CallReject(
                        whatfast_protocol::CallRejectResponse {},
                    ),
                    exit_after: false,
                })
            }
            Some(rpc_request::Request::CallHangup(request)) => {
                let session = self
                    .session
                    .clone()
                    .ok_or_else(|| Failure::fatal("not_connected", "WhatsApp is not linked yet"))?;
                session.calls.hangup(&request.call_id).await.map_err(
                    |(code, message, retryable)| Failure {
                        code,
                        message,
                        retryable,
                    },
                )?;
                Ok(Handled {
                    result: rpc_response::Result::CallHangup(
                        whatfast_protocol::CallHangupResponse {},
                    ),
                    exit_after: false,
                })
            }
            Some(rpc_request::Request::CallMute(request)) => {
                let session = self
                    .session
                    .clone()
                    .ok_or_else(|| Failure::fatal("not_connected", "WhatsApp is not linked yet"))?;
                session
                    .calls
                    .mute(&request.call_id, request.muted)
                    .map_err(|(code, message, retryable)| Failure {
                        code,
                        message,
                        retryable,
                    })?;
                Ok(Handled {
                    result: rpc_response::Result::CallMute(whatfast_protocol::CallMuteResponse {}),
                    exit_after: false,
                })
            }
            Some(rpc_request::Request::CallAudio(request)) => {
                let session = self
                    .session
                    .clone()
                    .ok_or_else(|| Failure::fatal("not_connected", "WhatsApp is not linked yet"))?;
                let pcm = crate::calls::from_le_bytes(&request.pcm);
                session.calls.audio_in(&request.call_id, pcm).map_err(
                    |(code, message, retryable)| Failure {
                        code,
                        message,
                        retryable,
                    },
                )?;
                Ok(Handled {
                    result: rpc_response::Result::CallAudio(
                        whatfast_protocol::CallAudioResponse {},
                    ),
                    exit_after: false,
                })
            }
            Some(rpc_request::Request::SetChatPin(request)) => {
                self.store.set_chat_flag(
                    &request.chat_id,
                    crate::db::ChatFlag::Pinned,
                    request.pinned,
                )?;
                let chat = self.store.chat(&request.chat_id)?.unwrap_or_default();
                self.push_event(crate::ingest::chat_upserted(chat.clone()));
                Ok(Handled {
                    result: rpc_response::Result::SetChatPin(
                        whatfast_protocol::SetChatPinResponse {
                            chat: Some(crate::wire::wire_chat(&chat)),
                        },
                    ),
                    exit_after: false,
                })
            }
            Some(rpc_request::Request::SetChatArchive(request)) => {
                self.store.set_chat_flag(
                    &request.chat_id,
                    crate::db::ChatFlag::Archived,
                    request.archived,
                )?;
                let chat = self.store.chat(&request.chat_id)?.unwrap_or_default();
                self.push_event(crate::ingest::chat_upserted(chat.clone()));
                Ok(Handled {
                    result: rpc_response::Result::SetChatArchive(
                        whatfast_protocol::SetChatArchiveResponse {
                            chat: Some(crate::wire::wire_chat(&chat)),
                        },
                    ),
                    exit_after: false,
                })
            }
            Some(rpc_request::Request::SetChatMute(request)) => {
                self.store
                    .set_chat_mute(&request.chat_id, request.mute_until_ms)?;
                let chat = self.store.chat(&request.chat_id)?.unwrap_or_default();
                self.push_event(crate::ingest::chat_upserted(chat.clone()));
                Ok(Handled {
                    result: rpc_response::Result::SetChatMute(
                        whatfast_protocol::SetChatMuteResponse {
                            chat: Some(crate::wire::wire_chat(&chat)),
                        },
                    ),
                    exit_after: false,
                })
            }
            Some(rpc_request::Request::SetChatBlock(request)) => {
                self.store.set_chat_flag(
                    &request.chat_id,
                    crate::db::ChatFlag::Blocked,
                    request.blocked,
                )?;
                let chat = self.store.chat(&request.chat_id)?.unwrap_or_default();
                Ok(Handled {
                    result: rpc_response::Result::SetChatBlock(
                        whatfast_protocol::SetChatBlockResponse {
                            chat: Some(crate::wire::wire_chat(&chat)),
                        },
                    ),
                    exit_after: false,
                })
            }
            Some(rpc_request::Request::GetChatInfo(request)) => {
                if request.chat_id.is_empty() {
                    return Err(Failure::fatal("invalid_argument", "chat_id is required"));
                }
                let info = self.chat_info(&request.chat_id).await?;
                Ok(Handled {
                    result: rpc_response::Result::GetChatInfo(info),
                    exit_after: false,
                })
            }
            Some(rpc_request::Request::GetChatAvatar(request)) => {
                let session = self
                    .session
                    .clone()
                    .ok_or_else(|| Failure::fatal("not_connected", "WhatsApp is not linked yet"))?;
                let known = crate::meta::avatar_known(&self.media_dir, &request.chat_id);
                let path = if known {
                    crate::meta::cached_avatar_path(&self.media_dir, &request.chat_id)
                } else {
                    crate::meta::ensure_avatar(
                        &session.client,
                        &self.media_dir,
                        &request.chat_id,
                        request.chat_id.ends_with("@g.us"),
                    )
                    .await
                    .map_err(|(code, message, retryable)| Failure {
                        code,
                        message,
                        retryable,
                    })?
                };
                Ok(Handled {
                    result: rpc_response::Result::GetChatAvatar(
                        whatfast_protocol::GetChatAvatarResponse {
                            chat_id: request.chat_id,
                            avatar_path: path,
                        },
                    ),
                    exit_after: false,
                })
            }
            Some(rpc_request::Request::GetParticipantAvatar(request)) => {
                let session = self
                    .session
                    .clone()
                    .ok_or_else(|| Failure::fatal("not_connected", "WhatsApp is not linked yet"))?;
                let known = crate::meta::avatar_known(&self.media_dir, &request.participant_id);
                let path = if known {
                    crate::meta::cached_avatar_path(&self.media_dir, &request.participant_id)
                } else {
                    crate::meta::ensure_avatar(
                        &session.client,
                        &self.media_dir,
                        &request.participant_id,
                        false,
                    )
                    .await
                    .map_err(|(code, message, retryable)| Failure {
                        code,
                        message,
                        retryable,
                    })?
                };
                Ok(Handled {
                    result: rpc_response::Result::GetParticipantAvatar(
                        whatfast_protocol::GetParticipantAvatarResponse {
                            participant_id: request.participant_id,
                            avatar_path: path,
                        },
                    ),
                    exit_after: false,
                })
            }
            Some(rpc_request::Request::GetMessageAttachment(request)) => {
                let session = self
                    .session
                    .clone()
                    .ok_or_else(|| Failure::fatal("not_connected", "WhatsApp is not linked yet"))?;
                let path = crate::media::fetch_message_media(
                    &session,
                    &self.store,
                    &self.media_dir,
                    &request.chat_id,
                    &request.message_id,
                )
                .await
                .map_err(|(code, message, retryable)| Failure {
                    code,
                    message,
                    retryable,
                })?;
                Ok(Handled {
                    result: rpc_response::Result::GetMessageAttachment(
                        whatfast_protocol::GetMessageAttachmentResponse {
                            chat_id: request.chat_id,
                            message_id: request.message_id,
                            local_path: path,
                        },
                    ),
                    exit_after: false,
                })
            }
            Some(rpc_request::Request::GetMessageImage(request)) => {
                let session = self
                    .session
                    .clone()
                    .ok_or_else(|| Failure::fatal("not_connected", "WhatsApp is not linked yet"))?;
                let path = crate::media::fetch_message_media(
                    &session,
                    &self.store,
                    &self.media_dir,
                    &request.chat_id,
                    &request.message_id,
                )
                .await
                .map_err(|(code, message, retryable)| Failure {
                    code,
                    message,
                    retryable,
                })?;
                Ok(Handled {
                    result: rpc_response::Result::GetMessageImage(
                        whatfast_protocol::GetMessageImageResponse {
                            chat_id: request.chat_id,
                            message_id: request.message_id,
                            image_path: path,
                            // The conversation rows render from the embedded
                            // thumbnail; the viewer needs no second file.
                            thumbnail_path: String::new(),
                        },
                    ),
                    exit_after: false,
                })
            }
            Some(rpc_request::Request::ListNewsletters(_)) => {
                let newsletters = self.store.list_newsletters()?;
                Ok(Handled {
                    result: rpc_response::Result::ListNewsletters(
                        whatfast_protocol::ListNewslettersResponse { newsletters },
                    ),
                    exit_after: false,
                })
            }
            Some(rpc_request::Request::SendAttachment(request)) => {
                let session = self
                    .session
                    .clone()
                    .ok_or_else(|| Failure::fatal("not_connected", "WhatsApp is not linked yet"))?;
                let row = crate::media::send_attachment(
                    &session,
                    &self.store,
                    &request.client_message_id,
                    &request.chat_id,
                    &request.file_path,
                    request.kind(),
                    &request.caption,
                    &request.reply_to_message_id,
                    &request.reply_to_chat_id,
                    request.voice_note,
                )
                .await
                .map_err(|(code, message, retryable)| Failure {
                    code,
                    message,
                    retryable,
                })?;
                let chat = crate::send::refresh_chat_pointer(&self.store, &row.chat_jid)
                    .map_err(Failure::from)?;
                self.push_event(crate::ingest::message_upserted(&row));
                self.push_event(crate::ingest::chat_upserted(chat));
                Ok(Handled {
                    result: rpc_response::Result::SendAttachment(
                        whatfast_protocol::SendAttachmentResponse {
                            message: Some(crate::wire::wire_message(&row)),
                        },
                    ),
                    exit_after: false,
                })
            }
            Some(rpc_request::Request::SendSticker(request)) => {
                let session = self
                    .session
                    .clone()
                    .ok_or_else(|| Failure::fatal("not_connected", "WhatsApp is not linked yet"))?;
                let row = crate::media::send_sticker(
                    &session,
                    &self.store,
                    &request.client_message_id,
                    &request.chat_id,
                    &request.webp_data,
                    &request.reply_to_message_id,
                    &request.reply_to_chat_id,
                )
                .await
                .map_err(|(code, message, retryable)| Failure {
                    code,
                    message,
                    retryable,
                })?;
                let chat = crate::send::refresh_chat_pointer(&self.store, &row.chat_jid)
                    .map_err(Failure::from)?;
                self.push_event(crate::ingest::message_upserted(&row));
                self.push_event(crate::ingest::chat_upserted(chat));
                Ok(Handled {
                    result: rpc_response::Result::SendSticker(
                        whatfast_protocol::SendStickerResponse {
                            message: Some(crate::wire::wire_message(&row)),
                        },
                    ),
                    exit_after: false,
                })
            }
            Some(rpc_request::Request::SendStickerFromLibrary(request)) => {
                // A library sticker is a stored sticker message: the resend
                // replays its wire bytes at the target.
                let session = self
                    .session
                    .clone()
                    .ok_or_else(|| Failure::fatal("not_connected", "WhatsApp is not linked yet"))?;
                let source = self
                    .store
                    .message_by_sticker_id(&request.sticker_id)?
                    .ok_or_else(|| {
                        Failure::fatal("not_found", "that sticker is not stored anymore")
                    })?;
                let row = crate::send::forward_message(
                    &session,
                    &self.store,
                    &source.0,
                    &source.1,
                    &request.chat_id,
                )
                .await
                .map_err(|(code, message, retryable)| Failure {
                    code,
                    message,
                    retryable,
                })?;
                Ok(Handled {
                    result: rpc_response::Result::SendStickerFromLibrary(
                        whatfast_protocol::SendStickerFromLibraryResponse {
                            message: Some(crate::wire::wire_message(&row)),
                        },
                    ),
                    exit_after: false,
                })
            }
            Some(rpc_request::Request::CreatePoll(request)) => {
                let session = self
                    .session
                    .clone()
                    .ok_or_else(|| Failure::fatal("not_connected", "WhatsApp is not linked yet"))?;
                let row = crate::send::create_poll(
                    &session,
                    &self.store,
                    &request.client_message_id,
                    &request.chat_id,
                    &request.question,
                    &request.options,
                    request.selectable_options_count,
                )
                .await
                .map_err(|(code, message, retryable)| Failure {
                    code,
                    message,
                    retryable,
                })?;
                let chat = crate::send::refresh_chat_pointer(&self.store, &row.chat_jid)
                    .map_err(Failure::from)?;
                self.push_event(crate::ingest::message_upserted(&row));
                self.push_event(crate::ingest::chat_upserted(chat));
                Ok(Handled {
                    result: rpc_response::Result::CreatePoll(
                        whatfast_protocol::CreatePollResponse {
                            message: Some(crate::wire::wire_message(&row)),
                        },
                    ),
                    exit_after: false,
                })
            }
            Some(rpc_request::Request::ForwardMessage(request)) => {
                let session = self
                    .session
                    .clone()
                    .ok_or_else(|| Failure::fatal("not_connected", "WhatsApp is not linked yet"))?;
                let row = crate::send::forward_message(
                    &session,
                    &self.store,
                    &request.source_chat_id,
                    &request.message_id,
                    &request.target_chat_id,
                )
                .await
                .map_err(|(code, message, retryable)| Failure {
                    code,
                    message,
                    retryable,
                })?;
                Ok(Handled {
                    result: rpc_response::Result::ForwardMessage(
                        whatfast_protocol::ForwardMessageResponse {
                            message: Some(crate::wire::wire_message(&row)),
                        },
                    ),
                    exit_after: false,
                })
            }
            Some(rpc_request::Request::Shutdown(_)) => Ok(Handled {
                result: rpc_response::Result::Shutdown(ShutdownResponse {}),
                exit_after: true,
            }),
            Some(other) => Err(Failure::fatal(
                "not_implemented",
                format!("{} is not ported yet", request_name(&other)),
            )),
            None => Err(Failure::fatal("invalid_argument", "empty request")),
        }
    }

    /// Emits one backend event through the same pipe the serve loop drains.
    /// The sequence number is assigned by the serve loop, the single writer.
    fn push_event(&self, event: BackendEvent) {
        let _ = self.events.send_blocking(event);
    }

    /// The next event sequence number, starting at one.
    pub fn next_sequence(&self) -> u64 {
        self.sequence.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Wraps one event in its envelope; the serve loop assigned its
    /// sequence already.
    pub fn event_envelope(&self, event: BackendEvent) -> Envelope {
        Envelope {
            protocol_version: PROTOCOL_VERSION,
            request_id: 0,
            body: Some(envelope::Body::Event(event)),
        }
    }

    /// Ends the session, if one is up. The serve loop calls this before the
    /// runtime tears down, so the store flushes under a graceful stop.
    pub async fn shutdown(&mut self) {
        if let Some(session) = self.session.take() {
            session.shutdown().await;
        }
    }

    fn respond(&self, request_id: u64, response: RpcResponse) -> Outcome {
        Outcome::Respond(self.envelope_for(request_id, response))
    }

    fn respond_then_exit(&self, request_id: u64, response: RpcResponse) -> Outcome {
        Outcome::RespondThenExit(self.envelope_for(request_id, response))
    }

    fn error_response(
        &self,
        request_id: u64,
        code: &str,
        message: &str,
        retryable: bool,
    ) -> Outcome {
        self.respond(request_id, error(code, message, retryable))
    }

    fn envelope_for(&self, request_id: u64, response: RpcResponse) -> Envelope {
        Envelope {
            protocol_version: PROTOCOL_VERSION,
            request_id,
            body: Some(envelope::Body::Response(response)),
        }
    }
    /// One conversation's info card: the fresh chat row, the participants
    /// of a group (names resolved through the contacts store), or the peer's
    /// presentation for a direct chat.
    async fn chat_info(
        &self,
        chat_id: &str,
    ) -> Result<whatfast_protocol::GetChatInfoResponse, Failure> {
        let chat = self
            .store
            .chat(chat_id)?
            .ok_or_else(|| Failure::fatal("not_found", "no such conversation"))?;
        let mut response = whatfast_protocol::GetChatInfoResponse {
            chat: Some(crate::wire::wire_chat(&chat)),
            address: chat.address_jid.clone(),
            ..Default::default()
        };
        if chat.address_jid.ends_with("@g.us")
            && let Some(session) = &self.session
        {
            let parsed = chat_id.parse::<whatsapp_rust::Jid>().map_err(|error| {
                Failure::fatal("invalid_argument", format!("not a WhatsApp JID: {error}"))
            })?;
            if let Ok(metadata) = session.client.groups().fetch_metadata(&parsed).await {
                response.description = metadata.description.clone().unwrap_or_default();
                response.created_at_ms = metadata
                    .creation_time
                    .map(|seconds| seconds as i64 * 1000)
                    .unwrap_or(0);
                response.created_by = metadata
                    .creator
                    .as_ref()
                    .map(crate::ingest::jid_text)
                    .unwrap_or_default();
                response.participant_count = metadata.participants.len() as u32;
                response.participants = metadata
                    .participants
                    .iter()
                    .map(|participant| {
                        let jid = crate::ingest::jid_text(&participant.jid);
                        let (contact_name, _push, _business, phone) =
                            self.store.contact(&jid).ok().flatten().unwrap_or_default();
                        let display_name = if contact_name.is_empty() {
                            phone.clone()
                        } else {
                            contact_name
                        };
                        whatfast_protocol::ChatParticipant {
                            participant_id: jid,
                            display_name,
                            phone_number: participant
                                .phone_number
                                .as_ref()
                                .map(crate::ingest::jid_text)
                                .unwrap_or_default(),
                            is_admin: matches!(
                                participant.participant_type,
                                whatsapp_rust::ParticipantType::Admin
                            ),
                            is_super_admin: matches!(
                                participant.participant_type,
                                whatsapp_rust::ParticipantType::SuperAdmin
                            ),
                            is_me: session
                                .client
                                .pn()
                                .is_some_and(|pn| crate::ingest::jid_text(&pn) == chat_id),
                        }
                    })
                    .collect();
                // A fresh subject also names the chat list row.
                if let Some(subject) = metadata.subject
                    && !subject.is_empty()
                    && subject != chat.name
                {
                    self.store.set_chat_name(chat_id, &subject)?;
                    if let Some(fresh) = self.store.chat(chat_id)? {
                        response.chat = Some(crate::wire::wire_chat(&fresh));
                    }
                }
            }
        }
        Ok(response)
    }
}

/// Parses the opaque chat cursor "at:jid" the store hands out.
fn parse_chat_cursor(cursor: &str) -> (i64, String) {
    let Some((at, jid)) = cursor.rsplit_once(':') else {
        return (0, String::new());
    };
    match at.parse::<i64>() {
        Ok(at) => (at, jid.to_owned()),
        Err(_) => (0, String::new()),
    }
}

fn connection_changed(state: ConnectionState, detail: String) -> BackendEvent {
    BackendEvent {
        sequence: 0,
        event: Some(whatfast_protocol::backend_event::Event::ConnectionChanged(
            whatfast_protocol::ConnectionChanged {
                state: state as i32,
                detail,
            },
        )),
    }
}

/// A dispatch failure that maps onto an `RpcError` code.
struct Failure {
    code: String,
    message: String,
    retryable: bool,
}

impl From<anyhow::Error> for Failure {
    fn from(error: anyhow::Error) -> Self {
        Self {
            code: "internal".to_owned(),
            message: format!("{error:#}"),
            retryable: false,
        }
    }
}

impl From<rusqlite::Error> for Failure {
    fn from(error: rusqlite::Error) -> Self {
        use rusqlite::Error as E;
        let (code, retryable) = match &error {
            E::QueryReturnedNoRows => ("not_found", false),
            E::SqliteFailure(_, Some(detail))
                if detail.contains("locked") || detail.contains("busy") =>
            {
                ("storage_busy", true)
            }
            _ => ("internal", false),
        };
        Self {
            code: code.to_owned(),
            message: format!("{error}"),
            retryable,
        }
    }
}

impl Failure {
    fn fatal(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_owned(),
            message: message.into(),
            retryable: false,
        }
    }
}

fn error(code: &str, message: &str, retryable: bool) -> RpcResponse {
    RpcResponse {
        result: Some(rpc_response::Result::Error(RpcError {
            code: code.to_owned(),
            message: message.to_owned(),
            retryable,
        })),
    }
}

/// Cuts a string to `limit` runes with an ellipsis, the Go's rule.
fn truncate_runes(value: &str, limit: usize) -> String {
    let trimmed = value.trim();
    if trimmed.chars().count() <= limit {
        return trimmed.to_owned();
    }
    let mut out: String = trimmed.chars().take(limit).collect();
    out.push('…');
    out
}

/// The call registry lives on the session; these thin helpers keep the
/// dispatch arms short and map the tuple errors once.
async fn calls_call_helper_place(
    session: &Session,
    events: &async_channel::Sender<whatfast_protocol::BackendEvent>,
    chat_id: &str,
) -> Result<String, Failure> {
    session
        .calls
        .place(&session.client, events.clone(), chat_id)
        .await
        .map_err(|(code, message, retryable)| Failure {
            code,
            message,
            retryable,
        })
}

async fn calls_call_helper_accept(
    session: &Session,
    events: &async_channel::Sender<whatfast_protocol::BackendEvent>,
    call_id: &str,
) -> Result<(), Failure> {
    session
        .calls
        .accept(&session.client, events.clone(), call_id)
        .await
        .map_err(|(code, message, retryable)| Failure {
            code,
            message,
            retryable,
        })
}

async fn calls_call_helper_reject(session: &Session, call_id: &str) -> Result<(), Failure> {
    session
        .calls
        .reject(&session.client, call_id)
        .await
        .map_err(|(code, message, retryable)| Failure {
            code,
            message,
            retryable,
        })
}

/// The bounded newest window with the first unread message, when any.
/// (0, "") means the ordinary newest page.
fn open_window(
    store: &std::sync::Arc<Store>,
    chat_id: &str,
) -> rusqlite::Result<(Vec<crate::db::MessageRow>, bool, bool, String)> {
    let first_unread: Option<String> = store.read(|conn| {
        conn.query_row(
            "SELECT id FROM messages WHERE chat_jid=?1 AND unread=1
             ORDER BY timestamp ASC, id ASC LIMIT 1",
            rusqlite::params![chat_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
    })?;
    match first_unread {
        Some(anchor) => {
            let Some((rows, has_older, has_newer)) = store.messages_around(chat_id, &anchor, 60)?
            else {
                return store
                    .messages_before(chat_id, 0, "", 60)
                    .map(|rows| (rows, false, false, String::new()));
            };
            Ok((rows, has_older, has_newer, anchor))
        }
        None => {
            let rows = store.messages_before(chat_id, 0, "", 60)?;
            let has_older = rows.len() == 60;
            Ok((rows, has_older, false, String::new()))
        }
    }
}

/// Names a request in an error message. Every variant of the oneof carries a
/// payload, so its debug form starts `Variant(Payload { .. })` and the name
/// is what precedes the parenthesis.
fn request_name(request: &rpc_request::Request) -> String {
    let debug = format!("{request:?}");
    match debug.split('(').next() {
        Some(name) if !name.is_empty() => name.to_owned(),
        _ => "request".to_owned(),
    }
}
