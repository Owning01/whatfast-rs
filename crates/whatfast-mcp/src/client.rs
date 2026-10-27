//! The client of the local bridge.
//!
//! The server speaks the same envelopes as the desktop, over the endpoint the
//! backend derives from the profile. When nothing is listening, it starts a
//! backend of its own (`--serve`) and keeps it alive with an idle timeout, so
//! an agent works while the window is closed.
//!
//! The tools are blocking on purpose — the JSON-RPC loop reads one request and
//! writes one answer — but the connection is not: a Windows pipe handle opened
//! synchronously serializes its operations, so a pending read would block the
//! next write forever (measured: the second request never left the process).
//! One task owns the stream and does one operation at a time, the same shape
//! the backend uses on its side.

use std::io;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use async_channel::{Receiver, Sender, unbounded};
use tokio::io::AsyncWriteExt as _;
use whatfast_protocol::{
    Envelope, HelloRequest, ReconnectRequest, RpcError, RpcRequest, ShutdownRequest, envelope,
    local, rpc_request, rpc_response::Result as RpcResult, transport::PROTOCOL_VERSION,
};

use crate::IDLE_EXIT;
use crate::framing::{read_answer, write_frame};

/// The deadline of the handshake with a backend that is already running.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// How long a backend that this server started has to be listening.
const START_TIMEOUT: Duration = Duration::from_secs(20);

/// How long the session gets to resume before the first tool call. A backend
/// started by this server has no `Reconnect` of its own: without it the
/// session stays offline and no message ever arrives.
const WELCOME_TIMEOUT: Duration = Duration::from_secs(20);

/// How long the caller waits for the endpoint of an open window before it
/// decides the backend is not there and starts one.
const MISSING_GRACE: Duration = Duration::from_millis(1000);

/// How many backends one server process may start.
const MAX_SPAWNS: u8 = 3;

/// Every way a tool call can fail.
#[derive(Debug)]
pub enum Error {
    /// No backend is listening, and the one this server started did not
    /// appear.
    Unavailable(String),
    /// The endpoint is not there: nothing is listening on the profile.
    Missing,
    /// The backend did not answer in time.
    Timeout,
    /// The backend answered with an error envelope.
    Remote { code: String, message: String },
    /// The answer did not make sense for the request that was sent.
    Protocol(String),
    /// The connection broke while a request was in flight.
    Io(String),
}

impl Error {
    /// The code of a remote refusal, or the kind of a local failure.
    pub fn code(&self) -> &str {
        match self {
            Self::Remote { code, .. } => code,
            Self::Unavailable(_) => "unavailable",
            Self::Missing => "missing",
            Self::Timeout => "timeout",
            Self::Protocol(_) => "protocol",
            Self::Io(_) => "io",
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable(message) => write!(formatter, "{message}"),
            Self::Missing => write!(formatter, "no backend is listening on this profile"),
            Self::Timeout => write!(formatter, "the backend did not answer in time"),
            Self::Remote { code, message } => write!(formatter, "{code}: {message}"),
            Self::Protocol(message) => write!(formatter, "unexpected answer: {message}"),
            Self::Io(message) => {
                write!(formatter, "the connection to the backend failed: {message}")
            }
        }
    }
}

/// The client of one profile.
pub struct Client {
    runtime: tokio::runtime::Runtime,
    data_dir: PathBuf,
    backend: PathBuf,
    endpoint: String,
    /// The harness this server speaks for, and whose token it presents.
    agent: String,
    connection: Option<Connection>,
    next_request_id: u64,
    /// How many backends this process started: the cap keeps a backend that
    /// dies on start from being spawned in a loop.
    spawns: u8,
}

/// One live connection: the queues the task of the stream owns.
struct Connection {
    outgoing: Sender<Envelope>,
    answers: Receiver<Envelope>,
}

impl Client {
    /// A client for the profile in `data_dir`, ready to start `backend`.
    pub fn new(
        data_dir: PathBuf,
        backend: PathBuf,
        agent: impl Into<String>,
    ) -> Result<Self, String> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|error| format!("the runtime of the local bridge did not start: {error}"))?;
        Ok(Self {
            runtime,
            endpoint: local::endpoint(&data_dir),
            data_dir,
            backend,
            agent: agent.into(),
            connection: None,
            // 1 is the handshake and 2 the resume of the session; the tools
            // start after both.
            next_request_id: 3,
            spawns: 0,
        })
    }

    /// The token the desktop wrote for this harness, when the connection was
    /// made from the application. Without one the backend decides: the profile
    /// of a fresh install has no agent yet, and the harness is the user.
    fn token(&self) -> String {
        let path = self
            .data_dir
            .join("agents")
            .join(format!("{}.token", self.agent));
        match std::fs::read_to_string(path) {
            Ok(text) => text.trim().to_owned(),
            Err(_) => String::new(),
        }
    }

    /// How this agent writes: at the pace of a person, or the instant the model
    /// finished.
    ///
    /// The answer lives in `agents.db`, the grant the user set for this agent.
    /// An agent without a row —a fresh install, or the desktop itself— keeps the
    /// prudent pace, which is the safe default: a message that waits is never
    /// the problem a message that arrives too fast is.
    pub fn pace(&self) -> whatfast_auto::Pace {
        let grant = whatfast_auto::Db::grant_of(&self.data_dir.join("agents.db"), &self.agent);
        match grant {
            Some(grant) if grant.humanize => whatfast_auto::Pace::prudent(),
            Some(_) => whatfast_auto::Pace::instant(),
            None => whatfast_auto::Pace::prudent(),
        }
    }

    /// Sends one request and waits for its answer.
    ///
    /// A connection that broke while the request was in flight is rebuilt
    /// once: a backend restarted between two tool calls is normal, and the
    /// request ids keep a late answer from being taken for the new one.
    pub fn call(
        &mut self,
        request: rpc_request::Request,
        timeout: Duration,
    ) -> Result<RpcResult, Error> {
        let mut retried = false;
        loop {
            self.ensure()?;
            let request_id = self.next_request_id;
            self.next_request_id += 1;
            let outgoing = Envelope {
                protocol_version: PROTOCOL_VERSION,
                request_id,
                body: Some(envelope::Body::Request(RpcRequest {
                    request: Some(request.clone()),
                })),
            };
            let outcome = match self.connection.as_ref() {
                Some(connection) => self
                    .runtime
                    .block_on(send_and_wait(connection, request_id, outgoing, timeout)),
                None => Err(Error::Io("the connection is gone".into())),
            };
            match outcome {
                Ok(RpcResult::Error(error)) => {
                    return Err(Error::Remote {
                        code: error.code,
                        message: error.message,
                    });
                }
                Ok(result) => return Ok(result),
                Err(Error::Io(_)) if !retried => {
                    retried = true;
                    self.connection = None;
                }
                Err(error) => return Err(error),
            }
        }
    }

    /// Stops the backend this process started, if it started one.
    ///
    /// A backend that was already running belongs to the application: the
    /// desktop keeps it and no agent may close it.
    pub fn shutdown(&mut self) {
        if self.spawns == 0 || self.connection.is_none() {
            return;
        }
        let _ = self.call(
            rpc_request::Request::Shutdown(ShutdownRequest {}),
            Duration::from_secs(5),
        );
        self.connection = None;
    }

    /// Makes sure a connection is up, starting a backend if nothing listens.
    fn ensure(&mut self) -> Result<(), Error> {
        if self.connection.is_some() {
            return Ok(());
        }
        // The normal case: the window is open and already serves the profile.
        // A pipe is rebound between two connections, so a missing endpoint is
        // retried before it is taken for a closed application.
        let grace = Instant::now() + MISSING_GRACE;
        loop {
            match self.try_open() {
                Ok(connection) => {
                    self.connection = Some(connection);
                    return Ok(());
                }
                Err(Error::Missing) if Instant::now() < grace => {
                    thread::sleep(Duration::from_millis(125));
                }
                Err(Error::Missing) => break,
                Err(error) => return Err(error),
            }
        }
        if self.spawns >= MAX_SPAWNS {
            return Err(Error::Unavailable(format!(
                "the backend is not listening on {}, and this server already started {MAX_SPAWNS}",
                self.endpoint
            )));
        }
        self.spawns += 1;
        self.start_backend()?;
        let deadline = Instant::now() + START_TIMEOUT;
        loop {
            match self.try_open() {
                Ok(connection) => {
                    self.connection = Some(connection);
                    return Ok(());
                }
                Err(Error::Missing) if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(150));
                }
                Err(Error::Missing) => {
                    return Err(Error::Unavailable(format!(
                        "the backend did not listen on {} within {} s; see {}",
                        self.endpoint,
                        START_TIMEOUT.as_secs(),
                        self.data_dir.join("backend.log").display()
                    )));
                }
                Err(error) => return Err(error),
            }
        }
    }

    /// Opens the endpoint and completes the handshake.
    fn try_open(&self) -> Result<Connection, Error> {
        let connection = self.runtime.block_on(self.connect())?;
        eprintln!("whatfast mcp: connected to {}", self.endpoint);
        Ok(connection)
    }

    /// Opens the endpoint, starts the task that owns the stream, and says
    /// hello.
    async fn connect(&self) -> Result<Connection, Error> {
        let (outgoing, queue) = unbounded::<Envelope>();
        let (frames, answers) = unbounded::<Envelope>();
        #[cfg(windows)]
        let stream = {
            use tokio::net::windows::named_pipe::ClientOptions;
            ClientOptions::new()
                .open(&self.endpoint)
                .map_err(open_error)?
        };
        #[cfg(unix)]
        let stream = tokio::net::UnixStream::connect(&self.endpoint)
            .await
            .map_err(open_error)?;
        self.runtime.spawn(hold_stream(stream, queue, frames));
        let connection = Connection { outgoing, answers };
        self.handshake(&connection).await?;
        self.welcome(&connection).await?;
        Ok(connection)
    }

    /// Resumes the WhatsApp session of the profile, the way the desktop does
    /// when it opens: a backend that nobody else started is offline until it
    /// is told to connect, and an offline backend receives nothing.
    ///
    /// A session that does not resume is not fatal — the local cache is still
    /// readable — so only a dead connection fails here.
    async fn welcome(&self, connection: &Connection) -> Result<(), Error> {
        let envelope = Envelope {
            protocol_version: PROTOCOL_VERSION,
            request_id: 2,
            body: Some(envelope::Body::Request(RpcRequest {
                request: Some(rpc_request::Request::Reconnect(ReconnectRequest {})),
            })),
        };
        connection
            .outgoing
            .send(envelope)
            .await
            .map_err(|_| Error::Io("the connection closed before the session resumed".into()))?;
        match await_answer(&connection.answers, 2, WELCOME_TIMEOUT).await {
            Ok(_) => Ok(()),
            Err(Error::Io(message)) => Err(Error::Io(message)),
            Err(error) => {
                eprintln!("whatfast mcp: the session did not resume yet: {error}");
                Ok(())
            }
        }
    }

    /// Says hello the way the desktop does: without it the backend answers
    /// `handshake_required` to everything.
    async fn handshake(&self, connection: &Connection) -> Result<(), Error> {
        let envelope = Envelope {
            protocol_version: PROTOCOL_VERSION,
            request_id: 1,
            body: Some(envelope::Body::Request(RpcRequest {
                request: Some(rpc_request::Request::Hello(HelloRequest {
                    desktop_version: format!("whatfast-mcp {}", env!("CARGO_PKG_VERSION")),
                    minimum_protocol_version: PROTOCOL_VERSION,
                    maximum_protocol_version: PROTOCOL_VERSION,
                    agent_id: self.agent.clone(),
                    agent_token: self.token(),
                })),
            })),
        };
        connection
            .outgoing
            .send(envelope)
            .await
            .map_err(|_| Error::Io("the connection closed before the handshake".into()))?;
        match await_answer(&connection.answers, 1, HANDSHAKE_TIMEOUT).await? {
            RpcResult::Hello(_) => Ok(()),
            RpcResult::Error(RpcError { code, message, .. }) => {
                Err(Error::Remote { code, message })
            }
            _ => Err(Error::Protocol(
                "the handshake answered something else".into(),
            )),
        }
    }

    /// Starts the backend of this application, detached, on the local
    /// endpoint only.
    fn start_backend(&self) -> Result<(), Error> {
        let mut command = Command::new(&self.backend);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt as _;
            command.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }
        let child = command
            .arg("--serve")
            .arg("--data-dir")
            .arg(&self.data_dir)
            .arg("--log-file")
            .arg(self.data_dir.join("backend.log"))
            .arg("--idle-exit")
            .arg(IDLE_EXIT.as_secs().to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        match child {
            // The handle is dropped on purpose: the backend outlives this
            // server, so the next agent session finds the profile already
            // linked. It ends by itself after `--idle-exit`, or when this
            // server closes.
            Ok(child) => {
                eprintln!(
                    "whatfast mcp: started the backend {} (pid {})",
                    self.backend.display(),
                    child.id()
                );
                Ok(())
            }
            Err(error) => Err(Error::Unavailable(format!(
                "the backend {} could not be started: {error}",
                self.backend.display()
            ))),
        }
    }
}

/// Owns one stream: takes the queued requests out, writes them, and forwards
/// the answers back. One operation at a time, on purpose (see the module).
async fn hold_stream<S>(mut stream: S, queue: Receiver<Envelope>, answers: Sender<Envelope>)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    loop {
        tokio::select! {
            outgoing = queue.recv() => match outgoing {
                Ok(envelope) => {
                    if write_frame(&mut stream, &envelope).await.is_err() {
                        break;
                    }
                }
                // Nobody holds the connection anymore: the server is done.
                Err(_) => break,
            },
            answer = read_answer(&mut stream) => match answer {
                Ok(Some(envelope)) => {
                    if answers.send(envelope).await.is_err() {
                        break;
                    }
                }
                // The backend closed the connection, or the frame broke.
                _ => break,
            },
        }
    }
    let _ = stream.shutdown().await;
}

/// Sends one envelope and waits for the answer with its id.
async fn send_and_wait(
    connection: &Connection,
    request_id: u64,
    envelope: Envelope,
    timeout: Duration,
) -> Result<RpcResult, Error> {
    connection
        .outgoing
        .send(envelope)
        .await
        .map_err(|_| Error::Io("the connection is gone".into()))?;
    await_answer(&connection.answers, request_id, timeout).await
}

/// Waits for the answer of one request, ignoring whatever else arrives.
async fn await_answer(
    answers: &Receiver<Envelope>,
    request_id: u64,
    timeout: Duration,
) -> Result<RpcResult, Error> {
    let deadline = Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            eprintln!("whatfast mcp: request {request_id} was not answered in time");
            return Err(Error::Timeout);
        }
        let envelope = match tokio::time::timeout(remaining, answers.recv()).await {
            Err(_) => {
                eprintln!("whatfast mcp: request {request_id} was not answered in time");
                return Err(Error::Timeout);
            }
            Ok(Err(_)) => return Err(Error::Io("the backend closed the connection".into())),
            Ok(Ok(envelope)) => envelope,
        };
        // An answer to another request is one that arrived late: the
        // correlation is by id, never by order.
        if envelope.request_id != request_id {
            continue;
        }
        let Some(envelope::Body::Response(response)) = envelope.body else {
            continue;
        };
        let Some(result) = response.result else {
            return Err(Error::Protocol("the answer carried no result".into()));
        };
        return Ok(result);
    }
}

/// Turns the failure of opening the endpoint into what the caller decides
/// with: whether the backend is not there or refuses this process.
fn open_error(error: io::Error) -> Error {
    #[cfg(windows)]
    let missing = matches!(error.raw_os_error(), Some(2) | Some(231));
    #[cfg(not(windows))]
    let missing = matches!(
        error.kind(),
        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
    );
    if missing {
        Error::Missing
    } else {
        Error::Unavailable(format!("opening the local endpoint: {error}"))
    }
}
