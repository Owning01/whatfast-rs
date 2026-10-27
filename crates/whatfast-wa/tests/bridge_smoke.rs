//! Spawns the sidecar binary and drives it through the pipe protocol the
//! desktop uses: handshake, one placeholder RPC, and a clean shutdown. The
//! process-level rules the desktop relies on — flags, the profile lock,
//! framing failures and the exit code that separates them — live here too.

use std::{
    io::{Read, Write, stderr},
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
};

use prost::Message as _;
use whatfast_protocol::{
    Envelope, RpcRequest, RpcResponse, ShutdownRequest, ShutdownResponse, envelope, rpc_request,
    rpc_response, transport::PROTOCOL_VERSION,
};

/// The binary under test, the same one the desktop spawns.
const SIDECAR: &str = env!("CARGO_BIN_EXE_whatfast-wa");

/// One launched sidecar with its pipes wired for reading and writing frames.
struct Sidecar {
    child: Child,
}

impl Sidecar {
    fn spawn(data_dir: &Path) -> Self {
        Self::spawn_with(data_dir, &[])
    }

    /// A sidecar serving `data_dir`, plus any extra arguments the test needs.
    fn spawn_with(data_dir: &Path, extra: &[&std::ffi::OsStr]) -> Self {
        let child = Command::new(SIDECAR)
            .arg("--stdio")
            .arg("--data-dir")
            .arg(data_dir)
            .args(extra)
            .stdout(Stdio::piped())
            .stdin(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawning the sidecar");
        Self { child }
    }

    fn send(&mut self, envelope: &Envelope) {
        let payload = envelope.encode_to_vec();
        self.write_header(payload.len() as u32);
        self.write_raw(&payload);
    }

    /// Writes bytes straight to the pipe, framing included: a test that sends
    /// a broken frame cannot go through `send`, which always frames.
    fn write_raw(&mut self, bytes: &[u8]) {
        let stdin = self.child.stdin.as_mut().expect("stdin open");
        stdin.write_all(bytes).expect("writing raw bytes");
        stdin.flush().expect("flushing raw bytes");
    }

    /// Writes a frame header alone, so the payload can arrive later or never.
    fn write_header(&mut self, length: u32) {
        self.write_raw(&length.to_be_bytes());
    }

    fn receive(&mut self) -> Envelope {
        let stdout = self.child.stdout.as_mut().expect("stdout open");
        let mut header = [0u8; 4];
        stdout.read_exact(&mut header).expect("reading the header");
        let mut payload = vec![0u8; u32::from_be_bytes(header) as usize];
        stdout.read_exact(&mut payload).expect("reading the body");
        Envelope::decode(payload.as_slice()).expect("decoding the envelope")
    }

    /// Reads envelopes until a response arrives, letting the backend events
    /// that interleave with it pass by. Requests always get exactly one
    /// response, but events may leave before it.
    fn receive_response(&mut self) -> Envelope {
        loop {
            let envelope = self.receive();
            if matches!(envelope.body, Some(envelope::Body::Response(_))) {
                return envelope;
            }
        }
    }

    /// Waits for the process to end on its own and reports whether it was
    /// a clean exit.
    fn wait(&mut self) -> bool {
        self.child.wait().is_ok_and(|status| status.success())
    }

    /// Completes the Hello handshake; the sidecar is serving afterwards.
    fn handshake(&mut self) {
        self.send(&request_envelope(
            1,
            rpc_request::Request::Hello(whatfast_protocol::HelloRequest {
                desktop_version: "smoke".into(),
                minimum_protocol_version: PROTOCOL_VERSION,
                maximum_protocol_version: PROTOCOL_VERSION,
                ..Default::default()
            }),
        ));
        assert!(matches!(
            response_body(&self.receive()),
            rpc_response::Result::Hello(_)
        ));
    }

    /// Drops stdin, drains the pipes, and reports how the process ended:
    /// `(clean exit, stdout bytes, stderr text)`. The pipes drain before the
    /// wait so a sidecar stuck behind a full pipe cannot deadlock the test.
    fn finish(&mut self) -> (bool, Vec<u8>, String) {
        drop(self.child.stdin.take());
        let mut stdout = Vec::new();
        if let Some(mut pipe) = self.child.stdout.take() {
            let _ = pipe.read_to_end(&mut stdout);
        }
        let mut stderr = String::new();
        if let Some(mut pipe) = self.child.stderr.take() {
            let _ = pipe.read_to_string(&mut stderr);
        }
        (self.wait(), stdout, stderr)
    }
}

impl Drop for Sidecar {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn request_envelope(request_id: u64, request: rpc_request::Request) -> Envelope {
    Envelope {
        protocol_version: PROTOCOL_VERSION,
        request_id,
        body: Some(envelope::Body::Request(RpcRequest {
            request: Some(request),
        })),
    }
}

fn response_body(envelope: &Envelope) -> &rpc_response::Result {
    match &envelope.body {
        Some(envelope::Body::Response(RpcResponse {
            result: Some(result),
        })) => result,
        other => panic!("expected a response, got {other:?}"),
    }
}

fn temp_data_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "whatfast-wa-smoke-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("a positive clock")
            .as_millis()
    ));
    std::fs::create_dir_all(&dir).expect("creating the data dir");
    dir
}

/// Runs a second sidecar against `data_dir` with no stdin at all, the way a
/// second launch from a shell or another desktop would.
fn second_backend(data_dir: &Path) -> Output {
    Command::new(SIDECAR)
        .arg("--stdio")
        .arg("--data-dir")
        .arg(data_dir)
        .output()
        .expect("running the second sidecar")
}

/// Cuts `bytes` into frames, asserting that every byte belongs to one: a
/// stray byte on stdout means the bridge is not the only writer.
fn frames(mut bytes: &[u8]) -> Vec<Envelope> {
    let mut decoded = Vec::new();
    while !bytes.is_empty() {
        assert!(bytes.len() >= 4, "a frame header was cut short");
        let length = u32::from_be_bytes(bytes[..4].try_into().expect("four header bytes")) as usize;
        assert!(length > 0, "a zero-length frame reached stdout");
        assert!(
            bytes.len() >= 4 + length,
            "a frame body was cut short: {length} bytes promised"
        );
        decoded.push(Envelope::decode(&bytes[4..4 + length]).expect("decoding a stdout frame"));
        bytes = &bytes[4 + length..];
    }
    decoded
}

/// Other spellings of the same directory: a second backend launched with one
/// of them must still see the lock, because the lock belongs to the directory
/// and not to the string that named it (on macOS every temp path crosses the
/// `/var` -> `/private/var` symlink, the shape reproduced here).
#[cfg(unix)]
fn equivalent_paths(data_dir: &Path) -> Vec<PathBuf> {
    let link = data_dir.with_extension("link");
    std::os::unix::fs::symlink(data_dir, &link).expect("linking the data dir");
    assert!(
        link.join("backend.lock").exists(),
        "the link must reach the same lock file"
    );
    vec![link]
}

/// The Windows half: the verbatim prefix skips Win32 path normalization, and
/// NTFS resolves the upper-cased spelling to the same file.
#[cfg(windows)]
fn equivalent_paths(data_dir: &Path) -> Vec<PathBuf> {
    let verbatim = PathBuf::from(format!(r"\\?\{}", data_dir.display()));
    assert!(
        verbatim.exists(),
        "the verbatim path must name the directory"
    );
    let mut spellings = vec![verbatim];
    let upper = PathBuf::from(data_dir.to_string_lossy().to_uppercase());
    if upper.exists() {
        spellings.push(upper);
    }
    spellings
}

#[test]
fn handshake_state_and_shutdown_over_the_pipe() {
    let data_dir = temp_data_dir("handshake");
    let mut sidecar = Sidecar::spawn(&data_dir);

    // The handshake must precede everything, and the sidecar must echo the
    // negotiated version.
    sidecar.send(&request_envelope(
        1,
        rpc_request::Request::Hello(whatfast_protocol::HelloRequest {
            desktop_version: "smoke".into(),
            minimum_protocol_version: PROTOCOL_VERSION,
            maximum_protocol_version: PROTOCOL_VERSION,
            ..Default::default()
        }),
    ));
    let hello = sidecar.receive();
    assert_eq!(hello.request_id, 1);
    assert_eq!(hello.protocol_version, PROTOCOL_VERSION);
    match response_body(&hello) {
        rpc_response::Result::Hello(response) => {
            assert_eq!(response.protocol_version, PROTOCOL_VERSION);
            assert!(!response.backend_version.is_empty());
        }
        other => panic!("expected a hello response, got {other:?}"),
    }

    // Before the client exists, the honest auth state is unpaired.
    sidecar.send(&request_envelope(
        2,
        rpc_request::Request::GetAuthState(whatfast_protocol::GetAuthStateRequest {}),
    ));
    match response_body(&sidecar.receive()) {
        rpc_response::Result::AuthState(state) => {
            assert!(!state.paired);
            assert!(!state.logged_in);
        }
        other => panic!("expected an auth state, got {other:?}"),
    }

    // The reads work from the first store: an empty profile is an empty
    // page, not an error.
    sidecar.send(&request_envelope(
        3,
        rpc_request::Request::ListChats(whatfast_protocol::ListChatsRequest {
            cursor: String::new(),
            limit: 10,
        }),
    ));
    match response_body(&sidecar.receive_response()) {
        rpc_response::Result::ListChats(page) => {
            assert!(page.chats.is_empty());
            assert_eq!(page.total_count, 0);
        }
        other => panic!("expected an empty chat page, got {other:?}"),
    }

    // Shutdown answers and the process exits on its own.
    sidecar.send(&request_envelope(
        4,
        rpc_request::Request::Shutdown(ShutdownRequest {}),
    ));
    let shutdown = sidecar.receive();
    assert!(matches!(
        response_body(&shutdown),
        rpc_response::Result::Shutdown(_)
    ));
    assert!(sidecar.wait(), "exit after shutdown");
    std::fs::remove_dir_all(&data_dir).ok();
}

#[test]
fn a_request_before_hello_is_rejected_but_the_process_lives() {
    let data_dir = temp_data_dir("premature");
    let mut sidecar = Sidecar::spawn(&data_dir);

    sidecar.send(&request_envelope(
        7,
        rpc_request::Request::GetAuthState(whatfast_protocol::GetAuthStateRequest {}),
    ));
    match response_body(&sidecar.receive()) {
        rpc_response::Result::Error(error) => assert_eq!(error.code, "handshake_required"),
        other => panic!("expected the handshake error, got {other:?}"),
    }

    // The process must still serve: hello now succeeds.
    sidecar.handshake();
    let _ = stderr().flush();
    std::fs::remove_dir_all(&data_dir).ok();
}

#[test]
fn start_pairing_reports_the_attempt_and_shutdown_still_exits() {
    let data_dir = temp_data_dir("pairing");
    let mut sidecar = Sidecar::spawn(&data_dir);
    sidecar.handshake();

    // An empty store pairs by starting the session: the QR loop begins and
    // its codes arrive later as events. This only asserts the attempt was
    // accepted; the scan itself needs a phone.
    sidecar.send(&request_envelope(
        2,
        rpc_request::Request::StartPairing(whatfast_protocol::StartPairingRequest {}),
    ));
    match response_body(&sidecar.receive_response()) {
        rpc_response::Result::StartPairing(response) => assert!(response.started),
        other => panic!("expected a pairing response, got {other:?}"),
    }

    sidecar.send(&request_envelope(
        3,
        rpc_request::Request::Shutdown(ShutdownRequest {}),
    ));
    assert!(matches!(
        response_body(&sidecar.receive_response()),
        rpc_response::Result::Shutdown(_)
    ));
    assert!(sidecar.wait(), "exit after shutdown with a live session");
    std::fs::remove_dir_all(&data_dir).ok();
}

#[test]
fn a_second_backend_cannot_own_the_same_directory() {
    let data_dir = temp_data_dir("locked");
    let mut holder = Sidecar::spawn(&data_dir);
    // The handshake proves the holder is serving, which proves it owns the
    // lock. Spawning the intruder any earlier is a startup race: whichever
    // process reaches the lock first owns the directory, and the loser exits
    // clean after its stdin EOF (that race is what flaked the macOS runner).
    holder.handshake();
    let intruder_output = second_backend(&data_dir);
    assert!(
        !intruder_output.status.success(),
        "a second backend must refuse to start"
    );
    // The wording is the desktop's fatal-exit needle: `classify_exit` marks
    // "profile is already in use" as never-retry, so a paraphrase here would
    // send the desktop into a restart loop.
    let message = String::from_utf8_lossy(&intruder_output.stderr);
    assert!(
        message.contains("profile is already in use"),
        "the refusal must say why: {message}"
    );
    // The lock excluded the intruder; the holder itself must be alive.
    assert!(
        holder
            .child
            .try_wait()
            .expect("polling the holder")
            .is_none(),
        "the holder must still be serving after the refusal"
    );
    drop(holder);
    std::fs::remove_dir_all(&data_dir).ok();
}

#[test]
fn the_automations_of_a_new_profile_are_empty() {
    let dir = temp_data_dir("schedules");
    let mut sidecar = Sidecar::spawn(&dir);
    sidecar.handshake();
    sidecar.send(&request_envelope(
        2,
        rpc_request::Request::ScheduleList(Default::default()),
    ));
    match response_body(&sidecar.receive_response()) {
        rpc_response::Result::ScheduleList(list) => assert!(list.schedules.is_empty()),
        other => panic!("expected an empty list, got {other:?}"),
    }
    sidecar.send(&request_envelope(
        3,
        rpc_request::Request::ScheduleRemove(whatfast_protocol::ScheduleRemoveRequest { id: 7 }),
    ));
    match response_body(&sidecar.receive_response()) {
        rpc_response::Result::ScheduleRemove(answer) => assert!(!answer.removed),
        other => panic!("expected an answer, got {other:?}"),
    }
    let (clean, _, error) = sidecar.finish();
    assert!(clean, "the sidecar did not exit cleanly: {error}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_question_to_a_harness_without_headless_is_refused() {
    let dir = temp_data_dir("quick-ask");
    let mut sidecar = Sidecar::spawn(&dir);
    sidecar.handshake();
    // Cursor is detected and registered, but it has no print mode: the fast
    // mode of the chat says so instead of hanging on a program that waits for
    // a window.
    sidecar.send(&request_envelope(
        2,
        rpc_request::Request::AgentAsk(whatfast_protocol::AgentAskRequest {
            agent_id: "cursor".into(),
            chat_id: "5491100000000@s.whatsapp.net".into(),
            question: "hola".into(),
            context: Vec::new(),
        }),
    ));
    match response_body(&sidecar.receive_response()) {
        rpc_response::Result::Error(error) => {
            assert_eq!(error.code, "failed_precondition");
            assert!(!error.retryable);
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
    let (clean, _, error) = sidecar.finish();
    assert!(clean, "the sidecar did not exit cleanly: {error}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_second_backend_is_refused_through_an_equivalent_path() {
    let data_dir = temp_data_dir("equivalent");
    let mut holder = Sidecar::spawn(&data_dir);
    holder.handshake();
    let spellings = equivalent_paths(&data_dir);
    for spelling in &spellings {
        // The lock belongs to the directory, so the spelling that named it
        // cannot give a second process a way in.
        let intruder_output = second_backend(spelling);
        assert!(
            !intruder_output.status.success(),
            "a second backend reached {} and started",
            spelling.display()
        );
        let message = String::from_utf8_lossy(&intruder_output.stderr);
        assert!(
            message.contains("profile is already in use"),
            "the refusal through {} must say why: {message}",
            spelling.display()
        );
    }
    drop(holder);
    for spelling in &spellings {
        // The unix branch leaves a symlink next to the data dir; the Windows
        // spellings are the directory itself and fail to unlink, harmlessly.
        let _ = std::fs::remove_file(spelling);
    }
    std::fs::remove_dir_all(&data_dir).ok();
}

#[test]
fn a_protocol_mismatch_after_the_handshake_answers_and_exits_cleanly() {
    let data_dir = temp_data_dir("mismatch");
    let mut sidecar = Sidecar::spawn(&data_dir);
    sidecar.handshake();

    // A handshaken stream that suddenly speaks another version is beyond
    // repair: the sidecar answers `unsupported_protocol` and exits. The exit
    // itself is clean — the desktop asked for a version it cannot have.
    let mut stray = request_envelope(
        2,
        rpc_request::Request::GetAuthState(whatfast_protocol::GetAuthStateRequest {}),
    );
    stray.protocol_version = PROTOCOL_VERSION + 1;
    sidecar.send(&stray);
    match response_body(&sidecar.receive()) {
        rpc_response::Result::Error(error) => {
            assert_eq!(error.code, "unsupported_protocol");
            assert!(!error.retryable);
        }
        other => panic!("expected the protocol error, got {other:?}"),
    }
    assert!(sidecar.wait(), "the exit after a mismatch is clean");
    std::fs::remove_dir_all(&data_dir).ok();
}

#[test]
fn an_unnegotiable_hello_answers_and_exits_cleanly() {
    let data_dir = temp_data_dir("no-overlap");
    let mut sidecar = Sidecar::spawn(&data_dir);

    sidecar.send(&request_envelope(
        1,
        rpc_request::Request::Hello(whatfast_protocol::HelloRequest {
            desktop_version: "smoke".into(),
            minimum_protocol_version: PROTOCOL_VERSION + 1,
            maximum_protocol_version: PROTOCOL_VERSION + 1,
            ..Default::default()
        }),
    ));
    match response_body(&sidecar.receive()) {
        rpc_response::Result::Error(error) => {
            assert_eq!(error.code, "unsupported_protocol");
            assert!(error.message.contains("is not supported by desktop"));
        }
        other => panic!("expected the protocol error, got {other:?}"),
    }
    assert!(sidecar.wait(), "the exit after a failed hello is clean");
    std::fs::remove_dir_all(&data_dir).ok();
}

#[test]
fn oversized_and_empty_frames_kill_the_process() {
    // One spawn per bad frame: the first one ends the process.
    for (label, header) in [("giant", (transport_max_frame() + 1) as u32), ("empty", 0)] {
        let data_dir = temp_data_dir(label);
        let mut sidecar = Sidecar::spawn(&data_dir);
        // Only the header is written: the process must refuse the length
        // without waiting for a payload it will never read.
        sidecar.write_header(header);
        let (clean, stdout, stderr) = sidecar.finish();
        assert!(!clean, "{label}: a broken frame must fail the process");
        assert!(stdout.is_empty(), "{label}: a broken frame answers nothing");
        assert!(
            stderr.contains("bridge read failed"),
            "{label}: the failure must be logged: {stderr}"
        );
        std::fs::remove_dir_all(&data_dir).ok();
    }
}

#[test]
fn a_truncated_frame_fails_instead_of_passing_as_a_clean_close() {
    let data_dir = temp_data_dir("truncated");
    let mut sidecar = Sidecar::spawn(&data_dir);
    // The header promises sixteen bytes and only ten arrive; EOF inside a
    // frame is a truncated frame, not the desktop ending the session.
    sidecar.write_header(16);
    sidecar.write_raw(b"half a fram");
    let (clean, stdout, _stderr) = sidecar.finish();
    assert!(!clean, "a truncated frame must fail the process");
    assert!(stdout.is_empty(), "a truncated frame answers nothing");
    std::fs::remove_dir_all(&data_dir).ok();
}

#[test]
fn a_request_split_across_pipe_writes_is_assembled() {
    let data_dir = temp_data_dir("partial");
    let mut sidecar = Sidecar::spawn(&data_dir);
    // Pipes deliver what they have: a header arriving alone must not be read
    // as a frame, and the payload must be joined to it.
    let payload = request_envelope(
        1,
        rpc_request::Request::Hello(whatfast_protocol::HelloRequest {
            desktop_version: "smoke".into(),
            minimum_protocol_version: PROTOCOL_VERSION,
            maximum_protocol_version: PROTOCOL_VERSION,
            ..Default::default()
        }),
    )
    .encode_to_vec();
    sidecar.write_header(payload.len() as u32);
    std::thread::sleep(std::time::Duration::from_millis(50));
    sidecar.write_raw(&payload);
    assert!(matches!(
        response_body(&sidecar.receive()),
        rpc_response::Result::Hello(_)
    ));
    std::fs::remove_dir_all(&data_dir).ok();
}

#[test]
fn a_frame_that_is_not_a_request_breaks_the_stream() {
    let data_dir = temp_data_dir("not-a-request");
    let mut sidecar = Sidecar::spawn(&data_dir);
    // The backend only ever receives requests; a response frame from the
    // desktop means the stream is out of sync and cannot resynchronize.
    sidecar.send(&Envelope {
        protocol_version: PROTOCOL_VERSION,
        request_id: 1,
        body: Some(envelope::Body::Response(RpcResponse {
            result: Some(rpc_response::Result::Shutdown(ShutdownResponse {})),
        })),
    });
    let (clean, stdout, stderr) = sidecar.finish();
    assert!(!clean, "an out-of-sync frame must fail the process");
    assert!(stdout.is_empty(), "an out-of-sync frame answers nothing");
    assert!(
        stderr.contains("not a request"),
        "the failure must be logged: {stderr}"
    );
    std::fs::remove_dir_all(&data_dir).ok();
}

#[test]
fn closing_stdin_without_shutdown_exits_cleanly() {
    let data_dir = temp_data_dir("eof");
    let mut sidecar = Sidecar::spawn(&data_dir);
    sidecar.handshake();
    // The desktop ends the process between sessions by closing the pipe, and
    // that is a finished session, not a failed one.
    let (clean, stdout, _stderr) = sidecar.finish();
    assert!(clean, "a closed stdin is a clean end");
    assert!(
        stdout.is_empty(),
        "a closed stdin asks for no further answer"
    );
    std::fs::remove_dir_all(&data_dir).ok();
}

#[test]
fn stdout_carries_only_frames() {
    let data_dir = temp_data_dir("stdout");
    let mut sidecar = Sidecar::spawn(&data_dir);
    sidecar.handshake();
    sidecar.send(&request_envelope(
        2,
        rpc_request::Request::Shutdown(ShutdownRequest {}),
    ));
    let (clean, stdout, stderr) = sidecar.finish();
    assert!(clean, "the session must end cleanly: {stderr}");
    // The Hello frame was already read by the handshake; what is left is the
    // shutdown answer, and every remaining byte must belong to a frame.
    let decoded = frames(&stdout);
    assert_eq!(decoded.len(), 1, "the shutdown answer, nothing else");
    assert_eq!(decoded[0].request_id, 2);
    assert!(matches!(
        &decoded[0].body,
        Some(envelope::Body::Response(RpcResponse {
            result: Some(rpc_response::Result::Shutdown(_)),
        }))
    ));
    assert!(
        stderr.is_empty(),
        "a healthy session logs nothing: {stderr}"
    );
    std::fs::remove_dir_all(&data_dir).ok();
}

#[test]
fn logs_reach_stderr_and_the_log_file_while_stdout_stays_clean() {
    let data_dir = temp_data_dir("log-file");
    let log_path = data_dir.join("sidecar.log");
    let mut sidecar = Sidecar::spawn_with(
        &data_dir,
        &[std::ffi::OsStr::new("--log-file"), log_path.as_os_str()],
    );
    // A bad frame always logs, so the destinations can be compared.
    sidecar.write_header(0);
    let (clean, stdout, stderr) = sidecar.finish();
    assert!(!clean, "the bad frame must fail the process");
    assert!(stdout.is_empty(), "logs must never land on stdout");
    assert!(
        stderr.contains("bridge read failed"),
        "stderr keeps explaining a crash even with --log-file: {stderr}"
    );
    let logged = std::fs::read_to_string(&log_path).expect("reading the log file");
    assert!(
        logged.contains("bridge read failed"),
        "the log file must carry the same line: {logged}"
    );
    std::fs::remove_dir_all(&data_dir).ok();
}

#[test]
fn the_required_flags_are_enforced_before_serving() {
    let data_dir = temp_data_dir("flags");

    // The two required flags are the Go sidecar's exit-1 refusals.
    let no_stdio = Command::new(SIDECAR)
        .arg("--data-dir")
        .arg(&data_dir)
        .output()
        .expect("running without --stdio");
    assert_eq!(no_stdio.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&no_stdio.stderr).contains("--stdio is required"));

    let no_data_dir = Command::new(SIDECAR)
        .arg("--stdio")
        .output()
        .expect("running without --data-dir");
    assert_eq!(no_data_dir.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&no_data_dir.stderr).contains("--data-dir is required"));

    // A malformed command line is the flag package's exit 2, and stdout stays
    // empty because the bridge owns it.
    let unknown = Command::new(SIDECAR)
        .args([
            "--stdio",
            "--data-dir",
            data_dir.to_str().expect("utf-8 path"),
            "--bogus",
        ])
        .output()
        .expect("running with an unknown flag");
    assert_eq!(unknown.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&unknown.stderr).contains("unknown argument"));
    assert!(unknown.stdout.is_empty(), "usage errors never use stdout");

    let dangling = Command::new(SIDECAR)
        .args(["--stdio", "--data-dir"])
        .output()
        .expect("running with a dangling flag");
    assert_eq!(dangling.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&dangling.stderr).contains("needs a value"));
}

#[test]
fn the_flags_accept_the_inline_value_form() {
    let data_dir = temp_data_dir("inline");
    let log_path = data_dir.join("sidecar.log");
    // `flag.Parse` read `-name=value` as well as `-name value`; an operator
    // pasting either form must reach a serving sidecar.
    let mut sidecar = Sidecar::spawn_with(
        &data_dir,
        &[
            std::ffi::OsStr::new(&format!("--data-dir={}", data_dir.display())),
            std::ffi::OsStr::new(&format!("--log-file={}", log_path.display())),
        ],
    );
    sidecar.handshake();
    assert!(log_path.exists(), "the inline log path is honoured");
    let (clean, _stdout, stderr) = sidecar.finish();
    assert!(clean, "the session must end cleanly: {stderr}");
    std::fs::remove_dir_all(&data_dir).ok();
}

/// The framing ceiling the sidecar enforces, mirrored from the transport so
/// the oversized-frame test follows the contract instead of a copy of it.
fn transport_max_frame() -> usize {
    whatfast_protocol::transport::MAX_FRAME_BYTES
}
