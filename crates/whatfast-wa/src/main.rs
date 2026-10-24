//! The WhatsApp sidecar binary: argument parsing, the profile lock, the
//! runtime, and nothing else. Everything lives in the `whatfast_wa` library
//! the integration tests also exercise.

use std::{
    io::{BufWriter, Write},
    path::PathBuf,
    process::ExitCode,
    sync::Arc,
    thread,
};

use anyhow::{Context, bail};
use async_channel::{Receiver, Sender};
use whatfast_protocol::Envelope;
use whatfast_wa::{
    bridge::{FrameError, read_frame, write_frame},
    db,
    server::{Outcome, Server},
};

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("whatfast-wa: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> anyhow::Result<ExitCode> {
    let mut stdio = false;
    let mut data_dir: Option<PathBuf> = None;
    let mut log_file: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--stdio" => stdio = true,
            "--data-dir" => {
                data_dir = Some(PathBuf::from(
                    args.next().context("--data-dir needs a value")?,
                ))
            }
            "--log-file" => {
                log_file = Some(PathBuf::from(
                    args.next().context("--log-file needs a value")?,
                ))
            }
            other => bail!("unknown argument {other}"),
        }
    }
    if !stdio {
        bail!("--stdio is required");
    }
    let data_dir = data_dir.context("--data-dir is required")?;
    std::fs::create_dir_all(&data_dir)
        .with_context(|| format!("creating the data directory {}", data_dir.display()))?;
    // Held for the rest of `run`: the exclusive lock is what keeps a second
    // backend off this directory.
    let profile_lock = acquire_profile_lock(&data_dir)?;
    init_logging(log_file.as_deref())?;

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("building the tokio runtime")?;
    // The chats.db opens for the process, session or no session: the store
    // outlives logouts and keeps the desktop's data alive across relinks.
    let chats_db = db::Store::open(&data_dir.join("chats.db"))
        .map_err(|error| anyhow::anyhow!("opening chats.db: {error}"))
        .map(Arc::new)?;
    let code = runtime.block_on(serve(data_dir, chats_db));
    // The lock outlives the runtime teardown on purpose: releasing it before
    // the store flushes would invite a successor onto a hot database.
    drop(profile_lock);
    Ok(code)
}

/// Serves the bridge until the desktop is done: stdin EOF, a `Shutdown`
/// response, or an unrecoverable framing error. Request responses and
/// backend events share one outgoing queue, in arrival order.
async fn serve(data_dir: PathBuf, chats_db: Arc<db::Store>) -> ExitCode {
    // The queues between the blocking pipe threads, the request loop, and
    // the session's event pump. Incoming is bounded the way the client's
    // transport bounds its own queues; outgoing is unbounded because a
    // blocked writer must never stall request handling.
    let (incoming_tx, incoming_rx) = async_channel::bounded::<Envelope>(256);
    let (outgoing_tx, outgoing_rx) = async_channel::unbounded::<Envelope>();
    let (events_tx, events_rx) = async_channel::bounded::<whatfast_protocol::BackendEvent>(256);

    // The reader is detached: after a Shutdown response the loop may finish
    // while the desktop still holds stdin open, and the process must not wait
    // for a pipe it is about to close anyway.
    thread::spawn(move || read_stdin(incoming_tx));
    let writer = thread::spawn(move || write_stdout(outgoing_rx));

    let mut srv = Server::new(data_dir, chats_db, events_tx);
    let mut broken = false;
    loop {
        tokio::select! {
            request = incoming_rx.recv() => match request {
                Ok(envelope) => match srv.handle(envelope).await {
                    Outcome::Respond(response) => {
                        if outgoing_tx.send(response).await.is_err() {
                            tracing::error!("the writer is gone");
                            broken = true;
                            break;
                        }
                    }
                    Outcome::RespondThenExit(response) => {
                        let _ = outgoing_tx.send(response).await;
                        break;
                    }
                },
                Err(_) => break, // stdin closed
            },
            event = events_rx.recv() => match event {
                Ok(mut event) => {
                    event.sequence = srv.next_sequence();
                    if outgoing_tx.send(srv.event_envelope(event)).await.is_err() {
                        tracing::error!("the writer is gone");
                        broken = true;
                        break;
                    }
                }
                Err(_) => continue, // the pump ends with the session; keep serving
            },
        }
    }
    srv.shutdown().await;
    // Dropping our sender lets the writer drain its queue, flush, and exit.
    drop(outgoing_tx);
    if broken {
        return ExitCode::FAILURE;
    }
    if let Err(error) = writer
        .join()
        .unwrap_or(Err(FrameError::Io(std::io::Error::other(
            "the writer thread panicked",
        ))))
    {
        tracing::error!("bridge write failed: {error}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

/// Reads framed requests off stdin until it closes or a frame breaks, then
/// reports how it ended. A clean close is not an error.
fn read_stdin(sender: Sender<Envelope>) -> Result<(), FrameError> {
    let stdin = std::io::stdin();
    let mut reader = stdin.lock();
    loop {
        match read_frame(&mut reader) {
            Ok(envelope) => {
                if sender.send_blocking(envelope).is_err() {
                    return Ok(()); // the loop is gone; stop quietly
                }
            }
            Err(FrameError::Closed) => return Ok(()),
            Err(error) => {
                tracing::error!("bridge read failed: {error}");
                return Err(error);
            }
        }
    }
}

/// Writes framed envelopes to stdout as they arrive, flushing each one.
fn write_stdout(receiver: Receiver<Envelope>) -> Result<(), FrameError> {
    let stdout = std::io::stdout();
    let mut writer = BufWriter::new(stdout.lock());
    while let Ok(envelope) = receiver.recv_blocking() {
        if let Err(error) = write_frame(&mut writer, &envelope) {
            tracing::error!("bridge write failed: {error}");
            return Err(error);
        }
    }
    writer.flush().map_err(FrameError::Io)
}

/// One backend per data directory: an exclusive lock held by the returned
/// handle, released when it drops or the process dies.
fn acquire_profile_lock(data_dir: &std::path::Path) -> anyhow::Result<std::fs::File> {
    use fs4::fs_std::FileExt as _;
    let path = data_dir.join("backend.lock");
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .with_context(|| format!("opening the profile lock {}", path.display()))?;
    // `try_lock_exclusive` answers `Ok(false)` when the lock is held, so both
    // the boolean and the error are refusals.
    match file.try_lock_exclusive() {
        Ok(true) => Ok(file),
        Ok(false) => Err(anyhow::anyhow!(
            "another backend owns {}",
            data_dir.display()
        )),
        Err(error) => Err(anyhow::anyhow!("locking {}: {error}", path.display())),
    }
}

/// stderr by default, the log file when the desktop passes one. Stdout is
/// never a log destination: the bridge owns it.
fn init_logging(log_file: Option<&std::path::Path>) -> anyhow::Result<()> {
    use tracing_subscriber::EnvFilter;
    let filter =
        EnvFilter::try_from_env("WHATFAST_WA_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    let result = if let Some(path) = log_file {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .context("opening the log file")?;
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(std::sync::Mutex::new(file))
            .try_init()
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(std::io::stderr)
            .try_init()
    };
    result.map_err(|error| anyhow::anyhow!("installing the logger: {error}"))
}
