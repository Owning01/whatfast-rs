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
//! - Every RPC the Go served has a real arm below: the match is exhaustive,
//!   so a new contract variant fails to compile here until it gets one.
//!   Unknown requests (no body at all) answer `invalid_argument`.

use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    sync::{Arc, Mutex},
};

use whatfast_protocol::{
    AuthStateResponse, BackendEvent, ConnectionState, Envelope, GetChatHistoryCoverageResponse,
    GetHistoryOverviewResponse, GetIntegrityStatusResponse, HelloResponse, HistoryCoverage,
    HistoryCoverageChanged, HistoryCoverageState, HistoryOverview, IntegrityStatus, Newsletter,
    OpenContactResponse, ReconnectResponse, RepairRecentReactionsResponse, RpcError, RpcResponse,
    SetMessagePinResponse, ShutdownResponse, StartPairingResponse, StatusGroup, VotePollResponse,
    envelope, rpc_request, rpc_response, transport::PROTOCOL_VERSION,
};

use crate::db::{HistoryCoverageData, IntegrityStatusData, Store, now_ms};
use crate::session::Session;
use rusqlite::OptionalExtension as _;
use whatsapp_rust::wacore_binary::JidExt as _;

/// How long a cached avatar answer (a picture or the not-set marker) stays
/// authoritative, the Go cache's TTL.
const AVATAR_TTL: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

/// WhatsApp expires a status after 24 hours; the list only shows that window.
const STATUS_WINDOW_MS: i64 = 24 * 60 * 60 * 1000;

/// The sticker packs the library answers with, the Go `stickerPackLimit`.
const STICKER_PACK_LIMIT: usize = 300;

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
    /// Set when the Hello answer is built; the next request flips the
    /// handshake, which is the point at which the serve loop already wrote
    /// that answer. Keeping the two apart is the Go's ordering: `handle`
    /// never enables the event tap, `Run` does after the write.
    hello_answered: bool,
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
    /// The older-history request in flight, if any: the Go serialized them
    /// with a mutex around one in-flight request, and a second call while
    /// one runs answers `history_busy`. Shared with the expiry task, which
    /// releases the slot when the request ends.
    older_history: Arc<Mutex<Option<(String, i64)>>>,
}

impl Server {
    pub fn new(
        data_dir: PathBuf,
        store: Arc<Store>,
        events: async_channel::Sender<BackendEvent>,
    ) -> Self {
        Self {
            handshaken: false,
            hello_answered: false,
            sequence: AtomicU64::new(0),
            media_dir: data_dir.join("media"),
            data_dir,
            store,
            events,
            session: None,
            older_history: Arc::new(Mutex::new(None)),
        }
    }

    /// Whether the handshake completed. The Go test of the same name reads
    /// its flag right after `handle` and expects it still off.
    pub fn handshaken(&self) -> bool {
        self.handshaken
    }

    /// Handles one request envelope end to end and produces the exact
    /// envelope to write back.
    pub async fn handle(&mut self, incoming: Envelope) -> Outcome {
        // The Hello answer left this process before the next request could
        // arrive: the serve loop writes one response at a time. That write is
        // the gate the Go's `Run` used to enable events, so the handshake is
        // recorded here rather than inside `handle`.
        if self.hello_answered {
            self.handshaken = true;
        }
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
        mut request: Option<rpc_request::Request>,
    ) -> Result<Handled, Failure> {
        // The desktop may still hold a jid a merge retired: every request
        // that names a chat speaks the canonical jid from here on.
        if let Some(request) = request.as_mut() {
            normalize_chat_id(request, &self.store);
        }
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
                self.hello_answered = true;
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
                // The store applies the Go `clampLimit` (50 by default, 200 at
                // most), so a zero limit keeps the oracle's page size.
                let (chats, total, next_cursor) =
                    self.store
                        .chats_page(before_at, &before_jid, request.limit)?;
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
                // The Go refused a half cursor: both halves or neither.
                if (request.before_timestamp_ms == 0) != request.before_message_id.is_empty() {
                    return Err(Failure::fatal(
                        "invalid_argument",
                        "before_timestamp_ms and before_message_id must both be set or both be empty",
                    ));
                }
                let limit = request.limit;
                let (rows, has_more) = self.store.messages_before_page(
                    &request.chat_id,
                    request.before_timestamp_ms,
                    &request.before_message_id,
                    limit,
                )?;
                let messages = self.wire_messages(&rows)?;
                Ok(Handled {
                    result: rpc_response::Result::ListMessages(
                        whatfast_protocol::ListMessagesResponse { messages, has_more },
                    ),
                    exit_after: false,
                })
            }
            Some(rpc_request::Request::ListMessagesAfter(request)) => {
                if request.chat_id.is_empty()
                    || request.after_timestamp_ms <= 0
                    || request.after_message_id.is_empty()
                {
                    return Err(Failure::fatal(
                        "invalid_argument",
                        "chat_id, after_timestamp_ms and after_message_id are required",
                    ));
                }
                let (rows, has_more) = self.store.messages_after(
                    &request.chat_id,
                    request.after_timestamp_ms,
                    &request.after_message_id,
                    request.limit,
                )?;
                let messages = self.wire_messages(&rows)?;
                Ok(Handled {
                    result: rpc_response::Result::ListMessagesAfter(
                        whatfast_protocol::ListMessagesAfterResponse { messages, has_more },
                    ),
                    exit_after: false,
                })
            }
            Some(rpc_request::Request::ListMessagesAround(request)) => {
                if request.chat_id.is_empty() || request.message_id.is_empty() {
                    return Err(Failure::fatal(
                        "invalid_argument",
                        "chat_id and message_id are required",
                    ));
                }
                let Some((rows, has_older, has_newer)) =
                    self.store
                        .messages_around(&request.chat_id, &request.message_id, 25)?
                else {
                    return Err(Failure::fatal(
                        "not_found",
                        "the anchor message is not stored",
                    ));
                };
                let messages = self.wire_messages(&rows)?;
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
                if request.chat_id.is_empty() {
                    return Err(Failure::fatal("invalid_argument", "chat_id is required"));
                }
                let (rows, has_older, has_newer, first_unread) =
                    open_window(&self.store, &request.chat_id)?;
                let messages = self.wire_messages(&rows)?;
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
                // The Go accepted a query of 2 to 256 bytes with valid UTF-8;
                // a Rust `String` is already valid UTF-8.
                if query.chars().count() < 2 || query.len() > 256 {
                    return Err(Failure::fatal(
                        "invalid_argument",
                        "query must be valid UTF-8 containing 2 to 256 bytes",
                    ));
                }
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
                if request.client_message_id.is_empty()
                    || request.chat_id.is_empty()
                    || request.text.is_empty()
                {
                    return Err(Failure::fatal(
                        "invalid_argument",
                        "client_message_id, chat_id and text are required",
                    ));
                }
                if request.text.len() > MAX_TEXT_BYTES {
                    return Err(Failure::fatal(
                        "invalid_argument",
                        "text must be valid UTF-8 up to 65536 bytes",
                    ));
                }
                if !is_uuid(&request.client_message_id) {
                    return Err(Failure::fatal(
                        "invalid_argument",
                        "client_message_id must be a UUID",
                    ));
                }
                self.validate_reply_target(
                    &request.chat_id,
                    &request.reply_to_chat_id,
                    &request.reply_to_message_id,
                )?;
                if !self.connected() {
                    return Err(Failure::retryable(
                        "not_connected",
                        "WhatsApp is not connected",
                    ));
                }
                let session = self.session.clone().ok_or_else(|| {
                    Failure::retryable("not_connected", "WhatsApp is not connected")
                })?;

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
                if request.client_reaction_id.is_empty()
                    || request.chat_id.is_empty()
                    || request.message_id.is_empty()
                {
                    return Err(Failure::fatal(
                        "invalid_argument",
                        "client_reaction_id, chat_id and message_id are required",
                    ));
                }
                if !is_uuid(&request.client_reaction_id) {
                    return Err(Failure::fatal(
                        "invalid_argument",
                        "client_reaction_id must be a UUID",
                    ));
                }
                if !valid_reaction_emoji(&request.emoji) {
                    return Err(Failure::fatal(
                        "invalid_argument",
                        "emoji must be empty or one emoji grapheme up to 64 bytes",
                    ));
                }
                if !self.connected() {
                    return Err(Failure::retryable(
                        "not_connected",
                        "WhatsApp is not connected",
                    ));
                }
                let session = self.session.clone().ok_or_else(|| {
                    Failure::retryable("not_connected", "WhatsApp is not connected")
                })?;
                let removed = request.emoji.is_empty();
                // One user action, one id: a retry replays the stored outcome
                // instead of sending the reaction again, and the same id with
                // another payload is refused, like `ReserveOutgoingReaction`.
                match self.store.reserve_outgoing_reaction(
                    &request.client_reaction_id,
                    &request.chat_id,
                    &request.message_id,
                    &request.emoji,
                )? {
                    crate::db::ReactionReservation::Completed(timestamp) => {
                        let reaction = self
                            .stored_reaction(&request.chat_id, &request.message_id)
                            .unwrap_or_else(|| crate::db::ReactionRow {
                                chat_jid: request.chat_id.clone(),
                                message_id: request.message_id.clone(),
                                sender_jid: crate::send::own_sender_jid(&session),
                                emoji: request.emoji.clone(),
                                timestamp,
                                from_me: true,
                            });
                        return Ok(Handled {
                            result: rpc_response::Result::SendReaction(
                                whatfast_protocol::SendReactionResponse {
                                    reaction: Some(crate::wire::wire_reaction(&reaction)),
                                    removed,
                                },
                            ),
                            exit_after: false,
                        });
                    }
                    crate::db::ReactionReservation::Send => {}
                }
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
                let reaction = self
                    .stored_reaction(&request.chat_id, &request.message_id)
                    .unwrap_or_else(|| crate::db::ReactionRow {
                        chat_jid: request.chat_id.clone(),
                        message_id: request.message_id.clone(),
                        sender_jid: crate::send::own_sender_jid(&session),
                        emoji: request.emoji.clone(),
                        timestamp: crate::db::now_ms(),
                        from_me: true,
                    });
                self.store.complete_outgoing_reaction(
                    &request.client_reaction_id,
                    &request.chat_id,
                    &request.message_id,
                    &request.emoji,
                    reaction.timestamp,
                )?;
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
                if request.chat_id.is_empty()
                    || request.message_id.is_empty()
                    || request.text.is_empty()
                {
                    return Err(Failure::fatal(
                        "invalid_argument",
                        "chat_id, message_id and text are required",
                    ));
                }
                if request.text.len() > MAX_TEXT_BYTES {
                    return Err(Failure::fatal(
                        "invalid_argument",
                        "text must be valid UTF-8 up to 65536 bytes",
                    ));
                }
                let session = self.session.clone().ok_or_else(|| {
                    Failure::retryable("not_connected", "WhatsApp is not connected")
                })?;
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
                if request.chat_id.is_empty() || request.message_id.is_empty() {
                    return Err(Failure::fatal(
                        "invalid_argument",
                        "chat_id and message_id are required",
                    ));
                }
                let session = self.session.clone().ok_or_else(|| {
                    Failure::retryable("not_connected", "WhatsApp is not connected")
                })?;
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
                // The Go resolved the named message first and marked exactly
                // the unread rows at or before it, refusing an outgoing
                // anchor by name.
                let unread = self
                    .store
                    .unread_through(&request.chat_id, &request.through_message_id)?;
                let rows = match unread {
                    crate::db::UnreadThrough::Missing => {
                        return Err(Failure::fatal("not_found", "message is not stored"));
                    }
                    crate::db::UnreadThrough::Outgoing => {
                        return Err(Failure::fatal("internal", "message is outgoing"));
                    }
                    crate::db::UnreadThrough::Rows(rows) => rows,
                };
                let session = self.session.clone().ok_or_else(|| {
                    Failure::retryable("not_connected", "WhatsApp is not connected")
                })?;
                let receipt = self.mark_read_rows(&session, &request.chat_id, rows).await;
                // The Go published the counter after every attempt, including
                // a partially persisted multi-sender read, so the chat state
                // stays authoritative even when a later receipt fails.
                if let Some(chat) = self.store.chat(&request.chat_id)? {
                    self.push_event(crate::ingest::chat_upserted(chat));
                }
                receipt?;
                Ok(Handled {
                    result: rpc_response::Result::MarkRead(whatfast_protocol::MarkReadResponse {}),
                    exit_after: false,
                })
            }
            Some(rpc_request::Request::SetTyping(request)) => {
                if request.chat_id.is_empty() {
                    return Err(Failure::fatal("invalid_argument", "chat_id is required"));
                }
                let session = self.session.clone().ok_or_else(|| {
                    Failure::retryable("not_connected", "WhatsApp is not connected")
                })?;
                crate::send::set_typing(&session, &request.chat_id, request.typing)
                    .await
                    .map_err(|(_, message, _)| Failure::retryable("typing_unavailable", message))?;
                Ok(Handled {
                    result: rpc_response::Result::SetTyping(
                        whatfast_protocol::SetTypingResponse {},
                    ),
                    exit_after: false,
                })
            }
            Some(rpc_request::Request::SendImage(request)) => {
                if request.client_message_id.is_empty()
                    || request.chat_id.is_empty()
                    || request.image_path.is_empty()
                {
                    return Err(Failure::fatal(
                        "invalid_argument",
                        "client_message_id, chat_id and image_path are required",
                    ));
                }
                if !is_uuid(&request.client_message_id) {
                    return Err(Failure::fatal(
                        "invalid_argument",
                        "client_message_id must be a UUID",
                    ));
                }
                if request.caption.len() > MAX_CAPTION_BYTES {
                    return Err(Failure::fatal(
                        "invalid_argument",
                        "caption must be valid UTF-8 up to 4096 bytes",
                    ));
                }
                self.validate_reply_target(
                    &request.chat_id,
                    &request.reply_to_chat_id,
                    &request.reply_to_message_id,
                )?;
                if !self.connected() {
                    return Err(Failure::retryable(
                        "not_connected",
                        "WhatsApp is not connected",
                    ));
                }
                let session = self.session.clone().ok_or_else(|| {
                    Failure::retryable("not_connected", "WhatsApp is not connected")
                })?;
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
                let session = self.session.clone().ok_or_else(|| {
                    Failure::retryable("not_connected", "WhatsApp is not connected")
                })?;
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
                let session = self.session.clone().ok_or_else(|| {
                    Failure::retryable("not_connected", "WhatsApp is not connected")
                })?;
                calls_call_helper_accept(&session, &self.events, &request.call_id).await?;
                Ok(Handled {
                    result: rpc_response::Result::CallAccept(
                        whatfast_protocol::CallAcceptResponse {},
                    ),
                    exit_after: false,
                })
            }
            Some(rpc_request::Request::CallReject(request)) => {
                let session = self.session.clone().ok_or_else(|| {
                    Failure::retryable("not_connected", "WhatsApp is not connected")
                })?;
                calls_call_helper_reject(&session, &request.call_id).await?;
                Ok(Handled {
                    result: rpc_response::Result::CallReject(
                        whatfast_protocol::CallRejectResponse {},
                    ),
                    exit_after: false,
                })
            }
            Some(rpc_request::Request::CallHangup(request)) => {
                let session = self.session.clone().ok_or_else(|| {
                    Failure::retryable("not_connected", "WhatsApp is not connected")
                })?;
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
                let session = self.session.clone().ok_or_else(|| {
                    Failure::retryable("not_connected", "WhatsApp is not connected")
                })?;
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
                let session = self.session.clone().ok_or_else(|| {
                    Failure::retryable("not_connected", "WhatsApp is not connected")
                })?;
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
                if request.chat_id.is_empty() {
                    return Err(Failure::fatal("invalid_argument", "chat_id is required"));
                }
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
                if request.chat_id.is_empty() {
                    return Err(Failure::fatal("invalid_argument", "chat_id is required"));
                }
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
                if request.chat_id.is_empty() {
                    return Err(Failure::fatal("invalid_argument", "chat_id is required"));
                }
                if request.mute_until_ms < 0 {
                    return Err(Failure::fatal(
                        "invalid_argument",
                        "mute_until_ms must not be negative",
                    ));
                }
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
                if request.chat_id.is_empty() {
                    return Err(Failure::fatal("invalid_argument", "chat_id is required"));
                }
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
                if request.chat_id.is_empty() {
                    return Err(Failure::fatal("invalid_argument", "chat_id is required"));
                }
                let session = self.session.clone().ok_or_else(|| {
                    Failure::retryable("not_connected", "WhatsApp is not connected")
                })?;
                let path = self
                    .resolve_avatar(
                        &session,
                        &request.chat_id,
                        request.chat_id.ends_with("@g.us"),
                    )
                    .await?;
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
                if request.participant_id.is_empty() {
                    return Err(Failure::fatal(
                        "invalid_argument",
                        "participant_id is required",
                    ));
                }
                let session = self.session.clone().ok_or_else(|| {
                    Failure::retryable("not_connected", "WhatsApp is not connected")
                })?;
                let path = self
                    .resolve_avatar(&session, &request.participant_id, false)
                    .await?;
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
                if request.chat_id.is_empty() || request.message_id.is_empty() {
                    return Err(Failure::fatal(
                        "invalid_argument",
                        "chat_id and message_id are required",
                    ));
                }
                let session = self.session.clone().ok_or_else(|| {
                    Failure::retryable("not_connected", "WhatsApp is not connected")
                })?;
                let connected = self.connected();
                let path = crate::media::fetch_message_media(
                    &session,
                    &self.store,
                    &self.media_dir,
                    &request.chat_id,
                    &request.message_id,
                )
                .await
                .map_err(|(_, message, _)| Failure {
                    code: "attachment_unavailable".into(),
                    message,
                    retryable: connected,
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
                if request.chat_id.is_empty() || request.message_id.is_empty() {
                    return Err(Failure::fatal(
                        "invalid_argument",
                        "chat_id and message_id are required",
                    ));
                }
                let session = self.session.clone().ok_or_else(|| {
                    Failure::retryable("not_connected", "WhatsApp is not connected")
                })?;
                let connected = self.connected();
                let path = crate::media::fetch_message_media(
                    &session,
                    &self.store,
                    &self.media_dir,
                    &request.chat_id,
                    &request.message_id,
                )
                .await
                .map_err(|(_, message, _)| Failure {
                    code: "image_unavailable".into(),
                    message,
                    retryable: connected,
                })?;
                Ok(Handled {
                    result: rpc_response::Result::GetMessageImage(
                        whatfast_protocol::GetMessageImageResponse {
                            chat_id: request.chat_id,
                            message_id: request.message_id,
                            image_path: path,
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
                if request.client_message_id.is_empty()
                    || request.chat_id.is_empty()
                    || request.file_path.is_empty()
                {
                    return Err(Failure::fatal(
                        "invalid_argument",
                        "client_message_id, chat_id and file_path are required",
                    ));
                }
                if !is_uuid(&request.client_message_id) {
                    return Err(Failure::fatal(
                        "invalid_argument",
                        "client_message_id must be a UUID",
                    ));
                }
                if request.caption.len() > MAX_CAPTION_BYTES {
                    return Err(Failure::fatal(
                        "invalid_argument",
                        "caption must be valid UTF-8 up to 4096 bytes",
                    ));
                }
                let kind = match request.kind() {
                    whatfast_protocol::AttachmentKind::Document => "document",
                    whatfast_protocol::AttachmentKind::Video => "video",
                    whatfast_protocol::AttachmentKind::Audio => "audio",
                    whatfast_protocol::AttachmentKind::Unspecified => {
                        return Err(Failure::fatal(
                            "invalid_argument",
                            "kind must be document, video or audio",
                        ));
                    }
                };
                if kind == "audio" && !request.caption.is_empty() {
                    return Err(Failure::fatal(
                        "invalid_argument",
                        "audio messages do not support captions",
                    ));
                }
                if kind != "audio" && request.voice_note {
                    return Err(Failure::fatal(
                        "invalid_argument",
                        "voice_note is only valid for audio attachments",
                    ));
                }
                self.validate_reply_target(
                    &request.chat_id,
                    &request.reply_to_chat_id,
                    &request.reply_to_message_id,
                )?;
                if !self.connected() {
                    return Err(Failure::retryable(
                        "not_connected",
                        "WhatsApp is not connected",
                    ));
                }
                let session = self.session.clone().ok_or_else(|| {
                    Failure::retryable("not_connected", "WhatsApp is not connected")
                })?;
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
                if request.client_message_id.is_empty()
                    || request.chat_id.is_empty()
                    || request.webp_data.is_empty()
                {
                    return Err(Failure::fatal(
                        "invalid_argument",
                        "client_message_id, chat_id and webp_data are required",
                    ));
                }
                if !is_uuid(&request.client_message_id) {
                    return Err(Failure::fatal(
                        "invalid_argument",
                        "client_message_id must be a UUID",
                    ));
                }
                self.validate_reply_target(
                    &request.chat_id,
                    &request.reply_to_chat_id,
                    &request.reply_to_message_id,
                )?;
                if !self.connected() {
                    return Err(Failure::retryable(
                        "not_connected",
                        "WhatsApp is not connected",
                    ));
                }
                let session = self.session.clone().ok_or_else(|| {
                    Failure::retryable("not_connected", "WhatsApp is not connected")
                })?;
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
                if request.client_message_id.is_empty()
                    || request.chat_id.is_empty()
                    || request.sticker_id.is_empty()
                {
                    return Err(Failure::fatal(
                        "invalid_argument",
                        "client_message_id, chat_id and sticker_id are required",
                    ));
                }
                if !is_uuid(&request.client_message_id) {
                    return Err(Failure::fatal(
                        "invalid_argument",
                        "client_message_id must be a UUID",
                    ));
                }
                self.validate_reply_target(
                    &request.chat_id,
                    &request.reply_to_chat_id,
                    &request.reply_to_message_id,
                )?;
                if !self.connected() {
                    return Err(Failure::retryable(
                        "not_connected",
                        "WhatsApp is not connected",
                    ));
                }
                let session = self.session.clone().ok_or_else(|| {
                    Failure::retryable("not_connected", "WhatsApp is not connected")
                })?;
                let source = self
                    .store
                    .message_by_sticker_id(&request.sticker_id)?
                    .ok_or_else(|| {
                        Failure::fatal(
                            "invalid_argument",
                            "sticker_id was not found in the library",
                        )
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
                let question = request.question.trim();
                if request.chat_id.is_empty()
                    || request.client_message_id.is_empty()
                    || question.is_empty()
                    || !(2..=12).contains(&request.options.len())
                {
                    return Err(Failure::fatal(
                        "invalid_argument",
                        "poll requires a chat, UUID, question, and 2-12 options",
                    ));
                }
                if !is_uuid(&request.client_message_id) {
                    return Err(Failure::fatal(
                        "invalid_argument",
                        "client_message_id must be a UUID",
                    ));
                }
                if request.selectable_options_count < 1
                    || request.selectable_options_count as usize > request.options.len()
                {
                    return Err(Failure::fatal(
                        "invalid_argument",
                        "selectable_options_count must be between 1 and the option count",
                    ));
                }
                let mut normalized: Vec<String> = Vec::with_capacity(request.options.len());
                for option in &request.options {
                    let option = option.trim();
                    if option.is_empty()
                        || option.len() > 100
                        || normalized.iter().any(|seen| seen == option)
                    {
                        return Err(Failure::fatal(
                            "invalid_argument",
                            "poll options must be unique non-empty strings up to 100 bytes",
                        ));
                    }
                    normalized.push(option.to_owned());
                }
                if !self.connected() {
                    return Err(Failure::retryable(
                        "not_connected",
                        "WhatsApp is not connected",
                    ));
                }
                let session = self.session.clone().ok_or_else(|| {
                    Failure::retryable("not_connected", "WhatsApp is not connected")
                })?;
                let row = crate::send::create_poll(
                    &session,
                    &self.store,
                    &request.client_message_id,
                    &request.chat_id,
                    question,
                    &normalized,
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
                if request.client_message_id.is_empty()
                    || request.source_chat_id.is_empty()
                    || request.message_id.is_empty()
                    || request.target_chat_id.is_empty()
                {
                    return Err(Failure::fatal(
                        "invalid_argument",
                        "client_message_id, source_chat_id, message_id and target_chat_id are required",
                    ));
                }
                if !is_uuid(&request.client_message_id) {
                    return Err(Failure::fatal(
                        "invalid_argument",
                        "client_message_id must be a UUID",
                    ));
                }
                if !self.connected() {
                    return Err(Failure::retryable(
                        "not_connected",
                        "WhatsApp is not connected",
                    ));
                }
                let session = self.session.clone().ok_or_else(|| {
                    Failure::retryable("not_connected", "WhatsApp is not connected")
                })?;
                // The forward carries its own client id: a retry with the same
                // id and the same source/target replays the stored copy, and
                // the id with another payload is refused. `send::forward_message`
                // predates the id, so the reservation is kept here.
                let fingerprint = format!(
                    "forward\u{0}{}\u{0}{}\u{0}{}",
                    request.source_chat_id, request.message_id, request.target_chat_id
                );
                if let Some((chat, stored, message_id)) =
                    self.store.outgoing_request(&request.client_message_id)?
                {
                    if stored != fingerprint {
                        return Err(Failure::fatal(
                            "internal",
                            "client_message_id already used with different payload",
                        ));
                    }
                    let row = self.store.message(&chat, &message_id)?.ok_or_else(|| {
                        Failure::fatal("not_found", "the forwarded copy is not stored")
                    })?;
                    return Ok(Handled {
                        result: rpc_response::Result::ForwardMessage(
                            whatfast_protocol::ForwardMessageResponse {
                                message: Some(crate::wire::wire_message(&row)),
                            },
                        ),
                        exit_after: false,
                    });
                }
                let row = crate::send::forward_message(
                    &session,
                    &self.store,
                    &request.source_chat_id,
                    &request.message_id,
                    &request.target_chat_id,
                )
                .await
                .map_err(|(code, message, retryable)| Failure {
                    // The Go answered its forward refusal as invalid_argument.
                    code: if code == "not_forwardable" {
                        "invalid_argument".into()
                    } else {
                        code
                    },
                    message,
                    retryable,
                })?;
                self.store.remember_outgoing_request(
                    &request.client_message_id,
                    &request.target_chat_id,
                    &fingerprint,
                    &row.id,
                )?;
                Ok(Handled {
                    result: rpc_response::Result::ForwardMessage(
                        whatfast_protocol::ForwardMessageResponse {
                            message: Some(crate::wire::wire_message(&row)),
                        },
                    ),
                    exit_after: false,
                })
            }
            Some(rpc_request::Request::OpenContact(request)) => {
                if request.contact_jid.is_empty() {
                    return Err(Failure::fatal(
                        "invalid_argument",
                        "contact_jid is required",
                    ));
                }
                let chat = self.open_contact(&request.contact_jid)?;
                Ok(Handled {
                    result: rpc_response::Result::OpenContact(OpenContactResponse {
                        chat: Some(chat),
                    }),
                    exit_after: false,
                })
            }
            Some(rpc_request::Request::ListStatuses(_)) => {
                // The Go resolved the special conversation first: without a
                // stored status chat there is nothing to list.
                let chat_id = self.store.resolve_chat_id("status@broadcast")?;
                if self.store.chat(&chat_id)?.is_none() {
                    return Err(Failure::fatal(
                        "not_found",
                        "the status conversation is not stored",
                    ));
                }
                let items = self
                    .store
                    .status_messages_since(crate::db::now_ms() - STATUS_WINDOW_MS)?;
                // One group per author, in the order the first status of the
                // author appears: newest author first. The origin keyed the
                // groups by sender and never split one author in two, so a
                // contact that posts twice with someone else in between still
                // gets a single ring.
                let mut groups: Vec<StatusGroup> = Vec::new();
                let mut position_of: std::collections::HashMap<&str, usize> =
                    std::collections::HashMap::new();
                for item in &items {
                    let author = item.sender_jid.as_str();
                    let mut wire = crate::wire::wire_message(item);
                    if let Some(&position) = position_of.get(author) {
                        groups[position].messages.push(wire);
                        continue;
                    }
                    let (sender_name, sender_phone_number) =
                        status_identity(&self.store, self.session.as_deref(), author);
                    wire.sender_name.clone_from(&sender_name);
                    wire.sender_phone_number.clone_from(&sender_phone_number);
                    position_of.insert(author, groups.len());
                    groups.push(StatusGroup {
                        sender_id: item.sender_jid.clone(),
                        sender_name,
                        sender_phone_number,
                        messages: vec![wire],
                    });
                }
                Ok(Handled {
                    result: rpc_response::Result::ListStatuses(
                        whatfast_protocol::ListStatusesResponse { chat_id, groups },
                    ),
                    exit_after: false,
                })
            }
            Some(rpc_request::Request::SubscribeNewsletter(request)) => {
                let Some(key) = newsletter_invite_key(&request.invite_key) else {
                    return Err(Failure::fatal("invalid_argument", "invite_key is required"));
                };
                let session = self.session.clone().ok_or_else(|| {
                    Failure::fatal("newsletter_unavailable", "WhatsApp is not connected")
                })?;
                let connected = self.connected();
                let metadata = session
                    .client
                    .newsletter()
                    .get_metadata_by_invite(&key)
                    .await
                    .map_err(|error| Failure {
                        code: "newsletter_unavailable".into(),
                        message: format!("{error}"),
                        retryable: connected,
                    })?;
                let jid = crate::ingest::jid_text(&metadata.jid);
                self.store.upsert_newsletter(&jid, &metadata.name, false)?;
                // Live counter updates are best effort: the Go only logged a
                // failure here.
                if let Err(error) = session
                    .client
                    .newsletter()
                    .subscribe_live_updates(&metadata.jid)
                    .await
                {
                    tracing::warn!("subscribe channel live updates: {error}");
                }
                let newsletter = Newsletter {
                    jid,
                    name: metadata.name,
                    subscribed_at_ms: crate::db::now_ms(),
                    muted: false,
                };
                Ok(Handled {
                    result: rpc_response::Result::SubscribeNewsletter(
                        whatfast_protocol::SubscribeNewsletterResponse {
                            newsletter: Some(newsletter),
                        },
                    ),
                    exit_after: false,
                })
            }
            Some(rpc_request::Request::ListNewsletterMessages(request)) => {
                if request.jid.is_empty() {
                    return Err(Failure::fatal("invalid_argument", "jid is required"));
                }
                let session = self.session.clone().ok_or_else(|| {
                    Failure::fatal("newsletter_unavailable", "WhatsApp is not connected")
                })?;
                let connected = self.connected();
                let messages = self
                    .newsletter_messages(&session, &request.jid, request.limit)
                    .await
                    .map_err(|failure| Failure {
                        code: "newsletter_unavailable".into(),
                        message: failure.message,
                        retryable: connected,
                    })?;
                Ok(Handled {
                    result: rpc_response::Result::ListNewsletterMessages(
                        whatfast_protocol::ListNewsletterMessagesResponse { messages },
                    ),
                    exit_after: false,
                })
            }
            Some(rpc_request::Request::UnsubscribeNewsletter(request)) => {
                if request.jid.is_empty() {
                    return Err(Failure::fatal("invalid_argument", "jid is required"));
                }
                // The Go dropped the subscription from its own store; the
                // WhatsApp server learns it from the channel itself.
                self.store.remove_newsletter(&request.jid)?;
                Ok(Handled {
                    result: rpc_response::Result::UnsubscribeNewsletter(
                        whatfast_protocol::UnsubscribeNewsletterResponse {},
                    ),
                    exit_after: false,
                })
            }
            Some(rpc_request::Request::ListStickers(_)) => {
                let packs = self.sticker_library()?;
                Ok(Handled {
                    result: rpc_response::Result::ListStickers(
                        whatfast_protocol::ListStickersResponse { packs },
                    ),
                    exit_after: false,
                })
            }
            Some(rpc_request::Request::SetMessagePin(request)) => {
                if request.chat_id.is_empty() || request.message_id.is_empty() {
                    return Err(Failure::fatal(
                        "invalid_argument",
                        "chat_id and message_id are required",
                    ));
                }
                if !self.connected() {
                    return Err(Failure::retryable(
                        "not_connected",
                        "WhatsApp is not connected",
                    ));
                }
                let session = self.session.clone().ok_or_else(|| {
                    Failure::retryable("not_connected", "WhatsApp is not connected")
                })?;
                self.set_message_pin(
                    &session,
                    &request.chat_id,
                    &request.message_id,
                    request.pinned,
                )
                .await?;
                Ok(Handled {
                    result: rpc_response::Result::SetMessagePin(SetMessagePinResponse {}),
                    exit_after: false,
                })
            }
            Some(rpc_request::Request::ListPinnedMessages(request)) => {
                if request.chat_id.is_empty() {
                    return Err(Failure::fatal("invalid_argument", "chat_id is required"));
                }
                let pins = self.pinned_messages(&request.chat_id)?;
                Ok(Handled {
                    result: rpc_response::Result::ListPinnedMessages(
                        whatfast_protocol::ListPinnedMessagesResponse { pins },
                    ),
                    exit_after: false,
                })
            }
            Some(rpc_request::Request::GetChatHistoryCoverage(request)) => {
                if request.chat_id.is_empty() {
                    return Err(Failure::fatal("invalid_argument", "chat_id is required"));
                }
                let coverage = self.store.history_coverage(&request.chat_id)?;
                Ok(Handled {
                    result: rpc_response::Result::GetChatHistoryCoverage(
                        GetChatHistoryCoverageResponse {
                            coverage: Some(Self::wire_history_coverage(&coverage)),
                        },
                    ),
                    exit_after: false,
                })
            }
            Some(rpc_request::Request::GetHistoryOverview(_)) => {
                let overview = self.store.history_overview()?;
                Ok(Handled {
                    result: rpc_response::Result::GetHistoryOverview(GetHistoryOverviewResponse {
                        overview: Some(HistoryOverview {
                            local_message_count: overview.local_message_count,
                            chats_with_messages: overview.chats_with_messages,
                            chats_checked: overview.chats_checked,
                            chats_unknown: overview.chats_unknown,
                            chats_without_anchor: overview.chats_without_anchor,
                        }),
                    }),
                    exit_after: false,
                })
            }
            Some(rpc_request::Request::GetIntegrityStatus(_)) => {
                let status = self.store.integrity_status()?;
                Ok(Handled {
                    result: rpc_response::Result::GetIntegrityStatus(GetIntegrityStatusResponse {
                        status: Some(Self::wire_integrity_status(&status)),
                    }),
                    exit_after: false,
                })
            }
            Some(rpc_request::Request::RepairLocalCache(_)) => {
                let status = self.store.repair_local_cache()?;
                Ok(Handled {
                    result: rpc_response::Result::RepairLocalCache(
                        whatfast_protocol::RepairLocalCacheResponse {
                            status: Some(Self::wire_integrity_status(&status)),
                        },
                    ),
                    exit_after: false,
                })
            }
            Some(rpc_request::Request::RequestOlderHistory(request)) => {
                let coverage = self.request_older_history(&request.chat_id).await?;
                Ok(Handled {
                    result: rpc_response::Result::RequestOlderHistory(
                        whatfast_protocol::RequestOlderHistoryResponse {
                            coverage: Some(Self::wire_history_coverage(&coverage)),
                        },
                    ),
                    exit_after: false,
                })
            }
            Some(rpc_request::Request::VotePoll(request)) => {
                let message = self
                    .vote_poll(
                        &request.chat_id,
                        &request.poll_message_id,
                        &request.selected_options,
                    )
                    .await?;
                Ok(Handled {
                    result: rpc_response::Result::VotePoll(VotePollResponse {
                        message: Some(message),
                    }),
                    exit_after: false,
                })
            }
            Some(rpc_request::Request::RepairRecentReactions(request)) => {
                let (requested, attempts) = self.repair_recent_reactions(&request.chat_id).await?;
                Ok(Handled {
                    result: rpc_response::Result::RepairRecentReactions(
                        RepairRecentReactionsResponse {
                            chat_id: request.chat_id.clone(),
                            requested,
                            attempts,
                        },
                    ),
                    exit_after: false,
                })
            }
            Some(rpc_request::Request::Shutdown(_)) => Ok(Handled {
                result: rpc_response::Result::Shutdown(ShutdownResponse {}),
                exit_after: true,
            }),
            // The drafts live in `agents.db`, which the serve loop owns: the
            // three are answered there, before a request reaches this match.
            Some(rpc_request::Request::DraftPropose(_))
            | Some(rpc_request::Request::DraftList(_))
            | Some(rpc_request::Request::DraftDecide(_))
            | Some(rpc_request::Request::DraftFinish(_)) => Err(Failure::fatal(
                "internal",
                "the drafts of the agents are answered by the serve loop",
            )),
            // The fast mode runs another program and waits for it: it is
            // answered by the serve loop, off the request path.
            Some(rpc_request::Request::AgentAsk(_)) => Err(Failure::fatal(
                "internal",
                "the questions to an agent are answered by the serve loop",
            )),
            // The guide writes a file on the machine: the serve loop owns that
            // path too.
            Some(rpc_request::Request::GuideInstall(_))
            | Some(rpc_request::Request::GuideRemove(_))
            | Some(rpc_request::Request::GuideStatus(_)) => Err(Failure::fatal(
                "internal",
                "the guide of the agents is answered by the serve loop",
            )),
            // The automations live in `agents.db` too, next to the drafts.
            Some(rpc_request::Request::ScheduleList(_))
            | Some(rpc_request::Request::SchedulePut(_))
            | Some(rpc_request::Request::SchedulePause(_))
            | Some(rpc_request::Request::ScheduleRemove(_)) => Err(Failure::fatal(
                "internal",
                "the automations are answered by the serve loop",
            )),
            // No catch-all: the match covers every variant of the contract,
            // so a new RPC fails to compile here until it gets a real arm.
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
        let chat = self.store.chat(chat_id)?.ok_or_else(|| {
            // The Go answered "unknown chat" as invalid_argument, not as a
            // missing resource.
            Failure::fatal("invalid_argument", "unknown chat")
        })?;
        let mut response = whatfast_protocol::GetChatInfoResponse {
            chat: Some(crate::wire::wire_chat(&chat)),
            address: chat.address_jid.clone(),
            ..Default::default()
        };
        if chat.address_jid.ends_with("@g.us") {
            // The Go asked the server for group facts and refused the call
            // when the client was not ready or the lookup failed.
            let Some(session) = &self.session else {
                return Err(Failure::retryable(
                    "chat_info_unavailable",
                    "whatsapp client is not ready",
                ));
            };
            let parsed = chat_id.parse::<whatsapp_rust::Jid>().map_err(|error| {
                Failure::fatal("invalid_argument", format!("not a WhatsApp JID: {error}"))
            })?;
            let metadata = session
                .client
                .groups()
                .fetch_metadata(&parsed)
                .await
                .map_err(|error| Failure::retryable("chat_info_unavailable", error.to_string()))?;
            {
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
                        let (contact_name, push_name, business_name, phone) =
                            self.store.contact(&jid).ok().flatten().unwrap_or_default();
                        let display_name = [
                            contact_name,
                            push_name,
                            business_name,
                            participant
                                .phone_number
                                .as_ref()
                                .map(crate::ingest::jid_text)
                                .unwrap_or_default(),
                            phone,
                        ]
                        .into_iter()
                        .find(|candidate| !candidate.is_empty())
                        .unwrap_or_else(|| jid.clone());
                        // The Go matched the member against both own
                        // identities: the phone number and the hidden one.
                        let own = [
                            session
                                .client
                                .pn()
                                .map(|pn| crate::ingest::jid_text(&pn.to_non_ad())),
                            session
                                .client
                                .lid()
                                .map(|lid| crate::ingest::jid_text(&lid.to_non_ad())),
                        ];
                        let phone_number = participant
                            .phone_number
                            .as_ref()
                            .map(crate::ingest::jid_text)
                            .unwrap_or_default();
                        let is_me = own.iter().flatten().any(|own| {
                            *own == jid || (!phone_number.is_empty() && *own == phone_number)
                        });
                        whatfast_protocol::ChatParticipant {
                            participant_id: jid,
                            display_name,
                            phone_number,
                            // The Go folded the super admin into `IsAdmin`.
                            is_admin: matches!(
                                participant.participant_type,
                                whatsapp_rust::ParticipantType::Admin
                                    | whatsapp_rust::ParticipantType::SuperAdmin
                            ),
                            is_super_admin: matches!(
                                participant.participant_type,
                                whatsapp_rust::ParticipantType::SuperAdmin
                            ),
                            is_me,
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

    /// Whether the live session reports a connected WhatsApp link, the
    /// `IsConnected` the Go used for `retryable`.
    fn connected(&self) -> bool {
        self.session
            .as_ref()
            .is_some_and(|session| session.state.connection() == ConnectionState::Connected)
    }

    /// Maps store rows to wire messages, with the poll definitions the page
    /// needs. One lookup per page, not one per row.
    fn wire_messages(
        &self,
        rows: &[crate::db::MessageRow],
    ) -> rusqlite::Result<Vec<whatfast_protocol::Message>> {
        if rows.is_empty() {
            return Ok(Vec::new());
        }
        let chat = rows[0].chat_jid.clone();
        let ids: Vec<String> = rows.iter().map(|row| row.id.clone()).collect();
        let polls = self.store.poll_snapshots(&chat, &ids)?;
        Ok(rows
            .iter()
            .map(|row| {
                let poll = polls.iter().find(|poll| poll.message_id == row.id);
                let mut wire = crate::wire::wire_message_polled(row, poll);
                if !row.from_me && !row.sender_jid.is_empty() {
                    let (name, phone) =
                        status_identity(&self.store, self.session.as_deref(), &row.sender_jid);
                    wire.sender_name = name;
                    wire.sender_phone_number = phone;
                }
                wire
            })
            .collect())
    }

    /// The reaction this account already stored on one message, if any: the
    /// row `send::send_reaction` wrote carries the real sender.
    fn stored_reaction(&self, chat_id: &str, message_id: &str) -> Option<crate::db::ReactionRow> {
        self.store
            .message(chat_id, message_id)
            .ok()
            .flatten()
            .and_then(|message| {
                message
                    .reactions
                    .into_iter()
                    .find(|reaction| reaction.from_me)
            })
    }

    /// The Go `validateReplyTarget`: a reply chat without its message is a
    /// bug, and a message that is not stored cannot be quoted.
    fn validate_reply_target(
        &self,
        chat_id: &str,
        reply_chat_id: &str,
        reply_message_id: &str,
    ) -> Result<(), Failure> {
        if reply_message_id.is_empty() {
            if !reply_chat_id.is_empty() {
                return Err(Failure::fatal(
                    "invalid_argument",
                    "reply_to_chat_id requires reply_to_message_id",
                ));
            }
            return Ok(());
        }
        let reply_chat = if reply_chat_id.is_empty() {
            chat_id
        } else {
            reply_chat_id
        };
        match self.store.message(reply_chat, reply_message_id) {
            Ok(Some(_)) => Ok(()),
            Ok(None) => Err(Failure::fatal(
                "invalid_argument",
                "reply target was not found in this chat",
            )),
            Err(error) => Err(Failure::from(error)),
        }
    }

    /// The avatar answer for one jid, revalidated when the cached answer is
    /// older than a day, the Go cache's TTL. A failure is `avatar_unavailable`
    /// and retryable, like the Go's `ChatAvatar`/`Avatar` wrapper.
    async fn resolve_avatar(
        &self,
        session: &Session,
        jid: &str,
        group: bool,
    ) -> Result<String, Failure> {
        if crate::meta::avatar_known(&self.media_dir, jid)
            && avatar_answer_is_fresh(&self.media_dir, jid)
        {
            return Ok(crate::meta::cached_avatar_path(&self.media_dir, jid));
        }
        crate::meta::ensure_avatar(&session.client, &self.media_dir, jid, group)
            .await
            .map_err(|(_, message, _)| Failure::retryable("avatar_unavailable", message))
    }

    /// Sends the read receipts for the unread rows the desktop acknowledged,
    /// grouped by transport chat and author like the Go `MarkRead`, and marks
    /// exactly those rows read.
    async fn mark_read_rows(
        &self,
        session: &Session,
        chat_id: &str,
        rows: Vec<crate::db::UnreadRow>,
    ) -> Result<(), Failure> {
        if rows.is_empty() {
            // No flagged row, but the cached counter may still say unread:
            // recompute it so a drifted badge heals on open.
            self.store.mark_read_ids(chat_id, &[])?;
            return Ok(());
        }
        let mut groups: Vec<(String, String, Vec<String>)> = Vec::new();
        for row in &rows {
            match groups
                .iter_mut()
                .find(|group| group.0 == row.transport_jid && group.1 == row.sender_jid)
            {
                Some(group) => group.2.push(row.id.clone()),
                None => groups.push((
                    row.transport_jid.clone(),
                    row.sender_jid.clone(),
                    vec![row.id.clone()],
                )),
            }
        }
        for (transport, sender, ids) in groups {
            let Ok(chat) = transport.parse::<whatsapp_rust::Jid>() else {
                return Err(Failure::fatal(
                    "invalid_argument",
                    format!("not a WhatsApp JID: {transport}"),
                ));
            };
            let sender_jid = sender.parse::<whatsapp_rust::Jid>().ok();
            let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
            session
                .client
                .mark_as_read(&chat, sender_jid.as_ref(), &ids)
                .await
                .map_err(|error| Failure::retryable("send_failed", format!("{error}")))?;
            let raw: Vec<String> = rows
                .iter()
                .filter(|row| row.transport_jid == transport && row.sender_jid == sender)
                .map(|row| row.id.clone())
                .collect();
            self.store.mark_read_ids(chat_id, &raw)?;
        }
        Ok(())
    }

    /// Opens (or adopts) the conversation of one direct contact and answers
    /// its chat row, the Go `OpenContact`: parse a direct jid, resolve both
    /// identity forms, fold the twins, and read the row.
    fn open_contact(&self, raw_jid: &str) -> Result<whatfast_protocol::Chat, Failure> {
        let parsed = raw_jid
            .parse::<whatsapp_rust::Jid>()
            .map_err(|_| Failure::fatal("invalid_argument", "invalid direct-contact JID"))?
            .to_non_ad();
        if !parsed.is_pn() && !parsed.is_lid() {
            return Err(Failure::fatal(
                "invalid_argument",
                "invalid direct-contact JID",
            ));
        }
        let transport = parsed.to_string();
        let mut addresses = vec![transport.clone()];
        if let Some(session) = &self.session {
            for alias in session.identity.aliases(&transport) {
                if alias != transport && !addresses.contains(&alias) {
                    addresses.push(alias);
                }
            }
        }
        let chat_id = self.adopt_conversation(&addresses)?;
        let chat = self
            .store
            .chat(&chat_id)?
            .ok_or_else(|| Failure::fatal("not_found", "no such conversation"))?;
        Ok(crate::wire::wire_chat(&chat))
    }

    /// Registers every address form under one conversation, folding whatever
    /// twins the store already had. The Go `EnsureConversation` did the same
    /// inside the store; this port owns the fold in the reconciliation pass,
    /// so the dispatch asks for it explicitly here.
    fn adopt_conversation(&self, addresses: &[String]) -> Result<String, Failure> {
        let existing = self.store.chats_of_addresses(addresses)?;
        // A chat keyed by the hidden identity wins, the Go rule: its
        // `preferred_jid` is the LID the phone keeps using.
        let winner = existing
            .iter()
            .find(|jid| jid.ends_with("@lid"))
            .or_else(|| existing.first())
            .cloned()
            .unwrap_or_else(|| addresses[0].clone());
        self.store.ensure_chat(&winner, &winner)?;
        for other in &existing {
            if other != &winner {
                self.store.merge_chats(&winner, other)?;
                self.push_event(chat_merged(other, &winner));
            }
        }
        for address in addresses {
            if let Some((old, _)) = self.store.adopt_address(address, &winner)?
                && old != winner
            {
                self.store.merge_chats(&winner, &old)?;
                self.push_event(chat_merged(&old, &winner));
            }
        }
        Ok(winner)
    }

    /// The channel posts of one newsletter, newest first, mapped to wire
    /// messages. The limit is the Go `NewsletterMessages` clamp: zero, a
    /// negative or an over-100 limit becomes 25.
    async fn newsletter_messages(
        &self,
        session: &Session,
        jid: &str,
        limit: i32,
    ) -> Result<Vec<whatfast_protocol::Message>, Failure> {
        let parsed = jid
            .parse::<whatsapp_rust::Jid>()
            .map_err(|_| Failure::fatal("newsletter_unavailable", "invalid channel jid"))?;
        if !parsed.is_newsletter() {
            return Err(Failure::fatal(
                "newsletter_unavailable",
                "invalid channel jid",
            ));
        }
        let count = if limit <= 0 || limit > 100 {
            25
        } else {
            limit as u32
        };
        let posts = session
            .client
            .newsletter()
            .get_messages(&parsed, count, None)
            .await
            .map_err(|error| Failure {
                code: "newsletter_unavailable".into(),
                message: format!("{error}"),
                retryable: false,
            })?;
        Ok(posts
            .iter()
            .filter_map(|post| newsletter_post_row(jid, post))
            .map(|row| crate::wire::wire_message(&row))
            .collect())
    }

    /// The sticker library, the three packs the Go `StickerLibrary` answered:
    /// the synced favourites, the account's own recent sends, and every
    /// sticker message still in local history.
    fn sticker_library(&self) -> rusqlite::Result<Vec<whatfast_protocol::StickerPack>> {
        let favorites = self.store.favorite_stickers(0)?;
        let favorite_ids: Vec<&str> = favorites
            .iter()
            .map(|favorite| favorite.id.as_str())
            .collect();
        let favorite_pack = favorites
            .iter()
            .map(|favorite| whatfast_protocol::Sticker {
                id: favorite.id.clone(),
                // Nothing downloads a favourite yet: the Go cached the bytes
                // through `DownloadFavoriteSticker`, a path this port's media
                // module does not have.
                local_path: favorite.local_path.clone(),
                mime_type: favorite.mime.clone(),
                animated: favorite.animated,
                width: favorite.width as i32,
                height: favorite.height as i32,
                favorite: true,
                last_used_ms: favorite.updated_at,
                source_chat_id: String::new(),
                source_message_id: String::new(),
            })
            .collect();
        let candidates = self.store.recent_sticker_messages(0)?;
        let mut all_pack = Vec::with_capacity(candidates.len().min(STICKER_PACK_LIMIT));
        let mut recent_pack = Vec::with_capacity(candidates.len().min(STICKER_PACK_LIMIT));
        for candidate in &candidates {
            if all_pack.len() >= STICKER_PACK_LIMIT && recent_pack.len() >= STICKER_PACK_LIMIT {
                break;
            }
            let favorite = favorite_ids.contains(&candidate.id.as_str());
            if all_pack.len() < STICKER_PACK_LIMIT
                && !all_pack
                    .iter()
                    .any(|sticker: &whatfast_protocol::Sticker| sticker.id == candidate.id)
            {
                all_pack.push(self.sticker_candidate(candidate, favorite));
            }
            if candidate.from_me
                && recent_pack.len() < STICKER_PACK_LIMIT
                && !recent_pack
                    .iter()
                    .any(|sticker: &whatfast_protocol::Sticker| sticker.id == candidate.id)
            {
                recent_pack.push(self.sticker_candidate(candidate, favorite));
            }
        }
        Ok(vec![
            whatfast_protocol::StickerPack {
                id: "favorites".into(),
                name: "Favourites".into(),
                stickers: favorite_pack,
            },
            whatfast_protocol::StickerPack {
                id: "recent".into(),
                name: "Recently used".into(),
                stickers: recent_pack,
            },
            whatfast_protocol::StickerPack {
                id: "all".into(),
                name: "From your chats".into(),
                stickers: all_pack,
            },
        ])
    }

    /// One history sticker as a wire entry: the cache path when the file is
    /// already there, the message that can supply it otherwise.
    fn sticker_candidate(
        &self,
        candidate: &crate::db::StickerCandidateRow,
        favorite: bool,
    ) -> whatfast_protocol::Sticker {
        let cached = crate::media::cached_media_path(
            &self.media_dir,
            &candidate.chat_jid,
            &candidate.message_id,
            &candidate.mime,
            "",
        );
        let local_path = if cached.exists() {
            cached.to_string_lossy().into_owned()
        } else {
            String::new()
        };
        whatfast_protocol::Sticker {
            id: candidate.id.clone(),
            local_path,
            mime_type: candidate.mime.clone(),
            animated: candidate.animated,
            width: candidate.width as i32,
            height: candidate.height as i32,
            favorite,
            last_used_ms: candidate.timestamp,
            source_chat_id: candidate.chat_jid.clone(),
            source_message_id: candidate.message_id.clone(),
        }
    }

    /// Pins or unpins one stored message for everyone, the Go
    /// `SetMessagePin`: group admins only, then the add-on, then the local
    /// pin table and its change event.
    async fn set_message_pin(
        &self,
        session: &Session,
        chat_id: &str,
        message_id: &str,
        pinned: bool,
    ) -> Result<(), Failure> {
        let target = self
            .store
            .message(chat_id, message_id)?
            .ok_or_else(|| Failure::fatal("not_found", "message is not stored"))?;
        let chat = target
            .transport_jid
            .parse::<whatsapp_rust::Jid>()
            .map_err(|_| {
                Failure::fatal(
                    "invalid_argument",
                    "not a WhatsApp JID: the stored transport address",
                )
            })?;
        if chat.is_group() {
            let metadata = session
                .client
                .groups()
                .fetch_metadata(&chat)
                .await
                .map_err(|error| Failure {
                    code: "send_failed".into(),
                    message: format!("{error}"),
                    retryable: true,
                })?;
            let own = session
                .client
                .pn()
                .map(|pn| crate::ingest::jid_text(&pn.to_non_ad()));
            let allowed = metadata.participants.iter().any(|participant| {
                let is_me = Some(crate::ingest::jid_text(&participant.jid)) == own
                    || participant
                        .phone_number
                        .as_ref()
                        .map(|phone| Some(crate::ingest::jid_text(phone)) == own)
                        .unwrap_or(false);
                is_me
                    && matches!(
                        participant.participant_type,
                        whatsapp_rust::ParticipantType::Admin
                            | whatsapp_rust::ParticipantType::SuperAdmin
                    )
            });
            if !allowed {
                return Err(Failure::fatal(
                    "permission_denied",
                    "only group admins can pin messages",
                ));
            }
        }
        let mut key = whatsapp_rust::waproto::whatsapp::MessageKey::default();
        key.remote_jid = Some(chat.to_string());
        key.from_me = Some(target.from_me);
        key.id = Some(target.id.clone());
        if chat.is_group() && !target.from_me {
            key.participant = Some(target.sender_jid.clone());
        }
        if pinned {
            // The Go sent no duration; the library always carries one in the
            // add-on, so the WhatsApp default (24 hours) applies.
            session
                .client
                .pin_message_raw(chat, key, whatsapp_rust::PinDuration::Hours24)
                .await
                .map_err(|error| Failure::retryable("send_failed", format!("{error}")))?;
        } else {
            session
                .client
                .unpin_message_raw(chat, key)
                .await
                .map_err(|error| Failure::retryable("send_failed", format!("{error}")))?;
        }
        let pinned_by = session
            .client
            .pn()
            .map(|pn| crate::ingest::jid_text(&pn.to_non_ad()))
            .unwrap_or_default();
        self.store.set_message_pinned(
            chat_id,
            message_id,
            &pinned_by,
            crate::db::now_ms(),
            pinned,
        )?;
        self.push_event(whatfast_protocol::BackendEvent {
            sequence: 0,
            event: Some(
                whatfast_protocol::backend_event::Event::PinnedMessagesChanged(
                    whatfast_protocol::PinnedMessagesChanged {
                        chat_id: chat_id.to_owned(),
                    },
                ),
            ),
        });
        Ok(())
    }

    /// The chat's live pins, newest first, each with its message when the row
    /// is still stored.
    fn pinned_messages(
        &self,
        chat_id: &str,
    ) -> Result<Vec<whatfast_protocol::PinnedMessage>, Failure> {
        let pins: Vec<crate::db::PinnedRow> = self.store.pinned_messages(chat_id)?;
        let mut wired = Vec::with_capacity(pins.len());
        for pin in pins {
            let row = self.store.message(chat_id, &pin.message_id)?;
            wired.push(whatfast_protocol::PinnedMessage {
                message_id: pin.message_id,
                pinned_at_ms: pin.pinned_at,
                pinned_by: pin.pinned_by,
                message: row.as_ref().map(crate::wire::wire_message),
                available: row.is_some(),
            });
        }
        Ok(wired)
    }

    /// The wire coverage of one stored row, the Go `wireHistoryCoverage`
    /// with the same state names.
    fn wire_history_coverage(data: &HistoryCoverageData) -> HistoryCoverage {
        let state = match data.state.as_str() {
            "requesting" => HistoryCoverageState::Requesting,
            "more_available" => HistoryCoverageState::MoreAvailable,
            "exhausted" => HistoryCoverageState::Exhausted,
            "no_anchor" => HistoryCoverageState::NoAnchor,
            "offline" => HistoryCoverageState::Offline,
            "failed" => HistoryCoverageState::Failed,
            "stalled" => HistoryCoverageState::Stalled,
            _ => HistoryCoverageState::Unknown,
        };
        HistoryCoverage {
            chat_id: data.chat_id.clone(),
            state: state as i32,
            revision: data.revision,
            local_message_count: data.local_message_count,
            oldest_message_timestamp_ms: data.oldest_message_timestamp,
            on_demand_messages_added: data.on_demand_added,
            last_requested_count: data.last_requested_count,
            last_received_count: data.last_received_count,
            last_added_count: data.last_added_count,
            last_requested_at_ms: data.last_requested_at,
            retry_after_ms: data.retry_after,
            detail: data.detail.clone(),
        }
    }

    /// The wire health of one stored status, the Go `wireIntegrityStatus`.
    fn wire_integrity_status(status: &IntegrityStatusData) -> IntegrityStatus {
        IntegrityStatus {
            database_ok: status.database_ok,
            detail: status.detail.clone(),
            pending_inbound: status.pending_inbound,
            failed_inbound: status.failed_inbound,
            oldest_pending_at_ms: status.oldest_pending_at,
            last_message_persisted_at_ms: status.last_message_persisted_at,
            last_reconciliation_at_ms: status.last_reconciliation_at,
            inconsistent_chats: status.inconsistent_chats,
        }
    }

    /// One coverage change as the desktop event, the Go `emitHistoryCoverage`.
    fn history_coverage_changed(data: &HistoryCoverageData) -> BackendEvent {
        BackendEvent {
            sequence: 0,
            event: Some(
                whatfast_protocol::backend_event::Event::HistoryCoverageChanged(
                    HistoryCoverageChanged {
                        coverage: Some(Self::wire_history_coverage(data)),
                    },
                ),
            ),
        }
    }

    /// Asks WhatsApp for the 50 messages older than the anchor, the Go
    /// `RequestOlderHistory`: the coverage state machine, the busy window
    /// and the offline path included. A successful send leaves the row in
    /// `requesting` and arms a 30 s expiry: history that lands grows the
    /// local count, and the expiry then reports `more_available` instead of
    /// `failed`, so a fulfilled request never ends as a failure.
    async fn request_older_history(&self, chat_id: &str) -> Result<HistoryCoverageData, Failure> {
        if chat_id.is_empty() {
            return Err(Failure::fatal("invalid_argument", "chat_id is required"));
        }
        let coverage = self.store.history_coverage(chat_id)?;
        let now = now_ms();
        if coverage.local_message_count == 0 {
            return Err(Failure::fatal(
                "history_unavailable",
                "no local message is available to anchor an older-history request",
            ));
        }
        if coverage.state == "exhausted" {
            return Err(Failure::fatal(
                "history_unavailable",
                "WhatsApp returned no older messages for this chat",
            ));
        }
        if coverage.retry_after > now {
            return Err(Failure::retryable(
                "history_busy",
                "older-history request is cooling down",
            ));
        }
        {
            let mut in_flight = self.older_history.lock().expect("older-history lock");
            if in_flight.is_some() {
                return Err(Failure::retryable(
                    "history_busy",
                    "another older-history request is already in progress",
                ));
            }
            *in_flight = Some((coverage.chat_id.clone(), now));
        }
        if !self.connected() {
            let saved = self.store.save_history_coverage(HistoryCoverageData {
                state: "offline".into(),
                detail: "WhatsApp is offline; reconnect before requesting older messages".into(),
                ..coverage
            })?;
            self.push_event(Self::history_coverage_changed(&saved));
            *self.older_history.lock().expect("older-history lock") = None;
            return Err(Failure::retryable(
                "not_connected",
                "WhatsApp is not connected",
            ));
        }
        let session = self.session.clone().ok_or_else(|| {
            *self.older_history.lock().expect("older-history lock") = None;
            Failure::retryable("not_connected", "WhatsApp is not connected")
        })?;
        let Some(anchor) = self.store.oldest_message_row(&coverage.chat_id)? else {
            *self.older_history.lock().expect("older-history lock") = None;
            return Err(Failure::fatal(
                "history_unavailable",
                "no local message is available to anchor an older-history request",
            ));
        };
        let transport = anchor
            .transport_jid
            .parse::<whatsapp_rust::Jid>()
            .map_err(|_| {
                *self.older_history.lock().expect("older-history lock") = None;
                Failure::fatal("invalid_argument", "not a WhatsApp JID: the stored anchor")
            })?;
        let before = coverage.local_message_count;
        let mut requesting = coverage;
        requesting.state = "requesting".into();
        requesting.last_requested_count = 50;
        requesting.last_received_count = 0;
        requesting.last_added_count = 0;
        requesting.last_requested_at = now;
        requesting.retry_after = 0;
        requesting.detail = "Requesting 50 older messages from WhatsApp".into();
        let requesting = self.store.save_history_coverage(requesting)?;
        self.push_event(Self::history_coverage_changed(&requesting));
        let send = session
            .client
            .fetch_message_history(&transport, &anchor.id, anchor.from_me, anchor.timestamp, 50)
            .await;
        if let Err(error) = send {
            let saved = self.store.save_history_coverage(HistoryCoverageData {
                state: "failed".into(),
                detail: format!("Failed to send older-history request: {error}"),
                retry_after: now_ms() + 15_000,
                ..requesting
            })?;
            self.push_event(Self::history_coverage_changed(&saved));
            *self.older_history.lock().expect("older-history lock") = None;
            return Err(Failure::retryable(
                "history_request_failed",
                format!("{error}"),
            ));
        }
        // The phone answers through the history-sync events, which land as
        // messages; when nothing lands in 30 s the request failed, the Go's
        // `expireOlderHistoryRequest`. The slot releases when the expiry
        // runs, so a second call while one runs still answers `history_busy`.
        let store = Arc::clone(&self.store);
        let events = self.events.clone();
        let in_flight = Arc::clone(&self.older_history);
        let chat_id = requesting.chat_id.clone();
        let requested_at = requesting.last_requested_at;
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            let current = match store.history_coverage(&chat_id) {
                Ok(current) => current,
                Err(_) => {
                    *in_flight.lock().expect("older-history lock") = None;
                    return;
                }
            };
            if current.state == "requesting" && current.last_requested_at == requested_at {
                let now = now_ms();
                let saved = if current.local_message_count > before {
                    HistoryCoverageData {
                        state: "more_available".into(),
                        detail: "More older messages can be requested".into(),
                        ..current
                    }
                } else {
                    HistoryCoverageData {
                        state: "failed".into(),
                        detail:
                            "WhatsApp did not return older messages before the request timed out"
                                .into(),
                        retry_after: now + 15_000,
                        ..current
                    }
                };
                if let Ok(saved) = store.save_history_coverage(saved) {
                    let _ = events
                        .send(BackendEvent {
                            sequence: 0,
                            event: Some(
                                whatfast_protocol::backend_event::Event::HistoryCoverageChanged(
                                    HistoryCoverageChanged {
                                        coverage: Some(Self::wire_history_coverage(&saved)),
                                    },
                                ),
                            ),
                        })
                        .await;
                }
            }
            let mut slot = in_flight.lock().expect("older-history lock");
            if *slot == Some((chat_id, requested_at)) {
                *slot = None;
            }
        });
        Ok(requesting)
    }

    /// Votes on a stored poll, the Go `VotePoll`: the target must be a poll
    /// with a stored definition, the options must exist and fit the
    /// selectable count, and the key the vote encrypts with must be stored
    /// (the creation stored it, the ingested definitions carry it).
    async fn vote_poll(
        &self,
        chat_id: &str,
        poll_message_id: &str,
        selected: &[String],
    ) -> Result<whatfast_protocol::Message, Failure> {
        if chat_id.is_empty() || poll_message_id.is_empty() {
            return Err(Failure::fatal(
                "invalid_argument",
                "chat_id and poll_message_id are required",
            ));
        }
        if !self.connected() {
            return Err(Failure::retryable(
                "not_connected",
                "WhatsApp is not connected",
            ));
        }
        let session = self
            .session
            .clone()
            .ok_or_else(|| Failure::retryable("not_connected", "WhatsApp is not connected"))?;
        let row = self
            .store
            .message(chat_id, poll_message_id)?
            .ok_or_else(|| Failure::fatal("not_found", "poll message is not stored"))?;
        if row.kind != "poll" {
            return Err(Failure::fatal("invalid_argument", "target is not a poll"));
        }
        let snapshots = self
            .store
            .poll_snapshots(&row.chat_jid, std::slice::from_ref(&row.id))?;
        let Some(definition) = snapshots.first() else {
            return Err(Failure::fatal(
                "invalid_argument",
                "poll has no stored definition",
            ));
        };
        if selected.len() > definition.selectable_count as usize && definition.selectable_count > 0
        {
            return Err(Failure::fatal(
                "invalid_argument",
                "too many poll options selected",
            ));
        }
        for name in selected {
            if !definition.options.iter().any(|option| &option.name == name) {
                return Err(Failure::fatal(
                    "invalid_argument",
                    format!("unknown poll option {name:?}"),
                ));
            }
        }
        let meta = self.store.poll_meta(&row.chat_jid, &row.id)?;
        let secret = meta
            .as_ref()
            .map(|meta| meta.message_secret.clone())
            .unwrap_or_default();
        if secret.is_empty() {
            return Err(Failure::fatal(
                "not_found",
                "the poll key is not stored for this vote",
            ));
        }
        let creator_name = if meta
            .as_ref()
            .is_some_and(|meta| !meta.creator_jid.is_empty())
        {
            meta.as_ref()
                .map(|meta| meta.creator_jid.clone())
                .unwrap_or_default()
        } else if row.from_me {
            session.state.own_user_id()
        } else {
            String::new()
        };
        if creator_name.is_empty() {
            return Err(Failure::fatal(
                "not_found",
                "the poll key is not stored for this vote",
            ));
        }
        let chat_jid = row
            .transport_jid
            .parse::<whatsapp_rust::Jid>()
            .map_err(|_| Failure::fatal("invalid_argument", "not a WhatsApp JID: the poll chat"))?;
        let creator_jid = creator_name.parse::<whatsapp_rust::Jid>().map_err(|_| {
            Failure::fatal("invalid_argument", "not a WhatsApp JID: the poll creator")
        })?;
        session
            .client
            .polls()
            .vote_raw(&chat_jid, &row.id, &creator_jid, &secret, selected)
            .await
            .map_err(|error| Failure::retryable("send_failed", format!("{error}")))?;
        let voter = session.state.own_user_id();
        self.store
            .apply_poll_vote(&row.chat_jid, &row.id, &voter, selected, now_ms(), true)?;
        self.push_event(crate::ingest::message_upserted(&row));
        let mut wired = self.wire_messages(std::slice::from_ref(&row))?;
        wired
            .pop()
            .ok_or_else(|| Failure::fatal("internal", "the voted poll left the store"))
    }

    /// Replays the pending placeholder resends of one chat, the Go
    /// `RepairRecentReactions`: without pending rows there is nothing to
    /// repair; every row goes out through its own resend request and only
    /// the answered ones complete.
    async fn repair_recent_reactions(&self, chat_id: &str) -> Result<(bool, u32), Failure> {
        if chat_id.is_empty() {
            return Err(Failure::fatal("invalid_argument", "chat_id is required"));
        }
        if !self.connected() {
            return Err(Failure::retryable(
                "not_connected",
                "WhatsApp is not connected",
            ));
        }
        let session = self
            .session
            .clone()
            .ok_or_else(|| Failure::retryable("not_connected", "WhatsApp is not connected"))?;
        let items = self.store.pending_legacy_replays(chat_id, 16)?;
        if items.is_empty() {
            return Err(Failure::fatal(
                "not_found",
                "chat is not marked for reaction repair",
            ));
        }
        let own = session
            .client
            .pn()
            .ok_or_else(|| Failure::retryable("not_connected", "WhatsApp is not connected"))?;
        let request_id = format!("repair-{}", now_ms());
        let mut attempts = 0_u32;
        let mut sent = 0_u32;
        let mut first_error = String::new();
        for item in &items {
            attempts = attempts.max(item.attempts + 1);
            self.store.mark_legacy_replay_requested(
                &item.chat_jid,
                &item.event_message_id,
                &request_id,
            )?;
            let chat_jid = item
                .transport_jid
                .parse::<whatsapp_rust::Jid>()
                .map_err(|_| {
                    Failure::fatal("invalid_argument", "not a WhatsApp JID: the repair chat")
                })?;
            let sender_jid = if item.from_me {
                own.clone()
            } else {
                item.sender_jid.parse::<whatsapp_rust::Jid>().map_err(|_| {
                    Failure::fatal("invalid_argument", "not a WhatsApp JID: the repair sender")
                })?
            };
            let timestamp = whatsapp_rust::chrono::DateTime::from_timestamp_millis(item.timestamp)
                .unwrap_or_else(whatsapp_rust::chrono::Utc::now);
            let info = std::sync::Arc::new(wacore::types::message::MessageInfo {
                source: wacore::types::message::MessageSource {
                    chat: chat_jid,
                    sender: sender_jid,
                    is_from_me: item.from_me,
                    is_group: item.transport_jid.ends_with("@g.us")
                        || item.transport_jid.ends_with("@broadcast"),
                    ..Default::default()
                },
                id: item.event_message_id.clone().into(),
                timestamp,
                ..Default::default()
            });
            match session
                .client
                .retry_pdo_placeholder_resend_request(&info)
                .await
            {
                Ok(_) => {
                    self.store
                        .complete_legacy_replay(&item.chat_jid, &item.event_message_id)?;
                    sent += 1;
                }
                Err(error) => {
                    if first_error.is_empty() {
                        first_error = format!("{error}");
                    }
                }
            }
        }
        if sent == 0 {
            return Err(Failure::retryable(
                "repair_request_failed",
                std::mem::take(&mut first_error),
            ));
        }
        Ok((true, attempts))
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

    /// A failure the desktop may retry: the Go marked connection, storage and
    /// network refusals that way.
    fn retryable(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_owned(),
            message: message.into(),
            retryable: true,
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
///
/// The Go `InitialMessageWindow` asked `MessagesAround` for `sideLimit=25`
/// (25 older, the anchor, 25 newer) and `MessagesBefore` for 50 when nothing
/// is unread.
fn open_window(
    store: &std::sync::Arc<Store>,
    chat_id: &str,
) -> rusqlite::Result<(Vec<crate::db::MessageRow>, bool, bool, String)> {
    let first_unread: Option<String> = store.read(|conn| {
        conn.query_row(
            "SELECT id FROM messages WHERE chat_jid=?1 AND unread=1 AND kind<>'reaction'
             ORDER BY timestamp ASC, id ASC LIMIT 1",
            rusqlite::params![chat_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
    })?;
    match first_unread {
        Some(anchor) => {
            let Some((rows, has_older, has_newer)) = store.messages_around(chat_id, &anchor, 25)?
            else {
                // The anchor vanished between the two reads: answer the
                // newest page, like the Go could not either.
                let (rows, has_older) = store.messages_before_page(chat_id, 0, "", 50)?;
                return Ok((rows, has_older, false, String::new()));
            };
            Ok((rows, has_older, has_newer, anchor))
        }
        None => {
            let (rows, has_older) = store.messages_before_page(chat_id, 0, "", 50)?;
            Ok((rows, has_older, false, String::new()))
        }
    }
}

/// Points one request's chat id at the canonical chat: the address and
/// redirect tables decide, and an unknown jid passes through untouched.
///
/// Requests whose chat is not their subject (the participant avatar keys on
/// the participant, the newsletter list has none) keep their own fields.
fn normalize_chat_id(request: &mut rpc_request::Request, store: &Store) {
    let field = match request {
        rpc_request::Request::ListMessages(r) => &mut r.chat_id,
        rpc_request::Request::ListMessagesAround(r) => &mut r.chat_id,
        rpc_request::Request::ListMessagesAfter(r) => &mut r.chat_id,
        rpc_request::Request::OpenMessageWindow(r) => &mut r.chat_id,
        rpc_request::Request::SendText(r) => &mut r.chat_id,
        rpc_request::Request::SendReaction(r) => &mut r.chat_id,
        rpc_request::Request::EditMessage(r) => &mut r.chat_id,
        rpc_request::Request::DeleteMessage(r) => &mut r.chat_id,
        rpc_request::Request::MarkRead(r) => &mut r.chat_id,
        rpc_request::Request::SetTyping(r) => &mut r.chat_id,
        rpc_request::Request::SendImage(r) => &mut r.chat_id,
        rpc_request::Request::SendAttachment(r) => &mut r.chat_id,
        rpc_request::Request::SendSticker(r) => &mut r.chat_id,
        rpc_request::Request::SendStickerFromLibrary(r) => &mut r.chat_id,
        rpc_request::Request::CreatePoll(r) => &mut r.chat_id,
        rpc_request::Request::ForwardMessage(r) => {
            if let Ok(canonical) = store.resolve_chat_id(&r.target_chat_id) {
                r.target_chat_id = canonical;
            }
            if let Ok(canonical) = store.resolve_chat_id(&r.source_chat_id) {
                r.source_chat_id = canonical;
            }
            return;
        }
        rpc_request::Request::GetMessageImage(r) => &mut r.chat_id,
        rpc_request::Request::GetMessageAttachment(r) => &mut r.chat_id,
        rpc_request::Request::GetChatInfo(r) => &mut r.chat_id,
        rpc_request::Request::GetChatAvatar(r) => &mut r.chat_id,
        rpc_request::Request::SetChatPin(r) => &mut r.chat_id,
        rpc_request::Request::SetChatArchive(r) => &mut r.chat_id,
        rpc_request::Request::SetChatMute(r) => &mut r.chat_id,
        rpc_request::Request::SetChatBlock(r) => &mut r.chat_id,
        rpc_request::Request::SetMessagePin(r) => &mut r.chat_id,
        rpc_request::Request::ListPinnedMessages(r) => &mut r.chat_id,
        rpc_request::Request::VotePoll(r) => &mut r.chat_id,
        rpc_request::Request::RequestOlderHistory(r) => &mut r.chat_id,
        rpc_request::Request::GetChatHistoryCoverage(r) => &mut r.chat_id,
        rpc_request::Request::RepairRecentReactions(r) => &mut r.chat_id,
        rpc_request::Request::CallPlace(r) => &mut r.chat_id,
        _ => return,
    };
    if field.is_empty() {
        return;
    }
    if let Ok(canonical) = store.resolve_chat_id(field) {
        *field = canonical;
    }
}

/// The largest text body the Go accepted: 65536 bytes, the proto's bound.
const MAX_TEXT_BYTES: usize = 65536;

/// The largest caption the Go accepted: 4096 bytes.
const MAX_CAPTION_BYTES: usize = 4096;

/// Whether the value has the canonical 8-4-4-4-12 UUID shape. The Go parsed
/// the id with `uuid.Parse`; the desktop only ever sends that form.
fn is_uuid(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 36 {
        return false;
    }
    for (index, byte) in bytes.iter().enumerate() {
        let separator = matches!(index, 8 | 13 | 18 | 23);
        if separator {
            if *byte != b'-' {
                return false;
            }
        } else if !byte.is_ascii_hexdigit() {
            return false;
        }
    }
    true
}

/// The Go `validReactionEmoji`: empty, or exactly one emoji grapheme up to 64
/// bytes. A cluster is counted without a segmentation crate by walking the
/// joiners the protocol uses (ZWJ, variation selectors, skin tones, keycaps
/// and regional-indicator pairs).
fn valid_reaction_emoji(value: &str) -> bool {
    if value.is_empty() {
        return true;
    }
    if value.len() > 64 {
        return false;
    }
    let mut clusters = 0usize;
    let mut open = false;
    let mut after_joiner = false;
    let mut regional_open = false;
    let mut chars = value.chars().peekable();
    while let Some(character) = chars.next() {
        match character {
            '\u{200d}' => {
                if !open {
                    return false;
                }
                after_joiner = true;
                regional_open = false;
            }
            '\u{fe0e}' | '\u{fe0f}' | '\u{20e3}' => {
                if !open {
                    return false;
                }
                after_joiner = false;
            }
            '0'..='9' | '#' | '*' => {
                // A keycap base only counts when its variation selector
                // follows.
                if chars.peek() != Some(&'\u{fe0f}') {
                    return false;
                }
                if !open || after_joiner {
                    clusters += 1;
                    open = true;
                }
                after_joiner = false;
                regional_open = false;
            }
            character if is_skin_tone(character) => {
                if !open {
                    return false;
                }
                after_joiner = false;
            }
            character if is_regional_indicator(character) => {
                if regional_open {
                    regional_open = false;
                } else {
                    clusters += 1;
                    open = true;
                    regional_open = true;
                }
                after_joiner = false;
            }
            character if is_emoji_base(character) => {
                if !after_joiner {
                    clusters += 1;
                    open = true;
                }
                after_joiner = false;
                regional_open = false;
            }
            _ => return false,
        }
    }
    clusters == 1 && !after_joiner
}

/// The emoji bases the Go accepted: the pictographic blocks plus the three
/// pictograms that live outside them.
fn is_emoji_base(character: char) -> bool {
    let code = character as u32;
    (0x1f000..=0x1faff).contains(&code)
        || (0x2600..=0x27bf).contains(&code)
        || matches!(character, '\u{a9}' | '\u{ae}' | '\u{2122}')
}

fn is_skin_tone(character: char) -> bool {
    (0x1f3fb..=0x1f3ff).contains(&(character as u32))
}

fn is_regional_indicator(character: char) -> bool {
    (0x1f1e6..=0x1f1ff).contains(&(character as u32))
}

/// The name and the phone number one status author is shown with.
///
/// port: `preferredContactName` of the Go: the saved name of the contact, then
/// its business name, then the push name it chose, then its phone number, and
/// the digits of the jid when the store knows nothing about the author. The
/// status senders arrive as `@lid`, so the store row is the only source of a
/// human name and the plain row of the message carries none.
fn status_identity(store: &Store, session: Option<&Session>, sender_jid: &str) -> (String, String) {
    let contact = store.contact(sender_jid).ok().flatten();
    let (mut name, mut phone) = match contact {
        Some((contact_name, push_name, business_name, phone_number)) => {
            let name = [contact_name, business_name, push_name, phone_number.clone()]
                .into_iter()
                .find(|candidate| !candidate.trim().is_empty())
                .unwrap_or_default();
            (name, phone_number)
        }
        None => (String::new(), String::new()),
    };
    if name.is_empty()
        && let Some(session) = session
        && let Some((id_name, id_phone)) = session.identity.lookup_name(sender_jid)
    {
        name = id_name;
        if phone.is_empty() {
            phone = id_phone;
        }
    }
    let digits = crate::wire::phone_digits(sender_jid);
    let name = if name.is_empty() {
        digits.clone()
    } else {
        name
    };
    let phone = if phone.is_empty() { digits } else { phone };
    (name, phone)
}

/// The invite key of a channel link, the Go `SubscribeNewsletter` trim: a
/// full `https://whatsapp.com/channel/<key>` keeps only its last segment.
fn newsletter_invite_key(raw: &str) -> Option<String> {
    let key = match raw.rsplit_once('/') {
        Some((_, tail)) if raw.contains("whatsapp.com/channel/") => tail,
        _ => raw,
    };
    let key = key.trim();
    if key.is_empty() || key.len() > 128 {
        return None;
    }
    Some(key.to_owned())
}

/// Whether the cached avatar answer for one jid is younger than the Go
/// cache's day. An expired answer is asked again instead of trusted forever.
fn avatar_answer_is_fresh(media_dir: &Path, jid: &str) -> bool {
    let prefix = format!("avatar-{}-", crate::meta::hash_hex(jid.as_bytes()));
    let Ok(entries) = std::fs::read_dir(media_dir) else {
        return false;
    };
    entries.filter_map(Result::ok).any(|entry| {
        if !entry.file_name().to_string_lossy().starts_with(&prefix) {
            return false;
        }
        entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|age| age < AVATAR_TTL)
    })
}

fn chat_merged(old_chat_id: &str, new_chat_id: &str) -> BackendEvent {
    BackendEvent {
        sequence: 0,
        event: Some(whatfast_protocol::backend_event::Event::ChatMerged(
            whatfast_protocol::ChatMerged {
                old_chat_id: old_chat_id.to_owned(),
                new_chat_id: new_chat_id.to_owned(),
            },
        )),
    }
}

/// One channel post as a store row, so the wire layer renders it exactly like
/// a stored message. Posts are not persisted: the Go mapped them on the way
/// out too.
fn newsletter_post_row(
    jid: &str,
    post: &whatsapp_rust::NewsletterMessage,
) -> Option<crate::db::MessageRow> {
    let content = post.message.as_ref()?;
    let mut row = crate::db::MessageRow {
        id: post
            .message_id
            .as_ref()
            .map(|id| id.to_string())
            .unwrap_or_default(),
        chat_jid: jid.to_owned(),
        transport_jid: jid.to_owned(),
        sender_jid: jid.to_owned(),
        timestamp: (post.timestamp as i64).saturating_mul(1000),
        from_me: post.is_sender,
        kind: "text".into(),
        ..Default::default()
    };
    fill_post_content(content, &mut row);
    Some(row)
}

/// The Go `extractMessageContent` arms a channel post actually uses. Content
/// kinds this port does not model (payments, events, interactive payloads)
/// stay as an empty text post.
fn fill_post_content(
    content: &whatsapp_rust::waproto::whatsapp::Message,
    row: &mut crate::db::MessageRow,
) {
    let text = |field: &Option<String>| field.clone().unwrap_or_default();
    if let Some(image) = content.image_message.as_option() {
        row.kind = "image".into();
        row.text = text(&image.caption);
        row.image_mime = text(&image.mimetype);
        row.image_caption = text(&image.caption);
        row.image_direct_path = text(&image.direct_path);
        row.image_media_key = image.media_key.clone().unwrap_or_default();
        row.image_file_sha256 = image.file_sha256.clone().unwrap_or_default();
        row.image_file_enc_sha256 = image.file_enc_sha256.clone().unwrap_or_default();
        row.image_width = image.width.unwrap_or(0) as i64;
        row.image_height = image.height.unwrap_or(0) as i64;
        row.image_size = image.file_length.unwrap_or(0) as i64;
        row.image_thumbnail = image.jpeg_thumbnail.clone().unwrap_or_default();
        return;
    }
    if let Some(sticker) = content.sticker_message.as_option() {
        row.kind = "sticker".into();
        row.text = "Sticker".into();
        row.image_mime = text(&sticker.mimetype);
        row.image_direct_path = text(&sticker.direct_path);
        row.image_media_key = sticker.media_key.clone().unwrap_or_default();
        row.image_file_sha256 = sticker.file_sha256.clone().unwrap_or_default();
        row.image_file_enc_sha256 = sticker.file_enc_sha256.clone().unwrap_or_default();
        row.image_width = sticker.width.unwrap_or(0) as i64;
        row.image_height = sticker.height.unwrap_or(0) as i64;
        row.image_size = sticker.file_length.unwrap_or(0) as i64;
        row.image_animated =
            sticker.is_animated.unwrap_or(false) || sticker.is_lottie.unwrap_or(false);
        return;
    }
    let video = content
        .video_message
        .as_option()
        .or_else(|| content.ptv_message.as_option());
    if let Some(video) = video {
        row.kind = "video".into();
        row.text = text(&video.caption);
        row.media_mime = text(&video.mimetype);
        row.media_caption = text(&video.caption);
        row.media_direct_path = text(&video.direct_path);
        row.media_media_key = video.media_key.clone().unwrap_or_default();
        row.media_file_sha256 = video.file_sha256.clone().unwrap_or_default();
        row.media_file_enc_sha256 = video.file_enc_sha256.clone().unwrap_or_default();
        row.media_width = video.width.unwrap_or(0) as i64;
        row.media_height = video.height.unwrap_or(0) as i64;
        row.media_size = video.file_length.unwrap_or(0) as i64;
        row.media_duration = video.seconds.unwrap_or(0) as i64;
        row.media_animated = video.gif_playback.unwrap_or(false);
        return;
    }
    if let Some(audio) = content.audio_message.as_option() {
        row.kind = "audio".into();
        let voice = audio.ptt.unwrap_or(false);
        row.text = if voice {
            "Voice message".into()
        } else {
            "Audio".into()
        };
        row.media_mime = text(&audio.mimetype);
        row.media_direct_path = text(&audio.direct_path);
        row.media_media_key = audio.media_key.clone().unwrap_or_default();
        row.media_file_sha256 = audio.file_sha256.clone().unwrap_or_default();
        row.media_file_enc_sha256 = audio.file_enc_sha256.clone().unwrap_or_default();
        row.media_size = audio.file_length.unwrap_or(0) as i64;
        row.media_duration = audio.seconds.unwrap_or(0) as i64;
        row.media_voice = voice;
        return;
    }
    let document = content.document_message.as_option().or_else(|| {
        content
            .document_with_caption_message
            .as_option()
            .and_then(|wrapper| wrapper.message.as_option())
            .and_then(|inner| inner.document_message.as_option())
    });
    if let Some(document) = document {
        row.kind = "document".into();
        let file_name = text(&document.file_name);
        let title = text(&document.title);
        row.text = [
            text(&document.caption),
            if file_name.is_empty() {
                title
            } else {
                file_name
            },
        ]
        .into_iter()
        .find(|candidate| !candidate.is_empty())
        .unwrap_or_else(|| "Document".into());
        row.media_mime = text(&document.mimetype);
        row.media_caption = text(&document.caption);
        row.media_file_name = text(&document.file_name);
        row.media_direct_path = text(&document.direct_path);
        row.media_media_key = document.media_key.clone().unwrap_or_default();
        row.media_file_sha256 = document.file_sha256.clone().unwrap_or_default();
        row.media_file_enc_sha256 = document.file_enc_sha256.clone().unwrap_or_default();
        row.media_size = document.file_length.unwrap_or(0) as i64;
        return;
    }
    if let Some(location) = content.location_message.as_option() {
        row.kind = "location".into();
        row.text = {
            let name = text(&location.name);
            let address = text(&location.address);
            if name.is_empty() { address } else { name }
        };
        row.location_lat = location.degrees_latitude.unwrap_or(0.0);
        row.location_lng = location.degrees_longitude.unwrap_or(0.0);
        row.location_name = text(&location.name);
        row.location_address = text(&location.address);
        row.location_url = text(&location.url);
        row.location_live = location.is_live.unwrap_or(false);
        return;
    }
    if let Some(live) = content.live_location_message.as_option() {
        row.kind = "location".into();
        row.text = text(&live.caption);
        row.location_lat = live.degrees_latitude.unwrap_or(0.0);
        row.location_lng = live.degrees_longitude.unwrap_or(0.0);
        row.location_name = text(&live.caption);
        row.location_live = true;
        return;
    }
    if let Some(poll) = poll_creation(content) {
        row.kind = "poll".into();
        let question = text(&poll.name);
        let mut poll_text = format!("Poll: {question}");
        for option in &poll.options {
            let name = text(&option.option_name);
            if !name.is_empty() {
                poll_text.push_str("\n• ");
                poll_text.push_str(&name);
            }
        }
        row.text = poll_text;
        return;
    }
    if let Some(conversation) = content.conversation.as_ref()
        && !conversation.is_empty()
    {
        row.text = conversation.clone();
        return;
    }
    if let Some(extended) = content.extended_text_message.as_option() {
        row.text = text(&extended.text);
        let matched = text(&extended.matched_text);
        if !matched.is_empty() {
            row.link_preview_url = matched;
            row.link_preview_title = text(&extended.title);
            row.link_preview_description = text(&extended.description);
            row.link_preview_thumbnail = extended.jpeg_thumbnail.clone().unwrap_or_default();
            row.link_preview_width = extended.thumbnail_width.unwrap_or(0) as i64;
            row.link_preview_height = extended.thumbnail_height.unwrap_or(0) as i64;
        }
    }
}

/// The poll payload of a channel post, whichever versioned field carried it.
fn poll_creation(
    content: &whatsapp_rust::waproto::whatsapp::Message,
) -> Option<&whatsapp_rust::waproto::whatsapp::message::PollCreationMessage> {
    content
        .poll_creation_message
        .as_option()
        .or_else(|| content.poll_creation_message_v2.as_option())
        .or_else(|| content.poll_creation_message_v3.as_option())
        .or_else(|| content.poll_creation_message_v5.as_option())
        .or_else(|| content.poll_creation_message_v6.as_option())
}

#[cfg(test)]
mod tests {
    use super::{is_uuid, newsletter_invite_key, valid_reaction_emoji};

    #[test]
    fn only_canonical_uuids_pass() {
        assert!(is_uuid("b0bca7f5-a85f-4a8c-8956-1722cf35ffbc"));
        assert!(!is_uuid(""));
        assert!(!is_uuid("not-a-uuid"));
        assert!(!is_uuid("b0bca7f5a85f4a8c89561722cf35ffbc"));
        assert!(!is_uuid("b0bca7f5-a85f-4a8c-8956-1722cf35ffb"));
        assert!(!is_uuid("b0bca7f5-a85f-4a8c-8956-1722cf35ffbg"));
        assert!(!is_uuid("b0bca7f5_a85f_4a8c_8956_1722cf35ffbc"));
    }

    /// The Go's table: one emoji grapheme, joiners included, and nothing that
    /// is plain text.
    #[test]
    fn a_reaction_emoji_is_one_emoji_grapheme() {
        for emoji in [
            "",
            "\u{1f44d}",
            "\u{1f44d}\u{1f3fb}",
            "\u{263a}\u{fe0f}",
            "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}\u{200d}\u{1f466}",
            "\u{1f1e6}\u{1f1f7}",
            "1\u{fe0f}\u{20e3}",
            "\u{2764}\u{fe0f}",
            "\u{a9}",
            "\u{2122}",
        ] {
            assert!(valid_reaction_emoji(emoji), "{emoji:?} should be valid");
        }
        for emoji in [
            "hello",
            " ",
            "\u{1f44d} \u{1f1ff}",
            "\u{1f44d}\u{1f1ff}",
            "\u{1f44d}\u{1f44d}",
            "\u{1f44d}\u{200d}",
            "\u{fe0f}",
            "1",
        ] {
            assert!(!valid_reaction_emoji(emoji), "{emoji:?} should be refused");
        }
    }

    #[test]
    fn an_invite_key_survives_a_full_channel_link() {
        assert_eq!(
            newsletter_invite_key("https://whatsapp.com/channel/0029Vabcdefghij0123456789")
                .as_deref(),
            Some("0029Vabcdefghij0123456789")
        );
        assert_eq!(
            newsletter_invite_key("  0029Vabc  ").as_deref(),
            Some("0029Vabc")
        );
        assert_eq!(newsletter_invite_key("   "), None);
        assert_eq!(newsletter_invite_key(&"x".repeat(129)), None);
    }
}
