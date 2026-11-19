//! The lifecycle of the requests of the client.
//!
//! port: taken from the origin's `desktop/src/rpc.rs`, with one difference
//! that the plan of the migration forced: the origin kept the deadline of a
//! request inside `PendingRequest::timeout`, and here the deadline belongs to
//! the transport crate (`RequestClass`). The *semantic* part of a pending
//! request — which chat, which message, which draft — stays here, because
//! only the user interface knows what to do with the answer.
//!
//! The transport crate owns the mechanics (the ids, the table of the requests
//! in flight, the expiration). This module owns the meaning: a `HashMap` of
//! the requests that are still waiting, keyed by the id the transport gave
//! them.

// Some of the requests of the table are only constructed by the drawing half
// of this batch (`crate::render` asks for the avatars and the images).
// TODO(port): drop this allow once `render.rs` is in the same crate.
#![allow(dead_code)]

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use anyhow::{Context as _, Result};
use whatfast_protocol::{
    BackendEvent, RpcResponse, envelope, fake, rpc_request,
    transport::{self, RequestClass, RequestTracker},
};

pub use whatfast_protocol::transport::{BridgeMessage, PROTOCOL_VERSION};

/// How often the client sweeps the requests that outlived their deadline.
pub(crate) const TIMEOUT_SWEEP_INTERVAL: Duration = Duration::from_secs(1);

/// How long the client waits before it probes a repaired reaction again.
///
/// port: the origin waited ten minutes and one second, so the probe never
/// landed on the same second as the repair it was checking.
pub(crate) const REACTION_REPAIR_RETRY: Duration = Duration::from_secs(10 * 60 + 1);

/// How many times a request is retried before it is given up.
const MAX_RETRIES: u8 = 3;

/// The semantic half of a request: what the client is waiting for.
///
/// The deadline is not here: it comes from [`PendingRequest::class`], which
/// maps the variant to the class of the transport.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PendingRequest {
    Hello,
    Auth,
    Pairing,
    Chats {
        cursor: String,
    },
    Messages {
        chat_id: String,
        prepend: bool,
        generation: u64,
    },
    Search {
        query: String,
        generation: u64,
    },
    OpenContact,
    MessagesAround {
        chat_id: String,
        message_id: String,
        generation: u64,
    },
    OpenMessageWindow {
        chat_id: String,
        generation: u64,
    },
    MessagesAfter {
        chat_id: String,
        generation: u64,
    },
    Avatar {
        chat_id: String,
    },
    ParticipantAvatar {
        participant_id: String,
    },
    ChatInfo {
        chat_id: String,
    },
    ChatMedia {
        chat_id: String,
        category: String,
    },
    MentionDirectory {
        chat_id: String,
    },
    SetTyping,
    CreateGroup,
    SendText {
        chat_id: String,
        draft_text: String,
        reply_to_message_id: Option<String>,
        /// (display name, JID) pairs backing `@Name` tokens in `draft_text`.
        mentions: Vec<(String, String)>,
        /// The row of the agent draft this send closes. `None` when the text
        /// came from the composer: two equal texts must not close each other's
        /// row.
        draft_id: Option<i64>,
    },
    SendImage {
        chat_id: String,
        draft_text: String,
        reply_to_message_id: Option<String>,
        /// La foto que viaja en este pedido: si falla, vuelve a la bandeja.
        image_path: String,
    },
    SendSticker,
    MessageImage {
        chat_id: String,
        message_id: String,
    },
    SendReaction {
        chat_id: String,
        message_id: String,
        emoji: String,
        client_reaction_id: String,
        attempt: u8,
    },
    RepairRecentReactions {
        chat_id: String,
    },
    MarkRead {
        chat_id: String,
        previous_unread: u32,
    },
    Logout,
    // v22 calls: control plane and the audio pump. The audio requests are
    // fire-and-forget by contract; a timeout on them is a dropped frame,
    // not an error toast.
    CallPlace,
    CallAccept,
    CallReject,
    CallHangup,
    CallMute,
    CallAudio,
    /// One access unit of the camera toward the sidecar, at 15 per second.
    CallVideo,
    /// The progress of the synchronization of the account.
    SyncStatus,
    /// Asks WhatsApp for more history of one conversation.
    OlderHistory {
        chat_id: String,
    },
    /// The coverage of the history of one conversation.
    HistoryCoverage {
        chat_id: String,
    },
    /// The summary of the history of every conversation.
    HistoryOverview,
    /// The health of the local database of the sidecar.
    IntegrityStatus,
    /// Repairs the local database of the sidecar.
    RepairLocalCache,
    /// The space the media cache takes on this device.
    StorageUsage,
    /// Deletes the files of the media cache of this device.
    ClearMediaCache,
    /// Asks the sidecar to reconnect to WhatsApp.
    Reconnect,
    /// Sends a file that is not an image.
    SendAttachment {
        chat_id: String,
    },
    /// Downloads the file of one attachment into the local cache.
    MessageAttachment {
        chat_id: String,
        message_id: String,
    },
    /// The library of stickers.
    Stickers,
    /// Sends one sticker of the library.
    SendStickerFromLibrary {
        chat_id: String,
        sticker_id: String,
    },
    /// Sends one message to another conversation.
    ForwardMessage {
        source_chat_id: String,
        message_id: String,
        target_chat_id: String,
    },
    /// Changes the text of one message.
    EditMessage {
        chat_id: String,
        message_id: String,
        /// The text the user typed, kept to put the draft back on a failure.
        draft_text: String,
    },
    /// Deletes one message.
    DeleteMessage {
        chat_id: String,
        message_id: String,
    },
    /// Pins or unpins one message.
    SetMessagePin {
        chat_id: String,
        message_id: String,
        /// The generation of the pin, so a stale answer does not reload the
        /// list of the pinned messages.
        generation: u64,
    },
    /// The pinned messages of one conversation.
    PinnedMessages {
        chat_id: String,
    },
    /// Creates one poll.
    CreatePoll {
        chat_id: String,
    },
    /// Votes one poll.
    VotePoll {
        chat_id: String,
        poll_message_id: String,
        generation: u64,
    },
    /// The statuses of the profile.
    Statuses,
    /// Subscribes to one newsletter.
    SubscribeNewsletter,
    /// The newsletters the account is subscribed to.
    Newsletters,
    /// The posts of one newsletter.
    NewsletterMessages {
        jid: String,
    },
    /// Unsubscribes from one newsletter.
    UnsubscribeNewsletter {
        jid: String,
    },
    /// Pins or unpins one conversation.
    SetChatPin {
        chat_id: String,
        /// The flag before the change, kept to roll it back on a failure.
        previous_pinned: bool,
    },
    /// Archives or unarchives one conversation.
    SetChatArchive {
        chat_id: String,
        /// The flag before the change, kept to roll it back on a failure.
        previous_archived: bool,
    },
    /// Mutes or unmutes one conversation.
    SetChatMute {
        chat_id: String,
        /// The flags before the change, kept to roll them back on a failure.
        previous_muted: bool,
        previous_mute_until_ms: i64,
    },
    /// Blocks or unblocks one conversation.
    SetChatBlock {
        chat_id: String,
        /// The flag before the change, kept to roll it back on a failure.
        previous_blocked: bool,
    },
    /// Asks the sidecar to stop.
    Shutdown,
    /// The fast mode of the chat: one question about the conversation, put to
    /// an agent that answers without a window.
    AgentAsk {
        chat_id: String,
    },
    /// The automations of the profile: the tasks of the timer.
    Schedules,
    /// Creates one task of the timer, or rewrites one the user already has.
    SchedulePut,
    /// Pauses one task of the timer, or lets it run again.
    SchedulePause {
        id: i64,
    },
    /// Removes one task of the timer for good.
    ScheduleRemove {
        id: i64,
    },
    /// The card the composer shows while the user types: the desktop asks
    /// for the first link of its draft. A late answer is thrown away when
    /// the draft already points at another link.
    LinkPreview {
        url: String,
    },
}

impl PendingRequest {
    /// The class of the request, which decides its deadline.
    ///
    /// port: the origin decided the deadline in `PendingRequest::timeout` and
    /// put the media downloads (`Avatar`, `ParticipantAvatar`,
    /// `MessageImage`) in the reading class, with the thirty seconds of a
    /// read. The mapping is kept as it was, so the deadlines of the client
    /// do not change.
    pub(crate) fn class(&self) -> RequestClass {
        match self {
            Self::Hello | Self::Auth => RequestClass::Control,
            Self::Reconnect | Self::Shutdown => RequestClass::Control,
            Self::Chats { .. }
            | Self::Messages { .. }
            | Self::Search { .. }
            | Self::OpenContact
            | Self::MessagesAround { .. }
            | Self::OpenMessageWindow { .. }
            | Self::MessagesAfter { .. }
            | Self::Avatar { .. }
            | Self::ParticipantAvatar { .. }
            | Self::ChatInfo { .. }
            | Self::ChatMedia { .. }
            | Self::MentionDirectory { .. }
            | Self::MessageImage { .. }
            | Self::SyncStatus
            | Self::OlderHistory { .. }
            | Self::HistoryCoverage { .. }
            | Self::HistoryOverview
            | Self::IntegrityStatus
            | Self::StorageUsage
            | Self::Stickers
            | Self::PinnedMessages { .. }
            | Self::Statuses
            | Self::Newsletters
            | Self::NewsletterMessages { .. }
            | Self::Schedules
            | Self::LinkPreview { .. } => RequestClass::Read,
            Self::Pairing
            | Self::SetTyping
            | Self::SendText { .. }
            | Self::SendImage { .. }
            | Self::SendSticker
            | Self::SendReaction { .. }
            | Self::RepairRecentReactions { .. }
            | Self::MarkRead { .. }
            | Self::Logout
            | Self::RepairLocalCache
            | Self::ClearMediaCache
            | Self::SendStickerFromLibrary { .. }
            | Self::ForwardMessage { .. }
            | Self::EditMessage { .. }
            | Self::DeleteMessage { .. }
            | Self::CallPlace
            | Self::CallAccept
            | Self::CallReject
            | Self::CallHangup
            | Self::CallMute
            | Self::CallAudio
            | Self::CallVideo
            | Self::SetMessagePin { .. }
            | Self::CreatePoll { .. }
            | Self::VotePoll { .. }
            | Self::SubscribeNewsletter
            | Self::UnsubscribeNewsletter { .. }
            | Self::SetChatPin { .. }
            | Self::SetChatArchive { .. }
            | Self::SetChatMute { .. }
            | Self::SetChatBlock { .. }
            | Self::CreateGroup
            | Self::SchedulePut
            | Self::SchedulePause { .. }
            | Self::ScheduleRemove { .. } => RequestClass::Write,
            // port: the origin had no deadline for these two, because its
            // transport had none. The one of the bridge gives the uploads and
            // the downloads of media twenty two minutes, which is what a big
            // file over a slow link needs.
            //
            // The question of the fast mode takes the same class for another
            // reason: the harness gives a question ninety seconds, more than
            // the sixty of a write, and an answer the client stopped waiting
            // for would be thrown away while the backend was still writing it.
            Self::SendAttachment { .. }
            | Self::MessageAttachment { .. }
            | Self::AgentAsk { .. } => RequestClass::Attachment,
        }
    }

    /// Whether the request moves media in or out of the local cache.
    pub(crate) fn is_media(&self) -> bool {
        matches!(
            self,
            Self::Avatar { .. }
                | Self::ParticipantAvatar { .. }
                | Self::MessageImage { .. }
                | Self::MessageAttachment { .. }
        )
    }
}

/// How long to wait before the next attempt of a failed request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Retry {
    pub(crate) attempt: u8,
    pub(crate) delay: Duration,
}

/// The backoff of a reaction that failed for a reason worth retrying.
pub(crate) fn reaction_retry(retryable: bool, attempt: u8) -> Option<Retry> {
    if !retryable || attempt >= MAX_RETRIES {
        return None;
    }
    let attempt = attempt + 1;
    Some(Retry {
        attempt,
        delay: Duration::from_secs(u64::from(attempt)),
    })
}

/// The backoff of an avatar that failed for a reason worth retrying.
pub(crate) fn avatar_retry(retryable: bool, retries_completed: u8) -> Option<Retry> {
    if !retryable || retries_completed >= MAX_RETRIES {
        return None;
    }
    let attempt = retries_completed + 1;
    Some(Retry {
        attempt,
        delay: Duration::from_secs(5 * u64::from(attempt)),
    })
}

/// What an inbound message of the backend means for the user interface.
pub(crate) enum RpcIncoming {
    Response {
        pending: PendingRequest,
        response: RpcResponse,
    },
    Event(BackendEvent),
    Exited(String),
    Invalid,
    Ignore,
}

/// The backend the client talks to: the Go sidecar or the demonstration one.
///
/// port: the origin had a single `BackendClient` that decided at run time
/// whether it was the fake or the real sidecar. Here the two are different
/// types, so the choice is a variant.
enum Backend {
    Real(transport::BackendClient),
    Fake(fake::FakeBackend),
}

impl Backend {
    /// The ordered stream of what the backend sends back.
    fn incoming(&self) -> async_channel::Receiver<BridgeMessage> {
        match self {
            Self::Real(client) => client.incoming.clone(),
            Self::Fake(client) => client.incoming.clone(),
        }
    }

    /// Queue a request for the backend.
    ///
    /// port: the origin had one transport and one error type. Here the two
    /// backends fail differently, so the error of the transport is the one
    /// that travels up.
    fn send(&self, request_id: u64, request: rpc_request::Request) -> Result<()> {
        match self {
            Self::Real(client) => client
                .send(request_id, request)
                .map_err(anyhow::Error::from),
            Self::Fake(client) => client
                .send(request_id, request)
                .map_err(anyhow::Error::from),
        }
    }
}

/// The client of the backend: it sends the requests, correlates the answers
/// and reports the ones that outlived their deadline.
pub(crate) struct RpcClient {
    backend: Backend,
    /// The mechanics: the ids and the deadlines.
    tracker: RequestTracker,
    /// The meaning: what each request that is still in flight is waiting for, plus start time for RTT.
    pending: HashMap<u64, (PendingRequest, Instant)>,
    /// The instant the deadlines are measured from.
    epoch: Instant,
}

/// The file the user sends to a conversation.
pub(crate) struct OutgoingAttachment {
    /// A UUID that the backend persists, so the same file is never sent
    /// twice.
    pub(crate) client_message_id: String,
    pub(crate) chat_id: String,
    /// The absolute path of the file the user picked.
    pub(crate) file_path: String,
    pub(crate) kind: whatfast_protocol::AttachmentKind,
    pub(crate) caption: String,
    pub(crate) reply_to_message_id: String,
    /// True for a voice note, which cannot carry a caption.
    pub(crate) voice_note: bool,
}

impl RpcClient {
    /// Start the backend and the client.
    ///
    /// port: the origin resolved the executable, created the profile
    /// directory and passed the flags to the sidecar inside the transport.
    /// Here the transport takes an already spawned process over, so the
    /// spawning lives in this module.
    pub(crate) fn start(fake: bool) -> Result<Self> {
        let backend = if fake {
            Backend::Fake(fake::FakeBackend::start(whatfast_model::paths::data_dir()))
        } else {
            Backend::Real(spawn_backend()?)
        };
        Ok(Self::new(backend))
    }

    /// A client with no backend behind it: every send fails.
    pub(crate) fn disconnected() -> Self {
        Self::new(Backend::Real(transport::BackendClient::disconnected()))
    }

    fn new(backend: Backend) -> Self {
        Self {
            backend,
            tracker: RequestTracker::default(),
            pending: HashMap::new(),
            epoch: Instant::now(),
        }
    }

    /// The ordered stream of what the backend sends back.
    pub(crate) fn incoming(&self) -> async_channel::Receiver<BridgeMessage> {
        self.backend.incoming()
    }

    /// Send a request and remember what it is waiting for.
    pub(crate) fn send(
        &mut self,
        request: rpc_request::Request,
        pending: PendingRequest,
    ) -> Result<u64> {
        let request_id = self.tracker.begin(pending.class(), self.epoch.elapsed())?;
        self.pending.insert(request_id, (pending, Instant::now()));
        if let Err(error) = self.backend.send(request_id, request) {
            // A request that could not be queued is not pending anymore.
            self.tracker.complete(request_id);
            self.pending.remove(&request_id);
            return Err(error);
        }
        Ok(request_id)
    }

    /// The progress of the synchronization of the account.
    pub(crate) fn request_sync_status(&mut self) -> Result<u64> {
        self.send(
            rpc_request::Request::GetSyncStatus(whatfast_protocol::GetSyncStatusRequest {}),
            PendingRequest::SyncStatus,
        )
    }

    /// Asks WhatsApp for more history of one conversation.
    pub(crate) fn request_older_history(&mut self, chat_id: String) -> Result<u64> {
        self.send(
            rpc_request::Request::RequestOlderHistory(
                whatfast_protocol::RequestOlderHistoryRequest {
                    chat_id: chat_id.clone(),
                },
            ),
            PendingRequest::OlderHistory { chat_id },
        )
    }

    /// The coverage of the history of one conversation.
    pub(crate) fn request_history_coverage(&mut self, chat_id: String) -> Result<u64> {
        self.send(
            rpc_request::Request::GetChatHistoryCoverage(
                whatfast_protocol::GetChatHistoryCoverageRequest {
                    chat_id: chat_id.clone(),
                },
            ),
            PendingRequest::HistoryCoverage { chat_id },
        )
    }

    /// The summary of the history of every conversation.
    pub(crate) fn request_history_overview(&mut self) -> Result<u64> {
        self.send(
            rpc_request::Request::GetHistoryOverview(
                whatfast_protocol::GetHistoryOverviewRequest {},
            ),
            PendingRequest::HistoryOverview,
        )
    }

    /// The health of the local database of the sidecar.
    pub(crate) fn request_integrity_status(&mut self) -> Result<u64> {
        self.send(
            rpc_request::Request::GetIntegrityStatus(
                whatfast_protocol::GetIntegrityStatusRequest {},
            ),
            PendingRequest::IntegrityStatus,
        )
    }

    /// Repairs the local database of the sidecar.
    pub(crate) fn request_repair_local_cache(&mut self) -> Result<u64> {
        self.send(
            rpc_request::Request::RepairLocalCache(whatfast_protocol::RepairLocalCacheRequest {}),
            PendingRequest::RepairLocalCache,
        )
    }

    /// The space the media cache takes on this device, grouped by kind.
    pub(crate) fn request_storage_usage(&mut self) -> Result<u64> {
        self.send(
            rpc_request::Request::StorageUsage(whatfast_protocol::StorageUsageRequest {}),
            PendingRequest::StorageUsage,
        )
    }

    /// Deletes the files of the media cache of this device.
    pub(crate) fn request_clear_media_cache(&mut self) -> Result<u64> {
        self.send(
            rpc_request::Request::ClearMediaCache(whatfast_protocol::ClearMediaCacheRequest {}),
            PendingRequest::ClearMediaCache,
        )
    }

    /// Asks the sidecar to reconnect to WhatsApp.
    pub(crate) fn request_reconnect(&mut self) -> Result<u64> {
        self.send(
            rpc_request::Request::Reconnect(whatfast_protocol::ReconnectRequest {}),
            PendingRequest::Reconnect,
        )
    }

    /// Sends a file that is not an image.
    pub(crate) fn request_send_attachment(
        &mut self,
        attachment: OutgoingAttachment,
    ) -> Result<u64> {
        let chat_id = attachment.chat_id.clone();
        self.send(
            rpc_request::Request::SendAttachment(whatfast_protocol::SendAttachmentRequest {
                client_message_id: attachment.client_message_id,
                chat_id: chat_id.clone(),
                file_path: attachment.file_path,
                kind: attachment.kind as i32,
                caption: attachment.caption,
                reply_to_message_id: attachment.reply_to_message_id,
                voice_note: attachment.voice_note,
                reply_to_chat_id: String::new(),
            }),
            PendingRequest::SendAttachment { chat_id },
        )
    }

    /// Downloads the file of one attachment into the local cache.
    pub(crate) fn request_message_attachment(
        &mut self,
        chat_id: String,
        message_id: String,
    ) -> Result<u64> {
        self.send(
            rpc_request::Request::GetMessageAttachment(
                whatfast_protocol::GetMessageAttachmentRequest {
                    chat_id: chat_id.clone(),
                    message_id: message_id.clone(),
                },
            ),
            PendingRequest::MessageAttachment {
                chat_id,
                message_id,
            },
        )
    }

    /// The library of stickers.
    pub(crate) fn request_stickers(&mut self) -> Result<u64> {
        self.send(
            rpc_request::Request::ListStickers(whatfast_protocol::ListStickersRequest {}),
            PendingRequest::Stickers,
        )
    }

    /// The card of one link of the draft, so the composer can draw it before
    /// the send. A page without a card answers empty, which is not an error.
    pub(crate) fn request_link_preview(&mut self, url: String) -> Result<u64> {
        self.send(
            rpc_request::Request::GetLinkPreview(whatfast_protocol::GetLinkPreviewRequest {
                url: url.clone(),
            }),
            PendingRequest::LinkPreview { url },
        )
    }

    /// Sends one sticker of the library.
    pub(crate) fn request_send_sticker_from_library(
        &mut self,
        client_message_id: String,
        chat_id: String,
        sticker_id: String,
    ) -> Result<u64> {
        self.send(
            rpc_request::Request::SendStickerFromLibrary(
                whatfast_protocol::SendStickerFromLibraryRequest {
                    client_message_id,
                    chat_id: chat_id.clone(),
                    sticker_id: sticker_id.clone(),
                    reply_to_message_id: String::new(),
                    reply_to_chat_id: String::new(),
                },
            ),
            PendingRequest::SendStickerFromLibrary {
                chat_id,
                sticker_id,
            },
        )
    }

    /// Sends one message to another conversation.
    pub(crate) fn request_forward_message(
        &mut self,
        client_message_id: String,
        source_chat_id: String,
        message_id: String,
        target_chat_id: String,
    ) -> Result<u64> {
        self.send(
            rpc_request::Request::ForwardMessage(whatfast_protocol::ForwardMessageRequest {
                client_message_id,
                source_chat_id: source_chat_id.clone(),
                message_id: message_id.clone(),
                target_chat_id: target_chat_id.clone(),
            }),
            PendingRequest::ForwardMessage {
                source_chat_id,
                message_id,
                target_chat_id,
            },
        )
    }

    /// Changes the text of one message.
    pub(crate) fn request_edit_message(
        &mut self,
        chat_id: String,
        message_id: String,
        text: String,
    ) -> Result<u64> {
        self.send(
            rpc_request::Request::EditMessage(whatfast_protocol::EditMessageRequest {
                chat_id: chat_id.clone(),
                message_id: message_id.clone(),
                text: text.clone(),
            }),
            PendingRequest::EditMessage {
                chat_id,
                message_id,
                draft_text: text,
            },
        )
    }

    /// Deletes one message.
    pub(crate) fn request_delete_message(
        &mut self,
        chat_id: String,
        message_id: String,
        delete_for_everyone: bool,
    ) -> Result<u64> {
        self.send(
            rpc_request::Request::DeleteMessage(whatfast_protocol::DeleteMessageRequest {
                chat_id: chat_id.clone(),
                message_id: message_id.clone(),
                delete_for_everyone,
            }),
            PendingRequest::DeleteMessage {
                chat_id,
                message_id,
            },
        )
    }

    /// Pins or unpins one message.
    pub(crate) fn request_set_message_pin(
        &mut self,
        chat_id: String,
        message_id: String,
        pinned: bool,
        generation: u64,
    ) -> Result<u64> {
        self.send(
            rpc_request::Request::SetMessagePin(whatfast_protocol::SetMessagePinRequest {
                chat_id: chat_id.clone(),
                message_id: message_id.clone(),
                pinned,
            }),
            PendingRequest::SetMessagePin {
                chat_id,
                message_id,
                generation,
            },
        )
    }

    /// The pinned messages of one conversation.
    pub(crate) fn request_pinned_messages(&mut self, chat_id: String) -> Result<u64> {
        self.send(
            rpc_request::Request::ListPinnedMessages(
                whatfast_protocol::ListPinnedMessagesRequest {
                    chat_id: chat_id.clone(),
                },
            ),
            PendingRequest::PinnedMessages { chat_id },
        )
    }

    /// Creates one poll.
    pub(crate) fn request_create_poll(
        &mut self,
        client_message_id: String,
        chat_id: String,
        question: String,
        options: Vec<String>,
        selectable_options_count: u32,
    ) -> Result<u64> {
        self.send(
            rpc_request::Request::CreatePoll(whatfast_protocol::CreatePollRequest {
                client_message_id,
                chat_id: chat_id.clone(),
                question,
                options,
                selectable_options_count,
            }),
            PendingRequest::CreatePoll { chat_id },
        )
    }

    /// Votes one poll.
    pub(crate) fn request_vote_poll(
        &mut self,
        chat_id: String,
        poll_message_id: String,
        selected_options: Vec<String>,
        generation: u64,
    ) -> Result<u64> {
        self.send(
            rpc_request::Request::VotePoll(whatfast_protocol::VotePollRequest {
                chat_id: chat_id.clone(),
                poll_message_id: poll_message_id.clone(),
                selected_options,
            }),
            PendingRequest::VotePoll {
                chat_id,
                poll_message_id,
                generation,
            },
        )
    }

    /// The statuses of the profile.
    pub(crate) fn request_statuses(&mut self) -> Result<u64> {
        self.send(
            rpc_request::Request::ListStatuses(whatfast_protocol::ListStatusesRequest {}),
            PendingRequest::Statuses,
        )
    }

    /// Subscribes to one newsletter.
    pub(crate) fn request_subscribe_newsletter(&mut self, invite_key: String) -> Result<u64> {
        self.send(
            rpc_request::Request::SubscribeNewsletter(
                whatfast_protocol::SubscribeNewsletterRequest { invite_key },
            ),
            PendingRequest::SubscribeNewsletter,
        )
    }

    /// The newsletters the account is subscribed to.
    pub(crate) fn request_newsletters(&mut self) -> Result<u64> {
        self.send(
            rpc_request::Request::ListNewsletters(whatfast_protocol::ListNewslettersRequest {}),
            PendingRequest::Newsletters,
        )
    }

    /// The posts of one newsletter.
    pub(crate) fn request_newsletter_messages(&mut self, jid: String, limit: i32) -> Result<u64> {
        self.send(
            rpc_request::Request::ListNewsletterMessages(
                whatfast_protocol::ListNewsletterMessagesRequest {
                    jid: jid.clone(),
                    limit,
                },
            ),
            PendingRequest::NewsletterMessages { jid },
        )
    }

    /// Unsubscribes from one newsletter.
    pub(crate) fn request_unsubscribe_newsletter(&mut self, jid: String) -> Result<u64> {
        self.send(
            rpc_request::Request::UnsubscribeNewsletter(
                whatfast_protocol::UnsubscribeNewsletterRequest { jid: jid.clone() },
            ),
            PendingRequest::UnsubscribeNewsletter { jid },
        )
    }

    /// Pins or unpins one conversation.
    pub(crate) fn request_set_chat_pin(
        &mut self,
        chat_id: String,
        pinned: bool,
        previous_pinned: bool,
    ) -> Result<u64> {
        self.send(
            rpc_request::Request::SetChatPin(whatfast_protocol::SetChatPinRequest {
                chat_id: chat_id.clone(),
                pinned,
            }),
            PendingRequest::SetChatPin {
                chat_id,
                previous_pinned,
            },
        )
    }

    /// Archives or unarchives one conversation.
    pub(crate) fn request_set_chat_archive(
        &mut self,
        chat_id: String,
        archived: bool,
        previous_archived: bool,
    ) -> Result<u64> {
        self.send(
            rpc_request::Request::SetChatArchive(whatfast_protocol::SetChatArchiveRequest {
                chat_id: chat_id.clone(),
                archived,
            }),
            PendingRequest::SetChatArchive {
                chat_id,
                previous_archived,
            },
        )
    }

    /// Mutes or unmutes one conversation.
    pub(crate) fn request_set_chat_mute(
        &mut self,
        chat_id: String,
        mute_until_ms: i64,
        previous_muted: bool,
        previous_mute_until_ms: i64,
    ) -> Result<u64> {
        self.send(
            rpc_request::Request::SetChatMute(whatfast_protocol::SetChatMuteRequest {
                chat_id: chat_id.clone(),
                mute_until_ms,
            }),
            PendingRequest::SetChatMute {
                chat_id,
                previous_muted,
                previous_mute_until_ms,
            },
        )
    }

    /// Blocks or unblocks one conversation.
    pub(crate) fn request_set_chat_block(
        &mut self,
        chat_id: String,
        blocked: bool,
        previous_blocked: bool,
    ) -> Result<u64> {
        self.send(
            rpc_request::Request::SetChatBlock(whatfast_protocol::SetChatBlockRequest {
                chat_id: chat_id.clone(),
                blocked,
            }),
            PendingRequest::SetChatBlock {
                chat_id,
                previous_blocked,
            },
        )
    }

    /// Asks the sidecar to stop.
    pub(crate) fn request_shutdown(&mut self) -> Result<u64> {
        self.send(
            rpc_request::Request::Shutdown(whatfast_protocol::ShutdownRequest {}),
            PendingRequest::Shutdown,
        )
    }

    /// Stops the sidecar and waits for it to end, before the update replaces
    /// its executable.
    ///
    /// The wait is bounded by the same grace of the transport: after that the
    /// sidecar is killed, and the files can move anyway.
    pub(crate) fn stop_backend(&mut self) {
        if let Backend::Real(client) = &self.backend {
            let _ = client.shutdown_and_wait(std::time::Duration::from_secs(8));
        }
    }

    /// Starts the sidecar again after a failed update stopped it. The event
    /// stream is the same one the window already reads.
    pub(crate) fn restart_backend(&mut self) -> Result<()> {
        let (mut command, executable) = backend_command()?;
        let child = command
            .spawn()
            .with_context(|| format!("start backend {}", executable.display()))?;
        #[cfg(windows)]
        let _ = backend_job::adopt(&child);
        match &mut self.backend {
            Backend::Real(client) => client.restart_with(child)?,
            Backend::Fake(_) => anyhow::bail!("a harness backend cannot be restarted"),
        }
        // The old requests died with the old process: none of them can be
        // answered now, so their entries must not wait for a deadline.
        self.tracker.clear();
        self.pending.clear();
        Ok(())
    }

    /// Classify an inbound message of the backend.
    ///
    /// port: the origin correlated the answers with its own table. Here the
    /// table of the transport decides whether the answer belongs to a
    /// request that is still in flight, and the table of this module says
    /// what that request was about.
    pub(crate) fn handle_incoming(&mut self, message: BridgeMessage) -> RpcIncoming {
        let envelope = match message {
            BridgeMessage::Envelope(envelope) => envelope,
            BridgeMessage::Exited(exit) => {
                self.tracker.clear();
                self.pending.clear();
                // The client is a behavioral reference without the restart
                // supervisor, so both exit classes share its existing terminal
                // screen while still consuming the structured result.
                let _legacy_exit_kind = exit.kind;
                return RpcIncoming::Exited(exit.message);
            }
        };
        let request_id = envelope.request_id;
        match envelope.body {
            Some(envelope::Body::Response(response)) => {
                self.tracker.complete(request_id);
                self.pending
                    .remove(&request_id)
                    .map_or(RpcIncoming::Ignore, |(pending, sent_at)| {
                        crate::dev_stats::record_rpc_rtt_ms(sent_at.elapsed().as_millis() as u64);
                        RpcIncoming::Response { pending, response }
                    })
            }
            Some(envelope::Body::Event(event)) if request_id == 0 => RpcIncoming::Event(event),
            _ => RpcIncoming::Invalid,
        }
    }

    /// The requests that outlived their deadline, oldest id first.
    pub(crate) fn expire(&mut self) -> Vec<(u64, PendingRequest)> {
        let expired = self.tracker.expire(self.epoch.elapsed());
        expired
            .into_iter()
            .filter_map(|(request_id, _)| {
                self.pending
                    .remove(&request_id)
                    .map(|(pending, _)| (request_id, pending))
            })
            .collect()
    }

    /// The requests that are still in flight.
    pub(crate) fn pending_requests(&self) -> impl Iterator<Item = &PendingRequest> {
        self.pending.values().map(|(req, _)| req)
    }

    /// How many media downloads are in flight.
    pub(crate) fn pending_media_count(&self) -> usize {
        self.pending_requests()
            .filter(|request| request.is_media())
            .count()
    }

    /// Whether a logout is still in flight.
    pub(crate) fn logout_pending(&self) -> bool {
        self.pending_requests()
            .any(|request| matches!(request, PendingRequest::Logout))
    }

    /// Forget every pending request.
    pub(crate) fn clear_pending(&mut self) {
        self.tracker.clear();
        self.pending.clear();
    }
}

/// The Windows job that owns the sidecar.
///
/// The backend is a child of this process, but a crash or a kill from the
/// Task Manager does not run the destructor that asks it to stop. A job with
/// `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` closes with the process, and Windows
/// ends every process of the job at that moment. Hiding the window in the
/// tray keeps the app alive, so the job is untouched until the real exit.
#[cfg(windows)]
mod backend_job {
    use std::{ffi::c_void, mem::size_of, sync::OnceLock};

    use windows_sys::Win32::{
        Foundation::{CloseHandle, HANDLE},
        System::{
            JobObjects::{
                AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
                SetInformationJobObject,
            },
            Threading::{OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE},
        },
    };

    /// The handle of the job, or zero when Windows refused to create it.
    static JOB: OnceLock<isize> = OnceLock::new();

    fn job() -> Option<HANDLE> {
        let handle = *JOB.get_or_init(|| unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() {
                return 0;
            }
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let configured = SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const c_void,
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            if configured == 0 {
                CloseHandle(job);
                return 0;
            }
            job as isize
        });
        (handle != 0).then_some(handle as HANDLE)
    }

    /// Puts the sidecar in the job. Best effort: a refusal only loses the
    /// guarantee of a crash, not the backend.
    pub(super) fn adopt(child: &std::process::Child) -> bool {
        let Some(job) = job() else {
            return false;
        };
        let process = unsafe { OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, child.id()) };
        if process.is_null() {
            return false;
        }
        let assigned = unsafe { AssignProcessToJobObject(job, process) } != 0;
        unsafe {
            CloseHandle(process);
        }
        assigned
    }

}

/// Spawn the Go sidecar and hand the process over to the transport.
///
/// port: the origin did this inside its `bridge.rs`, with the same flags and
/// the same `CREATE_NO_WINDOW` on Windows.
fn spawn_backend() -> Result<transport::BackendClient> {
    let (mut command, executable) = backend_command()?;
    let child = command
        .spawn()
        .with_context(|| format!("start backend {}", executable.display()))?;
    // El backend no puede sobrevivir a esta aplicacion, ni siquiera cuando el
    // proceso muere sin pasar por el `Drop` que le pide apagarse.
    #[cfg(windows)]
    let _ = backend_job::adopt(&child);
    Ok(transport::BackendClient::from_child(child)?)
}

/// The command of the sidecar, shared by the first start and by a restart
/// after a failed update.
fn backend_command() -> Result<(Command, PathBuf)> {
    let executable = backend_executable()?;
    let data_dir = whatfast_model::paths::data_dir();
    std::fs::create_dir_all(&data_dir)
        .with_context(|| format!("create app data directory {}", data_dir.display()))?;
    let log_file = data_dir.join("backend.log");
    let mut command = Command::new(&executable);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    command
        .arg("--stdio")
        .arg("--data-dir")
        .arg(&data_dir)
        .arg("--log-file")
        .arg(log_file)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    Ok((command, executable))
}

/// Where the Go sidecar is.
///
/// port: the environment variable first, then the executable next to the one
/// of the application.
fn backend_executable() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("WHATFAST_BACKEND") {
        return Ok(path.into());
    }
    let current = std::env::current_exe().context("resolve desktop executable")?;
    let name = if cfg!(windows) {
        "whatfast-backend.exe"
    } else {
        "whatfast-backend"
    };
    Ok(current.parent().unwrap_or(Path::new(".")).join(name))
}

