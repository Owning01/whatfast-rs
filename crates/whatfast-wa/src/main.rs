//! The WhatsApp sidecar binary: argument parsing, the profile lock, the
//! runtime, and nothing else. Everything lives in the `whatfast_wa` library
//! the integration tests also exercise.

use std::{
    io::{BufWriter, Write},
    path::PathBuf,
    process::ExitCode,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, anyhow};
use async_channel::{Receiver, Sender};
use prost::Message as _;
use whatfast_protocol::{
    AgentAskRequest, AgentAskResponse, Envelope, RpcError, RpcResponse, envelope, rpc_response,
};
use whatfast_wa::{
    bridge::{FrameError, read_frame, write_frame},
    db, drafts,
    guard::{self, Guard, Who},
    guides, scheduler, schedules,
    server::{Outcome, Server},
    socket::{self, Origin},
};

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(Failure::Usage(message)) => {
            eprintln!("whatfast-wa: {message}");
            // The Go sidecar's `flag.Parse` ended a malformed command line
            // with 2; everything the process fails at afterwards exits 1.
            ExitCode::from(2)
        }
        Err(Failure::Fatal(error)) => {
            eprintln!("whatfast-wa: {error:#}");
            ExitCode::FAILURE
        }
    }
}

/// Why `run` produced no exit code, split the way the Go sidecar split it:
/// a command line the flag package rejects (exit 2) and anything the backend
/// then fails at (exit 1).
enum Failure {
    Usage(String),
    Fatal(anyhow::Error),
}

impl From<anyhow::Error> for Failure {
    fn from(error: anyhow::Error) -> Self {
        Self::Fatal(error)
    }
}

/// The command line, parsed like the Go `flag` package read it: `--name
/// value` and `--name=value` both name the same flag.
struct Args {
    stdio: bool,
    /// Serve the local endpoint without the desktop's pipes: the mode the
    /// agent bridge starts when the window is closed.
    serve: bool,
    /// How long the local endpoint may stay unused before the process exits.
    /// Only meaningful with `--serve`: the desktop owns the lifetime of the
    /// backend it spawned itself.
    idle_exit: Option<Duration>,
    data_dir: Option<PathBuf>,
    log_file: Option<PathBuf>,
}

fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Args, Failure> {
    let mut parsed = Args {
        stdio: false,
        serve: false,
        idle_exit: None,
        data_dir: None,
        log_file: None,
    };
    while let Some(argument) = args.next() {
        let (flag, inline) = match argument.split_once('=') {
            Some((flag, value)) => (flag.to_owned(), Some(value.to_owned())),
            None => (argument, None),
        };
        match flag.as_str() {
            "--stdio" => {
                if inline.is_some() {
                    return Err(Failure::Usage("--stdio takes no value".into()));
                }
                parsed.stdio = true;
            }
            "--serve" => {
                if inline.is_some() {
                    return Err(Failure::Usage("--serve takes no value".into()));
                }
                parsed.serve = true;
            }
            "--idle-exit" => {
                let value = flag_value(&mut args, inline, "--idle-exit")?;
                let seconds: u64 = value
                    .parse()
                    .map_err(|_| Failure::Usage("--idle-exit takes seconds".into()))?;
                parsed.idle_exit = Some(Duration::from_secs(seconds));
            }
            "--data-dir" => {
                parsed.data_dir = Some(PathBuf::from(flag_value(&mut args, inline, "--data-dir")?))
            }
            "--log-file" => {
                parsed.log_file = Some(PathBuf::from(flag_value(&mut args, inline, "--log-file")?))
            }
            other => return Err(Failure::Usage(format!("unknown argument {other}"))),
        }
    }
    Ok(parsed)
}

/// A flag's value, inline (`--name=value`) or in the next argument
/// (`--name value`); a flag left without one is a command-line error.
fn flag_value(
    args: &mut impl Iterator<Item = String>,
    inline: Option<String>,
    flag: &str,
) -> Result<String, Failure> {
    match inline {
        Some(value) => Ok(value),
        None => args
            .next()
            .ok_or_else(|| Failure::Usage(format!("{flag} needs a value"))),
    }
}

fn run() -> Result<ExitCode, Failure> {
    let args = parse_args(std::env::args().skip(1))?;
    if !args.stdio && !args.serve {
        // The wording stays: the desktop's error classifier reads the failure
        // of a backend it could not talk to.
        return Err(Failure::Fatal(anyhow!("--stdio is required")));
    }
    // An empty value is the flag package's way of saying "not passed", so the
    // required check sees it as missing, exactly like the Go sidecar did.
    let data_dir = args
        .data_dir
        .filter(|path| !path.as_os_str().is_empty())
        .context("--data-dir is required")?;
    let log_file = args.log_file.filter(|path| !path.as_os_str().is_empty());
    std::fs::create_dir_all(&data_dir)
        .with_context(|| format!("creating the data directory {}", data_dir.display()))?;
    restrict_private(&data_dir, 0o700)
        .with_context(|| format!("restricting the data directory {}", data_dir.display()))?;
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
    let code = runtime.block_on(serve(data_dir, chats_db, args.stdio, args.idle_exit));
    // The lock outlives the runtime teardown on purpose: releasing it before
    // the store flushes would invite a successor onto a hot database.
    drop(profile_lock);
    Ok(code)
}

/// Serves the bridge until the desktop is done: stdin EOF, a `Shutdown`
/// response, an idle local endpoint, or an unrecoverable framing error.
/// Request responses and backend events share one outgoing queue, in arrival
/// order.
///
/// Two transports feed the same loop. The desktop owns the process through
/// stdin and stdout; an agent harness reaches the same `Server` through the
/// local endpoint derived from the profile. One process, one session, one
/// sequence of events.
async fn serve(
    data_dir: PathBuf,
    chats_db: Arc<db::Store>,
    stdio: bool,
    idle_exit: Option<Duration>,
) -> ExitCode {
    // The queues between the blocking pipe threads, the request loop, and
    // the session's event pump. Incoming is bounded the way the client's
    // transport bounds its own queues; outgoing is unbounded because a
    // blocked writer must never stall request handling.
    let (incoming_tx, incoming_rx) = async_channel::bounded::<(Origin, Envelope)>(256);
    let (outgoing_tx, outgoing_rx) = async_channel::unbounded::<Envelope>();
    let (events_tx, events_rx) = async_channel::bounded::<whatfast_protocol::BackendEvent>(256);
    // The end of stdin cannot be told by dropping a sender anymore: the local
    // endpoint holds one of its own, and the process has to exit on EOF.
    let (stdin_closed_tx, stdin_closed_rx) = async_channel::bounded::<()>(1);

    // The local endpoint is served in both modes: the desktop keeps it open
    // for the agents while the window runs, and `--serve` exists only to make
    // it reachable when the window is closed.
    let activity = Arc::new(AtomicU64::new(now_ms()));
    {
        let endpoint = whatfast_protocol::local::endpoint(&data_dir);
        let incoming = incoming_tx.clone();
        tokio::spawn(async move {
            if let Err(error) = socket::listen(endpoint.clone(), incoming).await {
                // The desktop still has its pipes: the process keeps serving.
                tracing::warn!("the local endpoint {endpoint} is not served: {error}");
            }
        });
    }

    // The reader is detached: after a Shutdown response the loop may finish
    // while the desktop still holds stdin open, and the process must not wait
    // for a pipe it is about to close anyway. A read that dies of a broken
    // frame rather than a clean close still fails the process, so the outcome
    // travels through a flag. The thread drops the sender only after the flag
    // is written, so a closed channel always carries the verdict with it.
    let framing_failed = Arc::new(AtomicBool::new(false));
    if stdio {
        let framing_failed = Arc::clone(&framing_failed);
        let incoming = incoming_tx.clone();
        thread::spawn(move || {
            if read_stdin(&incoming).is_err() {
                framing_failed.store(true, Ordering::SeqCst);
            }
            // Every frame the desktop wrote is already in the queue; the loop
            // drains it before leaving, so no response is lost on EOF.
            let _ = stdin_closed_tx.send_blocking(());
        });
    }
    // Without the desktop nobody reads the responses of the stdio transport:
    // the socket answers reach their own connection instead. The receiver of
    // the stdio queue is only taken when there is a writer for it.
    let writer = if stdio {
        Some(thread::spawn(move || write_stdout(outgoing_rx)))
    } else {
        None
    };

    let mut idle_timer = tokio::time::interval(Duration::from_secs(5));
    let idle_after = idle_exit.unwrap_or(Duration::MAX);

    // Events stay gated until the handshake completes, like the Go server's
    // `handshaken` flag: a desktop still negotiating versions has no event
    // listener up yet.
    let mut handshaken = false;
    // The guard reads `agents.db` and decides what a local connection may do.
    // A database that cannot be opened leaves the endpoint serving the user
    // alone instead of closing the desktop. It is shared with the automations,
    // which look for the tasks that came due on the same table.
    let guard = Guard::open(&data_dir, whatfast_auto::offset_now()).map(Arc::new);
    // The automations are looked at every half minute: a cron works in minutes,
    // and a task that came due while the backend was down runs on the first
    // turn after it starts.
    let mut schedule_timer = tokio::time::interval(if stdio {
        Duration::from_secs(30)
    } else {
        // A backend started for an agent has no window and no user behind it:
        // it looks less often so an idle profile stays idle.
        Duration::from_secs(60)
    });
    schedule_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut srv = Server::new(data_dir, Arc::clone(&chats_db), events_tx);
    let stop = loop {
        tokio::select! {
            request = incoming_rx.recv() => match request {
                Ok((origin, envelope)) => {
                    activity.store(now_ms(), Ordering::Relaxed);
                    if let Some(stop) = route(
                        &mut srv,
                        &outgoing_tx,
                        &mut handshaken,
                        guard.as_deref(),
                        &chats_db,
                        origin,
                        envelope,
                    )
                    .await
                    {
                        break stop;
                    }
                }
                Err(_) => break Stop::IncomingClosed,
            },
            // The desk is closed and the profile has an agent: the tasks that
            // came due run on their own, and leave their answer as a draft.
            _ = schedule_timer.tick(), if guard.is_some() => {
                if let Some(guard) = guard.as_ref() {
                    let started = scheduler::dispatch(guard, &chats_db, now_ms() as i64);
                    if started > 0 {
                        tracing::info!("{started} automations started");
                    }
                }
            },
            // The desktop closed stdin: the normal end between sessions. What
            // it wrote before closing is answered first, so a response is
            // never lost to the end of the pipe.
            _ = stdin_closed_rx.recv(), if stdio => {
                while let Ok((origin, envelope)) = incoming_rx.try_recv() {
                    let _ = route(
                        &mut srv,
                        &outgoing_tx,
                        &mut handshaken,
                        guard.as_deref(),
                        &chats_db,
                        origin,
                        envelope,
                    )
                    .await;
                }
                break Stop::IncomingClosed;
            }
            event = events_rx.recv(), if handshaken => match event {
                Ok(mut event) => {
                    // A local-only backend has no window to render them: the
                    // store already has the data, so they are read and
                    // dropped instead of stalling the session's pump.
                    if stdio {
                        event.sequence = srv.next_sequence();
                        if outgoing_tx.send(srv.event_envelope(event)).await.is_err() {
                            tracing::error!("the writer is gone");
                            break Stop::WriterGone;
                        }
                    }
                }
                Err(_) => continue, // the pump ends with the session; keep serving
            },
            _ = idle_timer.tick(), if idle_exit.is_some() => {
                let idle_ms = idle_after.as_millis() as u64;
                if now_ms().saturating_sub(activity.load(Ordering::Relaxed)) >= idle_ms {
                    tracing::info!("the local endpoint stayed idle: exiting");
                    break Stop::Idle;
                }
            },
        }
    };
    srv.shutdown().await;
    // Dropping our sender lets the writer drain its queue, flush, and exit;
    // joining it is also what flushes the last response before the exit.
    drop(outgoing_tx);
    let write_failed = match writer.map(|writer| writer.join()) {
        None => false,
        Some(Ok(Ok(()))) => false,
        Some(Ok(Err(error))) => {
            tracing::error!("bridge write failed: {error}");
            true
        }
        Some(Err(_)) => {
            tracing::error!("the writer thread panicked");
            true
        }
    };
    if write_failed || matches!(stop, Stop::WriterGone) {
        return ExitCode::FAILURE;
    }
    // A broken frame ends the channel exactly like a closed one, but it is not
    // a finished session: the desktop must see a failed process.
    if matches!(stop, Stop::IncomingClosed) && stdio && framing_failed.load(Ordering::SeqCst) {
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

/// Answers one request and sends the answer back where it came from.
///
/// It returns why the process has to stop, or `None` to keep serving. A local
/// connection that goes away is never a reason to stop: the desktop owns the
/// process, and an agent comes and goes.
///
/// The local endpoint is the only door that is not the desktop's own pipe, so
/// it is the one with a guard: the handshake names the agent, `agents.db` says
/// what it may do, and the answer of a list is trimmed to the chats its grant
/// names.
async fn route(
    srv: &mut Server,
    outgoing: &Sender<Envelope>,
    handshaken: &mut bool,
    guard: Option<&Guard>,
    store: &Arc<db::Store>,
    origin: Origin,
    envelope: Envelope,
) -> Option<Stop> {
    let (known, principal) = match &origin {
        Origin::Stdio => (Some(Who::User), None),
        Origin::Socket { principal, .. } => (principal.get(), Some(principal.clone())),
    };
    let request = guard::request_of(&envelope).cloned();
    let is_hello = matches!(
        request,
        Some(whatfast_protocol::rpc_request::Request::Hello(_))
    );
    // The hello of a local connection names the agent, and the answer fixes the
    // identity of the connection for the rest of its life.
    let mut who = known;
    if let (Some(guard), true) = (guard, is_hello)
        && let Some((agent_id, token)) = guard::hello_agent(&envelope)
    {
        let identified = guard.identify(&agent_id, &token);
        if let Some(principal) = &principal {
            principal.set(identified.clone());
        }
        who = Some(identified);
    }
    if let (Some(guard), Some(request)) = (guard, request.as_ref())
        && !is_hello
    {
        // A local connection that has not said hello has no rights: the
        // handshake of the process is not the handshake of the connection.
        let verdict = match &who {
            None => Err(guard::Denied {
                code: "handshake_required",
                message: "Hello must be the first request of a local connection".to_owned(),
            }),
            Some(identity) => guard.check(identity, request, store),
        };
        if let Err(denied) = verdict {
            tracing::info!(
                "local request refused: {} ({})",
                denied.message,
                denied.code
            );
            let response = denied.response();
            send_response(&origin, outgoing, answer(envelope.request_id, response)).await;
            return None;
        }
    }
    let started = std::time::Instant::now();
    // The fast mode of the chat asks a harness and waits for its answer. It is
    // the only request that runs another program, so it is answered here, off
    // the request path, and it never sends anything: the answer goes back to
    // the composer of the user.
    if let Some(whatfast_protocol::rpc_request::Request::AgentAsk(ask)) = request.as_ref()
        && (*handshaken || matches!(origin, Origin::Socket { .. }))
    {
        let response = answer(envelope.request_id, ask_agent(ask).await);
        if let (Some(guard), Some(who), Some(request)) = (guard, who.as_ref(), request.as_ref()) {
            log_call(guard, who, request, &response, started);
        }
        send_response(&origin, outgoing, response).await;
        return None;
    }
    // The drafts of the agents are answered here: their table lives in
    // `agents.db`, next to the session but not inside it.
    if let (Some(guard), Some(request), Some(identity)) = (guard, request.as_ref(), who.as_ref())
        && !is_hello
        && (*handshaken || matches!(origin, Origin::Socket { .. }))
    {
        let outcome =
            match request {
                whatfast_protocol::rpc_request::Request::DraftPropose(request) => Some(
                    drafts::propose(guard, store, identity, request, now_ms() as i64),
                ),
                whatfast_protocol::rpc_request::Request::DraftList(request) => {
                    Some(drafts::list(guard, store, identity, request))
                }
                whatfast_protocol::rpc_request::Request::DraftDecide(request) => {
                    Some(drafts::decide(guard, store, identity, request))
                }
                whatfast_protocol::rpc_request::Request::DraftFinish(request) => {
                    Some(drafts::finish(guard, store, identity, request))
                }
                whatfast_protocol::rpc_request::Request::GuideInstall(request) => {
                    // The guide is the same text for every harness: the path of
                    // the program is not part of it, and this backend is not the
                    // one a harness launches.
                    let exe = std::env::current_exe().unwrap_or_default();
                    Some(guides::install(identity, request, &exe))
                }
                whatfast_protocol::rpc_request::Request::GuideRemove(request) => {
                    Some(guides::remove(identity, request))
                }
                whatfast_protocol::rpc_request::Request::GuideStatus(request) => {
                    Some(guides::status(identity, request))
                }
                whatfast_protocol::rpc_request::Request::ScheduleList(request) => {
                    Some(schedules::list(guard, store, identity, request))
                }
                whatfast_protocol::rpc_request::Request::SchedulePut(request) => Some(
                    schedules::put(guard, store, identity, request, now_ms() as i64),
                ),
                whatfast_protocol::rpc_request::Request::SchedulePause(request) => Some(
                    schedules::pause(guard, store, identity, request, now_ms() as i64),
                ),
                whatfast_protocol::rpc_request::Request::ScheduleRemove(request) => {
                    Some(schedules::remove(guard, identity, request))
                }
                _ => None,
            };
        if let Some(outcome) = outcome {
            let response = match outcome {
                Ok(response) => response,
                Err(denied) => {
                    tracing::info!("draft refused: {} ({})", denied.message, denied.code);
                    denied.response()
                }
            };
            if let Who::Agent { id, .. } = identity {
                let call = whatfast_auto::Call {
                    agent_id: id.clone(),
                    tool: guard::tool_name(request),
                    chat_id: guard::chat_of(request),
                    ok: !matches!(response.result, Some(rpc_response::Result::Error(_))),
                    ms: started.elapsed().as_millis() as i64,
                    bytes: response.encoded_len() as i64,
                    detail: match response.result.as_ref() {
                        Some(rpc_response::Result::Error(error)) => {
                            Some(format!("{}: {}", error.code, error.message))
                        }
                        _ => None,
                    },
                };
                guard.log(identity, &call, now_ms() as i64);
            }
            send_response(&origin, outgoing, answer(envelope.request_id, response)).await;
            return None;
        }
    }
    let outcome = srv.handle(envelope).await;
    match outcome {
        Outcome::Respond(mut response) => {
            let hello = hello_response(&response);
            if let (Some(guard), Some(who)) = (guard, who.as_ref())
                && let Some(result) = response.body.as_mut()
                && let envelope::Body::Response(rpc) = result
            {
                guard.filter(who, rpc);
            }
            if let (Some(guard), Some(who), Some(request)) = (guard, who.as_ref(), request.as_ref())
            {
                log_call(guard, who, request, &response, started);
            }
            let delivered = send_response(&origin, outgoing, response).await;
            if !delivered && matches!(origin, Origin::Stdio) {
                tracing::error!("the writer is gone");
                return Some(Stop::WriterGone);
            }
            // Events become eligible only after the correlated Hello response
            // is on the wire.
            if hello {
                *handshaken = true;
            }
            None
        }
        Outcome::RespondThenExit(response) => {
            // A local client that asks for `Shutdown` owns the process exactly
            // like the desktop does.
            send_response(&origin, outgoing, response).await;
            Some(Stop::Shutdown)
        }
    }
}

/// Asks one harness the question of the fast mode, with the chat in context.
///
/// The harness is another program with its own network and its own login: it
/// gets a deadline and runs on the blocking pool, so the desktop keeps serving
/// its session meanwhile.
async fn ask_agent(request: &AgentAskRequest) -> RpcResponse {
    use whatfast_auto::runner::RunError;

    let prompt =
        whatfast_auto::question_prompt(&request.chat_id, &request.question, &request.context);
    let agent = request.agent_id.trim().to_owned();
    let asked = tokio::task::spawn_blocking(move || {
        whatfast_auto::ask::ask(&agent, &prompt, whatfast_auto::QUESTION_TIMEOUT)
    })
    .await;
    let result = match asked {
        Ok(Ok(answer)) => rpc_response::Result::AgentAsk(AgentAskResponse {
            agent_id: request.agent_id.clone(),
            text: answer.text,
            ms: answer.ms,
        }),
        Ok(Err(error)) => {
            let (code, retryable) = match &error {
                RunError::NoHeadless => ("failed_precondition", false),
                RunError::Timeout(_) => ("deadline_exceeded", true),
                RunError::Start(_) => ("unavailable", true),
                RunError::Failed { .. } | RunError::Empty => ("failed", true),
            };
            rpc_response::Result::Error(RpcError {
                code: code.to_owned(),
                message: error.to_string(),
                retryable,
            })
        }
        Err(error) => rpc_response::Result::Error(RpcError {
            code: "internal".to_owned(),
            message: format!("the question was not handed to the harness: {error}"),
            retryable: true,
        }),
    };
    RpcResponse {
        result: Some(result),
    }
}

/// Puts one response on the wire of the transport that asked, and says whether
/// it arrived.
async fn send_response(origin: &Origin, outgoing: &Sender<Envelope>, response: Envelope) -> bool {
    match origin {
        Origin::Stdio => outgoing.send(response).await.is_ok(),
        Origin::Socket { replies, .. } => replies.send(response).await.is_ok(),
    }
}

/// One response to a refusal, with the id of the request it answers.
fn answer(request_id: u64, response: RpcResponse) -> Envelope {
    Envelope {
        protocol_version: whatfast_protocol::transport::PROTOCOL_VERSION,
        request_id,
        body: Some(envelope::Body::Response(response)),
    }
}

/// Appends the call of an agent to `agent_calls`, with what it touched and how
/// long it took.
fn log_call(
    guard: &Guard,
    who: &Who,
    request: &whatfast_protocol::rpc_request::Request,
    response: &Envelope,
    started: std::time::Instant,
) {
    let Who::Agent { id, .. } = who else {
        return;
    };
    let bytes = match response.body.as_ref() {
        Some(envelope::Body::Response(response)) => response.encoded_len(),
        _ => 0,
    };
    let call = whatfast_auto::Call {
        agent_id: id.clone(),
        tool: guard::tool_name(request),
        chat_id: guard::chat_of(request),
        ok: failure_of(response).is_none(),
        ms: started.elapsed().as_millis() as i64,
        bytes: bytes as i64,
        detail: failure_of(response),
    };
    guard.log(who, &call, now_ms() as i64);
}

/// The message of an error response, when the answer is one.
fn failure_of(response: &Envelope) -> Option<String> {
    match response.body.as_ref()? {
        envelope::Body::Response(response) => match response.result.as_ref()? {
            rpc_response::Result::Error(error) => {
                Some(format!("{}: {}", error.code, error.message))
            }
            _ => None,
        },
        _ => None,
    }
}

/// Why the serve loop stopped; the reason decides the exit code.
enum Stop {
    /// Every producer closed: the desktop ended stdin, or the process was
    /// started without it and the last one went away. The normal end.
    IncomingClosed,
    /// A client asked for `Shutdown`.
    Shutdown,
    /// The desktop closed stdout, so no response can be delivered anymore.
    WriterGone,
    /// Nobody used the local endpoint for the whole `--idle-exit` window.
    Idle,
}

/// Milliseconds since the Unix epoch, the clock the profile uses everywhere.
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or_default()
}

/// Whether this response completes the handshake; the loop opens the event
/// tap only after it leaves the process.
fn hello_response(envelope: &Envelope) -> bool {
    matches!(
        &envelope.body,
        Some(envelope::Body::Response(RpcResponse {
            result: Some(rpc_response::Result::Hello(_)),
        }))
    )
}

/// Reads framed requests off stdin until it closes or a frame breaks, then
/// reports how it ended. A clean close is not an error. The sender is
/// borrowed: the caller decides when the channel closes.
fn read_stdin(sender: &Sender<(Origin, Envelope)>) -> Result<(), FrameError> {
    let stdin = std::io::stdin();
    let mut reader = stdin.lock();
    loop {
        match read_frame(&mut reader) {
            Ok(envelope) => {
                if sender.send_blocking((Origin::Stdio, envelope)).is_err() {
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
/// handles, released when they drop or the process dies.
///
/// Two files carry the lock: this sidecar's `backend.lock`, and the Go
/// sidecar's `.profile.lock`. The second one is what keeps a leftover Go
/// binary from running beside this one on the same profile — the two never
/// share a file name, so each alone would leave the other free.
fn acquire_profile_lock(data_dir: &std::path::Path) -> anyhow::Result<Vec<std::fs::File>> {
    use fs4::fs_std::FileExt as _;
    let mut locks = Vec::new();
    for name in ["backend.lock", ".profile.lock"] {
        let path = data_dir.join(name);
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .with_context(|| format!("opening the profile lock {}", path.display()))?;
        restrict_private(&path, 0o600)
            .with_context(|| format!("restricting the profile lock {}", path.display()))?;
        // `try_lock_exclusive` answers `Ok(false)` when the lock is held, so both
        // the boolean and the error are refusals.
        match file.try_lock_exclusive() {
            Ok(true) => {
                locks.push(file);
                Ok(())
            }
            Ok(false) => Err(anyhow::anyhow!(
                // The wording is load-bearing: the desktop's `classify_exit` marks
                // a refusal fatal by matching "profile is already in use" on
                // stderr, so a paraphrase here would turn a locked profile into
                // an endless restart loop.
                "Whatfast profile is already in use: {}",
                data_dir.display()
            )),
            Err(error) => Err(anyhow::anyhow!("locking {}: {error}", path.display())),
        }?;
    }
    Ok(locks)
}

/// Mirrors the Go sidecar's securefs modes: the profile tree stays
/// owner-only. Windows enforces this through the user-profile ACL instead,
/// exactly where the Go tests skip their mode checks.
#[cfg(unix)]
fn restrict_private(path: &std::path::Path, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
}

/// The Windows half of the mode repair: nothing to do under the profile ACL.
#[cfg(not(unix))]
fn restrict_private(_path: &std::path::Path, _mode: u32) -> std::io::Result<()> {
    Ok(())
}

/// stderr by default, stderr and the log file together when the desktop
/// passes one — the same MultiWriter the Go sidecar used, so a captured
/// stderr still explains a crash even with `--log-file` in play. Stdout is
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
        restrict_private(path, 0o600).context("restricting the log file")?;
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(std::sync::Mutex::new(Tee::new(file)))
            .try_init()
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(std::io::stderr)
            .try_init()
    };
    result.map_err(|error| anyhow::anyhow!("installing the logger: {error}"))
}

/// One log line fanned out to the file and to stderr, best effort on both:
/// a failing destination must never silence the other.
struct Tee {
    file: std::sync::Mutex<std::fs::File>,
}

impl Tee {
    fn new(file: std::fs::File) -> Self {
        Self {
            file: std::sync::Mutex::new(file),
        }
    }
}

impl Write for Tee {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if let Ok(mut file) = self.file.lock() {
            let _ = file.write_all(buf);
        }
        let _ = std::io::stderr().write_all(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if let Ok(mut file) = self.file.lock() {
            let _ = file.flush();
        }
        let _ = std::io::stderr().flush();
        Ok(())
    }
}
