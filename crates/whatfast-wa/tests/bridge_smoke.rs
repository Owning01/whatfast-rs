//! Spawns the sidecar binary and drives it through the pipe protocol the
//! desktop uses: handshake, one placeholder RPC, and a clean shutdown.

use std::{
    io::{Read, Write, stderr},
    path::PathBuf,
    process::{Child, Command, Stdio},
};

use prost::Message as _;
use whatfast_protocol::{
    Envelope, RpcRequest, RpcResponse, ShutdownRequest, envelope, rpc_request, rpc_response,
    transport::PROTOCOL_VERSION,
};

/// One launched sidecar with its pipes wired for reading and writing frames.
struct Sidecar {
    child: Child,
}

impl Sidecar {
    fn spawn(data_dir: &std::path::Path) -> Self {
        let child = Command::new(env!("CARGO_BIN_EXE_whatfast-wa"))
            .arg("--stdio")
            .arg("--data-dir")
            .arg(data_dir)
            .stdout(Stdio::piped())
            .stdin(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("spawning the sidecar");
        Self { child }
    }

    fn send(&mut self, envelope: &Envelope) {
        let payload = envelope.encode_to_vec();
        let stdin = self.child.stdin.as_mut().expect("stdin open");
        stdin
            .write_all(&(payload.len() as u32).to_be_bytes())
            .expect("writing the frame header");
        stdin.write_all(&payload).expect("writing the frame body");
        stdin.flush().expect("flushing the frame");
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
    sidecar.send(&request_envelope(
        8,
        rpc_request::Request::Hello(whatfast_protocol::HelloRequest {
            desktop_version: "smoke".into(),
            minimum_protocol_version: PROTOCOL_VERSION,
            maximum_protocol_version: PROTOCOL_VERSION,
        }),
    ));
    assert!(matches!(
        response_body(&sidecar.receive()),
        rpc_response::Result::Hello(_)
    ));
    let _ = stderr().flush();
    std::fs::remove_dir_all(&data_dir).ok();
}

#[test]
fn start_pairing_reports_the_attempt_and_shutdown_still_exits() {
    let data_dir = temp_data_dir("pairing");
    let mut sidecar = Sidecar::spawn(&data_dir);

    sidecar.send(&request_envelope(
        1,
        rpc_request::Request::Hello(whatfast_protocol::HelloRequest {
            desktop_version: "smoke".into(),
            minimum_protocol_version: PROTOCOL_VERSION,
            maximum_protocol_version: PROTOCOL_VERSION,
        }),
    ));
    assert!(matches!(
        response_body(&sidecar.receive()),
        rpc_response::Result::Hello(_)
    ));

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
    let holder = Sidecar::spawn(&data_dir);
    let intruder_output = Command::new(env!("CARGO_BIN_EXE_whatfast-wa"))
        .arg("--stdio")
        .arg("--data-dir")
        .arg(&data_dir)
        .output()
        .expect("running the second sidecar");
    assert!(
        !intruder_output.status.success(),
        "a second backend must refuse to start"
    );
    let message = String::from_utf8_lossy(&intruder_output.stderr);
    assert!(
        message.contains("another backend"),
        "the refusal must say why: {message}"
    );
    drop(holder);
    std::fs::remove_dir_all(&data_dir).ok();
}
