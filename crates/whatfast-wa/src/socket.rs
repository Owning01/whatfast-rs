//! The local socket transport: the same envelopes as the stdio bridge, for
//! clients that are not the parent process.
//!
//! The desktop spawns this backend and talks to it through its own pipes. An
//! agent harness is started by another program and cannot inherit them, so the
//! backend also listens on a local endpoint derived from the profile (see
//! `whatfast_protocol::local`). Both transports feed the same request loop and
//! the same `Server`: one session, one store, one sequence of events.
//!
//! A connection is request/response only. Events stay on the stdio bridge:
//! the desktop is the only process that renders them, and a second listener
//! would only delay the ones it already got.

use std::io;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_channel::{Sender, unbounded};
use prost::Message as _;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use whatfast_protocol::{
    Envelope, envelope,
    transport::{MAX_FRAME_BYTES, PROTOCOL_VERSION},
};

use crate::guard::Who;

/// How long a connection may stay open without a request before it is closed.
///
/// The agent harnesses keep their server process for the whole session, and an
/// idle connection holds nothing but a task; the guard is here so a client that
/// dies without closing its end cannot pin one forever.
const CONNECTION_IDLE: Duration = Duration::from_secs(30 * 60);

/// What one connection has proven about itself.
///
/// The handshake of the bridge is a flag of the process, not of the
/// connection: the desktop said hello once and every later envelope is taken.
/// A local connection must say hello on its own, so it starts with nothing and
/// the first envelope it sends names it.
#[derive(Clone, Debug, Default)]
pub struct Principal(Arc<Mutex<Option<Who>>>);

impl Principal {
    /// Who is asking, `None` until the connection said hello.
    pub fn get(&self) -> Option<Who> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// Fixes the identity of the connection for its whole life.
    pub fn set(&self, who: Who) {
        let mut slot = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *slot = Some(who);
    }
}

/// Where a request came from, and where its answer has to go back to.
#[derive(Clone, Debug)]
pub enum Origin {
    /// The desktop, over stdin and stdout.
    Stdio,
    /// One local connection: the answer goes back through this queue, and the
    /// principal says who the connection proved to be.
    Socket {
        /// The queue of the answers of this connection.
        replies: Sender<Envelope>,
        /// The identity of the connection, set by its hello.
        principal: Principal,
    },
}

/// Accepts connections on the endpoint until the process ends.
///
/// A failure here never takes the backend down: the desktop still speaks over
/// stdio. The caller logs the error and keeps serving.
pub async fn listen(endpoint: String, incoming: Sender<(Origin, Envelope)>) -> io::Result<()> {
    #[cfg(windows)]
    {
        use tokio::net::windows::named_pipe::ServerOptions;
        loop {
            // One pending instance at a time: the next one is created after
            // this one is taken, so a client never finds the name missing
            // while it is being rebound.
            let server = ServerOptions::new().create(&endpoint)?;
            server.connect().await?;
            let incoming = incoming.clone();
            tokio::spawn(async move {
                if let Err(error) = serve_connection(server, incoming).await {
                    tracing::debug!("local connection ended: {error}");
                }
            });
        }
    }
    #[cfg(unix)]
    {
        use tokio::net::UnixListener;
        let path = std::path::PathBuf::from(&endpoint);
        // A crash leaves the socket file behind; a live backend holds the
        // profile lock, so anything still there is stale.
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let listener = UnixListener::bind(&path)?;
        // The connection carries the chats of the user: owner-only, like the
        // rest of the profile tree.
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        }
        loop {
            let (stream, _) = listener.accept().await?;
            let incoming = incoming.clone();
            tokio::spawn(async move {
                if let Err(error) = serve_connection(stream, incoming).await {
                    tracing::debug!("local connection ended: {error}");
                }
            });
        }
    }
}

/// The protocol version of every local connection.
pub fn protocol_version() -> u32 {
    PROTOCOL_VERSION
}

/// Serves one connection until the client closes it or the bridge is gone.
///
/// One task owns the stream and does one thing at a time: a Windows pipe
/// handle opened synchronously serializes its operations, and a duplex split
/// hides that until a write waits forever behind a pending read. The select
/// below never has two operations of the same stream in flight.
async fn serve_connection<S>(mut stream: S, incoming: Sender<(Origin, Envelope)>) -> io::Result<()>
where
    S: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    let (replies, answers) = unbounded::<Envelope>();
    let principal = Principal::default();
    loop {
        tokio::select! {
            frame = tokio::time::timeout(CONNECTION_IDLE, read_frame(&mut stream)) => match frame {
                // Idle for too long: close the connection, the process keeps
                // serving.
                Err(_) => return Ok(()),
                // The client closed its end.
                Ok(Ok(None)) => return Ok(()),
                // A broken frame cannot be repaired.
                Ok(Err(error)) => return Err(error),
                Ok(Ok(Some(envelope))) => {
                    tracing::debug!(
                        "local request {} (v{})",
                        envelope.request_id,
                        envelope.protocol_version
                    );
                    let origin = Origin::Socket {
                        replies: replies.clone(),
                        principal: principal.clone(),
                    };
                    if incoming.send((origin, envelope)).await.is_err() {
                        return Ok(()); // the bridge is shutting down
                    }
                }
            },
            answer = answers.recv() => match answer {
                Ok(envelope) => {
                    tracing::debug!("local answer {}", envelope.request_id);
                    write_frame(&mut stream, &envelope).await?;
                }
                // The bridge holds a sender for as long as the connection
                // lives, so a closed queue is the bridge going away.
                Err(_) => return Ok(()),
            },
        }
    }
}

/// Reads one length-prefixed envelope, with `Ok(None)` for a clean close.
async fn read_frame<R>(reader: &mut R) -> io::Result<Option<Envelope>>
where
    R: AsyncRead + Unpin,
{
    let mut header = [0u8; 4];
    let mut filled = 0;
    while filled < header.len() {
        let read = reader.read(&mut header[filled..]).await?;
        if read == 0 {
            return match filled {
                0 => Ok(None),
                _ => Err(truncated()),
            };
        }
        filled += read;
    }
    let length = u32::from_be_bytes(header) as usize;
    if length == 0 || length > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invalid frame length {length}"),
        ));
    }
    let mut payload = vec![0u8; length];
    reader.read_exact(&mut payload).await?;
    let envelope = Envelope::decode(payload.as_slice())
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    // The backend only ever receives requests: a response or an event frame
    // means the stream is out of sync and cannot be resynchronized.
    if !matches!(envelope.body, Some(envelope::Body::Request(_))) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "local frame is not a request",
        ));
    }
    Ok(Some(envelope))
}

/// Writes one length-prefixed envelope and flushes it.
async fn write_frame<W>(writer: &mut W, envelope: &Envelope) -> io::Result<()>
where
    W: AsyncWrite + Unpin,
{
    let payload = envelope.encode_to_vec();
    if payload.len() > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "outgoing frame is too big",
        ));
    }
    writer
        .write_all(&(payload.len() as u32).to_be_bytes())
        .await?;
    writer.write_all(&payload).await?;
    writer.flush().await
}

fn truncated() -> io::Error {
    io::Error::new(io::ErrorKind::UnexpectedEof, "truncated frame")
}
