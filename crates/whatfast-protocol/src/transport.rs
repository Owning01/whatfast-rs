//! The framed transport between the native client and the Go sidecar.
//!
//! Port of `tauri/src-tauri/src/bridge.rs` from the origin repo, with the
//! Tauri dependency removed: the binary spawns the sidecar and hands the
//! process over, and this module owns the framing, the reader and writer
//! loops, the exit classification and the request lifecycle.
//!
//! The crate imposes no async runtime. The loops are blocking functions over
//! `std::io` that the caller runs on a thread or on whatever executor it
//! already has; the channels are `async_channel`, the same FIFO abstraction
//! the origin used, and they work with any executor.

use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Read, Write},
    process::Child,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use prost::Message as _;

use crate::{
    BackendEvent, Envelope, RpcRequest, RpcResponse, ShutdownRequest, envelope, rpc_request,
};

/// The protocol version this client speaks (the sidecar's v22 contract:
/// the calls wave).
pub const PROTOCOL_VERSION: u32 = 22;

/// The biggest frame the client reads or writes.
pub const MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;

/// The most requests the client keeps in flight at the same time.
pub const MAX_PENDING_REQUESTS: usize = 512;

/// The most stderr bytes kept around to explain a sidecar crash.
pub const MAX_EXIT_DIAGNOSTIC_BYTES: usize = 8 * 1024;

/// The deadline of a control request (handshake, auth, sync status).
pub const CONTROL_REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

/// The deadline of a read request (chats, messages, avatars).
pub const READ_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// The deadline of a write request (send, mark read, pairing).
pub const WRITE_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

/// The deadline of an attachment request (uploads and downloads of media).
pub const ATTACHMENT_REQUEST_TIMEOUT: Duration = Duration::from_secs(22 * 60);

/// The depth of the queue of envelopes waiting to be written to the sidecar.
const OUTGOING_CHANNEL_CAPACITY: usize = 256;

/// The depth of the queue of messages coming from the sidecar.
const INCOMING_CHANNEL_CAPACITY: usize = 1024;

/// The grace period the sidecar gets to exit on its own before it is killed.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(3);

/// Every failure of the framed transport.
#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("decode bridge envelope: {0}")]
    Decode(#[from] prost::DecodeError),
    #[error("outgoing frame exceeds {max} bytes")]
    OutgoingFrameTooLarge { max: usize },
    #[error("invalid bridge frame length {length}")]
    InvalidFrameLength { length: usize },
    #[error("backend sent protocol version {found} after v{expected} handshake")]
    ProtocolVersionMismatch { expected: u32, found: u32 },
    #[error("backend sent envelope without a body")]
    MissingBody,
    #[error("desktop receiver stopped")]
    ReceiverStopped,
    #[error("backend writer unavailable or backpressured")]
    WriterUnavailable,
    #[error("backend stdin unavailable")]
    StdinUnavailable,
    #[error("backend stdout unavailable")]
    StdoutUnavailable,
    #[error("backend stderr unavailable")]
    StderrUnavailable,
    #[error("too many pending backend requests")]
    TooManyPendingRequests,
    #[error("request IDs exhausted")]
    RequestIdsExhausted,
}

/// What the sidecar sent, or the reason why it is gone.
#[derive(Debug)]
pub enum BridgeMessage {
    Envelope(Box<Envelope>),
    Exited(BridgeExit),
}

/// Whether a sidecar failure is worth retrying.
///
/// A terminal failure never changes no matter how many times the client
/// restarts the sidecar: the profile is taken, the protocols do not match or
/// the local data is corrupt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BridgeExitKind {
    Transient,
    Fatal,
}

/// A dead sidecar and the reason why.
#[derive(Clone, Debug)]
pub struct BridgeExit {
    pub kind: BridgeExitKind,
    pub message: String,
}

/// Decide whether a sidecar failure is terminal.
///
/// The origin listed the needles by hand and every one of them is a failure
/// that a restart cannot fix: a protocol that does not match, a profile that
/// is already in use, or a corrupt local database. Everything else (a broken
/// pipe, a killed process) is transient and may be retried.
pub fn classify_exit(message: &str) -> BridgeExitKind {
    let message = message.to_ascii_lowercase();
    if [
        "unsupported_protocol",
        "protocol version",
        "profile is already in use",
        "database disk image is malformed",
        "file is not a database",
        "database corruption",
    ]
    .iter()
    .any(|needle| message.contains(needle))
    {
        BridgeExitKind::Fatal
    } else {
        BridgeExitKind::Transient
    }
}

/// Compose the message that explains a dead sidecar.
///
/// The origin built this string inline where the reader thread gave up; it
/// is a function here so the binary can reuse it for its own diagnostics.
pub fn exit_message(transport_error: &str, diagnostics: &str, status: Option<&str>) -> String {
    match (diagnostics.trim(), status) {
        ("", Some(status)) => {
            format!("backend transport failed: {transport_error}; backend {status}")
        }
        ("", None) => format!("backend transport failed: {transport_error}"),
        (diagnostic, Some(status)) => format!(
            "backend transport failed: {transport_error}; diagnostics: {diagnostic}; backend {status}"
        ),
        (diagnostic, None) => {
            format!("backend transport failed: {transport_error}; diagnostics: {diagnostic}")
        }
    }
}

/// Write one envelope as a length-prefixed frame.
pub fn write_frame(writer: &mut impl Write, envelope: &Envelope) -> Result<(), TransportError> {
    let bytes = envelope.encode_to_vec();
    if bytes.len() > MAX_FRAME_BYTES {
        return Err(TransportError::OutgoingFrameTooLarge {
            max: MAX_FRAME_BYTES,
        });
    }
    writer.write_all(&(bytes.len() as u32).to_be_bytes())?;
    writer.write_all(&bytes)?;
    writer.flush()?;
    Ok(())
}

/// Read one length-prefixed frame.
pub fn read_frame(reader: &mut impl Read) -> Result<Envelope, TransportError> {
    let mut prefix = [0_u8; 4];
    reader.read_exact(&mut prefix)?;
    let length = u32::from_be_bytes(prefix) as usize;
    if length == 0 || length > MAX_FRAME_BYTES {
        return Err(TransportError::InvalidFrameLength { length });
    }
    let mut payload = vec![0; length];
    reader.read_exact(&mut payload)?;
    Ok(Envelope::decode(payload.as_slice())?)
}

/// Write the queued envelopes to the sidecar until the queue is closed.
pub fn writer_loop(mut writer: impl Write, outgoing: async_channel::Receiver<Envelope>) {
    while let Ok(envelope) = outgoing.recv_blocking() {
        if write_frame(&mut writer, &envelope).is_err() {
            break;
        }
    }
}

/// Read the frames of the sidecar and publish them as an ordered stream.
pub fn reader_loop(
    mut reader: impl Read,
    incoming: &async_channel::Sender<BridgeMessage>,
) -> Result<(), TransportError> {
    loop {
        let envelope = read_frame(&mut reader)?;
        if envelope.protocol_version != PROTOCOL_VERSION {
            return Err(TransportError::ProtocolVersionMismatch {
                expected: PROTOCOL_VERSION,
                found: envelope.protocol_version,
            });
        }
        if envelope.body.is_none() {
            return Err(TransportError::MissingBody);
        }
        incoming
            .send_blocking(BridgeMessage::Envelope(Box::new(envelope)))
            .map_err(|_| TransportError::ReceiverStopped)?;
    }
}

/// The client of the sidecar: it owns the pipes, the queues and the process.
pub struct BackendClient {
    outgoing: async_channel::Sender<Envelope>,
    /// The ordered stream of what the sidecar sends back.
    pub incoming: async_channel::Receiver<BridgeMessage>,
    child: Option<Arc<Mutex<Child>>>,
}

impl BackendClient {
    /// Take over an already spawned sidecar.
    ///
    /// The origin resolved the executable, created the profile directory and
    /// passed the flags to the sidecar here; in the native client that belongs
    /// to the binary, so the process arrives already spawned and configured.
    ///
    /// The three pipes of the child are taken over: the client writes the
    /// requests to its stdin, reads the frames from its stdout and keeps its
    /// stderr to explain a crash.
    pub fn from_child(mut child: Child) -> Result<Self, TransportError> {
        let stdin = child.stdin.take().ok_or(TransportError::StdinUnavailable)?;
        let stdout = child
            .stdout
            .take()
            .ok_or(TransportError::StdoutUnavailable)?;
        let stderr = child
            .stderr
            .take()
            .ok_or(TransportError::StderrUnavailable)?;
        let child = Arc::new(Mutex::new(child));
        let diagnostics = Arc::new(Mutex::new(String::new()));
        let (outgoing, outgoing_rx) = async_channel::bounded(OUTGOING_CHANNEL_CAPACITY);
        let (incoming_tx, incoming) = async_channel::bounded(INCOMING_CHANNEL_CAPACITY);

        thread::Builder::new()
            .name("bridge-writer".into())
            .spawn(move || writer_loop(stdin, outgoing_rx))?;
        let stderr_thread = thread::Builder::new().name("bridge-stderr".into()).spawn({
            let diagnostics = diagnostics.clone();
            move || capture_stderr(stderr, diagnostics)
        })?;
        let reader_child = child.clone();
        thread::Builder::new()
            .name("bridge-reader".into())
            .spawn(move || {
                if let Err(error) = reader_loop(stdout, &incoming_tx) {
                    let transport_error = error.to_string();
                    let status = reader_child
                        .lock()
                        .ok()
                        .and_then(|mut child| child.wait().ok())
                        .map(|status| status.to_string());
                    let _ = stderr_thread.join();
                    let diagnostic = match diagnostics.lock() {
                        Ok(value) => value.clone(),
                        Err(_) => String::new(),
                    };
                    let message = exit_message(&transport_error, &diagnostic, status.as_deref());
                    eprintln!("{message}");
                    let _ = incoming_tx.send_blocking(BridgeMessage::Exited(BridgeExit {
                        kind: classify_exit(&message),
                        message,
                    }));
                }
            })?;

        Ok(Self {
            outgoing,
            incoming,
            child: Some(child),
        })
    }

    /// A client with no sidecar behind it: every send fails.
    pub fn disconnected() -> Self {
        let (outgoing, _) = async_channel::bounded(1);
        let (_, incoming) = async_channel::bounded(1);
        outgoing.close();
        Self {
            outgoing,
            incoming,
            child: None,
        }
    }

    /// Queue a request for the sidecar.
    pub fn send(
        &self,
        request_id: u64,
        request: rpc_request::Request,
    ) -> Result<(), TransportError> {
        self.outgoing
            .try_send(Envelope {
                protocol_version: PROTOCOL_VERSION,
                request_id,
                body: Some(envelope::Body::Request(RpcRequest {
                    request: Some(request),
                })),
            })
            .map_err(|_| TransportError::WriterUnavailable)
    }

    /// Send a request under a deadline and remember it until it answers.
    ///
    /// The pending entry is rolled back when the transport refuses the frame,
    /// so a failed send never leaves a request hanging.
    pub fn send_request(
        &self,
        tracker: &mut RequestTracker,
        class: RequestClass,
        request: rpc_request::Request,
        now: Duration,
    ) -> Result<u64, TransportError> {
        let request_id = tracker.begin(class, now)?;
        if let Err(error) = self.send(request_id, request) {
            tracker.complete(request_id);
            return Err(error);
        }
        Ok(request_id)
    }

    /// Stop accepting new work, request a graceful sidecar shutdown, and
    /// independently guarantee the child is reaped after the grace period.
    pub fn shutdown(&self) {
        let _ = self.send(u64::MAX, rpc_request::Request::Shutdown(ShutdownRequest {}));
        self.outgoing.close();
        if let Some(child) = &self.child {
            spawn_reaper(child.clone());
        }
    }
}

impl Drop for BackendClient {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Keep the sidecar's stderr around to explain a crash.
fn capture_stderr(stderr: impl Read, diagnostics: Arc<Mutex<String>>) {
    for line in BufReader::new(stderr).lines().map_while(Result::ok) {
        eprintln!("whatfast-backend: {line}");
        if let Ok(mut captured) = diagnostics.lock() {
            if captured.len() + line.len() + 1 > MAX_EXIT_DIAGNOSTIC_BYTES {
                let mut remove = (captured.len() + line.len() + 1 - MAX_EXIT_DIAGNOSTIC_BYTES)
                    .min(captured.len());
                // `String::drain` panics unless the end bound is a char
                // boundary; stderr bytes are external input and may be
                // multibyte, so floor to the previous boundary.
                while remove > 0 && !captured.is_char_boundary(remove) {
                    remove -= 1;
                }
                captured.drain(..remove);
            }
            captured.push_str(&line);
            captured.push('\n');
        }
    }
}

/// Reap the sidecar: give it the grace period, then kill it.
fn spawn_reaper(child: Arc<Mutex<Child>>) {
    let _ = thread::Builder::new()
        .name("backend-reaper".into())
        .spawn(move || {
            if let Ok(mut child) = child.lock() {
                let deadline = Instant::now() + SHUTDOWN_GRACE;
                while Instant::now() < deadline {
                    if child.try_wait().ok().flatten().is_some() {
                        return;
                    }
                    thread::sleep(Duration::from_millis(25));
                }
                let _ = child.kill();
                let _ = child.wait();
            }
        });
}

/// The class of a request, which decides its deadline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestClass {
    Control,
    Read,
    Write,
    Attachment,
}

impl RequestClass {
    /// How long the client waits for the answer of a request of this class.
    pub fn timeout(self) -> Duration {
        match self {
            Self::Control => CONTROL_REQUEST_TIMEOUT,
            Self::Read => READ_REQUEST_TIMEOUT,
            Self::Write => WRITE_REQUEST_TIMEOUT,
            Self::Attachment => ATTACHMENT_REQUEST_TIMEOUT,
        }
    }
}

/// A request the client sent and is still waiting for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingRequest {
    pub class: RequestClass,
    /// The instant, in the elapsed time of the caller's epoch, when the
    /// request stops being waited for.
    pub expires_at: Duration,
}

/// What an inbound message means for the request lifecycle.
#[derive(Debug)]
pub enum Incoming {
    /// The answer of a request that was still pending.
    Response {
        pending: PendingRequest,
        response: Box<RpcResponse>,
    },
    /// An event of the sidecar, in the order the sidecar emitted them.
    Event(Box<BackendEvent>),
    /// The sidecar is gone: no request will ever be answered.
    Exited(BridgeExit),
    /// The envelope is neither an answer nor an event.
    Invalid,
    /// A late answer of a request that already expired or completed.
    Ignore,
}

/// The table of the requests in flight and their deadlines.
///
/// The table is the client's only source of truth about what is pending: a
/// response is correlated by its request id, and the requests that outlived
/// their deadline are dropped by `expire`.
#[derive(Debug)]
pub struct RequestTracker {
    next_request_id: u64,
    pending: HashMap<u64, PendingRequest>,
}

impl Default for RequestTracker {
    fn default() -> Self {
        Self {
            // The origin starts the ids at one: zero is the id the sidecar
            // uses for the events, so it never names a request.
            next_request_id: 1,
            pending: HashMap::new(),
        }
    }
}

impl RequestTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a request and return the id that correlates its answer.
    pub fn begin(&mut self, class: RequestClass, now: Duration) -> Result<u64, TransportError> {
        if self.pending.len() >= MAX_PENDING_REQUESTS {
            return Err(TransportError::TooManyPendingRequests);
        }
        let request_id = self.next_request_id;
        if request_id == 0 || request_id == u64::MAX {
            return Err(TransportError::RequestIdsExhausted);
        }
        self.next_request_id = request_id
            .checked_add(1)
            .ok_or(TransportError::RequestIdsExhausted)?;
        let expires_at = now.saturating_add(class.timeout());
        self.pending
            .insert(request_id, PendingRequest { class, expires_at });
        Ok(request_id)
    }

    /// Take the pending request out of the table, if it is still there.
    pub fn complete(&mut self, request_id: u64) -> Option<PendingRequest> {
        self.pending.remove(&request_id)
    }

    /// The requests still in flight.
    pub fn requests(&self) -> impl Iterator<Item = &PendingRequest> {
        self.pending.values()
    }

    /// How many requests are in flight.
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    /// Drop the requests that outlived their deadline, oldest id first.
    pub fn expire(&mut self, now: Duration) -> Vec<(u64, PendingRequest)> {
        let mut expired_ids = self
            .pending
            .iter()
            .filter_map(|(request_id, pending)| (pending.expires_at <= now).then_some(*request_id))
            .collect::<Vec<_>>();
        expired_ids.sort_unstable();
        expired_ids
            .into_iter()
            .filter_map(|request_id| {
                self.complete(request_id)
                    .map(|request| (request_id, request))
            })
            .collect()
    }

    /// Forget every pending request.
    pub fn clear(&mut self) {
        self.pending.clear();
    }

    /// Classify an inbound message against the pending requests.
    pub fn handle(&mut self, message: BridgeMessage) -> Incoming {
        let envelope = match message {
            BridgeMessage::Envelope(envelope) => envelope,
            BridgeMessage::Exited(exit) => {
                // A dead sidecar never answers again: every pending request
                // is dropped instead of waiting for its deadline.
                self.clear();
                return Incoming::Exited(exit);
            }
        };
        let Envelope {
            request_id, body, ..
        } = *envelope;
        match body {
            Some(envelope::Body::Response(response)) => {
                self.complete(request_id)
                    .map_or(Incoming::Ignore, |pending| Incoming::Response {
                        pending,
                        response: Box::new(response),
                    })
            }
            Some(envelope::Body::Event(event)) if request_id == 0 => {
                Incoming::Event(Box::new(event))
            }
            _ => Incoming::Invalid,
        }
    }
}

/// The guard of the ordered event stream.
///
/// The sidecar numbers its events monotonically for the lifetime of one
/// process. The client uses the numbers to notice a lost event.
#[derive(Debug, Default)]
pub struct EventSequence {
    last: u64,
}

/// What the client observed about the sequence of an event.
#[derive(Debug, PartialEq, Eq)]
pub enum SequenceObservation {
    Accept,
    Gap { expected: u64, received: u64 },
    Invalid,
    Stale,
}

impl EventSequence {
    pub fn new() -> Self {
        Self::default()
    }

    /// Observe the sequence number of the next event.
    pub fn observe(&mut self, received: u64) -> SequenceObservation {
        if received == 0 {
            return SequenceObservation::Invalid;
        }
        if received <= self.last {
            return SequenceObservation::Stale;
        }
        let expected = self.last.saturating_add(1);
        let observation = if received > expected {
            SequenceObservation::Gap { expected, received }
        } else {
            SequenceObservation::Accept
        };
        self.last = received;
        observation
    }
}
