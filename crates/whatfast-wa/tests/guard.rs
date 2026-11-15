//! The permissions of an agent, checked without a pipe.
//!
//! The guard is the door of the local endpoint: these tests walk through it
//! with a profile of their own, a registered agent and the requests the MCP
//! server really sends.

use std::{path::PathBuf, sync::Arc};

use whatfast_auto::{ChatFilter, Grant, Scope, Window};
use whatfast_protocol::{
    GetChatInfoRequest, ListChatsRequest, ListMessagesRequest, RpcResponse, SendTextRequest,
    SetChatPinRequest, SetTypingRequest, rpc_response,
};
use whatfast_wa::{
    db::Store,
    guard::{Guard, Who},
};

/// The milliseconds since the epoch, the clock of the profile.
fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or_default()
}

/// A profile of its own, removed when the test ends.
struct Profile {
    dir: PathBuf,
}

impl Profile {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "whatfast-guard-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).expect("profile");
        Self { dir }
    }
}

impl Drop for Profile {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// The profile, its store and its guard.
fn profile(name: &str) -> (Profile, Arc<Store>, Guard) {
    let profile = Profile::new(name);
    let store = Arc::new(Store::open(&profile.dir.join("chats.db")).expect("chats.db"));
    store
        .ensure_chat(
            "5491100000000@s.whatsapp.net",
            "5491100000000@s.whatsapp.net",
        )
        .expect("direct chat");
    store
        .ensure_chat("120363169271016023@g.us", "120363169271016023@g.us")
        .expect("group chat");
    let guard = Guard::open(&profile.dir, -180).expect("agents.db");
    (profile, store, guard)
}

/// Registers an agent with the grant the test needs.
fn connect(
    guard: &Guard,
    id: &str,
    scope: Scope,
    chats: ChatFilter,
    per_hour: u16,
    window: Option<Window>,
) {
    let grant = Grant {
        scope,
        chats,
        per_hour,
        window,
        humanize: true,
    };
    guard
        .db()
        .connect_agent(id, id, &grant, "token-de-prueba", 1_000)
        .expect("registered");
}

fn list_messages(chat_id: &str) -> whatfast_protocol::rpc_request::Request {
    whatfast_protocol::rpc_request::Request::ListMessages(ListMessagesRequest {
        chat_id: chat_id.to_owned(),
        before_timestamp_ms: 0,
        before_message_id: String::new(),
        limit: 10,
    })
}

fn send_text(chat_id: &str) -> whatfast_protocol::rpc_request::Request {
    whatfast_protocol::rpc_request::Request::SendText(SendTextRequest {
        client_message_id: "3f0f7f0e-0000-4000-8000-000000000000".into(),
        chat_id: chat_id.to_owned(),
        text: "hola".into(),
        ..Default::default()
    })
}

fn send_attachment(chat_id: &str) -> whatfast_protocol::rpc_request::Request {
    whatfast_protocol::rpc_request::Request::SendAttachment(
        whatfast_protocol::SendAttachmentRequest {
            client_message_id: "3f0f7f0e-0000-4000-8000-000000000001".into(),
            chat_id: chat_id.to_owned(),
            file_path: "C:\\archivos\\informe.pdf".into(),
            kind: whatfast_protocol::AttachmentKind::Document as i32,
            ..Default::default()
        },
    )
}

fn send_image(chat_id: &str) -> whatfast_protocol::rpc_request::Request {
    whatfast_protocol::rpc_request::Request::SendImage(whatfast_protocol::SendImageRequest {
        client_message_id: "3f0f7f0e-0000-4000-8000-000000000002".into(),
        chat_id: chat_id.to_owned(),
        image_path: "C:\\archivos\\foto.jpg".into(),
        ..Default::default()
    })
}

#[test]
fn a_client_without_an_agent_is_the_user() {
    let (_profile, store, guard) = profile("user");
    assert_eq!(guard.identify("", ""), Who::User);
    assert!(
        guard
            .check(
                &Who::User,
                &list_messages("5491100000000@s.whatsapp.net"),
                &store
            )
            .is_ok()
    );
    assert!(
        guard
            .check(
                &Who::User,
                &send_text("5491100000000@s.whatsapp.net"),
                &store
            )
            .is_ok()
    );
}

#[test]
fn the_first_install_lets_the_harness_work_without_a_token() {
    let (_profile, store, guard) = profile("bootstrap");
    assert_eq!(guard.identify("claude-code", ""), Who::User);
    assert!(
        guard
            .check(
                &Who::User,
                &send_text("5491100000000@s.whatsapp.net"),
                &store
            )
            .is_ok()
    );
}

#[test]
fn a_registered_agent_needs_its_token() {
    let (_profile, _store, guard) = profile("token");
    connect(&guard, "codex", Scope::Read, ChatFilter::all(), 0, None);
    assert!(matches!(
        guard.identify("codex", "token-de-prueba"),
        Who::Agent { .. }
    ));
    assert!(matches!(guard.identify("codex", "otro"), Who::Denied(_)));
    assert!(matches!(guard.identify("codex", ""), Who::Denied(_)));
    assert!(matches!(guard.identify("cursor", ""), Who::Denied(_)));
}

#[test]
fn the_grant_decides_which_chats_it_reads() {
    let (_profile, store, guard) = profile("chats");
    connect(
        &guard,
        "codex",
        Scope::Read,
        ChatFilter::only(vec!["5491100000000@s.whatsapp.net".into()]),
        0,
        None,
    );
    let who = guard.identify("codex", "token-de-prueba");
    assert!(
        guard
            .check(&who, &list_messages("5491100000000@s.whatsapp.net"), &store)
            .is_ok()
    );
    let denied = guard
        .check(&who, &list_messages("120363169271016023@g.us"), &store)
        .expect_err("the group is not in the grant");
    assert_eq!(denied.code, "forbidden");
    assert!(denied.message.contains("120363169271016023@g.us"));
}

#[test]
fn a_read_grant_cannot_send() {
    let (_profile, store, guard) = profile("read");
    connect(&guard, "codex", Scope::Read, ChatFilter::all(), 0, None);
    let who = guard.identify("codex", "token-de-prueba");
    let denied = guard
        .check(&who, &send_text("5491100000000@s.whatsapp.net"), &store)
        .expect_err("read does not send");
    assert_eq!(denied.code, "forbidden");
}

/// The file tools of the MCP server are sends: the grant of the agent has to
/// cover them, and a file goes out through the same door as a text.
#[test]
fn a_send_grant_sends_files_to_its_chats() {
    let (_profile, store, guard) = profile("files");
    connect(
        &guard,
        "codex",
        Scope::Send,
        ChatFilter::only(vec!["5491100000000@s.whatsapp.net".into()]),
        0,
        None,
    );
    let who = guard.identify("codex", "token-de-prueba");
    assert!(
        guard
            .check(
                &who,
                &send_attachment("5491100000000@s.whatsapp.net"),
                &store
            )
            .is_ok()
    );
    assert!(
        guard
            .check(&who, &send_image("5491100000000@s.whatsapp.net"), &store)
            .is_ok()
    );
    // The chat outside the grant stays closed for a file too.
    let denied = guard
        .check(&who, &send_attachment("120363169271016023@g.us"), &store)
        .expect_err("the group is not in the grant");
    assert_eq!(denied.code, "forbidden");
}

/// The user widens the grant of an agent that is still connected: the next
/// request of the open connection has to see the new permission. This is the
/// case of the interface: the harness process lives for days and never repeats
/// its handshake.
#[test]
fn a_widened_grant_reaches_a_connection_that_is_already_open() {
    let (_profile, store, guard) = profile("live-grant");
    connect(
        &guard,
        "codex",
        Scope::Draft,
        ChatFilter::all(),
        0,
        None,
    );
    let who = guard.identify("codex", "token-de-prueba");
    assert_eq!(
        guard
            .check(&who, &send_text("5491100000000@s.whatsapp.net"), &store)
            .expect_err("a draft grant does not send")
            .code,
        "forbidden"
    );
    let widened = Grant {
        scope: Scope::Send,
        chats: ChatFilter::all(),
        per_hour: 0,
        window: None,
        humanize: true,
    };
    guard.db().set_grant("codex", &widened).expect("widen");
    assert!(
        guard
            .check(&who, &send_text("5491100000000@s.whatsapp.net"), &store)
            .is_ok(),
        "the widening did not reach the open connection"
    );
}

/// The user disconnects an agent: the open connection loses the door at once,
/// without waiting for a new handshake.
#[test]
fn a_revoked_connection_is_refused_without_a_new_handshake() {
    let (_profile, store, guard) = profile("live-revoke");
    connect(&guard, "codex", Scope::Send, ChatFilter::all(), 0, None);
    let who = guard.identify("codex", "token-de-prueba");
    assert!(
        guard
            .check(&who, &send_text("5491100000000@s.whatsapp.net"), &store)
            .is_ok()
    );
    guard.db().revoke("codex", 2_000).expect("revoke");
    let denied = guard
        .check(&who, &send_text("5491100000000@s.whatsapp.net"), &store)
        .expect_err("a revoked agent keeps no door");
    assert_eq!(denied.code, "unauthorized");
}

#[test]
fn a_draft_grant_cannot_send_files() {
    let (_profile, store, guard) = profile("files-draft");
    connect(&guard, "codex", Scope::Draft, ChatFilter::all(), 0, None);
    let who = guard.identify("codex", "token-de-prueba");
    assert_eq!(
        guard
            .check(
                &who,
                &send_attachment("5491100000000@s.whatsapp.net"),
                &store
            )
            .expect_err("draft does not send a file")
            .code,
        "forbidden"
    );
    assert_eq!(
        guard
            .check(&who, &send_image("5491100000000@s.whatsapp.net"), &store)
            .expect_err("draft does not send a picture")
            .code,
        "forbidden"
    );
}

#[test]
fn a_send_grant_sends_inside_its_hours() {
    let (_profile, store, guard) = profile("send");
    connect(
        &guard,
        "codex",
        Scope::Send,
        ChatFilter::only(vec!["5491100000000@s.whatsapp.net".into()]),
        10,
        Some(Window::PRUDENT),
    );
    let who = guard.identify("codex", "token-de-prueba");
    // The window is 08:00-23:00 local (UTC-3). Read one fixed instant and
    // compare its minute with the literal bounds: the expectation must not
    // call the helper under test.
    let now = now_ms();
    let minute = ((now - 180 * 60_000).rem_euclid(86_400_000) / 60_000) as u16;
    let inside = (8 * 60..23 * 60).contains(&minute);
    let verdict = guard.check(&who, &send_text("5491100000000@s.whatsapp.net"), &store);
    if inside {
        assert!(verdict.is_ok(), "inside the window: {verdict:?}");
    } else {
        assert_eq!(
            verdict.expect_err("outside the window").code,
            "out_of_window"
        );
    }
}

#[test]
fn a_closed_window_holds_the_message() {
    let (_profile, store, guard) = profile("window");
    let night = Window {
        start: 3 * 60,
        end: 4 * 60,
    };
    connect(
        &guard,
        "codex",
        Scope::Send,
        ChatFilter::all(),
        0,
        Some(night),
    );
    let who = guard.identify("codex", "token-de-prueba");
    // The window is 03:00-04:00 local (UTC-3). Read one fixed instant and
    // compare its minute with the literal bounds: the expectation must not
    // call the helper under test.
    let now = now_ms();
    let minute = ((now - 180 * 60_000).rem_euclid(86_400_000) / 60_000) as u16;
    let inside = (3 * 60..4 * 60).contains(&minute);
    let verdict = guard.check(&who, &send_text("5491100000000@s.whatsapp.net"), &store);
    if inside {
        assert!(verdict.is_ok());
    } else {
        assert_eq!(
            verdict.expect_err("the window is closed").code,
            "out_of_window"
        );
    }
}

#[test]
fn the_ceiling_of_the_hour_is_counted() {
    let (_profile, store, guard) = profile("ceiling");
    connect(&guard, "codex", Scope::Send, ChatFilter::all(), 1, None);
    let who = guard.identify("codex", "token-de-prueba");
    assert!(
        guard
            .check(&who, &send_text("5491100000000@s.whatsapp.net"), &store)
            .is_ok()
    );
    // One send already went out in this hour: the second is refused.
    guard
        .db()
        .record_call(
            &whatfast_auto::Call {
                agent_id: "codex".into(),
                tool: "send_text".into(),
                chat_id: Some("5491100000000@s.whatsapp.net".into()),
                ok: true,
                ms: 10,
                bytes: 100,
                detail: None,
            },
            now_ms(),
        )
        .expect("logged");
    let denied = guard
        .check(&who, &send_text("5491100000000@s.whatsapp.net"), &store)
        .expect_err("the ceiling is one");
    assert_eq!(denied.code, "rate_limited");
}

#[test]
fn the_tools_of_the_session_are_forbidden() {
    let (_profile, store, guard) = profile("forbidden");
    connect(&guard, "codex", Scope::Send, ChatFilter::all(), 0, None);
    let who = guard.identify("codex", "token-de-prueba");
    let pin = whatfast_protocol::rpc_request::Request::SetChatPin(SetChatPinRequest {
        chat_id: "5491100000000@s.whatsapp.net".into(),
        pinned: true,
    });
    assert_eq!(
        guard.check(&who, &pin, &store).expect_err("no pace").code,
        "forbidden"
    );
}

#[test]
fn typing_needs_the_send_scope() {
    let (_profile, store, guard) = profile("typing");
    connect(&guard, "codex", Scope::Draft, ChatFilter::all(), 0, None);
    let who = guard.identify("codex", "token-de-prueba");
    let typing = whatfast_protocol::rpc_request::Request::SetTyping(SetTypingRequest {
        chat_id: "5491100000000@s.whatsapp.net".into(),
        typing: true,
    });
    assert_eq!(
        guard.check(&who, &typing, &store).expect_err("no").code,
        "forbidden"
    );
    assert!(
        guard
            .check(&who, &list_messages("5491100000000@s.whatsapp.net"), &store)
            .is_ok()
    );
}

#[test]
fn the_lists_are_trimmed_to_the_grant() {
    let (_profile, _store, guard) = profile("trim");
    connect(
        &guard,
        "codex",
        Scope::Read,
        ChatFilter::only(vec!["5491100000000@s.whatsapp.net".into()]),
        0,
        None,
    );
    let who = guard.identify("codex", "token-de-prueba");
    let mut response = RpcResponse {
        result: Some(rpc_response::Result::ListChats(
            whatfast_protocol::ListChatsResponse {
                chats: vec![
                    whatfast_protocol::Chat {
                        id: "5491100000000@s.whatsapp.net".into(),
                        transport_jid: "5491100000000@s.whatsapp.net".into(),
                        ..Default::default()
                    },
                    whatfast_protocol::Chat {
                        id: "120363169271016023@g.us".into(),
                        transport_jid: "120363169271016023@g.us".into(),
                        ..Default::default()
                    },
                ],
                total_count: 2,
                next_cursor: String::new(),
            },
        )),
    };
    guard.filter(&who, &mut response);
    let Some(rpc_response::Result::ListChats(list)) = response.result else {
        panic!("the answer has to stay a list");
    };
    assert_eq!(list.chats.len(), 1);
    assert_eq!(list.chats[0].id, "5491100000000@s.whatsapp.net");
    assert_eq!(list.total_count, 1);
}

#[test]
fn the_user_gets_the_whole_list() {
    let (_profile, _store, guard) = profile("whole");
    let mut response = RpcResponse {
        result: Some(rpc_response::Result::ListChats(
            whatfast_protocol::ListChatsResponse {
                chats: vec![whatfast_protocol::Chat {
                    id: "a".into(),
                    ..Default::default()
                }],
                total_count: 1,
                next_cursor: String::new(),
            },
        )),
    };
    guard.filter(&Who::User, &mut response);
    match response.result {
        Some(rpc_response::Result::ListChats(list)) => assert_eq!(list.chats.len(), 1),
        _ => panic!("expected the list"),
    }
}

#[test]
fn a_read_grant_reads_the_list() {
    let (_profile, store, guard) = profile("list-read");
    connect(&guard, "codex", Scope::Read, ChatFilter::all(), 0, None);
    let who = guard.identify("codex", "token-de-prueba");
    let request = whatfast_protocol::rpc_request::Request::ListChats(ListChatsRequest::default());
    assert!(guard.check(&who, &request, &store).is_ok());
}

#[test]
fn an_info_request_of_a_chat_follows_the_grant() {
    let (_profile, store, guard) = profile("info");
    connect(
        &guard,
        "codex",
        Scope::Read,
        ChatFilter::only(vec!["120363169271016023@g.us".into()]),
        0,
        None,
    );
    let who = guard.identify("codex", "token-de-prueba");
    // The group of the list answers, and the direct chat that is not in it
    // does not.
    let allowed = whatfast_protocol::rpc_request::Request::GetChatInfo(GetChatInfoRequest {
        chat_id: "120363169271016023@g.us".into(),
    });
    assert!(guard.check(&who, &allowed, &store).is_ok());
    let direct = whatfast_protocol::rpc_request::Request::GetChatInfo(GetChatInfoRequest {
        chat_id: "5491100000000@s.whatsapp.net".into(),
    });
    assert_eq!(
        guard.check(&who, &direct, &store).expect_err("absent").code,
        "forbidden"
    );
}

#[test]
fn a_revoked_agent_cannot_come_back() {
    let (_profile, _store, guard) = profile("revoked");
    connect(&guard, "codex", Scope::Send, ChatFilter::all(), 0, None);
    guard.db().revoke("codex", 2_000).expect("revoked");
    assert!(matches!(
        guard.identify("codex", "token-de-prueba"),
        Who::Denied(_)
    ));
}

#[test]
fn the_calls_of_an_agent_are_logged() {
    let (_profile, store, guard) = profile("log");
    connect(&guard, "codex", Scope::Read, ChatFilter::all(), 0, None);
    let who = guard.identify("codex", "token-de-prueba");
    guard.log(
        &who,
        &whatfast_auto::Call {
            agent_id: "codex".into(),
            tool: "messages_read".into(),
            chat_id: Some("5491100000000@s.whatsapp.net".into()),
            ok: true,
            ms: 3,
            bytes: 42,
            detail: None,
        },
        5_000,
    );
    // The log of the user is not written for the desktop itself.
    guard.log(
        &Who::User,
        &whatfast_auto::Call {
            agent_id: "usuario".into(),
            tool: "messages_read".into(),
            chat_id: None,
            ok: true,
            ms: 1,
            bytes: 1,
            detail: None,
        },
        5_001,
    );
    let calls = guard.db().calls("codex", 10).expect("read");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].tool, "messages_read");
    assert_eq!(guard.db().calls("usuario", 10).expect("read").len(), 0);
    assert!(
        guard
            .check(&who, &list_messages("5491100000000@s.whatsapp.net"), &store)
            .is_ok()
    );
}

#[test]
fn an_approved_draft_ends_sent_or_failed() {
    use whatfast_protocol::{
        DraftDecideRequest, DraftFinishRequest, DraftProposeRequest, rpc_response,
    };
    use whatfast_wa::drafts;

    let (_profile, store, guard) = profile("finish");
    connect(&guard, "codex", Scope::Draft, ChatFilter::all(), 0, None);
    let who = guard.identify("codex", "token-de-prueba");

    let proposed = drafts::propose(
        &guard,
        &store,
        &who,
        &DraftProposeRequest {
            chat_id: "5491100000000@s.whatsapp.net".into(),
            text: "lo veo y te digo".into(),
            reply_to_message_id: String::new(),
            reason: "responde la pregunta".into(),
            confidence: 0.8,
            agent_id: String::new(),
        },
        now_ms(),
    )
    .expect("propose");
    let Some(rpc_response::Result::DraftPropose(proposed)) = proposed.result else {
        panic!("expected a draft");
    };
    let id = proposed.draft_id;

    // A draft that was not approved cannot be finished: the window sends what
    // the user approved, nothing else.
    let refused = drafts::finish(
        &guard,
        &store,
        &who,
        &DraftFinishRequest {
            draft_id: id,
            sent: true,
            error: String::new(),
        },
    )
    .expect_err("not approved");
    assert_eq!(refused.code, "failed_precondition");

    let approved = drafts::decide(
        &guard,
        &store,
        &who,
        &DraftDecideRequest {
            draft_id: id,
            send: true,
        },
    )
    .expect("decide");
    let Some(rpc_response::Result::DraftDecide(approved)) = approved.result else {
        panic!("expected a draft");
    };
    assert_eq!(approved.draft.expect("draft").state, "approved");

    let finished = drafts::finish(
        &guard,
        &store,
        &Who::User,
        &DraftFinishRequest {
            draft_id: id,
            sent: true,
            error: String::new(),
        },
    )
    .expect("finish");
    let Some(rpc_response::Result::DraftFinish(finished)) = finished.result else {
        panic!("expected a draft");
    };
    assert_eq!(finished.draft.expect("draft").state, "sent");

    // And the same draft, once sent, is not finished twice.
    let twice = drafts::finish(
        &guard,
        &store,
        &Who::User,
        &DraftFinishRequest {
            draft_id: id,
            sent: false,
            error: "no se pudo".into(),
        },
    )
    .expect_err("already sent");
    assert_eq!(twice.code, "failed_precondition");
}
