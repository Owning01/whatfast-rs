//! The cases of the origin's fake backend, ported 1:1.
//!
//! Source of the cases:
//! - `desktop/src/bridge.rs` (`mod tests`) of the origin repo: the four
//!   cases that exercise the fake simulator. The cases of the framing of
//!   the origin live in `tests/transport.rs`.
//!
//! The protobuf types live at the root of the crate (the origin kept them
//! under `proto::`), so `proto::Envelope` is `whatfast_protocol::Envelope`
//! here and `proto::message::Content` is
//! `whatfast_protocol::message::Content`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::thread;
use std::time::Duration;

use whatfast_protocol::fake::{
    FakeConfig, emit_fake_event, fake_chat, fake_live_event_pair, fake_loop, fake_message,
    fake_send_attachment,
};
use whatfast_protocol::transport::{BridgeMessage, PROTOCOL_VERSION};
use whatfast_protocol::{
    AttachmentKind, ConnectionChanged, CreatePollRequest, DeleteMessageRequest, EditMessageRequest,
    Envelope, ForwardMessageRequest, GetAuthStateRequest, GetChatAvatarRequest, GetChatInfoRequest,
    GetHistoryOverviewRequest, GetIntegrityStatusRequest, GetMessageAttachmentRequest,
    GetMessageImageRequest, GetParticipantAvatarRequest, GetSyncStatusRequest, HelloRequest,
    ListChatsRequest, ListMessagesAfterRequest, ListMessagesAroundRequest, ListMessagesRequest,
    ListNewsletterMessagesRequest, ListNewslettersRequest, ListPinnedMessagesRequest,
    ListStatusesRequest, ListStickersRequest, LogoutRequest, MarkReadRequest, OpenContactRequest,
    OpenMessageWindowRequest, RepairLocalCacheRequest, RepairRecentReactionsRequest,
    RequestOlderHistoryRequest, RpcRequest, SearchLocalRequest, SendAttachmentRequest,
    SendImageRequest, SendReactionRequest, SendStickerFromLibraryRequest, SendStickerRequest,
    SendTextRequest, SetChatArchiveRequest, SetChatBlockRequest, SetChatMuteRequest,
    SetChatPinRequest, SetMessagePinRequest, SetTypingRequest, ShutdownRequest,
    SubscribeNewsletterRequest, UnsubscribeNewsletterRequest, VotePollRequest, backend_event,
    envelope, rpc_request, rpc_response,
};

/// A fake backend running in a thread of the test.
struct Harness {
    outgoing: async_channel::Sender<Envelope>,
    incoming: async_channel::Receiver<BridgeMessage>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Harness {
    /// Start a fake backend with the deterministic configuration of the
    /// tests: no pairing, no performance fixture and no live events.
    fn start(profile: &Path) -> Self {
        let (outgoing, outgoing_rx) = async_channel::bounded(64);
        let (incoming_tx, incoming) = async_channel::bounded(256);
        let profile = profile.to_path_buf();
        let worker = thread::spawn(move || {
            fake_loop(
                outgoing_rx,
                incoming_tx,
                Arc::new(Mutex::new(1)),
                Arc::new(AtomicBool::new(false)),
                FakeConfig {
                    profile_dir: profile,
                    pairing: false,
                    performance_fixture: false,
                    live_events: false,
                },
            );
        });
        Self {
            outgoing,
            incoming,
            worker: Some(worker),
        }
    }

    /// Send one request and wait for its answer, skipping the events.
    fn ask(&self, request: rpc_request::Request) -> rpc_response::Result {
        self.ask_with_id(1, request)
    }

    /// Send one request under a request id and wait for its answer.
    fn ask_with_id(&self, request_id: u64, request: rpc_request::Request) -> rpc_response::Result {
        self.outgoing
            .send_blocking(Envelope {
                protocol_version: PROTOCOL_VERSION,
                request_id,
                body: Some(envelope::Body::Request(RpcRequest {
                    request: Some(request),
                })),
            })
            .expect("the fake backend is listening");
        loop {
            match self
                .incoming
                .recv_blocking()
                .expect("the fake backend answered")
            {
                BridgeMessage::Envelope(envelope) => {
                    if let Some(envelope::Body::Response(response)) = envelope.body {
                        assert_eq!(envelope.request_id, request_id);
                        return response.result.expect("the answer has a result");
                    }
                }
                BridgeMessage::Exited(_) => panic!("the fake backend exited"),
            }
        }
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = self.outgoing.close();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// A private directory of the test run for the media of the fake.
fn profile_dir(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("whatfast-fake-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).expect("create the profile of the test");
    path
}

/// One request of every RPC of the protocol.
fn every_request() -> Vec<rpc_request::Request> {
    vec![
        rpc_request::Request::Hello(HelloRequest::default()),
        rpc_request::Request::GetAuthState(GetAuthStateRequest::default()),
        rpc_request::Request::StartPairing(Default::default()),
        rpc_request::Request::ListChats(ListChatsRequest {
            limit: 20,
            ..Default::default()
        }),
        rpc_request::Request::ListMessages(ListMessagesRequest {
            chat_id: "chat-0".into(),
            limit: 10,
            ..Default::default()
        }),
        rpc_request::Request::SendText(SendTextRequest {
            client_message_id: "client-1".into(),
            chat_id: "chat-0".into(),
            text: "hola".into(),
            ..Default::default()
        }),
        rpc_request::Request::MarkRead(MarkReadRequest {
            chat_id: "chat-0".into(),
            through_message_id: "message-1".into(),
        }),
        rpc_request::Request::Logout(LogoutRequest::default()),
        rpc_request::Request::Shutdown(ShutdownRequest::default()),
        rpc_request::Request::GetChatAvatar(GetChatAvatarRequest {
            chat_id: "chat-0".into(),
        }),
        rpc_request::Request::SendReaction(SendReactionRequest {
            chat_id: "chat-0".into(),
            message_id: "message-1".into(),
            emoji: "🐈".into(),
            client_reaction_id: "reaction-1".into(),
        }),
        rpc_request::Request::GetParticipantAvatar(GetParticipantAvatarRequest {
            participant_id: "15550000000@s.whatsapp.net".into(),
        }),
        rpc_request::Request::RepairRecentReactions(RepairRecentReactionsRequest {
            chat_id: "chat-0".into(),
        }),
        rpc_request::Request::SendImage(SendImageRequest {
            client_message_id: "client-2".into(),
            chat_id: "chat-0".into(),
            image_path: "/tmp/picture.png".into(),
            caption: "photo".into(),
            ..Default::default()
        }),
        rpc_request::Request::GetMessageImage(GetMessageImageRequest {
            chat_id: "chat-0".into(),
            message_id: "message-37".into(),
        }),
        rpc_request::Request::SearchLocal(SearchLocalRequest {
            query: "friend".into(),
        }),
        rpc_request::Request::OpenContact(OpenContactRequest {
            contact_jid: "15550000000@s.whatsapp.net".into(),
        }),
        rpc_request::Request::ListMessagesAround(ListMessagesAroundRequest {
            chat_id: "chat-0".into(),
            message_id: "message-500".into(),
        }),
        rpc_request::Request::SendSticker(SendStickerRequest {
            client_message_id: "client-3".into(),
            chat_id: "chat-0".into(),
            webp_data: vec![1, 2, 3],
            ..Default::default()
        }),
        rpc_request::Request::OpenMessageWindow(OpenMessageWindowRequest {
            chat_id: "chat-0".into(),
        }),
        rpc_request::Request::ListMessagesAfter(ListMessagesAfterRequest {
            chat_id: "chat-0".into(),
            after_timestamp_ms: 10,
            after_message_id: "message-10".into(),
            limit: 5,
        }),
        rpc_request::Request::GetChatInfo(GetChatInfoRequest {
            chat_id: "chat-5".into(),
        }),
        rpc_request::Request::SetTyping(SetTypingRequest {
            chat_id: "chat-0".into(),
            typing: true,
        }),
        rpc_request::Request::GetMessageAttachment(GetMessageAttachmentRequest {
            chat_id: "chat-0".into(),
            message_id: "message-1".into(),
        }),
        rpc_request::Request::SendAttachment(SendAttachmentRequest {
            client_message_id: "client-4".into(),
            chat_id: "chat-0".into(),
            file_path: "/tmp/voice.ogg".into(),
            kind: AttachmentKind::Audio as i32,
            voice_note: true,
            ..Default::default()
        }),
        rpc_request::Request::ListStickers(ListStickersRequest::default()),
        rpc_request::Request::SendStickerFromLibrary(SendStickerFromLibraryRequest {
            client_message_id: "client-5".into(),
            chat_id: "chat-0".into(),
            sticker_id: "fav-1".into(),
            ..Default::default()
        }),
        rpc_request::Request::CreatePoll(CreatePollRequest {
            client_message_id: "client-6".into(),
            chat_id: "chat-0".into(),
            question: "Where?".into(),
            options: vec!["here".into(), "there".into()],
            selectable_options_count: 1,
        }),
        rpc_request::Request::VotePoll(VotePollRequest {
            chat_id: "chat-0".into(),
            poll_message_id: "message-1001".into(),
            selected_options: vec!["here".into()],
        }),
        rpc_request::Request::SetMessagePin(SetMessagePinRequest {
            chat_id: "chat-0".into(),
            message_id: "message-3".into(),
            pinned: true,
        }),
        rpc_request::Request::ListPinnedMessages(ListPinnedMessagesRequest {
            chat_id: "chat-0".into(),
        }),
        rpc_request::Request::ForwardMessage(ForwardMessageRequest {
            client_message_id: "client-7".into(),
            source_chat_id: "chat-0".into(),
            message_id: "message-1".into(),
            target_chat_id: "chat-1".into(),
        }),
        rpc_request::Request::EditMessage(EditMessageRequest {
            chat_id: "chat-0".into(),
            message_id: "message-1".into(),
            text: "edited".into(),
        }),
        rpc_request::Request::GetSyncStatus(GetSyncStatusRequest::default()),
        rpc_request::Request::RequestOlderHistory(RequestOlderHistoryRequest {
            chat_id: "chat-0".into(),
        }),
        rpc_request::Request::GetChatHistoryCoverage(Default::default()),
        rpc_request::Request::GetHistoryOverview(GetHistoryOverviewRequest::default()),
        rpc_request::Request::Reconnect(Default::default()),
        rpc_request::Request::GetIntegrityStatus(GetIntegrityStatusRequest::default()),
        rpc_request::Request::RepairLocalCache(RepairLocalCacheRequest::default()),
        rpc_request::Request::DeleteMessage(DeleteMessageRequest {
            chat_id: "chat-0".into(),
            message_id: "message-1".into(),
            delete_for_everyone: true,
        }),
        rpc_request::Request::ListStatuses(ListStatusesRequest::default()),
        rpc_request::Request::SubscribeNewsletter(SubscribeNewsletterRequest {
            invite_key: "123456789012".into(),
        }),
        rpc_request::Request::ListNewsletters(ListNewslettersRequest::default()),
        rpc_request::Request::ListNewsletterMessages(ListNewsletterMessagesRequest {
            jid: "news@newsletter".into(),
            limit: 8,
        }),
        rpc_request::Request::UnsubscribeNewsletter(UnsubscribeNewsletterRequest {
            jid: "news@newsletter".into(),
        }),
        rpc_request::Request::SetChatPin(SetChatPinRequest {
            chat_id: "chat-0".into(),
            pinned: true,
        }),
        rpc_request::Request::SetChatArchive(SetChatArchiveRequest {
            chat_id: "chat-0".into(),
            archived: true,
        }),
        rpc_request::Request::SetChatMute(SetChatMuteRequest {
            chat_id: "chat-0".into(),
            mute_until_ms: 1_900_000_000_000,
        }),
        rpc_request::Request::SetChatBlock(SetChatBlockRequest {
            chat_id: "chat-0".into(),
            blocked: true,
        }),
    ]
}

#[test]
fn fake_handshake_opens_event_boundary_after_response() {
    let (outgoing, outgoing_rx) = async_channel::bounded(2);
    let (incoming_tx, incoming) = async_channel::bounded(2);
    let handshaken = Arc::new(AtomicBool::new(false));
    let worker_handshaken = handshaken.clone();
    let profile = profile_dir("handshake");
    let worker = thread::spawn(move || {
        fake_loop(
            outgoing_rx,
            incoming_tx,
            Arc::new(Mutex::new(1)),
            worker_handshaken,
            FakeConfig {
                profile_dir: profile,
                pairing: false,
                performance_fixture: false,
                live_events: false,
            },
        );
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
            rpc_request::Request::GetAuthState(GetAuthStateRequest::default()),
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
        fake_send_attachment(request(AttachmentKind::Audio, "", true))
    else {
        panic!("valid voice note did not return send_attachment");
    };
    let message = response.message.unwrap();
    let Some(whatfast_protocol::message::Content::Attachment(attachment)) = message.content else {
        panic!("fake response did not contain attachment content");
    };
    assert_eq!(attachment.kind, "audio");
    assert_eq!(attachment.mime_type, "audio/ogg; codecs=opus");
    assert!(attachment.voice_note);
    assert!(attachment.local_path.is_empty());
    assert_eq!(message.reply_to_message_id, "reply-id");
    assert_eq!(message.reply_to_chat_id, "source-chat-id");

    assert!(matches!(
        fake_send_attachment(request(AttachmentKind::Unspecified, "", false)),
        rpc_response::Result::Error(_)
    ));
    assert!(matches!(
        fake_send_attachment(request(AttachmentKind::Video, "", true)),
        rpc_response::Result::Error(_)
    ));
    assert!(matches!(
        fake_send_attachment(request(AttachmentKind::Audio, "caption", false)),
        rpc_response::Result::Error(_)
    ));
}

#[test]
fn fake_live_pair_matches_backend_event_order() {
    let (message_event, chat_event) = fake_live_event_pair(7, 1234);
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
        assert!(emit_fake_event(
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
fn fake_answers_every_request_of_the_protocol() {
    let profile = profile_dir("every-request");
    let harness = Harness::start(&profile);
    let requests = every_request();
    assert_eq!(requests.len(), 50);
    for (index, request) in requests.iter().enumerate() {
        harness
            .outgoing
            .send_blocking(Envelope {
                protocol_version: PROTOCOL_VERSION,
                request_id: index as u64 + 1,
                body: Some(envelope::Body::Request(RpcRequest {
                    request: Some(request.clone()),
                })),
            })
            .expect("the fake backend is listening");
    }
    let mut answers: HashMap<u64, rpc_response::Result> = HashMap::new();
    while answers.len() < requests.len() {
        match harness
            .incoming
            .recv_blocking()
            .expect("the fake backend answered")
        {
            BridgeMessage::Envelope(envelope) => {
                if let Some(envelope::Body::Response(response)) = envelope.body {
                    answers.insert(
                        envelope.request_id,
                        response.result.expect("the answer has a result"),
                    );
                }
            }
            BridgeMessage::Exited(_) => panic!("the fake backend exited"),
        }
    }
    assert_eq!(answers.len(), 50);
    for (request_id, result) in &answers {
        assert!(
            !matches!(result, rpc_response::Result::Error(_)),
            "the request {request_id} was answered with an error"
        );
    }
}

#[test]
fn fake_dataset_is_deterministic() {
    let chat = fake_chat(0);
    assert_eq!(chat.id, "chat-0");
    // The index zero is a group: the origin decides the kind with a
    // multiple of five, and zero is a multiple of five.
    assert_eq!(chat.kind, whatfast_protocol::ChatKind::Group as i32);
    assert_eq!(chat.title, "Weekend plans 0");
    assert_eq!(
        chat.last_message_preview,
        "This is virtualized conversation number 0"
    );
    // The time of the fake is the real one: the history of the chat goes
    // back from now.
    assert!(chat.last_message_timestamp_ms > 1_700_000_000_000);
    // Zero is a multiple of seven too: the chat zero has one unread.
    assert_eq!(chat.unread_count, 1);
    assert!(chat.pinned);
    assert_eq!(chat.phone_number, "");
    assert_eq!(chat.contact_name, "");
    assert_eq!(chat.push_name, "");

    let direct = fake_chat(1);
    assert_eq!(direct.id, "chat-1");
    assert_eq!(direct.kind, whatfast_protocol::ChatKind::Direct as i32);
    assert_eq!(direct.title, "Fast friend 1");
    assert_eq!(direct.phone_number, "+15550000001");
    assert_eq!(direct.contact_name, "Fast friend 1");
    assert_eq!(direct.push_name, "Friend 1");
    assert!(direct.pinned);

    let group = fake_chat(5);
    assert_eq!(group.kind, whatfast_protocol::ChatKind::Group as i32);
    assert_eq!(group.title, "Weekend plans 5");
    assert_eq!(group.phone_number, "");
    assert_eq!(group.contact_name, "");
    assert_eq!(group.push_name, "");
    assert!(!group.pinned);

    let unread = fake_chat(7);
    assert_eq!(unread.unread_count, 4);
    assert_eq!(unread.title, "Fast friend 7");

    let message = fake_message("chat-0", 1, Path::new("unused"));
    assert_eq!(message.id, "message-1");
    assert_eq!(message.chat_id, "chat-0");
    assert_eq!(message.sender_id, "friend@s.whatsapp.net");
    assert_eq!(message.sender_name, "Fast friend");
    assert_eq!(message.sender_phone_number, "+15551234567");
    assert!(!message.from_me);
    // The time of the fake is the real one: the history goes back from
    // now, a message every minute and a half.
    assert!(message.timestamp_ms > 1_700_000_000_000);
    let newer = fake_message("chat-0", 2, Path::new("unused"));
    assert!(newer.timestamp_ms > message.timestamp_ms);
    assert_eq!(
        message.status,
        whatfast_protocol::MessageStatus::Read as i32
    );
    let Some(whatfast_protocol::message::Content::Text(text)) = &message.content else {
        panic!("the message of the dataset is text");
    };
    assert_eq!(text.text, "Fast native message 1");
    assert!(text.link_preview.is_none());
    assert!(message.reactions.is_empty());
    assert_eq!(message.reply_to_message_id, "");

    let long = fake_message("chat-0", 9, Path::new("unused"));
    let Some(whatfast_protocol::message::Content::Text(long_text)) = &long.content else {
        panic!("the message of the dataset is text");
    };
    assert_eq!(
        long_text.text,
        "A longer message 9\nwith a second line to exercise variable-height virtualization 🐈"
    );

    let preview = fake_message("chat-0", 13, Path::new("unused"));
    let Some(whatfast_protocol::message::Content::Text(preview_text)) = &preview.content else {
        panic!("the message of the dataset is text");
    };
    let preview_link = preview_text
        .link_preview
        .as_ref()
        .expect("the link preview");
    assert_eq!(preview_link.url, "https://example.com/whatfast");
    assert_eq!(preview_link.title, "Whatfast link preview");
    assert_eq!(preview.reply_to_message_id, "message-12");

    let reaction = fake_message("chat-0", 11, Path::new("unused"));
    assert_eq!(reaction.reactions.len(), 1);
    assert_eq!(reaction.reactions[0].emoji, "🐈");
    assert_eq!(reaction.reactions[0].message_id, "message-11");
}

#[test]
fn fake_media_is_written_on_disk() {
    let profile = profile_dir("media");
    let picture = whatfast_protocol::fake::ensure_fake_image(&profile, 37);
    assert!(picture.exists());
    assert!(picture.starts_with(profile.join("media")));
    let bytes = std::fs::read(&picture).expect("the picture of the fake");
    assert_eq!(
        &bytes[..8],
        &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]
    );
    assert!(bytes.len() > 1_000);
    // The header of the PNG: 320x240, eight bits per channel, truecolor.
    assert_eq!(&bytes[12..16], b"IHDR");
    assert_eq!(u32::from_be_bytes(bytes[16..20].try_into().unwrap()), 320);
    assert_eq!(u32::from_be_bytes(bytes[20..24].try_into().unwrap()), 240);
    assert_eq!(bytes[24], 8);
    assert_eq!(bytes[25], 2);

    let audio = whatfast_protocol::fake::ensure_fake_audio(&profile);
    assert!(audio.exists());
    let bytes = std::fs::read(&audio).expect("the audio of the fake");
    assert_eq!(&bytes[..4], b"RIFF");
    assert_eq!(&bytes[8..12], b"WAVE");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&picture)
            .expect("the metadata")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        let mode = std::fs::metadata(&audio)
            .expect("the metadata")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}

#[test]
fn fake_pins_and_chat_settings_survive_the_loop() {
    let profile = profile_dir("state");
    let harness = Harness::start(&profile);

    let rpc_response::Result::SetMessagePin(_) =
        harness.ask(rpc_request::Request::SetMessagePin(SetMessagePinRequest {
            chat_id: "chat-0".into(),
            message_id: "message-3".into(),
            pinned: true,
        }))
    else {
        panic!("the pin was not set");
    };
    let rpc_response::Result::ListPinnedMessages(pins) = harness.ask(
        rpc_request::Request::ListPinnedMessages(ListPinnedMessagesRequest {
            chat_id: "chat-0".into(),
        }),
    ) else {
        panic!("the pins were not listed");
    };
    assert_eq!(pins.pins.len(), 1);
    assert_eq!(pins.pins[0].message_id, "message-3");
    assert!(pins.pins[0].available);

    let rpc_response::Result::SetChatArchive(_) = harness.ask(
        rpc_request::Request::SetChatArchive(SetChatArchiveRequest {
            chat_id: "chat-0".into(),
            archived: true,
        }),
    ) else {
        panic!("the archive was not set");
    };
    let rpc_response::Result::ListChats(chats) =
        harness.ask(rpc_request::Request::ListChats(ListChatsRequest {
            limit: 1,
            ..Default::default()
        }))
    else {
        panic!("the chats were not listed");
    };
    assert_eq!(chats.chats.len(), 1);
    assert!(chats.chats[0].archived);
    assert_eq!(chats.total_count, 10_000);
}

#[test]
fn fake_pagination_and_windows_follow_the_dataset() {
    let profile = profile_dir("pagination");
    let harness = Harness::start(&profile);

    let rpc_response::Result::ListChats(chats) =
        harness.ask(rpc_request::Request::ListChats(ListChatsRequest {
            cursor: "10".into(),
            limit: 5,
        }))
    else {
        panic!("the chats were not listed");
    };
    assert_eq!(chats.chats.len(), 5);
    assert_eq!(chats.chats[0].id, "chat-10");
    assert_eq!(chats.next_cursor, "15");

    let rpc_response::Result::ListMessages(messages) =
        harness.ask(rpc_request::Request::ListMessages(ListMessagesRequest {
            chat_id: "chat-0".into(),
            limit: 3,
            ..Default::default()
        }))
    else {
        panic!("the messages were not listed");
    };
    assert_eq!(messages.messages.len(), 3);
    assert_eq!(messages.messages[0].id, "message-997");
    assert_eq!(messages.messages[2].id, "message-999");
    assert!(messages.has_more);

    let rpc_response::Result::OpenMessageWindow(window) = harness.ask(
        rpc_request::Request::OpenMessageWindow(OpenMessageWindowRequest {
            chat_id: "chat-0".into(),
        }),
    ) else {
        panic!("the window was not opened");
    };
    assert_eq!(window.messages.len(), 51);
    assert_eq!(window.first_unread_message_id, "message-950");
    assert!(window.has_older);
    assert!(window.has_newer);
}

#[test]
fn fake_search_and_newsletter_are_deterministic() {
    let profile = profile_dir("search");
    let harness = Harness::start(&profile);

    let rpc_response::Result::SearchLocal(search) =
        harness.ask(rpc_request::Request::SearchLocal(SearchLocalRequest {
            query: "fast friend 1".into(),
        }))
    else {
        panic!("the search failed");
    };
    assert!(!search.contacts.is_empty());
    assert_eq!(search.contacts[0].chat_id, "chat-1");
    assert_eq!(search.contacts[0].contact_jid, "15550000001@s.whatsapp.net");
    // A query of three characters or more also brings the messages of the
    // last page of the dataset.
    assert_eq!(search.messages.len(), 20);

    let rpc_response::Result::SearchLocal(search) =
        harness.ask(rpc_request::Request::SearchLocal(SearchLocalRequest {
            query: "fast native".into(),
        }))
    else {
        panic!("the search failed");
    };
    assert_eq!(search.messages.len(), 20);
    assert_eq!(search.messages[0].message_id, "message-975");

    let rpc_response::Result::SubscribeNewsletter(newsletter) = harness.ask(
        rpc_request::Request::SubscribeNewsletter(SubscribeNewsletterRequest {
            invite_key: "123456789012345678901234567890".into(),
        }),
    ) else {
        panic!("the newsletter was not subscribed");
    };
    let newsletter = newsletter.newsletter.expect("the newsletter of the answer");
    assert_eq!(newsletter.jid, "123456789012@newsletter");
    assert_eq!(newsletter.name, "Channel 123456789012345678901234");
}

#[test]
fn fake_handshake_answers_the_hello_of_the_protocol() {
    let profile = profile_dir("hello");
    let harness = Harness::start(&profile);
    let rpc_response::Result::Hello(hello) =
        harness.ask(rpc_request::Request::Hello(HelloRequest::default()))
    else {
        panic!("the handshake was not answered");
    };
    assert_eq!(hello.backend_version, "fake-0.1");
    assert_eq!(hello.protocol_version, PROTOCOL_VERSION);
    assert_eq!(hello.protocol_version, 21);
}

#[test]
fn fake_paired_configuration_answers_as_unpaired() {
    let profile = profile_dir("pairing");
    let (outgoing, outgoing_rx) = async_channel::bounded(8);
    let (incoming_tx, incoming) = async_channel::bounded(8);
    let worker = thread::spawn(move || {
        fake_loop(
            outgoing_rx,
            incoming_tx,
            Arc::new(Mutex::new(1)),
            Arc::new(AtomicBool::new(false)),
            FakeConfig {
                profile_dir: profile,
                pairing: true,
                performance_fixture: false,
                live_events: false,
            },
        );
    });
    outgoing
        .send_blocking(Envelope {
            protocol_version: PROTOCOL_VERSION,
            request_id: 1,
            body: Some(envelope::Body::Request(RpcRequest {
                request: Some(rpc_request::Request::GetAuthState(
                    GetAuthStateRequest::default(),
                )),
            })),
        })
        .unwrap();
    let result = loop {
        match incoming.recv_blocking().unwrap() {
            BridgeMessage::Envelope(envelope) => {
                if let Some(envelope::Body::Response(response)) = envelope.body {
                    break response.result.expect("the answer has a result");
                }
            }
            BridgeMessage::Exited(_) => panic!("the fake backend exited"),
        }
    };
    let rpc_response::Result::AuthState(state) = result else {
        panic!("the state of the auth was not answered");
    };
    assert!(!state.paired);
    assert!(!state.logged_in);
    assert_eq!(state.own_user_id, "me@s.whatsapp.net");
    assert_eq!(
        state.connection_state,
        whatfast_protocol::ConnectionState::Pairing as i32
    );
    drop(outgoing);
    worker.join().unwrap();
}

#[test]
fn fake_backend_answers_through_its_public_surface() {
    let profile = profile_dir("backend");
    let backend = whatfast_protocol::fake::FakeBackend::with_config(FakeConfig {
        profile_dir: profile,
        pairing: false,
        performance_fixture: false,
        live_events: false,
    });
    let mut tracker = whatfast_protocol::transport::RequestTracker::new();
    let request_id = backend
        .send_request(
            &mut tracker,
            whatfast_protocol::transport::RequestClass::Control,
            rpc_request::Request::GetAuthState(GetAuthStateRequest::default()),
            Duration::from_secs(0),
        )
        .expect("the request was queued");
    assert_eq!(request_id, 1);
    assert_eq!(tracker.pending_count(), 1);
    // The answer arrives on the same stream the sidecar uses.
    loop {
        match backend
            .incoming
            .recv_blocking()
            .expect("the fake backend answered")
        {
            BridgeMessage::Envelope(envelope) => {
                if let Some(envelope::Body::Response(response)) = envelope.body {
                    assert_eq!(envelope.request_id, request_id);
                    let rpc_response::Result::AuthState(state) =
                        response.result.expect("the answer has a result")
                    else {
                        panic!("the state of the auth was not answered");
                    };
                    assert!(state.paired);
                    assert!(state.logged_in);
                    break;
                }
            }
            BridgeMessage::Exited(_) => panic!("the fake backend exited"),
        }
    }
}

#[test]
fn fake_answers_the_message_of_a_deleted_message() {
    let profile = profile_dir("delete");
    let harness = Harness::start(&profile);
    let rpc_response::Result::DeleteMessage(deleted) =
        harness.ask(rpc_request::Request::DeleteMessage(DeleteMessageRequest {
            chat_id: "chat-0".into(),
            message_id: "message-1".into(),
            delete_for_everyone: true,
        }))
    else {
        panic!("the message was not deleted");
    };
    let message = deleted.message.expect("the message of the answer");
    assert!(message.revoked);
    assert_eq!(message.id, "message-1");
    assert_eq!(message.chat_id, "chat-0");
    assert!(message.from_me);
}
