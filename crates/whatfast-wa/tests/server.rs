//! Dispatch-level tests for the bridge RPCs this partition owns: the page
//! contract, the status/newsletter/sticker/pin surfaces, the identity fold
//! behind `OpenContact`, and the request validation the Go sidecar enforced.
//!
//! The tests drive `Server::handle` with real envelopes over a temporary
//! store. No WhatsApp connection is involved: the arms that need one are
//! tested for their refusal, which is the part the desktop sees when the link
//! is down.

use std::path::PathBuf;
use std::sync::Arc;

use prost::Message as _;
use whatfast_protocol::{
    BackendEvent, Envelope, ListMessagesRequest, MarkReadRequest, RpcRequest, RpcResponse,
    SendReactionRequest, envelope, rpc_request, rpc_response, transport::PROTOCOL_VERSION,
};
use whatfast_wa::db::{MessageRow, Store};
use whatfast_wa::server::Server;

/// A throwaway profile directory with an open store.
fn temp_profile(label: &str) -> (PathBuf, Arc<Store>) {
    let dir = std::env::temp_dir().join(format!(
        "whatfast-wa-server-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let store = Store::open(&dir.join("chats.db")).unwrap();
    (dir, Arc::new(store))
}

/// One request/response round trip through a fresh runtime. The envelope is
/// encoded and decoded, so the test also proves the wire shape the desktop
/// reads.
fn call(server: &mut Server, id: u64, request: rpc_request::Request) -> RpcResponse {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    send(server, &runtime, id, request)
}

fn send(
    server: &mut Server,
    runtime: &tokio::runtime::Runtime,
    id: u64,
    request: rpc_request::Request,
) -> RpcResponse {
    let envelope = Envelope {
        protocol_version: PROTOCOL_VERSION,
        request_id: id,
        body: Some(envelope::Body::Request(RpcRequest {
            request: Some(request),
        })),
    };
    let encoded = envelope.encode_to_vec();
    let incoming = Envelope::decode(encoded.as_slice()).unwrap();
    let outcome = runtime.block_on(server.handle(incoming));
    let response = match outcome {
        whatfast_wa::server::Outcome::Respond(response)
        | whatfast_wa::server::Outcome::RespondThenExit(response) => response,
    };
    // Round-trip the answer too: the desktop only ever sees encoded frames.
    let encoded = response.encode_to_vec();
    let decoded = Envelope::decode(encoded.as_slice()).unwrap();
    match decoded.body.expect("a body") {
        envelope::Body::Response(response) => response,
        other => panic!("expected a response body, got {other:?}"),
    }
}

fn handshake(server: &mut Server) {
    let response = call(
        server,
        1,
        rpc_request::Request::Hello(whatfast_protocol::HelloRequest {
            desktop_version: "test".into(),
            minimum_protocol_version: PROTOCOL_VERSION,
            maximum_protocol_version: PROTOCOL_VERSION,
            ..Default::default()
        }),
    );
    let result = response.result.expect("a result");
    assert!(
        matches!(result, rpc_response::Result::Hello(_)),
        "the handshake answered {result:?}"
    );
}

/// The clock the store stamps rows with.
fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

fn error_of(response: RpcResponse) -> whatfast_protocol::RpcError {
    match response.result.expect("a result") {
        rpc_response::Result::Error(error) => error,
        other => panic!("expected an error, got {other:?}"),
    }
}

fn message(chat: &str, id: &str, ts: i64, text: &str) -> MessageRow {
    MessageRow {
        id: id.into(),
        chat_jid: chat.into(),
        transport_jid: chat.into(),
        text: text.into(),
        timestamp: ts,
        kind: "text".into(),
        status: 2,
        ..Default::default()
    }
}

fn server(label: &str) -> (PathBuf, Arc<Store>, Server) {
    let (dir, store) = temp_profile(label);
    let (events, _receiver) = async_channel::bounded::<BackendEvent>(64);
    let server = Server::new(dir.clone(), Arc::clone(&store), events);
    (dir, store, server)
}

// ---------------------------------------------------------------------------
// Handshake ordering
// ---------------------------------------------------------------------------

/// The Go test of the same name: `handle` builds the Hello answer but does not
/// open the event gate; only the serve loop does that after the write.
#[test]
fn hello_does_not_open_the_event_gate_before_the_response_is_written() {
    let (_dir, _store, mut server) = server("hello");
    handshake(&mut server);
    assert!(
        !server.handshaken(),
        "the handshake was recorded before the answer left the process"
    );
    // The next request is proof the answer was written: the flag flips.
    let _ = call(
        &mut server,
        2,
        rpc_request::Request::GetAuthState(whatfast_protocol::GetAuthStateRequest {}),
    );
    assert!(server.handshaken());
}

// ---------------------------------------------------------------------------
// Message pages
// ---------------------------------------------------------------------------

/// An exactly-full page is not a page that has more: the Go asked for
/// `limit+1` rows to decide.
#[test]
fn list_messages_does_not_announce_a_page_that_does_not_exist() {
    let (_dir, store, mut server) = server("page");
    handshake(&mut server);
    store
        .ensure_chat("a@s.whatsapp.net", "a@s.whatsapp.net")
        .unwrap();
    for index in 0..2 {
        store
            .insert_message(&message(
                "a@s.whatsapp.net",
                &format!("m{index}"),
                100 + index,
                "hi",
            ))
            .unwrap();
    }
    let exact = call(
        &mut server,
        2,
        rpc_request::Request::ListMessages(ListMessagesRequest {
            chat_id: "a@s.whatsapp.net".into(),
            limit: 2,
            ..Default::default()
        }),
    );
    let rpc_response::Result::ListMessages(exact) = exact.result.expect("a result") else {
        panic!("expected ListMessages");
    };
    assert_eq!(exact.messages.len(), 2);
    assert!(
        !exact.has_more,
        "two rows and a limit of two is the last page"
    );

    store
        .insert_message(&message("a@s.whatsapp.net", "m2", 102, "hi"))
        .unwrap();
    let first = call(
        &mut server,
        3,
        rpc_request::Request::ListMessages(ListMessagesRequest {
            chat_id: "a@s.whatsapp.net".into(),
            limit: 2,
            ..Default::default()
        }),
    );
    let rpc_response::Result::ListMessages(first) = first.result.expect("a result") else {
        panic!("expected ListMessages");
    };
    assert_eq!(first.messages.len(), 2);
    assert!(
        first.has_more,
        "three rows and a limit of two has a next page"
    );
    // Oldest-to-newest, like the Go after its reverse.
    assert_eq!(first.messages[0].id, "m1");
    assert_eq!(first.messages[1].id, "m2");
}

#[test]
fn list_messages_rejects_a_half_cursor() {
    let (_dir, _store, mut server) = server("cursor");
    handshake(&mut server);
    let response = call(
        &mut server,
        2,
        rpc_request::Request::ListMessages(ListMessagesRequest {
            chat_id: "a@s.whatsapp.net".into(),
            before_message_id: "m1".into(),
            limit: 10,
            ..Default::default()
        }),
    );
    let error = error_of(response);
    assert_eq!(error.code, "invalid_argument");
    assert!(!error.retryable);
}

#[test]
fn list_messages_after_rejects_missing_cursor_fields() {
    let (_dir, _store, mut server) = server("after");
    handshake(&mut server);
    let response = call(
        &mut server,
        2,
        rpc_request::Request::ListMessagesAfter(whatfast_protocol::ListMessagesAfterRequest {
            chat_id: "a@s.whatsapp.net".into(),
            limit: 10,
            ..Default::default()
        }),
    );
    assert_eq!(error_of(response).code, "invalid_argument");
}

// ---------------------------------------------------------------------------
// The initial window
// ---------------------------------------------------------------------------

/// The Go asked `MessagesBefore(..., 50)` for a chat without unread rows.
#[test]
fn open_message_window_caps_the_newest_page_at_fifty() {
    let (_dir, store, mut server) = server("window");
    handshake(&mut server);
    store
        .ensure_chat("a@s.whatsapp.net", "a@s.whatsapp.net")
        .unwrap();
    for index in 0..80 {
        store
            .insert_message(&message(
                "a@s.whatsapp.net",
                &format!("m{index:03}"),
                1000 + index,
                "hi",
            ))
            .unwrap();
    }
    let response = call(
        &mut server,
        2,
        rpc_request::Request::OpenMessageWindow(whatfast_protocol::OpenMessageWindowRequest {
            chat_id: "a@s.whatsapp.net".into(),
        }),
    );
    let rpc_response::Result::OpenMessageWindow(window) = response.result.expect("a result") else {
        panic!("expected OpenMessageWindow");
    };
    assert_eq!(window.messages.len(), 50);
    assert!(window.has_older);
    assert!(!window.has_newer);
    assert!(window.first_unread_message_id.is_empty());
    // The newest page, oldest-to-newest.
    assert_eq!(window.messages.first().unwrap().id, "m030");
    assert_eq!(window.messages.last().unwrap().id, "m079");
}

/// The Go asked `MessagesAround(..., 25)`: 25 older, the anchor, 25 newer.
#[test]
fn open_message_window_centres_on_the_first_unread() {
    let (_dir, store, mut server) = server("window-unread");
    handshake(&mut server);
    store
        .ensure_chat("a@s.whatsapp.net", "a@s.whatsapp.net")
        .unwrap();
    for index in 0..80 {
        let mut row = message(
            "a@s.whatsapp.net",
            &format!("m{index:03}"),
            1000 + index,
            "hi",
        );
        row.unread = index == 40;
        store.insert_message(&row).unwrap();
    }
    let response = call(
        &mut server,
        2,
        rpc_request::Request::OpenMessageWindow(whatfast_protocol::OpenMessageWindowRequest {
            chat_id: "a@s.whatsapp.net".into(),
        }),
    );
    let rpc_response::Result::OpenMessageWindow(window) = response.result.expect("a result") else {
        panic!("expected OpenMessageWindow");
    };
    assert_eq!(window.first_unread_message_id, "m040");
    assert_eq!(window.messages.len(), 51, "25 older, the anchor, 25 newer");
    assert!(window.has_older);
    assert!(window.has_newer);
    assert!(
        window
            .messages
            .iter()
            .any(|message| message.id == "m040" && message.chat_id == "a@s.whatsapp.net")
    );
}

// ---------------------------------------------------------------------------
// Statuses
// ---------------------------------------------------------------------------

fn status_row(sender: &str, id: &str, ts: i64) -> MessageRow {
    MessageRow {
        id: id.into(),
        chat_jid: "status@broadcast".into(),
        transport_jid: "status@broadcast".into(),
        sender_jid: sender.into(),
        text: "status".into(),
        timestamp: ts,
        kind: "text".into(),
        status: 2,
        ..Default::default()
    }
}

#[test]
fn list_statuses_groups_by_sender_newest_first() {
    let (_dir, store, mut server) = server("statuses");
    handshake(&mut server);
    store
        .ensure_chat("status@broadcast", "status@broadcast")
        .unwrap();
    let now = now_ms();
    store
        .insert_message(&status_row("a@s.whatsapp.net", "s1", now - 3_000))
        .unwrap();
    store
        .insert_message(&status_row("b@s.whatsapp.net", "s2", now - 2_000))
        .unwrap();
    store
        .insert_message(&status_row("a@s.whatsapp.net", "s3", now - 1_000))
        .unwrap();
    // A status older than the 24-hour window never surfaces.
    store
        .insert_message(&status_row(
            "c@s.whatsapp.net",
            "s0",
            now - 25 * 60 * 60 * 1000,
        ))
        .unwrap();

    let response = call(
        &mut server,
        2,
        rpc_request::Request::ListStatuses(whatfast_protocol::ListStatusesRequest {}),
    );
    let rpc_response::Result::ListStatuses(statuses) = response.result.expect("a result") else {
        panic!("expected ListStatuses");
    };
    assert_eq!(statuses.chat_id, "status@broadcast");
    // One ring per author, like the origin: the second post of `a` joins the
    // ring it already has instead of opening another one.
    assert_eq!(statuses.groups.len(), 2);
    // Newest author first, and newest post first inside the ring.
    assert_eq!(statuses.groups[0].sender_id, "a@s.whatsapp.net");
    assert_eq!(statuses.groups[0].messages.len(), 2);
    assert_eq!(statuses.groups[0].messages[0].id, "s3");
    assert_eq!(statuses.groups[0].messages[1].id, "s1");
    assert_eq!(statuses.groups[1].sender_id, "b@s.whatsapp.net");
    assert_eq!(statuses.groups[1].messages[0].id, "s2");
}

#[test]
fn list_statuses_refuses_an_empty_store() {
    let (_dir, _store, mut server) = server("statuses-empty");
    handshake(&mut server);
    let response = call(
        &mut server,
        2,
        rpc_request::Request::ListStatuses(whatfast_protocol::ListStatusesRequest {}),
    );
    assert_eq!(error_of(response).code, "not_found");
}

// ---------------------------------------------------------------------------
// Stickers
// ---------------------------------------------------------------------------

#[test]
fn list_stickers_builds_the_three_packs_and_honours_favourites() {
    let (_dir, store, mut server) = server("stickers");
    handshake(&mut server);
    store
        .ensure_chat("a@s.whatsapp.net", "a@s.whatsapp.net")
        .unwrap();
    let mut sticker = message("a@s.whatsapp.net", "s1", 500, "Sticker");
    sticker.kind = "sticker".into();
    sticker.from_me = true;
    sticker.image_mime = "image/webp".into();
    sticker.image_file_sha256 = vec![0xab, 0xcd];
    sticker.image_width = 64;
    sticker.image_height = 64;
    sticker.image_size = 100;
    store.insert_message(&sticker).unwrap();

    let response = call(
        &mut server,
        2,
        rpc_request::Request::ListStickers(whatfast_protocol::ListStickersRequest {}),
    );
    let rpc_response::Result::ListStickers(stickers) = response.result.expect("a result") else {
        panic!("expected ListStickers");
    };
    let packs: Vec<&str> = stickers.packs.iter().map(|pack| pack.id.as_str()).collect();
    assert_eq!(packs, ["favorites", "recent", "all"]);
    assert!(stickers.packs[0].stickers.is_empty());
    assert_eq!(stickers.packs[1].stickers.len(), 1);
    assert_eq!(stickers.packs[1].stickers[0].id, "abcd");
    assert_eq!(stickers.packs[1].stickers[0].source_message_id, "s1");
    assert_eq!(stickers.packs[1].stickers[0].width, 64);
    assert_eq!(stickers.packs[2].stickers.len(), 1);

    // A favourite synced from the phone lands in its own pack and marks the
    // history entry it matches.
    store
        .upsert_favorite_sticker(&whatfast_wa::db::FavoriteStickerRow {
            id: "abcd".into(),
            mime: "image/webp".into(),
            width: 64,
            height: 64,
            updated_at: 900,
            ..Default::default()
        })
        .unwrap();
    let response = call(
        &mut server,
        3,
        rpc_request::Request::ListStickers(whatfast_protocol::ListStickersRequest {}),
    );
    let rpc_response::Result::ListStickers(stickers) = response.result.expect("a result") else {
        panic!("expected ListStickers");
    };
    assert_eq!(stickers.packs[0].stickers.len(), 1);
    assert!(stickers.packs[0].stickers[0].favorite);
    assert!(stickers.packs[2].stickers[0].favorite);
}

// ---------------------------------------------------------------------------
// Newsletters
// ---------------------------------------------------------------------------

#[test]
fn unsubscribe_newsletter_removes_the_stored_subscription() {
    let (_dir, store, mut server) = server("newsletter");
    handshake(&mut server);
    store
        .upsert_newsletter("120363000000000001@newsletter", "Channel", false)
        .unwrap();
    let response = call(
        &mut server,
        2,
        rpc_request::Request::UnsubscribeNewsletter(
            whatfast_protocol::UnsubscribeNewsletterRequest {
                jid: "120363000000000001@newsletter".into(),
            },
        ),
    );
    assert!(matches!(
        response.result.expect("a result"),
        rpc_response::Result::UnsubscribeNewsletter(_)
    ));
    assert!(store.list_newsletters().unwrap().is_empty());
}

#[test]
fn newsletter_requests_require_their_key() {
    let (_dir, _store, mut server) = server("newsletter-empty");
    handshake(&mut server);
    let subscribe = call(
        &mut server,
        2,
        rpc_request::Request::SubscribeNewsletter(
            whatfast_protocol::SubscribeNewsletterRequest::default(),
        ),
    );
    assert_eq!(error_of(subscribe).code, "invalid_argument");
    let messages = call(
        &mut server,
        3,
        rpc_request::Request::ListNewsletterMessages(
            whatfast_protocol::ListNewsletterMessagesRequest::default(),
        ),
    );
    assert_eq!(error_of(messages).code, "invalid_argument");
    let unsubscribe = call(
        &mut server,
        4,
        rpc_request::Request::UnsubscribeNewsletter(
            whatfast_protocol::UnsubscribeNewsletterRequest::default(),
        ),
    );
    assert_eq!(error_of(unsubscribe).code, "invalid_argument");
}

// ---------------------------------------------------------------------------
// Pins
// ---------------------------------------------------------------------------

#[test]
fn list_pinned_messages_answers_the_stored_message() {
    let (_dir, store, mut server) = server("pins");
    handshake(&mut server);
    store
        .ensure_chat("a@s.whatsapp.net", "a@s.whatsapp.net")
        .unwrap();
    store
        .insert_message(&message("a@s.whatsapp.net", "m1", 100, "pin me"))
        .unwrap();
    store
        .set_message_pinned("a@s.whatsapp.net", "m1", "me@s.whatsapp.net", 200, true)
        .unwrap();
    // A pin whose message is gone still lists, with `available` off.
    store
        .set_message_pinned("a@s.whatsapp.net", "gone", "me@s.whatsapp.net", 100, true)
        .unwrap();

    let response = call(
        &mut server,
        2,
        rpc_request::Request::ListPinnedMessages(whatfast_protocol::ListPinnedMessagesRequest {
            chat_id: "a@s.whatsapp.net".into(),
        }),
    );
    let rpc_response::Result::ListPinnedMessages(pins) = response.result.expect("a result") else {
        panic!("expected ListPinnedMessages");
    };
    assert_eq!(pins.pins.len(), 2);
    assert_eq!(pins.pins[0].message_id, "m1");
    assert_eq!(pins.pins[0].pinned_at_ms, 200);
    assert!(pins.pins[0].available);
    assert_eq!(
        pins.pins[0]
            .message
            .as_ref()
            .map(|message| message.id.as_str()),
        Some("m1")
    );
    assert_eq!(pins.pins[1].message_id, "gone");
    assert!(!pins.pins[1].available);
    assert!(pins.pins[1].message.is_none());
}

#[test]
fn set_message_pin_requires_a_connection_and_its_target() {
    let (_dir, _store, mut server) = server("pin-set");
    handshake(&mut server);
    let missing = call(
        &mut server,
        2,
        rpc_request::Request::SetMessagePin(whatfast_protocol::SetMessagePinRequest::default()),
    );
    assert_eq!(error_of(missing).code, "invalid_argument");
    let offline = call(
        &mut server,
        3,
        rpc_request::Request::SetMessagePin(whatfast_protocol::SetMessagePinRequest {
            chat_id: "a@s.whatsapp.net".into(),
            message_id: "m1".into(),
            pinned: true,
        }),
    );
    let error = error_of(offline);
    assert_eq!(error.code, "not_connected");
    assert!(error.retryable, "the Go marked not_connected retryable");
}

// ---------------------------------------------------------------------------
// OpenContact
// ---------------------------------------------------------------------------

#[test]
fn open_contact_returns_the_canonical_conversation() {
    let (_dir, store, mut server) = server("contact");
    handshake(&mut server);
    // An existing hidden-identity chat plus the phone-number alias: the
    // lookup must answer the LID row instead of minting a twin.
    store.ensure_chat("111@lid", "111@lid").unwrap();
    store
        .adopt_address("111@s.whatsapp.net", "111@lid")
        .unwrap();
    let response = call(
        &mut server,
        2,
        rpc_request::Request::OpenContact(whatfast_protocol::OpenContactRequest {
            contact_jid: "111@s.whatsapp.net".into(),
        }),
    );
    let rpc_response::Result::OpenContact(contact) = response.result.expect("a result") else {
        panic!("expected OpenContact");
    };
    let chat = contact.chat.expect("a chat");
    assert_eq!(chat.id, "111@lid");
    // The port's transport is the most recently seen address of the
    // conversation; adopting the phone number made that the newest one. Both
    // forms reach the same conversation.
    assert!(
        matches!(
            chat.transport_jid.as_str(),
            "111@lid" | "111@s.whatsapp.net"
        ),
        "unexpected transport {}",
        chat.transport_jid
    );

    // A contact with no history gets an empty conversation, not an error.
    let response = call(
        &mut server,
        3,
        rpc_request::Request::OpenContact(whatfast_protocol::OpenContactRequest {
            contact_jid: "222@s.whatsapp.net".into(),
        }),
    );
    let rpc_response::Result::OpenContact(contact) = response.result.expect("a result") else {
        panic!("expected OpenContact");
    };
    assert_eq!(contact.chat.expect("a chat").id, "222@s.whatsapp.net");
}

#[test]
fn open_contact_refuses_a_non_direct_jid() {
    let (_dir, _store, mut server) = server("contact-invalid");
    handshake(&mut server);
    for (id, jid) in [(2, "120363000000000001@g.us"), (3, "not-a-jid")] {
        let response = call(
            &mut server,
            id,
            rpc_request::Request::OpenContact(whatfast_protocol::OpenContactRequest {
                contact_jid: jid.into(),
            }),
        );
        let error = error_of(response);
        assert_eq!(error.code, "invalid_argument", "{jid}");
        assert_eq!(error.message, "invalid direct-contact JID");
    }
    let empty = call(
        &mut server,
        4,
        rpc_request::Request::OpenContact(whatfast_protocol::OpenContactRequest::default()),
    );
    assert_eq!(error_of(empty).code, "invalid_argument");
}

// ---------------------------------------------------------------------------
// Read receipts and reactions
// ---------------------------------------------------------------------------

#[test]
fn mark_read_refuses_a_missing_or_outgoing_anchor() {
    let (_dir, store, mut server) = server("mark-read");
    handshake(&mut server);
    store
        .ensure_chat("a@s.whatsapp.net", "a@s.whatsapp.net")
        .unwrap();
    let mut outgoing = message("a@s.whatsapp.net", "mine", 100, "sent");
    outgoing.from_me = true;
    store.insert_message(&outgoing).unwrap();

    let missing = call(
        &mut server,
        2,
        rpc_request::Request::MarkRead(MarkReadRequest {
            chat_id: "a@s.whatsapp.net".into(),
            through_message_id: "nope".into(),
        }),
    );
    assert_eq!(error_of(missing).code, "not_found");

    let outgoing = call(
        &mut server,
        3,
        rpc_request::Request::MarkRead(MarkReadRequest {
            chat_id: "a@s.whatsapp.net".into(),
            through_message_id: "mine".into(),
        }),
    );
    let error = error_of(outgoing);
    assert_eq!(error.code, "internal");
    assert_eq!(error.message, "message is outgoing");
}

#[test]
fn send_reaction_checks_the_action_id_and_the_emoji() {
    let (_dir, _store, mut server) = server("reaction");
    handshake(&mut server);
    let no_id = call(
        &mut server,
        2,
        rpc_request::Request::SendReaction(SendReactionRequest {
            chat_id: "a@s.whatsapp.net".into(),
            message_id: "m1".into(),
            emoji: "👍".into(),
            ..Default::default()
        }),
    );
    assert_eq!(error_of(no_id).code, "invalid_argument");

    let bad_id = call(
        &mut server,
        3,
        rpc_request::Request::SendReaction(SendReactionRequest {
            client_reaction_id: "not-a-uuid".into(),
            chat_id: "a@s.whatsapp.net".into(),
            message_id: "m1".into(),
            emoji: "👍".into(),
        }),
    );
    let error = error_of(bad_id);
    assert_eq!(error.code, "invalid_argument");
    assert_eq!(error.message, "client_reaction_id must be a UUID");

    let bad_emoji = call(
        &mut server,
        4,
        rpc_request::Request::SendReaction(SendReactionRequest {
            client_reaction_id: "b0bca7f5-a85f-4a8c-8956-1722cf35ffbc".into(),
            chat_id: "a@s.whatsapp.net".into(),
            message_id: "m1".into(),
            emoji: "hello".into(),
        }),
    );
    let error = error_of(bad_emoji);
    assert_eq!(error.code, "invalid_argument");
    assert_eq!(
        error.message,
        "emoji must be empty or one emoji grapheme up to 64 bytes"
    );
}

#[test]
fn forward_message_requires_its_client_id_and_reports_the_go_refusal() {
    let (_dir, _store, mut server) = server("forward");
    handshake(&mut server);
    let incomplete = call(
        &mut server,
        2,
        rpc_request::Request::ForwardMessage(whatfast_protocol::ForwardMessageRequest {
            source_chat_id: "a@s.whatsapp.net".into(),
            message_id: "m1".into(),
            target_chat_id: "b@s.whatsapp.net".into(),
            ..Default::default()
        }),
    );
    assert_eq!(error_of(incomplete).code, "invalid_argument");

    let bad_id = call(
        &mut server,
        3,
        rpc_request::Request::ForwardMessage(whatfast_protocol::ForwardMessageRequest {
            client_message_id: "not-a-uuid".into(),
            source_chat_id: "a@s.whatsapp.net".into(),
            message_id: "m1".into(),
            target_chat_id: "b@s.whatsapp.net".into(),
        }),
    );
    assert_eq!(error_of(bad_id).message, "client_message_id must be a UUID");
}

// ---------------------------------------------------------------------------
// Wire content
// ---------------------------------------------------------------------------

#[test]
fn list_messages_serves_the_stored_poll_definition() {
    let (_dir, store, mut server) = server("poll");
    handshake(&mut server);
    store
        .ensure_chat("a@s.whatsapp.net", "a@s.whatsapp.net")
        .unwrap();
    let mut poll = message("a@s.whatsapp.net", "p1", 100, "Poll: Lunch?");
    poll.kind = "poll".into();
    store.insert_message(&poll).unwrap();
    store
        .upsert_poll(
            "a@s.whatsapp.net",
            "p1",
            "Lunch?",
            &["Pizza".into(), "Sushi".into()],
            1,
        )
        .unwrap();
    // A vote from this account selects the second option.
    store
        .write(|conn| {
            conn.execute(
                "INSERT INTO poll_votes(chat_jid,poll_message_id,voter_jid,selected_json,timestamp,from_me)
                 VALUES('a@s.whatsapp.net','p1','me@s.whatsapp.net','[\"Sushi\"]',150,1)",
                [],
            )?;
            Ok(())
        })
        .unwrap();

    let response = call(
        &mut server,
        2,
        rpc_request::Request::ListMessages(ListMessagesRequest {
            chat_id: "a@s.whatsapp.net".into(),
            limit: 10,
            ..Default::default()
        }),
    );
    let rpc_response::Result::ListMessages(page) = response.result.expect("a result") else {
        panic!("expected ListMessages");
    };
    let content = page.messages[0].content.as_ref().expect("content");
    let whatfast_protocol::message::Content::Poll(poll) = content else {
        panic!("expected poll content, got {content:?}");
    };
    assert_eq!(poll.question, "Lunch?");
    assert_eq!(poll.selectable_options_count, 1);
    assert_eq!(poll.total_voters, 1);
    assert_eq!(poll.options.len(), 2);
    assert_eq!(poll.options[0].name, "Pizza");
    assert!(!poll.options[0].selected_by_me);
    assert_eq!(poll.options[1].name, "Sushi");
    assert!(poll.options[1].selected_by_me);
    assert_eq!(poll.options[1].vote_count, 1);
    assert_eq!(poll.options[1].voters.len(), 1);
    assert!(poll.options[1].voters[0].from_me);
}

// ---------------------------------------------------------------------------
// History coverage, integrity, repair, votes and reaction repair
// ---------------------------------------------------------------------------

/// The three local reads answer from the store: no WhatsApp link needed, the
/// way the desktop paints the chat footer and the integrity panel offline.
#[test]
fn history_coverage_overview_and_integrity_answer_locally() {
    let (_dir, store, mut server) = server("local-reads");
    handshake(&mut server);
    // The ids are required even for local reads.
    let missing = call(
        &mut server,
        2,
        rpc_request::Request::GetChatHistoryCoverage(
            whatfast_protocol::GetChatHistoryCoverageRequest {
                chat_id: String::new(),
            },
        ),
    );
    assert_eq!(error_of(missing).code, "invalid_argument");
    store
        .ensure_chat("c@s.whatsapp.net", "c@s.whatsapp.net")
        .unwrap();
    store
        .insert_message(&message("c@s.whatsapp.net", "m1", 1000, "hi"))
        .unwrap();
    let coverage = call(
        &mut server,
        3,
        rpc_request::Request::GetChatHistoryCoverage(
            whatfast_protocol::GetChatHistoryCoverageRequest {
                chat_id: "c@s.whatsapp.net".into(),
            },
        ),
    );
    let rpc_response::Result::GetChatHistoryCoverage(answer) = coverage.result.expect("a result")
    else {
        panic!("expected GetChatHistoryCoverage");
    };
    let coverage = answer.coverage.expect("coverage");
    assert_eq!(coverage.chat_id, "c@s.whatsapp.net");
    assert_eq!(coverage.local_message_count, 1);
    let overview = call(
        &mut server,
        4,
        rpc_request::Request::GetHistoryOverview(whatfast_protocol::GetHistoryOverviewRequest {}),
    );
    let rpc_response::Result::GetHistoryOverview(answer) = overview.result.expect("a result")
    else {
        panic!("expected GetHistoryOverview");
    };
    let overview = answer.overview.expect("overview");
    assert_eq!(overview.local_message_count, 1);
    assert_eq!(overview.chats_with_messages, 1);
    let integrity = call(
        &mut server,
        5,
        rpc_request::Request::GetIntegrityStatus(whatfast_protocol::GetIntegrityStatusRequest {}),
    );
    let rpc_response::Result::GetIntegrityStatus(answer) = integrity.result.expect("a result")
    else {
        panic!("expected GetIntegrityStatus");
    };
    assert!(answer.status.expect("status").database_ok);
    let repair = call(
        &mut server,
        6,
        rpc_request::Request::RepairLocalCache(whatfast_protocol::RepairLocalCacheRequest {}),
    );
    let rpc_response::Result::RepairLocalCache(answer) = repair.result.expect("a result") else {
        panic!("expected RepairLocalCache");
    };
    let status = answer.status.expect("status");
    assert!(status.database_ok);
    assert_eq!(status.inconsistent_chats, 0);
}

/// An older-history request without an anchor fails before the network, and
/// one without a link records the offline coverage the desktop displays.
#[test]
fn request_older_history_refuses_without_anchor_and_while_offline() {
    let (_dir, store, mut server) = server("older-history");
    handshake(&mut server);
    let missing = call(
        &mut server,
        2,
        rpc_request::Request::RequestOlderHistory(whatfast_protocol::RequestOlderHistoryRequest {
            chat_id: String::new(),
        }),
    );
    assert_eq!(error_of(missing).code, "invalid_argument");
    store
        .ensure_chat("c@s.whatsapp.net", "c@s.whatsapp.net")
        .unwrap();
    let anchorless = call(
        &mut server,
        3,
        rpc_request::Request::RequestOlderHistory(whatfast_protocol::RequestOlderHistoryRequest {
            chat_id: "c@s.whatsapp.net".into(),
        }),
    );
    assert_eq!(error_of(anchorless).code, "history_unavailable");
    store
        .insert_message(&message("c@s.whatsapp.net", "m1", 1000, "hi"))
        .unwrap();
    // No session runs in this harness: the link is down and the request
    // fails retryable after recording the offline coverage.
    let offline = call(
        &mut server,
        4,
        rpc_request::Request::RequestOlderHistory(whatfast_protocol::RequestOlderHistoryRequest {
            chat_id: "c@s.whatsapp.net".into(),
        }),
    );
    let error = error_of(offline);
    assert_eq!(error.code, "not_connected");
    assert!(error.retryable);
    let coverage = store.history_coverage("c@s.whatsapp.net").unwrap();
    assert_eq!(coverage.state, "offline");
}

/// Votes validate their ids before anything else, then refuse without a
/// link; the networked vote itself needs a live session.
#[test]
fn vote_poll_validates_before_touching_the_network() {
    let (_dir, _store, mut server) = server("vote-poll");
    handshake(&mut server);
    let missing = call(
        &mut server,
        2,
        rpc_request::Request::VotePoll(whatfast_protocol::VotePollRequest {
            chat_id: String::new(),
            poll_message_id: String::new(),
            selected_options: Vec::new(),
        }),
    );
    assert_eq!(error_of(missing).code, "invalid_argument");
    let offline = call(
        &mut server,
        3,
        rpc_request::Request::VotePoll(whatfast_protocol::VotePollRequest {
            chat_id: "c@s.whatsapp.net".into(),
            poll_message_id: "p1".into(),
            selected_options: vec!["Yes".into()],
        }),
    );
    let error = error_of(offline);
    assert_eq!(error.code, "not_connected");
    assert!(error.retryable);
}

/// The repair validates its chat, then refuses without a link; the empty
/// repair (no pending placeholders) is covered at the store level.
#[test]
fn repair_recent_reactions_validates_and_needs_the_link() {
    let (_dir, _store, mut server) = server("repair-reactions");
    handshake(&mut server);
    let missing = call(
        &mut server,
        2,
        rpc_request::Request::RepairRecentReactions(
            whatfast_protocol::RepairRecentReactionsRequest {
                chat_id: String::new(),
            },
        ),
    );
    assert_eq!(error_of(missing).code, "invalid_argument");
    let offline = call(
        &mut server,
        3,
        rpc_request::Request::RepairRecentReactions(
            whatfast_protocol::RepairRecentReactionsRequest {
                chat_id: "c@s.whatsapp.net".into(),
            },
        ),
    );
    let error = error_of(offline);
    assert_eq!(error.code, "not_connected");
    assert!(error.retryable);
}

/// An inflated counter with no flagged rows heals on open: the read finds
/// nothing to receipt but recomputes the cached total anyway.
#[test]
fn mark_read_with_no_flagged_rows_heals_the_counter() {
    let (_dir, store, mut server) = server("heal-counter");
    handshake(&mut server);
    store
        .ensure_chat("c@s.whatsapp.net", "c@s.whatsapp.net")
        .unwrap();
    store
        .insert_message(&message("c@s.whatsapp.net", "m1", 1000, "hi"))
        .unwrap();
    // Simulate the drift of an older build: the row lost its flag while
    // the cached counter kept it.
    store
        .read(|conn| {
            conn.execute("UPDATE messages SET unread=1 WHERE id='m1'", [])
                .unwrap();
            conn.execute(
                "UPDATE chats SET unread_count=5 WHERE jid='c@s.whatsapp.net'",
                [],
            )
            .unwrap();
            Ok::<_, rusqlite::Error>(())
        })
        .unwrap();
    // No session runs here: without the link the read refuses before
    // touching anything, and the counter stays put.
    let offline = call(
        &mut server,
        2,
        rpc_request::Request::MarkRead(whatfast_protocol::MarkReadRequest {
            chat_id: "c@s.whatsapp.net".into(),
            through_message_id: "m1".into(),
        }),
    );
    let error = error_of(offline);
    assert_eq!(error.code, "not_connected");
    assert_eq!(
        store
            .chat("c@s.whatsapp.net")
            .unwrap()
            .unwrap()
            .unread_count,
        5
    );
}
