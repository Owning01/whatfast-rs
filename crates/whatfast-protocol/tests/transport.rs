//! The cases of the origin's transport, ported 1:1.
//!
//! Source of the cases:
//! - `tauri/src-tauri/src/bridge.rs` (`mod tests`) of the origin repo.
//! - `desktop/src/rpc.rs` (`mod tests`) of the origin repo: the request
//!   lifecycle (ids, deadlines, correlation) lives there.
//! - `tauri/src-tauri/src/lib.rs` (`mod tests`): the order of the event
//!   stream.
//!
//! The protobuf types live at the root of the crate (the origin kept them
//! under `proto::`), so `proto::Envelope` is `whatfast_protocol::Envelope`
//! here and `proto::message::Content` is
//! `whatfast_protocol::message::Content`.

use std::path::Path;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::thread;
use std::time::Duration;

use whatfast_protocol::transport::{
    ATTACHMENT_REQUEST_TIMEOUT, BackendClient, BridgeExitKind, BridgeMessage,
    CONTROL_REQUEST_TIMEOUT, EventSequence, Incoming, MAX_FRAME_BYTES, MAX_PENDING_REQUESTS,
    PROTOCOL_VERSION, PendingRequest, READ_REQUEST_TIMEOUT, RequestClass, RequestTracker,
    SequenceObservation, WRITE_REQUEST_TIMEOUT, classify_exit, exit_message, read_frame,
    write_frame,
};
use whatfast_protocol::{
    AttachmentContent, AttachmentKind, AuthStateResponse, BackendEvent, Chat, ChatKind,
    ChatUpserted, ConnectionChanged, ConnectionState, Envelope, GetAuthStateRequest, HelloRequest,
    HelloResponse, Message, MessageStatus, MessageUpserted, RpcError, RpcRequest, RpcResponse,
    SendAttachmentRequest, SendAttachmentResponse, TextContent, backend_event, envelope,
    message as proto_message, rpc_request, rpc_response,
};

/// The origin's fake sidecar, reduced to the parts the transport contract
/// depends on: the handshake, the event boundary and the deterministic
/// fixtures the ported cases assert on.
///
/// The origin's simulator answers the fifty RPCs of the protocol with demo
/// data; that dataset is app data, not transport, and it stays with the app.
mod fake {
    use super::*;

    /// Answer the requests of the client until the queue is closed.
    pub fn fake_loop(
        outgoing: async_channel::Receiver<Envelope>,
        incoming: async_channel::Sender<BridgeMessage>,
        handshaken: Arc<AtomicBool>,
    ) {
        while let Ok(envelope) = outgoing.recv_blocking() {
            let request_id = envelope.request_id;
            let Some(envelope::Body::Request(request)) = envelope.body else {
                continue;
            };
            let is_hello = matches!(
                request.request.as_ref(),
                Some(rpc_request::Request::Hello(_))
            );
            let result = match request.request {
                Some(rpc_request::Request::Hello(_)) => {
                    rpc_response::Result::Hello(HelloResponse {
                        backend_version: "fake-0.1".into(),
                        protocol_version: PROTOCOL_VERSION,
                    })
                }
                Some(rpc_request::Request::GetAuthState(_)) => {
                    rpc_response::Result::AuthState(AuthStateResponse {
                        paired: true,
                        logged_in: true,
                        own_user_id: "me@s.whatsapp.net".into(),
                        connection_state: ConnectionState::Connected as i32,
                    })
                }
                _ => fake_invalid_argument("the fake sidecar does not answer this request"),
            };
            if incoming
                .send_blocking(BridgeMessage::Envelope(Box::new(Envelope {
                    protocol_version: PROTOCOL_VERSION,
                    request_id,
                    body: Some(envelope::Body::Response(RpcResponse {
                        result: Some(result),
                    })),
                })))
                .is_err()
            {
                return;
            }
            if is_hello {
                // The real backend only exposes events after the Hello
                // response has been written, so the deterministic backend
                // must keep the same subscription boundary.
                handshaken.store(true, Ordering::Release);
            }
        }
    }

    /// Publish one event with the next sequence number.
    pub fn emit_fake_event(
        incoming: &async_channel::Sender<BridgeMessage>,
        sequence: &Mutex<u64>,
        event: backend_event::Event,
    ) -> bool {
        let mut sequence = sequence.lock().expect("fake event sequence poisoned");
        let envelope = Envelope {
            protocol_version: PROTOCOL_VERSION,
            request_id: 0,
            body: Some(envelope::Body::Event(BackendEvent {
                sequence: *sequence,
                event: Some(event),
            })),
        };
        *sequence = sequence.saturating_add(1);
        incoming
            .send_blocking(BridgeMessage::Envelope(Box::new(envelope)))
            .is_ok()
    }

    /// The error answer of the fake sidecar.
    pub fn fake_invalid_argument(message: impl Into<String>) -> rpc_response::Result {
        rpc_response::Result::Error(RpcError {
            code: "invalid_argument".into(),
            message: message.into(),
            retryable: false,
        })
    }

    /// The attachment answer of the fake sidecar.
    pub fn fake_send_attachment(request: SendAttachmentRequest) -> rpc_response::Result {
        let Ok(kind) = AttachmentKind::try_from(request.kind) else {
            return fake_invalid_argument("unknown attachment kind");
        };
        if kind == AttachmentKind::Unspecified {
            return fake_invalid_argument("attachment kind is required");
        }
        if request.voice_note && kind != AttachmentKind::Audio {
            return fake_invalid_argument("voice_note is only valid for audio attachments");
        }
        if kind == AttachmentKind::Audio && !request.caption.is_empty() {
            return fake_invalid_argument("audio messages do not support captions");
        }
        let (kind_name, mime_type, file_name) = match kind {
            AttachmentKind::Document => (
                "document",
                "application/octet-stream",
                Path::new(&request.file_path)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("attachment")
                    .to_owned(),
            ),
            AttachmentKind::Video => ("video", "video/mp4", String::new()),
            AttachmentKind::Audio => (
                "audio",
                if request.voice_note {
                    "audio/ogg; codecs=opus"
                } else {
                    "audio/ogg"
                },
                String::new(),
            ),
            // The kind was validated above, so this arm never runs.
            AttachmentKind::Unspecified => {
                return fake_invalid_argument("attachment kind is required");
            }
        };
        rpc_response::Result::SendAttachment(SendAttachmentResponse {
            message: Some(Message {
                id: request.client_message_id,
                chat_id: request.chat_id,
                sender_id: "me@s.whatsapp.net".into(),
                sender_name: "You".into(),
                from_me: true,
                timestamp_ms: 1_900_000_000_000,
                status: MessageStatus::Sent as i32,
                content: Some(proto_message::Content::Attachment(AttachmentContent {
                    kind: kind_name.into(),
                    caption: request.caption,
                    mime_type: mime_type.into(),
                    file_name,
                    // Match the real backend's ownership boundary: a selected
                    // source path is never exposed as a renderable cache asset.
                    local_path: String::new(),
                    voice_note: request.voice_note,
                    downloadable: true,
                    ..Default::default()
                })),
                reply_to_message_id: request.reply_to_message_id,
                reply_to_chat_id: request.reply_to_chat_id,
                ..Default::default()
            }),
        })
    }

    /// The pair of events the fake sidecar emits for a live message.
    pub fn fake_live_event_pair(
        counter: u64,
        timestamp_ms: i64,
    ) -> (backend_event::Event, backend_event::Event) {
        let message = Message {
            id: format!("live-{counter}"),
            chat_id: "chat-0".into(),
            sender_id: "friend@s.whatsapp.net".into(),
            sender_name: "Fast friend".into(),
            sender_phone_number: "+15551234567".into(),
            timestamp_ms,
            status: MessageStatus::Delivered as i32,
            content: Some(proto_message::Content::Text(TextContent {
                text: format!("Live message {counter} streamed into the bottom of the chat"),
                link_preview: None,
            })),
            ..Default::default()
        };
        let mut chat = fake_chat(0);
        chat.last_message_preview = message
            .content
            .as_ref()
            .and_then(|content| match content {
                proto_message::Content::Text(text) => Some(text.text.clone()),
                _ => None,
            })
            .unwrap_or_default();
        chat.last_message_timestamp_ms = timestamp_ms;
        chat.unread_count = 1;
        (
            backend_event::Event::MessageUpserted(MessageUpserted {
                message: Some(message),
            }),
            backend_event::Event::ChatUpserted(ChatUpserted { chat: Some(chat) }),
        )
    }

    /// The deterministic chat of the fake sidecar.
    ///
    /// port: the origin used `is_multiple_of`, which needs a newer compiler
    /// than the MSRV of this crate, so the same test uses the remainder.
    pub fn fake_chat(id: usize) -> Chat {
        Chat {
            id: format!("chat-{id}"),
            kind: if id.is_multiple_of(5) {
                ChatKind::Group as i32
            } else {
                ChatKind::Direct as i32
            },
            title: if id.is_multiple_of(5) {
                format!("Weekend plans {id}")
            } else {
                format!("Fast friend {id}")
            },
            last_message_preview: format!("This is virtualized conversation number {id}"),
            last_message_timestamp_ms: 1_900_000_000_000 - id as i64,
            unread_count: (id.is_multiple_of(7)) as u32 * ((id % 4) + 1) as u32,
            pinned: id < 3,
            phone_number: if !id.is_multiple_of(5) {
                format!("+1555{id:07}")
            } else {
                String::new()
            },
            contact_name: if !id.is_multiple_of(5) {
                format!("Fast friend {id}")
            } else {
                String::new()
            },
            push_name: if !id.is_multiple_of(5) {
                format!("Friend {id}")
            } else {
                String::new()
            },
            ..Default::default()
        }
    }
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
fn fake_handshake_opens_event_boundary_after_response() {
    let (outgoing, outgoing_rx) = async_channel::bounded(2);
    let (incoming_tx, incoming) = async_channel::bounded(2);
    let handshaken = Arc::new(AtomicBool::new(false));
    let worker_handshaken = handshaken.clone();
    let worker = thread::spawn(move || {
        fake::fake_loop(outgoing_rx, incoming_tx, worker_handshaken);
    });
    let request = |request_id, request| Envelope {
        protocol_version: PROTOCOL_VERSION,
        request_id,
        body: Some(envelope::Body::Request(RpcRequest {
            request: Some(request),
        })),
    };
    outgoing
        .send_blocking(request(
            1,
            rpc_request::Request::GetAuthState(GetAuthStateRequest {}),
        ))
        .unwrap();
    let _ = incoming.recv_blocking().unwrap();
    assert!(!handshaken.load(Ordering::Acquire));

    outgoing
        .send_blocking(request(
            2,
            rpc_request::Request::Hello(HelloRequest::default()),
        ))
        .unwrap();
    let response = incoming.recv_blocking().unwrap();
    assert!(matches!(response, BridgeMessage::Envelope(_)));
    for _ in 0..100 {
        if handshaken.load(Ordering::Acquire) {
            break;
        }
        thread::sleep(Duration::from_millis(1));
    }
    assert!(handshaken.load(Ordering::Acquire));
    drop(outgoing);
    worker.join().unwrap();
}

#[test]
fn fake_attachment_responses_match_v16_contract() {
    let request = |kind, caption: &str, voice_note| SendAttachmentRequest {
        client_message_id: "request-id".into(),
        chat_id: "chat-id".into(),
        file_path: "/tmp/voice.ogg".into(),
        kind: kind as i32,
        caption: caption.into(),
        reply_to_message_id: "reply-id".into(),
        reply_to_chat_id: "source-chat-id".into(),
        voice_note,
    };
    let rpc_response::Result::SendAttachment(response) =
        fake::fake_send_attachment(request(AttachmentKind::Audio, "", true))
    else {
        panic!("valid voice note did not return send_attachment");
    };
    let message = response.message.unwrap();
    let Some(proto_message::Content::Attachment(attachment)) = message.content else {
        panic!("fake response did not contain attachment content");
    };
    assert_eq!(attachment.kind, "audio");
    assert_eq!(attachment.mime_type, "audio/ogg; codecs=opus");
    assert!(attachment.voice_note);
    assert!(attachment.local_path.is_empty());
    assert_eq!(message.reply_to_message_id, "reply-id");
    assert_eq!(message.reply_to_chat_id, "source-chat-id");

    assert!(matches!(
        fake::fake_send_attachment(request(AttachmentKind::Unspecified, "", false)),
        rpc_response::Result::Error(_)
    ));
    assert!(matches!(
        fake::fake_send_attachment(request(AttachmentKind::Video, "", true)),
        rpc_response::Result::Error(_)
    ));
    assert!(matches!(
        fake::fake_send_attachment(request(AttachmentKind::Audio, "caption", false)),
        rpc_response::Result::Error(_)
    ));
}

#[test]
fn fake_live_pair_matches_backend_event_order() {
    let (message_event, chat_event) = fake::fake_live_event_pair(7, 1234);
    let backend_event::Event::MessageUpserted(message) = message_event else {
        panic!("first event was not the message upsert");
    };
    let backend_event::Event::ChatUpserted(chat) = chat_event else {
        panic!("second event was not the derived chat upsert");
    };
    let message = message.message.unwrap();
    let chat = chat.chat.unwrap();
    assert_eq!(message.timestamp_ms, chat.last_message_timestamp_ms);
    assert_eq!(chat.unread_count, 1);
    assert!(chat.last_message_preview.contains("Live message 7"));
}

#[test]
fn fake_event_emission_is_monotonic() {
    let (incoming, received) = async_channel::bounded(2);
    let sequence = Mutex::new(41);
    for detail in ["first", "second"] {
        assert!(fake::emit_fake_event(
            &incoming,
            &sequence,
            backend_event::Event::ConnectionChanged(ConnectionChanged {
                detail: detail.into(),
                ..Default::default()
            }),
        ));
    }
    let sequences: Vec<_> = (0..2)
        .map(|_| match received.recv_blocking().unwrap() {
            BridgeMessage::Envelope(envelope) => match envelope.body {
                Some(envelope::Body::Event(event)) => event.sequence,
                _ => panic!("fake event used the wrong envelope body"),
            },
            BridgeMessage::Exited(_) => panic!("fake bridge exited"),
        })
        .collect();
    assert_eq!(sequences, [41, 42]);
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
fn event_sequence_accepts_ordered_events_and_reports_gaps() {
    let mut sequence = EventSequence::new();
    assert_eq!(
        sequence.observe(5),
        SequenceObservation::Gap {
            expected: 1,
            received: 5,
        }
    );
    assert_eq!(sequence.observe(5), SequenceObservation::Stale);
    assert_eq!(sequence.observe(0), SequenceObservation::Invalid);
    assert_eq!(
        sequence.observe(7),
        SequenceObservation::Gap {
            expected: 6,
            received: 7,
        }
    );
    assert_eq!(sequence.observe(8), SequenceObservation::Accept);
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
