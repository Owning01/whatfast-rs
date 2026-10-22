//! The deterministic fake backend of the protocol.
//!
//! Port of the fake server of `desktop/src/bridge.rs` of the origin repo.
//! The origin kept the simulator inside its GPUI client: it answered the
//! fifty RPCs of the protocol with a deterministic dataset (ten thousand
//! chats, a thousand messages per chat, photos, stickers, reactions, polls
//! and newsletters) so the interface could be developed and tested without a
//! WhatsApp account and without the network.
//!
//! Here the simulator is a server of the protocol: it takes the requests of
//! the client, answers them with the same payloads and publishes the events
//! on the same channel the real sidecar uses. The dataset is deterministic
//! (indices and hashes, no clock and no randomness), so the visual QA of the
//! native UI has a stable reference.
//!
//! Two decisions of this port:
//!
//! - The crate does not invent paths: the media of the fake (the photos and
//!   the audio) is written under the profile directory the binary resolved
//!   and passes in, and the files are private to their owner (`0600`).
//! - The photos are written by a small PNG encoder of this module instead of
//!   by the `image` crate, because the crate may not add a dependency that
//!   is not authorized. The pixels are the ones of the origin.

use std::{
    collections::HashMap,
    env,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use crate::transport::{BridgeMessage, PROTOCOL_VERSION};
use crate::{
    AttachmentContent, AttachmentKind, AuthStateResponse, BackendEvent, Chat, ChatKind,
    ChatParticipant, ChatUpserted, ConnectionState, Envelope, GetChatHistoryCoverageResponse,
    GetChatInfoResponse, GetHistoryOverviewResponse, GetIntegrityStatusResponse,
    GetMessageAttachmentResponse, GetMessageImageResponse, HelloResponse, HistoryCoverage,
    HistoryCoverageChanged, HistoryCoverageState, HistoryOverview, ImageContent, IntegrityStatus,
    LinkPreview, ListChatsResponse, ListMessagesAfterResponse, ListMessagesAroundResponse,
    ListMessagesResponse, ListNewsletterMessagesResponse, ListNewslettersResponse,
    ListPinnedMessagesResponse, ListStatusesResponse, ListStickersResponse, Message,
    MessageSearchResult, MessageStatus, MessageUpserted, Newsletter, OpenContactResponse,
    OpenMessageWindowResponse, PinnedMessage, PinnedMessagesChanged, PollContent, PollOption,
    PollVoter, Reaction, RpcError, RpcRequest, RpcResponse, SearchLocalResponse,
    SendAttachmentResponse, SendImageResponse, SendReactionResponse,
    SendStickerFromLibraryResponse, SendStickerResponse, SendTextResponse, Sticker, StickerPack,
    SubscribeNewsletterResponse, SyncPhase, SyncStatus, TextContent, backend_event, envelope,
    message as proto_message, rpc_request, rpc_response,
};

/// How many chats the fake has.
const TOTAL_CHATS: usize = 10_000;

/// How many messages a chat has in the default fixture.
const MESSAGE_TOTAL: usize = 1_000;

/// How many messages a chat has in the performance fixture.
const PERF_MESSAGE_TOTAL: usize = 2_000;

/// Every how many messages the dataset has a document.
///
/// The photo has its own stride (every 37 messages) and these four strides
/// are coprime between them and with the one of the photo, so no message of
/// the dataset carries two files.
const DOCUMENT_EVERY: usize = 59;

/// Every how many messages the dataset has a video.
const VIDEO_EVERY: usize = 73;

/// Every how many messages the dataset has an audio file.
const AUDIO_EVERY: usize = 67;

/// Every how many messages the dataset has a voice note.
const VOICE_NOTE_EVERY: usize = 97;

/// The name of the document of the dataset.
const DOCUMENT_FILE: &str = "informe.pdf";

/// The name of the video of the dataset.
const VIDEO_FILE: &str = "vacaciones.mp4";

/// The name of the audio file of the dataset.
const AUDIO_FILE: &str = "cancion.wav";

/// The name of the voice note of the dataset.
const VOICE_NOTE_FILE: &str = "nota-de-voz.wav";

/// How many participants a group has in the performance fixture.
const PERF_PARTICIPANTS: usize = 1_000;

/// The depth of the queue of the requests of the client.
const OUTGOING_CAPACITY: usize = 256;

/// The depth of the queue of the answers and the events of the fake.
const INCOMING_CAPACITY: usize = 1024;

/// The wait of the live loop between the pairs of events.
const LIVE_EVENT_INTERVAL: Duration = Duration::from_secs(4);

/// The wait of the live loop before the first pair of events.
const LIVE_EVENT_DELAY: Duration = Duration::from_secs(3);

/// Why the fake backend refused a request.
#[derive(Debug, thiserror::Error)]
pub enum FakeError {
    /// The queue of the requests is full or closed.
    #[error("the fake backend's request queue is full or closed")]
    QueueUnavailable,
    /// The request could not be registered in the table of the pending ones.
    #[error("the request could not be registered: {0}")]
    Request(#[from] crate::transport::TransportError),
}

/// The configuration of the fake backend.
#[derive(Clone, Debug)]
pub struct FakeConfig {
    /// The profile directory of the user.
    ///
    /// The fake writes its media under `<profile_dir>/media`: the crate does
    /// not invent paths, the binary passes the profile it resolved.
    pub profile_dir: PathBuf,
    /// The fake answers as an unpaired client and emits a pairing QR.
    pub pairing: bool,
    /// The performance fixture: the chats have more messages and the groups
    /// have more participants.
    pub performance_fixture: bool,
    /// Whether the live loop publishes a message and a chat update every
    /// four seconds once the handshake is done.
    pub live_events: bool,
}

impl FakeConfig {
    /// The configuration the origin read from the environment.
    ///
    /// `WHATFAST_FAKE_PAIRING` makes the fake answer as an unpaired client,
    /// `WHATFAST_PERF_FIXTURE` swaps the dataset for the performance one and
    /// `WHATFAST_FAKE_LIVE_EVENTS=0` turns the live loop off.
    pub fn from_env(profile_dir: impl Into<PathBuf>) -> Self {
        Self {
            profile_dir: profile_dir.into(),
            pairing: env::var_os("WHATFAST_FAKE_PAIRING").is_some(),
            performance_fixture: env::var_os("WHATFAST_PERF_FIXTURE").is_some(),
            live_events: env::var("WHATFAST_FAKE_LIVE_EVENTS").as_deref() != Ok("0"),
        }
    }

    /// The media tree of the profile: the only tree the asset protocol
    /// serves.
    pub fn media_dir(&self) -> PathBuf {
        self.profile_dir.join("media")
    }
}

/// A running fake backend: the same surface the sidecar transport exposes.
pub struct FakeBackend {
    outgoing: async_channel::Sender<Envelope>,
    /// The ordered stream of the answers and the events of the fake.
    pub incoming: async_channel::Receiver<BridgeMessage>,
}

impl FakeBackend {
    /// Start the fake backend with the configuration of the environment.
    pub fn start(profile_dir: impl Into<PathBuf>) -> Self {
        Self::with_config(FakeConfig::from_env(profile_dir))
    }

    /// Start the fake backend with an explicit configuration.
    pub fn with_config(config: FakeConfig) -> Self {
        let (outgoing, outgoing_rx) = async_channel::bounded(OUTGOING_CAPACITY);
        let (incoming_tx, incoming) = async_channel::bounded(INCOMING_CAPACITY);
        let event_sequence = Arc::new(Mutex::new(1_u64));
        let handshaken = Arc::new(AtomicBool::new(false));
        if config.live_events {
            let live_incoming = incoming_tx.clone();
            let live_sequence = event_sequence.clone();
            let live_handshaken = handshaken.clone();
            // A thread that cannot be spawned leaves the live loop out: the
            // client still gets the answers of the requests.
            let _ = thread::Builder::new()
                .name("fake-live-events".into())
                .spawn(move || fake_live_loop(live_incoming, live_sequence, live_handshaken));
        }
        let worker_sequence = event_sequence.clone();
        let worker_handshaken = handshaken.clone();
        // Same: a thread that cannot be spawned leaves the queue closed and
        // the client sees the backend as gone, like a sidecar that died.
        let _ = thread::Builder::new()
            .name("fake-backend".into())
            .spawn(move || {
                fake_loop(
                    outgoing_rx,
                    incoming_tx,
                    worker_sequence,
                    worker_handshaken,
                    config,
                )
            });
        Self { outgoing, incoming }
    }

    /// Queue a request for the fake backend.
    pub fn send(&self, request_id: u64, request: rpc_request::Request) -> Result<(), FakeError> {
        self.outgoing
            .try_send(Envelope {
                protocol_version: PROTOCOL_VERSION,
                request_id,
                body: Some(envelope::Body::Request(RpcRequest {
                    request: Some(request),
                })),
            })
            .map_err(|_| FakeError::QueueUnavailable)
    }

    /// Send a request under a deadline and remember it until it answers.
    ///
    /// The fake is a drop-in of the sidecar transport for the client: the
    /// table of the pending requests and the deadlines are the ones of the
    /// transport, so the UI can talk to the fake and to the sidecar with the
    /// same code.
    pub fn send_request(
        &self,
        tracker: &mut crate::transport::RequestTracker,
        class: crate::transport::RequestClass,
        request: rpc_request::Request,
        now: Duration,
    ) -> Result<u64, FakeError> {
        let request_id = tracker.begin(class, now)?;
        if let Err(error) = self.send(request_id, request) {
            // A request that could not be queued is not pending anymore.
            tracker.complete(request_id);
            return Err(error);
        }
        Ok(request_id)
    }
}

/// The coverage of the history of a chat, as the fake reports it.
pub fn fake_history_coverage(chat_id: String) -> HistoryCoverage {
    HistoryCoverage {
        chat_id,
        state: HistoryCoverageState::Unknown as i32,
        revision: 1,
        local_message_count: 1_000,
        oldest_message_timestamp_ms: 1_700_000_000_000,
        on_demand_messages_added: 0,
        last_requested_count: 0,
        last_received_count: 0,
        last_added_count: 0,
        last_requested_at_ms: 0,
        retry_after_ms: 0,
        detail: "Older WhatsApp history has not been checked".into(),
    }
}

/// The integrity of the local database, as the fake reports it.
pub fn fake_integrity_status() -> IntegrityStatus {
    IntegrityStatus {
        database_ok: true,
        detail: "Local database and derived chat state are consistent".into(),
        pending_inbound: 0,
        failed_inbound: 0,
        oldest_pending_at_ms: 0,
        last_message_persisted_at_ms: 1_700_000_000_000,
        last_reconciliation_at_ms: 1_700_000_000_000,
        inconsistent_chats: 0,
    }
}

/// Answer the requests of the client until the queue is closed.
///
/// The origin's fake was the simulator of the GPUI client: it took the
/// envelopes of the client, answered every request of the protocol and
/// published the events on the same stream.
pub fn fake_loop(
    outgoing: async_channel::Receiver<Envelope>,
    incoming: async_channel::Sender<BridgeMessage>,
    event_sequence: Arc<Mutex<u64>>,
    handshaken: Arc<AtomicBool>,
    config: FakeConfig,
) {
    let pairing = config.pairing;
    let performance_fixture = config.performance_fixture;
    let profile_dir = config.profile_dir.clone();
    let message_total = if performance_fixture {
        PERF_MESSAGE_TOTAL
    } else {
        MESSAGE_TOTAL
    };
    // In-memory pins so ListPinnedMessages reflects what SetMessagePin
    // stored, matching the real backend instead of always returning empty.
    // NOTE: pins live only for the lifetime of this fake client loop (per
    // fake backend instance) and are lost on restart; the real backend
    // persists them. This is intentional for the deterministic fixture.
    let mut pins: HashMap<String, Vec<PinnedMessage>> = HashMap::new();
    // In-memory chat settings so the fake mirrors the real backend: archive,
    // mute and block survive for the lifetime of the fake client loop and are
    // echoed back by the setter RPCs and by ListChats.
    let mut chat_settings: HashMap<String, Chat> = HashMap::new();
    while let Ok(envelope) = outgoing.recv_blocking() {
        let request_id = envelope.request_id;
        let Some(envelope::Body::Request(request)) = envelope.body else {
            continue;
        };
        let is_hello = matches!(
            request.request.as_ref(),
            Some(rpc_request::Request::Hello(_))
        );
        let result = match request.request {
            Some(rpc_request::Request::Hello(_)) => rpc_response::Result::Hello(HelloResponse {
                backend_version: "fake-0.1".into(),
                protocol_version: PROTOCOL_VERSION,
            }),
            Some(rpc_request::Request::GetAuthState(_)) => {
                rpc_response::Result::AuthState(AuthStateResponse {
                    paired: !pairing,
                    logged_in: !pairing,
                    own_user_id: "me@s.whatsapp.net".into(),
                    connection_state: if pairing {
                        ConnectionState::Pairing as i32
                    } else {
                        ConnectionState::Connected as i32
                    },
                })
            }
            Some(rpc_request::Request::StartPairing(_)) => {
                emit_fake_event(
                    &incoming,
                    &event_sequence,
                    backend_event::Event::PairingQr(crate::PairingQr {
                        code: "2@WHATFAST-FAKE-PAIRING-CODE".into(),
                        expires_at_ms: 4_102_444_800_000,
                    }),
                );
                rpc_response::Result::StartPairing(crate::StartPairingResponse { started: true })
            }
            Some(rpc_request::Request::GetSyncStatus(_)) => {
                rpc_response::Result::SyncStatus(crate::SyncStatusResponse {
                    status: Some(SyncStatus {
                        phase: SyncPhase::Complete as i32,
                        revision: 1,
                        chats_processed: 10_000,
                        messages_processed: message_total as u64,
                        messages_received: message_total as u64,
                        messages_decoded: message_total as u64,
                        messages_failed: 0,
                        whatsapp_progress: 100,
                        started_at_ms: 0,
                        completed_at_ms: 0,
                        detail: "Up to date".into(),
                    }),
                })
            }
            Some(rpc_request::Request::GetChatHistoryCoverage(request)) => {
                rpc_response::Result::GetChatHistoryCoverage(GetChatHistoryCoverageResponse {
                    coverage: Some(fake_history_coverage(request.chat_id)),
                })
            }
            Some(rpc_request::Request::RequestOlderHistory(request)) => {
                let mut coverage = fake_history_coverage(request.chat_id);
                coverage.state = HistoryCoverageState::Requesting as i32;
                coverage.detail = "Requesting 50 older messages from WhatsApp".into();
                let mut completed = coverage.clone();
                completed.state = HistoryCoverageState::Exhausted as i32;
                completed.revision += 1;
                completed.detail = "WhatsApp returned no older messages".into();
                emit_fake_event(
                    &incoming,
                    &event_sequence,
                    backend_event::Event::HistoryCoverageChanged(HistoryCoverageChanged {
                        coverage: Some(completed),
                    }),
                );
                rpc_response::Result::RequestOlderHistory(crate::RequestOlderHistoryResponse {
                    coverage: Some(coverage),
                })
            }
            Some(rpc_request::Request::GetHistoryOverview(_)) => {
                rpc_response::Result::GetHistoryOverview(GetHistoryOverviewResponse {
                    overview: Some(HistoryOverview {
                        local_message_count: message_total as u64,
                        chats_with_messages: 10_000,
                        chats_checked: 0,
                        chats_unknown: 10_000,
                        chats_without_anchor: 0,
                    }),
                })
            }
            Some(rpc_request::Request::GetIntegrityStatus(_)) => {
                rpc_response::Result::GetIntegrityStatus(GetIntegrityStatusResponse {
                    status: Some(fake_integrity_status()),
                })
            }
            Some(rpc_request::Request::RepairLocalCache(_)) => {
                rpc_response::Result::RepairLocalCache(crate::RepairLocalCacheResponse {
                    status: Some(fake_integrity_status()),
                })
            }
            Some(rpc_request::Request::ListChats(request)) => {
                let start = request.cursor.parse::<usize>().unwrap_or_default();
                let limit = request.limit.min(100) as usize;
                let total = TOTAL_CHATS;
                let chats = (start..(start + limit).min(total))
                    .map(fake_chat)
                    .map(|mut chat| {
                        if let Some(settings) = chat_settings.get(&chat.id) {
                            chat.pinned = settings.pinned;
                            chat.archived = settings.archived;
                            chat.muted = settings.muted;
                            chat.blocked = settings.blocked;
                            chat.mute_until_ms = settings.mute_until_ms;
                        }
                        chat
                    })
                    .collect();
                let next = if start + limit < total {
                    (start + limit).to_string()
                } else {
                    String::new()
                };
                rpc_response::Result::ListChats(ListChatsResponse {
                    chats,
                    total_count: total as u64,
                    next_cursor: next,
                })
            }
            Some(rpc_request::Request::ListMessages(request)) => {
                let end = if request.before_timestamp_ms == 0 {
                    message_total
                } else {
                    request.before_timestamp_ms as usize
                };
                let start = end.saturating_sub(request.limit as usize);
                let messages = (start..end)
                    .map(|id| fake_message(&request.chat_id, id, &profile_dir))
                    .collect();
                rpc_response::Result::ListMessages(ListMessagesResponse {
                    messages,
                    has_more: start > 0,
                })
            }
            Some(rpc_request::Request::OpenMessageWindow(request)) => {
                let anchor = message_total.saturating_sub(50);
                let start = anchor.saturating_sub(25);
                let end = (anchor + 26).min(message_total);
                rpc_response::Result::OpenMessageWindow(OpenMessageWindowResponse {
                    messages: (start..end)
                        .map(|id| fake_message(&request.chat_id, id, &profile_dir))
                        .collect(),
                    has_older: start > 0,
                    has_newer: end < message_total,
                    first_unread_message_id: format!("message-{anchor}"),
                })
            }
            Some(rpc_request::Request::ListMessagesAfter(request)) => {
                let start = request
                    .after_message_id
                    .strip_prefix("message-")
                    .and_then(|value| value.parse::<usize>().ok())
                    .map_or(0, |id| id + 1);
                let end = (start + request.limit as usize).min(message_total);
                rpc_response::Result::ListMessagesAfter(ListMessagesAfterResponse {
                    messages: (start..end)
                        .map(|id| fake_message(&request.chat_id, id, &profile_dir))
                        .collect(),
                    has_more: end < message_total,
                })
            }
            Some(rpc_request::Request::SearchLocal(request)) => {
                let query = request.query.to_lowercase();
                let contacts = (0..8)
                    .filter_map(|id| {
                        let chat = fake_chat(id);
                        (chat.title.to_lowercase().contains(&query)
                            || chat.phone_number.contains(&query))
                        .then_some(crate::ContactSearchResult {
                            contact_jid: format!("1555{id:07}@s.whatsapp.net"),
                            chat_id: chat.id,
                            display_name: chat.title,
                            secondary_name: chat.push_name,
                            phone_number: chat.phone_number,
                        })
                    })
                    .take(8)
                    .collect();
                let groups = (0..40)
                    .filter(|id| id % 5 == 0)
                    .map(fake_chat)
                    .filter(|chat| chat.title.to_lowercase().contains(&query))
                    .take(6)
                    .collect();
                let messages = if query.chars().count() >= 3 {
                    (message_total.saturating_sub(25)..message_total)
                        .map(|id| {
                            let chat = fake_chat(id % 20);
                            MessageSearchResult {
                                chat_id: chat.id.clone(),
                                message_id: format!("message-{id}"),
                                chat_title: chat.title.clone(),
                                sender_name: if id % 2 == 0 { "You" } else { "Fast friend" }.into(),
                                timestamp_ms: id as i64,
                                snippet: format!("Fast native message {id} matching {query}"),
                                kind: "text".into(),
                                archived: chat.archived,
                                chat: Some(chat),
                            }
                        })
                        .take(20)
                        .collect()
                } else {
                    Vec::new()
                };
                rpc_response::Result::SearchLocal(SearchLocalResponse {
                    contacts,
                    groups,
                    messages,
                })
            }
            Some(rpc_request::Request::OpenContact(_request)) => {
                let mut chat = fake_chat(10_001);
                chat.title = "New WhatsApp contact".into();
                rpc_response::Result::OpenContact(OpenContactResponse { chat: Some(chat) })
            }
            Some(rpc_request::Request::ListMessagesAround(request)) => {
                let anchor = request
                    .message_id
                    .strip_prefix("message-")
                    .and_then(|value| value.parse::<usize>().ok())
                    .unwrap_or(500);
                let start = anchor.saturating_sub(25);
                let end = (anchor + 26).min(message_total);
                rpc_response::Result::ListMessagesAround(ListMessagesAroundResponse {
                    messages: (start..end)
                        .map(|id| fake_message(&request.chat_id, id, &profile_dir))
                        .collect(),
                    has_older: start > 0,
                    has_newer: end < message_total,
                    anchor_message_id: request.message_id,
                })
            }
            Some(rpc_request::Request::GetChatAvatar(request)) => {
                let seed = chat_seed(&request.chat_id);
                let picture = ensure_fake_avatar(&profile_dir, seed);
                rpc_response::Result::GetChatAvatar(crate::GetChatAvatarResponse {
                    chat_id: request.chat_id,
                    avatar_path: picture.to_string_lossy().into_owned(),
                })
            }
            Some(rpc_request::Request::GetParticipantAvatar(request)) => {
                rpc_response::Result::GetParticipantAvatar(crate::GetParticipantAvatarResponse {
                    participant_id: request.participant_id,
                    avatar_path: String::new(),
                })
            }
            Some(rpc_request::Request::GetChatInfo(request)) => {
                let id = request
                    .chat_id
                    .strip_prefix("chat-")
                    .and_then(|value| value.parse::<usize>().ok())
                    .unwrap_or(0);
                let chat = fake_chat(id);
                let group = chat.kind == ChatKind::Group as i32;
                let participants: Vec<ChatParticipant> = if group {
                    (0..if performance_fixture {
                        PERF_PARTICIPANTS
                    } else {
                        12
                    })
                        .map(|index| ChatParticipant {
                            participant_id: format!("1555{index:07}@s.whatsapp.net"),
                            display_name: format!("Fast friend {index}"),
                            phone_number: format!("+1555{index:07}"),
                            is_admin: index < 2,
                            is_super_admin: index == 0,
                            is_me: index == 3,
                        })
                        .collect()
                } else {
                    Vec::new()
                };
                rpc_response::Result::GetChatInfo(GetChatInfoResponse {
                    address: if group {
                        format!("1234567890-{id}@g.us")
                    } else {
                        format!("1555{id:07}@s.whatsapp.net")
                    },
                    about: if group {
                        String::new()
                    } else {
                        "Hey there! I am using Whatfast.".into()
                    },
                    verified_name: if !group && id % 3 == 0 {
                        format!("Whatfast Business {id}")
                    } else {
                        String::new()
                    },
                    description: if group {
                        format!("Planning the weekend, one message at a time. Group {id}.")
                    } else {
                        String::new()
                    },
                    created_at_ms: if group { 1_712_345_678_000 } else { 0 },
                    created_by: if group {
                        "Fast friend 0".into()
                    } else {
                        String::new()
                    },
                    participant_count: participants.len() as u32,
                    participants,
                    announce_only: group && id % 10 == 0,
                    locked: group && id % 15 == 0,
                    disappearing_timer_seconds: if group && id % 20 == 0 {
                        7 * 24 * 3600
                    } else {
                        0
                    },
                    is_community: false,
                    join_approval_required: group && id % 25 == 0,
                    chat: Some(chat),
                })
            }
            Some(rpc_request::Request::SendText(request)) => {
                rpc_response::Result::SendText(SendTextResponse {
                    message: Some(Message {
                        id: request.client_message_id,
                        chat_id: request.chat_id,
                        sender_id: "me@s.whatsapp.net".into(),
                        sender_name: "You".into(),
                        from_me: true,
                        timestamp_ms: 1_900_000_000_000,
                        status: MessageStatus::Sent as i32,
                        content: Some(proto_message::Content::Text(TextContent {
                            text: request.text,
                            link_preview: None,
                        })),
                        reply_to_message_id: request.reply_to_message_id,
                        reply_to_chat_id: request.reply_to_chat_id,
                        ..Default::default()
                    }),
                })
            }
            Some(rpc_request::Request::ForwardMessage(request)) => {
                rpc_response::Result::ForwardMessage(crate::ForwardMessageResponse {
                    message: Some(Message {
                        id: request.client_message_id,
                        chat_id: request.target_chat_id,
                        sender_id: "me@s.whatsapp.net".into(),
                        sender_name: "You".into(),
                        from_me: true,
                        timestamp_ms: 1_900_000_000_000,
                        status: MessageStatus::Sent as i32,
                        forwarding_score: 1,
                        content: Some(proto_message::Content::Text(TextContent {
                            text: format!("Forwarded message {}", request.message_id),
                            link_preview: None,
                        })),
                        ..Default::default()
                    }),
                })
            }
            Some(rpc_request::Request::EditMessage(request)) => {
                rpc_response::Result::EditMessage(crate::EditMessageResponse {
                    message: Some(Message {
                        id: request.message_id,
                        chat_id: request.chat_id,
                        sender_id: "me@s.whatsapp.net".into(),
                        sender_name: "You".into(),
                        from_me: true,
                        timestamp_ms: 1_900_000_000_000,
                        status: MessageStatus::Sent as i32,
                        edited: true,
                        content: Some(proto_message::Content::Text(TextContent {
                            text: request.text,
                            link_preview: None,
                        })),
                        ..Default::default()
                    }),
                })
            }
            Some(rpc_request::Request::DeleteMessage(request)) => {
                let revoked_message = Message {
                    id: request.message_id.clone(),
                    chat_id: request.chat_id.clone(),
                    from_me: true,
                    timestamp_ms: 1_900_000_000_000,
                    status: MessageStatus::Sent as i32,
                    revoked: true,
                    ..Default::default()
                };
                emit_fake_event(
                    &incoming,
                    &event_sequence,
                    backend_event::Event::MessageUpserted(MessageUpserted {
                        message: Some(revoked_message.clone()),
                    }),
                );
                rpc_response::Result::DeleteMessage(crate::DeleteMessageResponse {
                    message: Some(revoked_message),
                })
            }
            Some(rpc_request::Request::ListStatuses(_)) => {
                let mut statuses = Vec::new();
                for (group_index, sender) in [
                    ("maya@s.whatsapp.net", "Maya Nair"),
                    ("priya@s.whatsapp.net", "Priya Nair"),
                ]
                .into_iter()
                .enumerate()
                {
                    let (sender_id, sender_name) = sender;
                    let messages = (0..2)
                        .map(|index| {
                            let mut message = fake_message(
                                "status@broadcast",
                                1_500 - group_index * 50 - index,
                                &profile_dir,
                            );
                            message.sender_id = sender_id.to_string();
                            message.sender_name = sender_name.to_string();
                            message.sender_phone_number = if group_index == 0 {
                                "5493811000001".into()
                            } else {
                                "5493811000002".into()
                            };
                            message
                        })
                        .collect();
                    statuses.push(crate::StatusGroup {
                        sender_id: sender_id.to_string(),
                        sender_name: sender_name.to_string(),
                        sender_phone_number: if group_index == 0 {
                            "5493811000001".into()
                        } else {
                            "5493811000002".into()
                        },
                        messages,
                    });
                }
                rpc_response::Result::ListStatuses(ListStatusesResponse {
                    chat_id: "status@broadcast".into(),
                    groups: statuses,
                })
            }
            Some(rpc_request::Request::SubscribeNewsletter(request)) => {
                // Truncate by chars, never by bytes: byte slicing (`&s[..n]`)
                // panics when n lands inside a multibyte char (emoji, CJK).
                let short_key: String = request.invite_key.chars().take(12).collect();
                let display_key: String = request.invite_key.chars().take(24).collect();
                let jid = format!("{short_key}@newsletter");
                rpc_response::Result::SubscribeNewsletter(SubscribeNewsletterResponse {
                    newsletter: Some(Newsletter {
                        jid: jid.clone(),
                        name: format!("Channel {display_key}"),
                        subscribed_at_ms: 1_900_000_000_000,
                        muted: false,
                    }),
                })
            }
            Some(rpc_request::Request::ListNewsletters(_)) => {
                rpc_response::Result::ListNewsletters(ListNewslettersResponse {
                    newsletters: vec![
                        Newsletter {
                            jid: "news@newsletter".into(),
                            name: "Whatfast News".into(),
                            subscribed_at_ms: 1_900_000_000_000,
                            muted: false,
                        },
                        Newsletter {
                            jid: "dev@newsletter".into(),
                            name: "Whatfast Dev".into(),
                            subscribed_at_ms: 1_890_000_000_000,
                            muted: false,
                        },
                    ],
                })
            }
            Some(rpc_request::Request::ListNewsletterMessages(request)) => {
                let messages = (0..8)
                    .map(|index| {
                        let mut message = fake_message(&request.jid, 1_000 - index, &profile_dir);
                        message.chat_id = request.jid.clone();
                        message.sender_id = request.jid.clone();
                        message.sender_name = "Channel".into();
                        message
                    })
                    .collect();
                rpc_response::Result::ListNewsletterMessages(ListNewsletterMessagesResponse {
                    messages,
                })
            }
            Some(rpc_request::Request::UnsubscribeNewsletter(_)) => {
                rpc_response::Result::UnsubscribeNewsletter(crate::UnsubscribeNewsletterResponse {})
            }
            Some(rpc_request::Request::SetChatPin(request)) => {
                let chat = chat_settings
                    .entry(request.chat_id.clone())
                    .or_insert_with(|| fixture_chat(&request.chat_id, None));
                chat.pinned = request.pinned;
                rpc_response::Result::SetChatPin(crate::SetChatPinResponse {
                    chat: Some(fixture_chat(&request.chat_id, Some(chat))),
                })
            }
            Some(rpc_request::Request::SetChatArchive(request)) => {
                let chat = chat_settings
                    .entry(request.chat_id.clone())
                    .or_insert_with(|| fixture_chat(&request.chat_id, None));
                chat.archived = request.archived;
                rpc_response::Result::SetChatArchive(crate::SetChatArchiveResponse {
                    chat: Some(fixture_chat(&request.chat_id, Some(chat))),
                })
            }
            Some(rpc_request::Request::SetChatMute(request)) => {
                let chat = chat_settings
                    .entry(request.chat_id.clone())
                    .or_insert_with(|| fixture_chat(&request.chat_id, None));
                chat.mute_until_ms = request.mute_until_ms;
                chat.muted = request.mute_until_ms > 0;
                rpc_response::Result::SetChatMute(crate::SetChatMuteResponse {
                    chat: Some(fixture_chat(&request.chat_id, Some(chat))),
                })
            }
            Some(rpc_request::Request::SetChatBlock(request)) => {
                let chat = chat_settings
                    .entry(request.chat_id.clone())
                    .or_insert_with(|| fixture_chat(&request.chat_id, None));
                chat.blocked = request.blocked;
                rpc_response::Result::SetChatBlock(crate::SetChatBlockResponse {
                    chat: Some(fixture_chat(&request.chat_id, Some(chat))),
                })
            }
            Some(rpc_request::Request::SendImage(request)) => {
                rpc_response::Result::SendImage(SendImageResponse {
                    message: Some(Message {
                        id: request.client_message_id,
                        chat_id: request.chat_id,
                        sender_id: "me@s.whatsapp.net".into(),
                        sender_name: "You".into(),
                        from_me: true,
                        timestamp_ms: 1_900_000_000_000,
                        status: MessageStatus::Sent as i32,
                        content: Some(proto_message::Content::Image(ImageContent {
                            caption: request.caption,
                            mime_type: String::new(),
                            local_path: request.image_path.clone(),
                            thumbnail_path: request.image_path,
                            downloadable: true,
                            ..Default::default()
                        })),
                        reply_to_message_id: request.reply_to_message_id,
                        reply_to_chat_id: request.reply_to_chat_id,
                        ..Default::default()
                    }),
                })
            }
            Some(rpc_request::Request::SendSticker(request)) => {
                rpc_response::Result::SendSticker(SendStickerResponse {
                    message: Some(Message {
                        id: request.client_message_id,
                        chat_id: request.chat_id,
                        sender_id: "me@s.whatsapp.net".into(),
                        sender_name: "You".into(),
                        from_me: true,
                        timestamp_ms: 1_900_000_000_000,
                        status: MessageStatus::Sent as i32,
                        content: Some(proto_message::Content::Image(ImageContent {
                            mime_type: "image/webp".into(),
                            sticker: true,
                            width: 512,
                            height: 512,
                            ..Default::default()
                        })),
                        reply_to_message_id: request.reply_to_message_id,
                        reply_to_chat_id: request.reply_to_chat_id,
                        ..Default::default()
                    }),
                })
            }
            Some(rpc_request::Request::GetMessageImage(request)) => {
                let numeric_id = request
                    .message_id
                    .strip_prefix("message-")
                    .and_then(|value| value.parse::<usize>().ok())
                    .unwrap_or(0);
                let picture = ensure_fake_image(&profile_dir, numeric_id);
                let path = picture.to_string_lossy().into_owned();
                rpc_response::Result::GetMessageImage(GetMessageImageResponse {
                    chat_id: request.chat_id,
                    message_id: request.message_id,
                    image_path: path.clone(),
                    thumbnail_path: path,
                })
            }
            Some(rpc_request::Request::GetMessageAttachment(request)) => {
                // The file of the message: the one that the message carries.
                // A message without a file has nothing to download.
                let local_path = request
                    .message_id
                    .strip_prefix("message-")
                    .and_then(|value| value.parse::<usize>().ok())
                    .and_then(fake_attachment_of)
                    .map(|attachment| fake_attachment_path(attachment, &profile_dir))
                    .map(|path| path.to_string_lossy().into_owned())
                    .unwrap_or_default();
                rpc_response::Result::GetMessageAttachment(GetMessageAttachmentResponse {
                    chat_id: request.chat_id,
                    message_id: request.message_id,
                    local_path,
                })
            }
            Some(rpc_request::Request::SendAttachment(request)) => fake_send_attachment(request),
            Some(rpc_request::Request::ListStickers(_)) => {
                rpc_response::Result::ListStickers(ListStickersResponse {
                    packs: vec![
                        StickerPack {
                            id: "favorites".into(),
                            name: "Favourites".into(),
                            stickers: vec![
                                fake_sticker("fav-1", true),
                                fake_sticker("fav-2", true),
                            ],
                        },
                        StickerPack {
                            id: "recent".into(),
                            name: "Recently used".into(),
                            stickers: vec![fake_sticker("recent-1", false)],
                        },
                    ],
                })
            }
            Some(rpc_request::Request::SendStickerFromLibrary(request)) => {
                if request.sticker_id.is_empty() {
                    fake_invalid_argument("client_message_id, chat_id and sticker_id are required")
                } else {
                    rpc_response::Result::SendStickerFromLibrary(SendStickerFromLibraryResponse {
                        message: Some(Message {
                            id: request.client_message_id,
                            chat_id: request.chat_id,
                            sender_id: "me@s.whatsapp.net".into(),
                            sender_name: "You".into(),
                            from_me: true,
                            timestamp_ms: 1_900_000_000_000,
                            status: MessageStatus::Sent as i32,
                            content: Some(proto_message::Content::Image(ImageContent {
                                mime_type: "image/webp".into(),
                                sticker: true,
                                width: 512,
                                height: 512,
                                ..Default::default()
                            })),
                            reply_to_message_id: request.reply_to_message_id,
                            reply_to_chat_id: request.reply_to_chat_id,
                            ..Default::default()
                        }),
                    })
                }
            }
            Some(rpc_request::Request::SendReaction(request)) => {
                let removed = request.emoji.is_empty();
                rpc_response::Result::SendReaction(SendReactionResponse {
                    reaction: Some(Reaction {
                        chat_id: request.chat_id,
                        message_id: request.message_id,
                        sender_id: "me@s.whatsapp.net".into(),
                        emoji: request.emoji,
                        timestamp_ms: 1_900_000_000_000,
                        from_me: true,
                        sender_name: "You".into(),
                        ..Default::default()
                    }),
                    removed,
                })
            }
            Some(rpc_request::Request::RepairRecentReactions(request)) => {
                rpc_response::Result::RepairRecentReactions(crate::RepairRecentReactionsResponse {
                    chat_id: request.chat_id,
                    requested: false,
                    attempts: 0,
                })
            }
            Some(rpc_request::Request::CreatePoll(request)) => {
                let mut message = fake_message(&request.chat_id, 1_001, &profile_dir);
                message.from_me = true;
                message.content = Some(proto_message::Content::Poll(PollContent {
                    question: request.question,
                    options: request
                        .options
                        .into_iter()
                        .map(|name| PollOption {
                            name,
                            vote_count: 0,
                            selected_by_me: false,
                            voters: Vec::new(),
                        })
                        .collect(),
                    selectable_options_count: request.selectable_options_count,
                    total_voters: 0,
                }));
                rpc_response::Result::CreatePoll(crate::CreatePollResponse {
                    message: Some(message),
                })
            }
            Some(rpc_request::Request::VotePoll(request)) => {
                let mut message = fake_message(&request.chat_id, 1_001, &profile_dir);
                message.id = request.poll_message_id;
                message.content = Some(proto_message::Content::Poll(PollContent {
                    question: "Fake poll".into(),
                    options: request
                        .selected_options
                        .into_iter()
                        .map(|name| PollOption {
                            name,
                            vote_count: 1,
                            selected_by_me: true,
                            voters: vec![PollVoter {
                                user_id: "me@s.whatsapp.net".into(),
                                display_name: "You".into(),
                                from_me: true,
                                ..Default::default()
                            }],
                        })
                        .collect(),
                    selectable_options_count: 1,
                    total_voters: 1,
                }));
                rpc_response::Result::VotePoll(crate::VotePollResponse {
                    message: Some(message),
                })
            }
            Some(rpc_request::Request::SetMessagePin(request)) => {
                let stored = pins.entry(request.chat_id.clone()).or_default();
                if request.pinned {
                    if !stored
                        .iter()
                        .any(|pin| pin.message_id == request.message_id)
                    {
                        let numeric_id = request
                            .message_id
                            .strip_prefix("message-")
                            .and_then(|value| value.parse::<usize>().ok())
                            .unwrap_or(0);
                        let mut message = fake_message(&request.chat_id, numeric_id, &profile_dir);
                        message.id = request.message_id.clone();
                        stored.push(PinnedMessage {
                            message_id: request.message_id.clone(),
                            pinned_at_ms: 1_900_000_000_000,
                            pinned_by: "me@s.whatsapp.net".into(),
                            message: Some(message),
                            available: true,
                        });
                    }
                } else {
                    stored.retain(|pin| pin.message_id != request.message_id);
                }
                emit_fake_event(
                    &incoming,
                    &event_sequence,
                    backend_event::Event::PinnedMessagesChanged(PinnedMessagesChanged {
                        chat_id: request.chat_id,
                    }),
                );
                rpc_response::Result::SetMessagePin(crate::SetMessagePinResponse {})
            }
            Some(rpc_request::Request::ListPinnedMessages(request)) => {
                rpc_response::Result::ListPinnedMessages(ListPinnedMessagesResponse {
                    pins: pins.get(&request.chat_id).cloned().unwrap_or_default(),
                })
            }
            Some(rpc_request::Request::MarkRead(_)) => {
                rpc_response::Result::MarkRead(crate::MarkReadResponse {})
            }
            Some(rpc_request::Request::SetTyping(_)) => {
                rpc_response::Result::SetTyping(crate::SetTypingResponse {})
            }
            Some(rpc_request::Request::Logout(_)) => {
                rpc_response::Result::Logout(crate::LogoutResponse {})
            }
            Some(rpc_request::Request::Reconnect(_)) => {
                rpc_response::Result::Reconnect(crate::ReconnectResponse { started: true })
            }
            Some(rpc_request::Request::Shutdown(_)) => {
                rpc_response::Result::Shutdown(crate::ShutdownResponse {})
            }
            None => continue,
        };
        if incoming
            .send_blocking(BridgeMessage::Envelope(Box::new(Envelope {
                protocol_version: PROTOCOL_VERSION,
                request_id,
                body: Some(envelope::Body::Response(RpcResponse {
                    result: Some(result),
                })),
            })))
            .is_err()
        {
            return;
        }
        if is_hello {
            // The real backend only exposes events after the Hello response has
            // been written, so the deterministic backend must keep the same
            // subscription boundary.
            handshaken.store(true, Ordering::Release);
        }
    }
}

/// The error the fake answers with when a request is malformed.
fn fake_invalid_argument(message: impl Into<String>) -> rpc_response::Result {
    rpc_response::Result::Error(RpcError {
        code: "invalid_argument".into(),
        message: message.into(),
        retryable: false,
    })
}

/// A sticker of the library of the fake.
pub fn fake_sticker(id: &str, favorite: bool) -> Sticker {
    Sticker {
        id: id.into(),
        local_path: String::new(),
        mime_type: "image/webp".into(),
        animated: false,
        width: 512,
        height: 512,
        favorite,
        last_used_ms: 1_900_000_000_000,
        source_chat_id: String::new(),
        source_message_id: String::new(),
    }
}

/// The answer of a request that sends an attachment.
///
/// The origin validated the attachment here and answered with an error of
/// the protocol when the request did not make sense.
pub fn fake_send_attachment(request: crate::SendAttachmentRequest) -> rpc_response::Result {
    let Ok(kind) = AttachmentKind::try_from(request.kind) else {
        return fake_invalid_argument("unknown attachment kind");
    };
    if kind == AttachmentKind::Unspecified {
        return fake_invalid_argument("attachment kind is required");
    }
    if request.voice_note && kind != AttachmentKind::Audio {
        return fake_invalid_argument("voice_note is only valid for audio attachments");
    }
    if kind == AttachmentKind::Audio && !request.caption.is_empty() {
        return fake_invalid_argument("audio messages do not support captions");
    }
    let (kind_name, default_mime) = match kind {
        AttachmentKind::Document => ("document", "application/octet-stream"),
        AttachmentKind::Video => ("video", "video/mp4"),
        AttachmentKind::Audio => ("audio", "audio/ogg"),
        // The kind was validated above: the arm is here to keep the match
        // exhaustive without a panic.
        AttachmentKind::Unspecified => {
            return fake_invalid_argument("attachment kind is required");
        }
    };
    // The type of the file comes from its extension: the fake does not open
    // the file, but the interface shows the type next to the name.
    let mime_type = if request.voice_note {
        // A voice note is always Ogg Opus: that is what the protocol asks
        // for.
        "audio/ogg; codecs=opus".to_string()
    } else {
        fake_mime_of(&request.file_path, default_mime).to_string()
    };
    let file_name = Path::new(&request.file_path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("attachment")
        .to_owned();
    rpc_response::Result::SendAttachment(SendAttachmentResponse {
        message: Some(Message {
            id: request.client_message_id,
            chat_id: request.chat_id,
            sender_id: "me@s.whatsapp.net".into(),
            sender_name: "You".into(),
            from_me: true,
            timestamp_ms: 1_900_000_000_000,
            status: MessageStatus::Sent as i32,
            content: Some(proto_message::Content::Attachment(AttachmentContent {
                kind: kind_name.into(),
                caption: request.caption,
                mime_type,
                file_name,
                // Match the real backend's ownership boundary: a selected
                // source path is never exposed as a renderable cache asset.
                local_path: String::new(),
                file_size: fake_file_size(&request.file_path),
                voice_note: request.voice_note,
                downloadable: true,
                ..Default::default()
            })),
            reply_to_message_id: request.reply_to_message_id,
            reply_to_chat_id: request.reply_to_chat_id,
            ..Default::default()
        }),
    })
}

/// The type of a file, guessed from its extension.
///
/// The fake does not open the file: the extension is enough to fill the type
/// that the interface shows next to the name.
fn fake_mime_of<'a>(file_path: &str, fallback: &'a str) -> &'a str {
    let extension = Path::new(file_path)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match extension.as_str() {
        "pdf" => "application/pdf",
        "txt" => "text/plain",
        "csv" => "text/csv",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "zip" => "application/zip",
        "mp4" => "video/mp4",
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        "mkv" => "video/x-matroska",
        "mp3" => "audio/mpeg",
        "ogg" | "opus" => "audio/ogg",
        "m4a" => "audio/mp4",
        "wav" => "audio/wav",
        "flac" => "audio/flac",
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        _ => fallback,
    }
}

/// The size of a file on disk, zero when it is not there.
fn fake_file_size(file_path: &str) -> u64 {
    std::fs::metadata(file_path)
        .map(|meta| meta.len())
        .unwrap_or_default()
}

/// Materialize a tiny real PCM WAV once per profile so fake-mode audio and
/// voice notes have a playable file to point the inline player at. The file
/// lives under the managed media tree, which is the only path the asset
/// protocol serves.
pub fn ensure_fake_audio(profile_dir: &Path) -> PathBuf {
    ensure_fake_audio_named(profile_dir, "fake-audio.wav")
}

/// Materialize a real PCM WAV with the name that the dataset uses.
///
/// The wave is the same tone of the fake: a playable file that the inline
/// player of the interface can open without another dependency.
pub fn ensure_fake_audio_named(profile_dir: &Path, name: &str) -> PathBuf {
    let media_dir = profile_dir.join("media");
    let path = media_dir.join(name);
    if path.exists() {
        return path;
    }
    if ensure_media_dir(&media_dir).is_err() {
        return path;
    }
    let wav = wav::encode_tone(8_000, 2, 440.0);
    if std::fs::write(&path, wav).is_ok() {
        let _ = restrict_permissions(&path);
    }
    path
}

/// Materialize a real document once per profile, so the document of the
/// dataset opens with the reader of the system instead of failing.
///
/// The file is a PDF of one page: it starts with the header of the version
/// 1.4 and ends with the marker of the end of the file.
pub fn ensure_fake_document(profile_dir: &Path) -> PathBuf {
    let media_dir = profile_dir.join("media");
    let path = media_dir.join(DOCUMENT_FILE);
    if path.exists() {
        return path;
    }
    if ensure_media_dir(&media_dir).is_err() {
        return path;
    }
    if std::fs::write(&path, pdf::encode_one_page("Informe de Whatfast")).is_ok() {
        let _ = restrict_permissions(&path);
    }
    path
}

/// Materialize a real video once per profile, so the video of the dataset
/// opens with the player of the system instead of failing.
///
/// A full movie is not something the fake can build: the file carries the
/// header of the format and a chunk of data, which is what the system needs
/// to recognize it as a video.
pub fn ensure_fake_video(profile_dir: &Path) -> PathBuf {
    let media_dir = profile_dir.join("media");
    let path = media_dir.join(VIDEO_FILE);
    if path.exists() {
        return path;
    }
    if ensure_media_dir(&media_dir).is_err() {
        return path;
    }
    if std::fs::write(&path, mp4::encode_stub(4_096)).is_ok() {
        let _ = restrict_permissions(&path);
    }
    path
}

/// The file of the attachment, written under the media tree of the profile.
fn fake_attachment_path(attachment: FakeAttachment, profile_dir: &Path) -> PathBuf {
    match attachment {
        FakeAttachment::Document => ensure_fake_document(profile_dir),
        FakeAttachment::Video => ensure_fake_video(profile_dir),
        FakeAttachment::Audio => ensure_fake_audio_named(profile_dir, AUDIO_FILE),
        FakeAttachment::VoiceNote => ensure_fake_audio_named(profile_dir, VOICE_NOTE_FILE),
    }
}

/// The content of the attachment of a message of the dataset.
///
/// The file is real: it lives under the media tree of the profile, so the
/// interface can open it, play it or download it.
fn fake_attachment_content(
    attachment: FakeAttachment,
    profile_dir: &Path,
) -> proto_message::Content {
    let path = fake_attachment_path(attachment, profile_dir);
    let (kind, caption, mime_type, file_name, duration_seconds, width, height, voice_note) =
        match attachment {
            FakeAttachment::Document => (
                "document",
                "El informe",
                "application/pdf",
                DOCUMENT_FILE,
                0,
                0,
                0,
                false,
            ),
            FakeAttachment::Video => (
                "video",
                "El viaje",
                "video/mp4",
                VIDEO_FILE,
                12,
                1280,
                720,
                false,
            ),
            FakeAttachment::Audio => ("audio", "", "audio/wav", AUDIO_FILE, 2, 0, 0, false),
            FakeAttachment::VoiceNote => ("audio", "", "audio/wav", VOICE_NOTE_FILE, 2, 0, 0, true),
        };
    proto_message::Content::Attachment(AttachmentContent {
        kind: kind.into(),
        caption: caption.into(),
        mime_type: mime_type.into(),
        file_name: file_name.into(),
        local_path: path.to_string_lossy().into_owned(),
        file_size: std::fs::metadata(&path)
            .map(|meta| meta.len())
            .unwrap_or_default(),
        width,
        height,
        duration_seconds,
        voice_note,
        downloadable: true,
        ..Default::default()
    })
}

/// The kind of file that the dataset puts in a message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FakeAttachment {
    /// A document: a PDF that the reader of the system opens.
    Document,
    /// A video: an MP4 that the player of the system opens.
    Video,
    /// An audio file: a song that the player of the system opens.
    Audio,
    /// A voice note: the audio of a message recorded with the phone.
    VoiceNote,
}

/// The file that the message of the dataset carries, if it carries one.
///
/// The index decides it, the same way the photo does: the history has a file
/// of another kind every few messages, so the interface can draw the
/// document, the video, the audio and the voice note.
fn fake_attachment_of(id: usize) -> Option<FakeAttachment> {
    if id == 0 {
        return None;
    }
    if id % DOCUMENT_EVERY == 0 {
        Some(FakeAttachment::Document)
    } else if id % VIDEO_EVERY == 0 {
        Some(FakeAttachment::Video)
    } else if id % AUDIO_EVERY == 0 {
        Some(FakeAttachment::Audio)
    } else if id % VOICE_NOTE_EVERY == 0 {
        Some(FakeAttachment::VoiceNote)
    } else {
        None
    }
}

/// Materialize a real picture on disk, so the media path (thumbnail, full
/// viewer, gallery) is exercised by the fake backend instead of showing an
/// endless loading placeholder.
///
/// The origin drew the gradient with the `image` crate and wrote it as a
/// PNG. The pixels here are the same ones; the container is written by the
/// PNG encoder of this module, because the crate may not depend on `image`.
pub fn ensure_fake_image(profile_dir: &Path, seed: usize) -> PathBuf {
    let media_dir = profile_dir.join("media");
    let path = media_dir.join(format!("fake-image-{seed}.png"));
    if path.exists() {
        return path;
    }
    if ensure_media_dir(&media_dir).is_err() {
        return path;
    }
    let (width, height) = (320_u32, 240_u32);
    let pixels = fake_image_pixels(seed, width, height);
    let png = png::encode_rgb(width, height, &pixels);
    if std::fs::write(&path, png).is_ok() {
        let _ = restrict_permissions(&path);
    }
    path
}

/// The picture of the avatar of a conversation.
///
/// The reference draws a photo in the row of every conversation, so the
/// backend of the demonstration gives one to each chat: a square picture
/// with a gradient of its own, the same way the pictures of the messages
/// are made.
pub fn ensure_fake_avatar(profile_dir: &Path, seed: usize) -> PathBuf {
    let media_dir = profile_dir.join("media");
    let path = media_dir.join(format!("fake-avatar-{seed}.png"));
    if path.exists() {
        return path;
    }
    if ensure_media_dir(&media_dir).is_err() {
        return path;
    }
    let (width, height) = (128_u32, 128_u32);
    let pixels = fake_avatar_pixels(seed, width, height);
    let png = png::encode_rgb(width, height, &pixels);
    if std::fs::write(&path, png).is_ok() {
        let _ = restrict_permissions(&path);
    }
    path
}

/// The number that decides the picture of a conversation.
fn chat_seed(chat_id: &str) -> usize {
    chat_id
        .strip_prefix("chat-")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or_else(|| {
            chat_id.bytes().fold(0usize, |total, byte| {
                total.wrapping_mul(31).wrapping_add(byte as usize)
            })
        })
}

/// The pixels of the picture of the avatar: a gradient with a spot of light.
fn fake_avatar_pixels(seed: usize, width: u32, height: u32) -> Vec<u8> {
    let base = [
        (seed as u8).wrapping_mul(53).wrapping_add(64),
        (seed as u8).wrapping_mul(29).wrapping_add(92),
        (seed as u8).wrapping_mul(71).wrapping_add(48),
    ];
    let center_x = 0.3 + ((seed % 7) as f32) * 0.06;
    let center_y = 0.35 + ((seed % 5) as f32) * 0.07;
    let mut pixels = Vec::with_capacity((width as usize) * (height as usize) * 3);
    for y in 0..height {
        let fy = y as f32 / height as f32;
        for x in 0..width {
            let fx = x as f32 / width as f32;
            // El degradado va del color de base al oscuro en una diagonal.
            let f = ((fx + fy) / 2.0).min(1.0);
            // Y una mancha clara, como una foto con luz.
            let dx = fx - center_x;
            let dy = fy - center_y;
            let light = (1.0 - (dx * dx + dy * dy) * 5.0).max(0.0);
            pixels.push(((base[0] as f32) * (1.0 - f) + 24.0 * f + 60.0 * light).min(255.0) as u8);
            pixels.push(((base[1] as f32) * (1.0 - f) + 30.0 * f + 52.0 * light).min(255.0) as u8);
            pixels.push(((base[2] as f32) * (1.0 - f) + 38.0 * f + 44.0 * light).min(255.0) as u8);
        }
    }
    pixels
}

/// The pixels of the picture of a seed: the gradient of the origin.
fn fake_image_pixels(seed: usize, width: u32, height: u32) -> Vec<u8> {
    let base = [
        (seed as u8).wrapping_mul(37).wrapping_add(58),
        (seed as u8).wrapping_mul(91).wrapping_add(36),
        (seed as u8).wrapping_mul(53).wrapping_add(74),
    ];
    let mut pixels = Vec::with_capacity((width as usize) * (height as usize) * 3);
    for y in 0..height {
        let fy = y as f32 / height as f32;
        for x in 0..width {
            let fx = x as f32 / width as f32;
            pixels.push((base[0] as f32 * (1.0 - fy) + 30.0 * fy) as u8);
            pixels.push((base[1] as f32 * (1.0 - fx) + 26.0 * fx) as u8);
            pixels.push((base[2] as f32 * (1.0 - (fx + fy) / 2.0) + 40.0 * (fx + fy) / 2.0) as u8);
        }
    }
    pixels
}

/// Create the media tree of the profile, private to its owner.
fn ensure_media_dir(media_dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(media_dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(media_dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Restrict a file of the profile to its owner: the media is private.
#[cfg(unix)]
fn restrict_permissions(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
}

/// Restrict a file of the profile to its owner: the media is private.
///
/// Windows has no mode bits: the profile directory is already per-user
/// (`%LOCALAPPDATA%`), so the inherited ACL is the boundary.
#[cfg(not(unix))]
fn restrict_permissions(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

/// Publish an event of the fake on the stream of the client.
pub fn emit_fake_event(
    incoming: &async_channel::Sender<BridgeMessage>,
    sequence: &Mutex<u64>,
    event: backend_event::Event,
) -> bool {
    // A poisoned lock means a thread died holding it: the counter is still
    // usable, so the stream keeps going.
    let mut sequence = match sequence.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    let envelope = Envelope {
        protocol_version: PROTOCOL_VERSION,
        request_id: 0,
        body: Some(envelope::Body::Event(BackendEvent {
            sequence: *sequence,
            event: Some(event),
        })),
    };
    *sequence = sequence.saturating_add(1);
    incoming
        .send_blocking(BridgeMessage::Envelope(Box::new(envelope)))
        .is_ok()
}

/// The live loop of the fake: it publishes a message and the derived chat
/// update once the handshake is done.
pub fn fake_live_loop(
    incoming: async_channel::Sender<BridgeMessage>,
    sequence: Arc<Mutex<u64>>,
    handshaken: Arc<AtomicBool>,
) {
    while !handshaken.load(Ordering::Acquire) {
        if incoming.is_closed() {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
    // The deterministic fixture also exercises the real-time path: incoming
    // messages appear at the bottom while the chat row moves to the top.
    thread::sleep(LIVE_EVENT_DELAY);
    for counter in 1_u64.. {
        let timestamp_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_millis() as i64);
        let (message_event, chat_event) = fake_live_event_pair(counter, timestamp_ms);
        // Match the real backend: publish the stored message before its derived
        // chat preview/unread-count update.
        if !emit_fake_event(&incoming, &sequence, message_event)
            || !emit_fake_event(&incoming, &sequence, chat_event)
        {
            return;
        }
        thread::sleep(LIVE_EVENT_INTERVAL);
    }
}

/// The pair of events of one incoming message: the message and the chat it
/// updates.
pub fn fake_live_event_pair(
    counter: u64,
    timestamp_ms: i64,
) -> (backend_event::Event, backend_event::Event) {
    let message = Message {
        id: format!("live-{counter}"),
        chat_id: "chat-0".into(),
        sender_id: "friend@s.whatsapp.net".into(),
        sender_name: "Fast friend".into(),
        sender_phone_number: "+15551234567".into(),
        timestamp_ms,
        status: MessageStatus::Delivered as i32,
        content: Some(proto_message::Content::Text(TextContent {
            text: format!("Live message {counter} streamed into the bottom of the chat"),
            link_preview: None,
        })),
        ..Default::default()
    };
    let mut chat = fake_chat(0);
    chat.last_message_preview = message
        .content
        .as_ref()
        .and_then(|content| match content {
            proto_message::Content::Text(text) => Some(text.text.clone()),
            _ => None,
        })
        .unwrap_or_default();
    chat.last_message_timestamp_ms = timestamp_ms;
    chat.unread_count = 1;
    (
        backend_event::Event::MessageUpserted(MessageUpserted {
            message: Some(message),
        }),
        backend_event::Event::ChatUpserted(ChatUpserted { chat: Some(chat) }),
    )
}

/// A chat of the dataset.
///
/// The dataset is deterministic: the index of the chat decides every field,
/// so the visual QA of the UI has a stable reference.
/// The chat of the fixture for an id, with the flags the client changed.
///
/// The setters return the whole chat, the way the real sidecar does: a client
/// that replaces the row with a partial one loses the title and the avatar.
pub fn fixture_chat(chat_id: &str, settings: Option<&Chat>) -> Chat {
    let number = chat_id
        .strip_prefix("chat-")
        .and_then(|rest| rest.parse::<usize>().ok());
    let mut chat = match number {
        Some(number) => fake_chat(number),
        None => Chat {
            id: chat_id.to_string(),
            kind: ChatKind::Direct as i32,
            ..Default::default()
        },
    };
    if let Some(settings) = settings {
        chat.pinned = settings.pinned;
        chat.archived = settings.archived;
        chat.muted = settings.muted;
        chat.blocked = settings.blocked;
        chat.mute_until_ms = settings.mute_until_ms;
    }
    chat
}

pub fn fake_chat(id: usize) -> Chat {
    Chat {
        id: format!("chat-{id}"),
        kind: if id % 5 == 0 {
            ChatKind::Group as i32
        } else {
            ChatKind::Direct as i32
        },
        title: if id % 5 == 0 {
            format!("Weekend plans {id}")
        } else {
            format!("Fast friend {id}")
        },
        last_message_preview: format!("This is virtualized conversation number {id}"),
        last_message_timestamp_ms: 1_900_000_000_000 - id as i64,
        unread_count: (id % 7 == 0) as u32 * ((id % 4) + 1) as u32,
        pinned: id < 3,
        phone_number: if id % 5 != 0 {
            format!("+1555{id:07}")
        } else {
            String::new()
        },
        contact_name: if id % 5 != 0 {
            format!("Fast friend {id}")
        } else {
            String::new()
        },
        push_name: if id % 5 != 0 {
            format!("Friend {id}")
        } else {
            String::new()
        },
        ..Default::default()
    }
}

/// A message of the dataset.
///
/// Every field is decided by the index of the message: the sender, the
/// content, the photo, the reaction and the quote.
pub fn fake_message(chat_id: &str, id: usize, profile_dir: &Path) -> Message {
    Message {
        id: format!("message-{id}"),
        chat_id: chat_id.into(),
        sender_id: if id % 2 == 0 {
            "me@s.whatsapp.net"
        } else {
            "friend@s.whatsapp.net"
        }
        .into(),
        sender_name: if id % 2 == 0 { "You" } else { "Fast friend" }.into(),
        sender_phone_number: if id % 2 == 0 {
            String::new()
        } else {
            "+15551234567".into()
        },
        from_me: id % 2 == 0,
        timestamp_ms: id as i64,
        status: MessageStatus::Read as i32,
        content: Some(if id % 37 == 0 && id > 0 {
            // A real picture on disk, so the media path (thumbnail, full
            // viewer, gallery) is exercised by the fake backend instead of
            // showing an endless loading placeholder.
            let picture = ensure_fake_image(profile_dir, id);
            let path = picture.to_string_lossy().into_owned();
            proto_message::Content::Image(ImageContent {
                caption: format!("Photo {id}"),
                mime_type: "image/png".into(),
                local_path: path.clone(),
                thumbnail_path: path,
                width: 320,
                height: 240,
                file_size: std::fs::metadata(&picture)
                    .map(|meta| meta.len())
                    .unwrap_or(0),
                downloadable: true,
                ..Default::default()
            })
        } else if let Some(attachment) = fake_attachment_of(id) {
            // A real file on disk, so the media path (document, video, audio
            // and voice note) is exercised by the fake backend instead of
            // showing a message with nothing to open.
            fake_attachment_content(attachment, profile_dir)
        } else {
            proto_message::Content::Text(TextContent {
                text: if id % 9 == 0 {
                    format!(
                        "A longer message {id}\nwith a second line to exercise variable-height virtualization 🐈"
                    )
                } else {
                    format!("Fast native message {id}")
                },
                link_preview: (id % 13 == 0).then(|| LinkPreview {
                    url: "https://example.com/whatfast".into(),
                    title: "Whatfast link preview".into(),
                    description: "OpenGraph metadata supplied by the fake backend".into(),
                    ..Default::default()
                }),
            })
        }),
        reactions: if id % 11 == 0 {
            vec![Reaction {
                chat_id: chat_id.into(),
                message_id: format!("message-{id}"),
                sender_id: "friend@s.whatsapp.net".into(),
                emoji: "🐈".into(),
                timestamp_ms: id as i64,
                sender_name: "Fast friend".into(),
                sender_phone_number: "+15551234567".into(),
                ..Default::default()
            }]
        } else {
            Vec::new()
        },
        reply_to_message_id: if id > 0 && id % 13 == 0 {
            format!("message-{}", id - 1)
        } else {
            String::new()
        },
        ..Default::default()
    }
}

/// A small PNG writer: the fake needs real image files on disk and the
/// crate may not depend on `image`.
///
/// The container is a PNG with the "Sub" filter and a fixed-Huffman deflate
/// stream. The pixels are the ones of the origin; only the bytes of the
/// container differ from the ones the `image` crate would write.
mod png {
    /// The signature of a PNG file.
    const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];

    /// The bytes per pixel of a truecolor image.
    const BYTES_PER_PIXEL: usize = 3;

    /// Write an RGB image as a PNG.
    pub fn encode_rgb(width: u32, height: u32, pixels: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(pixels.len() / 2 + 128);
        out.extend_from_slice(&SIGNATURE);
        let mut header = Vec::with_capacity(13);
        header.extend_from_slice(&width.to_be_bytes());
        header.extend_from_slice(&height.to_be_bytes());
        // 8 bits per channel, truecolor, deflate, adaptive filtering and no
        // interlacing.
        header.extend_from_slice(&[8, 2, 0, 0, 0]);
        write_chunk(&mut out, b"IHDR", &header);
        let raw = filter_rows(width, height, pixels);
        write_chunk(&mut out, b"IDAT", &zlib_compress(&raw));
        write_chunk(&mut out, b"IEND", &[]);
        out
    }

    /// Filter the rows of the image with the "Sub" filter: every byte minus
    /// the byte of the same channel of the pixel on the left.
    ///
    /// The gradient of the fake is smooth, so the filter leaves long runs of
    /// zeroes and the deflate stream below shrinks the file a lot.
    fn filter_rows(width: u32, height: u32, pixels: &[u8]) -> Vec<u8> {
        let stride = width as usize * BYTES_PER_PIXEL;
        let mut raw = Vec::with_capacity((stride + 1) * height as usize);
        for y in 0..height as usize {
            raw.push(1);
            let row = &pixels[y * stride..(y + 1) * stride];
            for (index, byte) in row.iter().enumerate() {
                let left = if index >= BYTES_PER_PIXEL {
                    row[index - BYTES_PER_PIXEL]
                } else {
                    0
                };
                raw.push(byte.wrapping_sub(left));
            }
        }
        raw
    }

    /// Wrap the filtered rows in a zlib stream.
    fn zlib_compress(data: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(data.len() / 4 + 16);
        // Deflate with a 32K window and the fastest level: the header is
        // valid because 0x7801 is a multiple of 31.
        out.push(0x78);
        out.push(0x01);
        deflate_fixed(data, &mut out);
        out.extend_from_slice(&adler32(data).to_be_bytes());
        out
    }

    /// Write the data as a single fixed-Huffman deflate block.
    ///
    /// The encoder knows one trick: a run of identical bytes becomes a
    /// literal and a length/distance pair with the distance one, which is
    /// what the filtered gradient needs.
    fn deflate_fixed(data: &[u8], out: &mut Vec<u8>) {
        let mut writer = BitWriter::new(out);
        writer.write_bits(1, 1); // The last block.
        writer.write_bits(1, 2); // The fixed Huffman codes.
        let mut index = 0;
        while index < data.len() {
            let mut run = 1;
            while index + run < data.len() && data[index + run] == data[index] && run < 258 {
                run += 1;
            }
            if run >= 4 {
                // The first byte of the run is a literal: a match with the
                // distance one copies the byte before it, so the run needs
                // its own byte before the match repeats it.
                write_literal(&mut writer, data[index]);
                write_length(&mut writer, run - 1);
                write_distance(&mut writer, 1);
                index += run;
            } else {
                for offset in 0..run {
                    write_literal(&mut writer, data[index + offset]);
                }
                index += run;
            }
        }
        write_symbol(&mut writer, 256); // The end of the block.
        writer.flush();
    }

    /// The base of every length of the deflate format.
    const LENGTH_BASE: [u16; 29] = [
        3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115,
        131, 163, 195, 227, 258,
    ];

    /// The extra bits of every length of the deflate format.
    const LENGTH_EXTRA: [u8; 29] = [
        0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
    ];

    /// The base of every distance of the deflate format.
    const DISTANCE_BASE: [u16; 30] = [
        1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
        2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
    ];

    /// The extra bits of every distance of the deflate format.
    const DISTANCE_EXTRA: [u8; 30] = [
        0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12,
        13, 13,
    ];

    /// Write a literal byte with the fixed Huffman codes.
    fn write_literal(writer: &mut BitWriter, byte: u8) {
        write_symbol(writer, u16::from(byte));
    }

    /// Write a symbol with the fixed Huffman codes of the deflate format.
    fn write_symbol(writer: &mut BitWriter, symbol: u16) {
        match symbol {
            0..=143 => writer.write_code(0x30 + u32::from(symbol), 8),
            144..=255 => writer.write_code(0x190 + u32::from(symbol) - 144, 9),
            256..=279 => writer.write_code(u32::from(symbol) - 256, 7),
            _ => writer.write_code(0xc0 + u32::from(symbol) - 280, 8),
        }
    }

    /// Write a length with the fixed Huffman codes.
    fn write_length(writer: &mut BitWriter, length: usize) {
        let mut code = 0;
        while code + 1 < LENGTH_BASE.len() && LENGTH_BASE[code + 1] as usize <= length {
            code += 1;
        }
        let extra = LENGTH_EXTRA[code];
        let bits = extra as u32;
        write_symbol(writer, 257 + code as u16);
        if bits > 0 {
            writer.write_bits((length - LENGTH_BASE[code] as usize) as u32, bits);
        }
    }

    /// Write a distance with the fixed Huffman codes.
    fn write_distance(writer: &mut BitWriter, distance: usize) {
        let mut code = 0;
        while code + 1 < DISTANCE_BASE.len() && DISTANCE_BASE[code + 1] as usize <= distance {
            code += 1;
        }
        let extra = DISTANCE_EXTRA[code];
        let bits = extra as u32;
        writer.write_code(code as u32, 5);
        if bits > 0 {
            writer.write_bits((distance - DISTANCE_BASE[code] as usize) as u32, bits);
        }
    }

    /// The writer of the bits of a deflate stream.
    struct BitWriter<'a> {
        out: &'a mut Vec<u8>,
        buffer: u32,
        bits: u32,
    }

    impl<'a> BitWriter<'a> {
        fn new(out: &'a mut Vec<u8>) -> Self {
            Self {
                out,
                buffer: 0,
                bits: 0,
            }
        }

        /// Write the bits of a value, the least significant one first: the
        /// extra bits of the deflate format.
        fn write_bits(&mut self, value: u32, count: u32) {
            self.buffer |= value << self.bits;
            self.bits += count;
            while self.bits >= 8 {
                self.out.push((self.buffer & 0xff) as u8);
                self.buffer >>= 8;
                self.bits -= 8;
            }
        }

        /// Write a Huffman code, the most significant bit first.
        fn write_code(&mut self, code: u32, count: u32) {
            for bit in (0..count).rev() {
                self.write_bits((code >> bit) & 1, 1);
            }
        }

        /// Write the last byte of the stream.
        fn flush(&mut self) {
            if self.bits > 0 {
                self.out.push((self.buffer & 0xff) as u8);
                self.buffer = 0;
                self.bits = 0;
            }
        }
    }

    /// Write a chunk of a PNG file: its length, its kind, its data and the
    /// CRC of the kind and the data.
    fn write_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        out.extend_from_slice(kind);
        out.extend_from_slice(data);
        let mut crc_input = Vec::with_capacity(4 + data.len());
        crc_input.extend_from_slice(kind);
        crc_input.extend_from_slice(data);
        out.extend_from_slice(&crc32(&crc_input).to_be_bytes());
    }

    /// The CRC-32 of the chunks of a PNG file.
    fn crc32(data: &[u8]) -> u32 {
        let mut table = [0_u32; 256];
        for (index, entry) in table.iter_mut().enumerate() {
            let mut value = index as u32;
            for _ in 0..8 {
                value = if value & 1 != 0 {
                    0xedb88320 ^ (value >> 1)
                } else {
                    value >> 1
                };
            }
            *entry = value;
        }
        let mut crc = 0xffff_ffff_u32;
        for byte in data {
            crc = table[((crc ^ u32::from(*byte)) & 0xff) as usize] ^ (crc >> 8);
        }
        crc ^ 0xffff_ffff
    }

    /// The checksum of the zlib stream.
    fn adler32(data: &[u8]) -> u32 {
        let mut low = 1_u32;
        let mut high = 0_u32;
        for byte in data {
            low = (low + u32::from(*byte)) % 65521;
            high = (high + low) % 65521;
        }
        (high << 16) | low
    }
}

/// A small PCM WAV writer: the fake needs a real audio file on disk and the
/// crate may not depend on an encoder.
///
/// The wave is mono of sixteen bits with a tone of the given frequency: it
/// is a file that every player of the system opens.
mod wav {
    /// Write a mono PCM WAV with a tone of the given frequency.
    pub fn encode_tone(sample_rate: u32, seconds: u32, hertz: f32) -> Vec<u8> {
        let samples = (sample_rate * seconds) as usize;
        let mut data = Vec::with_capacity(samples * 2);
        for index in 0..samples {
            let time = index as f32 / sample_rate as f32;
            let value = (time * hertz * std::f32::consts::TAU).sin() * 0.25;
            data.extend_from_slice(&((value * i16::MAX as f32) as i16).to_le_bytes());
        }
        let mut out = Vec::with_capacity(44 + data.len());
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        out.extend_from_slice(b"WAVE");
        out.extend_from_slice(b"fmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&sample_rate.to_le_bytes());
        out.extend_from_slice(&(sample_rate * 2).to_le_bytes());
        out.extend_from_slice(&2u16.to_le_bytes());
        out.extend_from_slice(&16u16.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&data);
        out
    }
}

/// A small PDF writer: the fake needs a real document on disk and the crate
/// may not depend on a library of documents.
///
/// The file is the header of the version 1.4, five objects (the catalogue,
/// the pages, the page, the text of the page and the font), the table of the
/// cross references and the marker of the end of the file.
mod pdf {
    /// The header of a PDF file of the version 1.4.
    const HEADER: &str = "%PDF-1.4\n";

    /// Write a document of one page with a line of text.
    pub fn encode_one_page(text: &str) -> Vec<u8> {
        let content = format!("BT /F1 18 Tf 72 760 Td ({}) Tj ET\n", escape(text));
        let mut out = Vec::new();
        out.extend_from_slice(HEADER.as_bytes());
        // Every object of the document, with the offset where it starts.
        let mut offsets = Vec::new();
        offsets.push(out.len());
        out.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
        offsets.push(out.len());
        out.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n");
        offsets.push(out.len());
        out.extend_from_slice(
            b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] \
              /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>\nendobj\n",
        );
        offsets.push(out.len());
        out.extend_from_slice(
            format!("4 0 obj\n<< /Length {} >>\nstream\n", content.len()).as_bytes(),
        );
        out.extend_from_slice(content.as_bytes());
        out.extend_from_slice(b"endstream\nendobj\n");
        offsets.push(out.len());
        out.extend_from_slice(
            b"5 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n",
        );
        // The table of the cross references: the free entry of the head and
        // the offset of every object.
        let table = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n", offsets.len() + 1).as_bytes());
        out.extend_from_slice(b"0000000000 65535 f \n");
        for offset in &offsets {
            out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{table}\n%%EOF\n",
                offsets.len() + 1
            )
            .as_bytes(),
        );
        out
    }

    /// Escape the parentheses and the backslashes of the text of the page.
    fn escape(text: &str) -> String {
        text.replace('\\', r"\\")
            .replace('(', r"\(")
            .replace(')', r"\)")
    }
}

/// A small MP4 writer: the fake needs a real video file on disk and the
/// crate may not depend on an encoder.
///
/// A movie is not something the fake can build: the file carries the box of
/// the type of the format (its brands) and a box of data, which is what the
/// system needs to recognize it as a video and to open it with its player.
mod mp4 {
    /// The brands that the header of the file declares.
    const BRANDS: [&[u8]; 4] = [b"isom", b"iso2", b"avc1", b"mp41"];

    /// Write the header of the format and a chunk of data of the given size.
    pub fn encode_stub(payload: usize) -> Vec<u8> {
        let mut out = Vec::with_capacity(payload + 64);
        // The box of the type: the size, the name, the brand and the version.
        let size = 8 + 4 + 4 + BRANDS.iter().map(|brand| brand.len()).sum::<usize>();
        out.extend_from_slice(&(size as u32).to_be_bytes());
        out.extend_from_slice(b"ftyp");
        out.extend_from_slice(BRANDS[0]);
        out.extend_from_slice(&0x0000_0200u32.to_be_bytes());
        for brand in &BRANDS[1..] {
            out.extend_from_slice(brand);
        }
        // The box of the data of the video.
        out.extend_from_slice(&((payload + 8) as u32).to_be_bytes());
        out.extend_from_slice(b"mdat");
        out.resize(out.len() + payload, 0);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A private directory of the test run for the media of the fake.
    fn profile_dir(name: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("whatfast-fake-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create the profile of the test");
        path
    }

    #[test]
    fn the_dataset_carries_a_file_of_every_kind() {
        let profile = profile_dir("attachments");
        let cases = [
            (DOCUMENT_EVERY * 16, "document", DOCUMENT_FILE, false),
            (VIDEO_EVERY * 13, "video", VIDEO_FILE, false),
            (AUDIO_EVERY * 14, "audio", AUDIO_FILE, false),
            (VOICE_NOTE_EVERY * 10, "audio", VOICE_NOTE_FILE, true),
        ];
        for (id, kind, file_name, voice_note) in cases {
            let message = fake_message("chat-0", id, &profile);
            let Some(proto_message::Content::Attachment(attachment)) = message.content else {
                panic!("the message {id} carries a file");
            };
            assert_eq!(attachment.kind, kind);
            assert_eq!(attachment.file_name, file_name);
            assert_eq!(attachment.voice_note, voice_note);
            assert!(attachment.downloadable);
            assert!(attachment.file_size > 0);
            let path = PathBuf::from(&attachment.local_path);
            assert!(path.is_file(), "the file of the message {id} is on disk");
            assert!(path.starts_with(profile.join("media")));
        }
    }

    #[test]
    fn the_files_of_the_dataset_are_the_ones_of_the_formats() {
        let profile = profile_dir("formats");
        let document = ensure_fake_document(&profile);
        let bytes = std::fs::read(&document).expect("the document of the fake");
        assert!(bytes.starts_with(b"%PDF-1.4"));
        assert!(bytes.ends_with(b"%%EOF\n"));

        let video = ensure_fake_video(&profile);
        let bytes = std::fs::read(&video).expect("the video of the fake");
        assert_eq!(&bytes[4..8], b"ftyp");
        assert_eq!(&bytes[8..12], b"isom");

        let audio = ensure_fake_audio_named(&profile, AUDIO_FILE);
        let bytes = std::fs::read(&audio).expect("the audio of the fake");
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WAVE");
    }

    #[test]
    fn only_the_messages_of_the_strides_carry_a_file() {
        assert!(fake_attachment_of(0).is_none());
        assert!(fake_attachment_of(1).is_none());
        assert_eq!(
            fake_attachment_of(DOCUMENT_EVERY * 16),
            Some(FakeAttachment::Document)
        );
        assert_eq!(
            fake_attachment_of(VIDEO_EVERY * 13),
            Some(FakeAttachment::Video)
        );
        assert_eq!(
            fake_attachment_of(AUDIO_EVERY * 14),
            Some(FakeAttachment::Audio)
        );
        assert_eq!(
            fake_attachment_of(VOICE_NOTE_EVERY * 10),
            Some(FakeAttachment::VoiceNote)
        );
        // The photo has its own stride: no message carries a photo and a
        // file at the same time.
        for id in (0..PERF_MESSAGE_TOTAL).filter(|id| id % 37 == 0) {
            assert!(
                fake_attachment_of(id).is_none(),
                "the message {id} carries the photo and a file"
            );
        }
    }
}
