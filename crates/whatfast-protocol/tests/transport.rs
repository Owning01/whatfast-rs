//! The cases of the origin's transport, ported 1:1.
//!
//! Source of the cases:
//! - `tauri/src-tauri/src/bridge.rs` (`mod tests`) of the origin repo.
//! - `desktop/src/rpc.rs` (`mod tests`) of the origin repo: the request
//!   lifecycle (ids, deadlines, correlation) lives there.
//!
//! The protobuf types live at the root of the crate (the origin kept them
//! under `proto::`), so `proto::Envelope` is `whatfast_protocol::Envelope`
//! here and `proto::message::Content` is
//! `whatfast_protocol::message::Content`.

use std::time::Duration;

use whatfast_protocol::transport::{
    ATTACHMENT_REQUEST_TIMEOUT, BackendClient, BridgeExitKind, BridgeMessage,
    CONTROL_REQUEST_TIMEOUT, Incoming, MAX_FRAME_BYTES, MAX_PENDING_REQUESTS, PROTOCOL_VERSION,
    PendingRequest, READ_REQUEST_TIMEOUT, RequestClass, RequestTracker, WRITE_REQUEST_TIMEOUT,
    classify_exit, exit_message, read_frame, write_frame, writer_loop,
};
use whatfast_protocol::{Envelope, HelloRequest, RpcRequest, RpcResponse, envelope, rpc_request};

/// A writer that fails leaves its queue closed: the next send fails at once
/// instead of filling the buffer until every request expires.
#[test]
fn a_dead_writer_closes_the_outgoing_queue() {
    struct Dead;
    impl std::io::Write for Dead {
        fn write(&mut self, _buffer: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "the sidecar does not read",
            ))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let hello = || Envelope {
        protocol_version: PROTOCOL_VERSION,
        request_id: 1,
        body: Some(envelope::Body::Request(RpcRequest {
            request: Some(rpc_request::Request::Hello(HelloRequest::default())),
        })),
    };
    let (outgoing, outgoing_rx) = async_channel::bounded(2);
    outgoing.try_send(hello()).expect("the queue takes one");
    let error = writer_loop(Dead, outgoing_rx).expect_err("the writer reports the failure");
    assert!(
        matches!(error, whatfast_protocol::transport::TransportError::Io(_)),
        "{error:?}"
    );
    assert!(
        outgoing.try_send(hello()).is_err(),
        "the queue is closed after the writer died"
    );
}

#[test]
fn frames_round_trip() {
    let value = Envelope {
        protocol_version: PROTOCOL_VERSION,
        request_id: 42,
        body: None,
    };
    let mut bytes = Vec::new();
    write_frame(&mut bytes, &value).unwrap();
    let decoded = read_frame(&mut bytes.as_slice()).unwrap();
    assert_eq!(decoded.request_id, 42);
}

#[test]
fn oversized_prefix_is_rejected_before_allocation() {
    let bytes = ((MAX_FRAME_BYTES + 1) as u32).to_be_bytes();
    assert!(read_frame(&mut bytes.as_slice()).is_err());
}

#[test]
fn exit_classification_never_retries_terminal_profile_protocol_or_data_failures() {
    for message in [
        "Whatfast profile is already in use",
        "unsupported_protocol",
        "backend sent protocol version 99",
        "database disk image is malformed",
        "file is not a database",
    ] {
        assert_eq!(classify_exit(message), BridgeExitKind::Fatal, "{message}");
    }
    assert_eq!(
        classify_exit("failed to fill whole buffer; backend signal: 9"),
        BridgeExitKind::Transient
    );
}

#[test]
fn lifecycle_assigns_monotonic_ids_and_completes_once() {
    let mut tracker = RequestTracker::new();
    let first = tracker
        .begin(RequestClass::Control, Duration::ZERO)
        .unwrap();
    let second = tracker.begin(RequestClass::Write, Duration::ZERO).unwrap();

    assert_eq!((first, second), (1, 2));
    assert_eq!(
        tracker.complete(first),
        Some(PendingRequest {
            class: RequestClass::Control,
            expires_at: CONTROL_REQUEST_TIMEOUT,
        })
    );
    assert_eq!(tracker.complete(first), None);
    assert_eq!(tracker.requests().count(), 1);

    tracker.clear();
    assert_eq!(tracker.requests().count(), 0);
    assert_eq!(
        tracker.begin(RequestClass::Read, Duration::ZERO).unwrap(),
        3
    );
}

#[test]
fn request_classes_expire_at_exact_deterministic_deadlines() {
    let mut tracker = RequestTracker::new();
    tracker.begin(RequestClass::Write, Duration::ZERO).unwrap();
    tracker
        .begin(RequestClass::Control, Duration::ZERO)
        .unwrap();

    assert_eq!(
        tracker.expire(CONTROL_REQUEST_TIMEOUT),
        vec![(
            2,
            PendingRequest {
                class: RequestClass::Control,
                expires_at: CONTROL_REQUEST_TIMEOUT,
            }
        )]
    );
    assert_eq!(tracker.requests().count(), 1);
    assert_eq!(
        tracker.expire(WRITE_REQUEST_TIMEOUT),
        vec![(
            1,
            PendingRequest {
                class: RequestClass::Write,
                expires_at: WRITE_REQUEST_TIMEOUT,
            }
        )]
    );
}

#[test]
fn expired_media_releases_capacity_for_following_requests() {
    // port: the origin marked the avatar and image requests as media; the
    // media class of the protocol is the attachment class.
    let mut tracker = RequestTracker::new();
    let start = Duration::from_secs(10);
    for _ in 1..=4 {
        tracker.begin(RequestClass::Attachment, start).unwrap();
    }
    assert_eq!(
        tracker
            .requests()
            .filter(|request| request.class == RequestClass::Attachment)
            .count(),
        4
    );

    assert_eq!(tracker.expire(start + ATTACHMENT_REQUEST_TIMEOUT).len(), 4);
    tracker
        .begin(RequestClass::Attachment, start + ATTACHMENT_REQUEST_TIMEOUT)
        .unwrap();
    assert_eq!(
        tracker
            .requests()
            .filter(|request| request.class == RequestClass::Attachment)
            .count(),
        1
    );
}

#[test]
fn failed_transport_send_rolls_back_pending_entry() {
    let client = BackendClient::disconnected();
    let mut tracker = RequestTracker::new();
    let result = client.send_request(
        &mut tracker,
        RequestClass::Control,
        rpc_request::Request::Hello(HelloRequest::default()),
        Duration::ZERO,
    );
    assert!(result.is_err());
    assert_eq!(tracker.requests().count(), 0);
}

#[test]
fn inbound_policy_correlates_responses_once_and_ignores_late_duplicates() {
    let mut tracker = RequestTracker::new();
    let request_id = tracker
        .begin(RequestClass::Control, Duration::ZERO)
        .unwrap();
    let response = RpcResponse::default();
    let message = || {
        BridgeMessage::Envelope(Box::new(Envelope {
            protocol_version: PROTOCOL_VERSION,
            request_id,
            body: Some(envelope::Body::Response(response.clone())),
        }))
    };

    assert!(matches!(
        tracker.handle(message()),
        Incoming::Response {
            pending: PendingRequest {
                class: RequestClass::Control,
                ..
            },
            ..
        }
    ));
    assert!(matches!(tracker.handle(message()), Incoming::Ignore));
}

#[test]
fn a_dead_sidecar_drops_every_pending_request() {
    let mut tracker = RequestTracker::new();
    tracker.begin(RequestClass::Read, Duration::ZERO).unwrap();
    tracker.begin(RequestClass::Write, Duration::ZERO).unwrap();

    let exit = BridgeMessage::Exited(whatfast_protocol::transport::BridgeExit {
        kind: BridgeExitKind::Transient,
        message: "backend transport failed: broken pipe".into(),
    });
    assert!(matches!(tracker.handle(exit), Incoming::Exited(_)));
    assert_eq!(tracker.requests().count(), 0);
}

#[test]
fn pending_requests_are_capped_at_the_origin_limit() {
    let mut tracker = RequestTracker::new();
    for _ in 0..MAX_PENDING_REQUESTS {
        tracker.begin(RequestClass::Read, Duration::ZERO).unwrap();
    }
    assert!(tracker.begin(RequestClass::Read, Duration::ZERO).is_err());
}

#[test]
fn the_reader_rejects_a_frame_of_a_foreign_protocol_version() {
    let value = Envelope {
        protocol_version: PROTOCOL_VERSION + 1,
        request_id: 1,
        body: Some(envelope::Body::Request(RpcRequest {
            request: Some(rpc_request::Request::Hello(HelloRequest::default())),
        })),
    };
    let mut bytes = Vec::new();
    write_frame(&mut bytes, &value).unwrap();
    let (incoming_tx, _incoming) = async_channel::bounded(1);
    let mut reader = bytes.as_slice();
    assert!(whatfast_protocol::transport::reader_loop(&mut reader, &incoming_tx).is_err());
}

#[test]
fn the_exit_message_carries_the_diagnostics_and_the_status() {
    assert_eq!(
        exit_message("failed to fill whole buffer", "", Some("exit status: 9")),
        "backend transport failed: failed to fill whole buffer; backend exit status: 9"
    );
    assert_eq!(
        exit_message(
            "failed to fill whole buffer",
            "whatfast-backend: boom",
            None
        ),
        "backend transport failed: failed to fill whole buffer; diagnostics: whatfast-backend: boom"
    );
    assert_eq!(
        exit_message("failed to fill whole buffer", "", None),
        "backend transport failed: failed to fill whole buffer"
    );
}

#[test]
fn a_read_request_outlives_a_control_request() {
    assert_eq!(READ_REQUEST_TIMEOUT, Duration::from_secs(30));
    assert!(READ_REQUEST_TIMEOUT > CONTROL_REQUEST_TIMEOUT);
    assert!(WRITE_REQUEST_TIMEOUT > READ_REQUEST_TIMEOUT);
    assert!(ATTACHMENT_REQUEST_TIMEOUT > WRITE_REQUEST_TIMEOUT);
}
