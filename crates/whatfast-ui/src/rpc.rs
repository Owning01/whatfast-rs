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
    MentionDirectory {
        chat_id: String,
    },
    SetTyping,
    SendText {
        chat_id: String,
        draft_text: String,
        reply_to_message_id: Option<String>,
        /// (display name, JID) pairs backing `@Name` tokens in `draft_text`.
        mentions: Vec<(String, String)>,
    },
    SendImage {
        chat_id: String,
        draft_text: String,
        reply_to_message_id: Option<String>,
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
            | Self::MentionDirectory { .. }
            | Self::MessageImage { .. } => RequestClass::Read,
            Self::Pairing
            | Self::SetTyping
            | Self::SendText { .. }
            | Self::SendImage { .. }
            | Self::SendSticker
            | Self::SendReaction { .. }
            | Self::RepairRecentReactions { .. }
            | Self::MarkRead { .. }
            | Self::Logout => RequestClass::Write,
        }
    }

    /// Whether the request moves media in or out of the local cache.
    pub(crate) fn is_media(&self) -> bool {
        matches!(
            self,
            Self::Avatar { .. } | Self::ParticipantAvatar { .. } | Self::MessageImage { .. }
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
    /// The meaning: what each request that is still in flight is waiting for.
    pending: HashMap<u64, PendingRequest>,
    /// The instant the deadlines are measured from.
    epoch: Instant,
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
        self.pending.insert(request_id, pending);
        if let Err(error) = self.backend.send(request_id, request) {
            // A request that could not be queued is not pending anymore.
            self.tracker.complete(request_id);
            self.pending.remove(&request_id);
            return Err(error);
        }
        Ok(request_id)
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
                    .map_or(RpcIncoming::Ignore, |pending| RpcIncoming::Response {
                        pending,
                        response,
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
                    .map(|pending| (request_id, pending))
            })
            .collect()
    }

    /// The requests that are still in flight.
    pub(crate) fn pending_requests(&self) -> impl Iterator<Item = &PendingRequest> {
        self.pending.values()
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

/// Spawn the Go sidecar and hand the process over to the transport.
///
/// port: the origin did this inside its `bridge.rs`, with the same flags and
/// the same `CREATE_NO_WINDOW` on Windows.
fn spawn_backend() -> Result<transport::BackendClient> {
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
    let child = command
        .arg("--stdio")
        .arg("--data-dir")
        .arg(&data_dir)
        .arg("--log-file")
        .arg(log_file)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("start backend {}", executable.display()))?;
    Ok(transport::BackendClient::from_child(child)?)
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
