//! The whole chain with a registered agent: MCP server, local endpoint, guard.
//!
//! These tests start the real backend on a profile of their own, connect
//! through the local endpoint with the token the desktop writes, and ask for
//! what the grant allows and for what it does not. Nothing is sent to WhatsApp:
//! the conversation of the profile is local and has no session behind it, so a
//! send that the guard allows would still fail -which is why the tests only
//! check the calls that answer from the store, and the ones the guard refuses
//! before they reach the store.

use std::path::{Path, PathBuf};
use std::time::Duration;

use whatfast_auto::{ChatFilter, Db, Grant, Scope};
use whatfast_mcp::client::Client;
use whatfast_protocol::{
    GetChatInfoRequest, ListChatsRequest, ListMessagesRequest, SendTextRequest, rpc_request,
};

/// The backend of this workspace, next to the test binaries.
fn backend() -> PathBuf {
    let exe = std::env::current_exe().expect("the path of the test");
    // `target/debug/deps/permisos-<hash>.exe` -> `target/debug/whatfast-wa.exe`.
    let debug = exe
        .parent()
        .and_then(Path::parent)
        .expect("the target directory");
    let name = if cfg!(windows) {
        "whatfast-wa.exe"
    } else {
        "whatfast-wa"
    };
    debug.join(name)
}

/// A profile of its own with one chat and one message.
struct Profile {
    dir: PathBuf,
}

impl Profile {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "whatfast-permisos-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).expect("profile");
        let store = whatfast_wa::db::Store::open(&dir.join("chats.db")).expect("chats.db");
        store
            .ensure_chat(
                "5491100000000@s.whatsapp.net",
                "5491100000000@s.whatsapp.net",
            )
            .expect("chat");
        store
            .ensure_chat("120363169271016023@g.us", "120363169271016023@g.us")
            .expect("group");
        Self { dir }
    }

    /// Registers the agent with its token, the way the screen of agents will.
    fn connect(&self, agent: &str, scope: Scope, chats: ChatFilter, per_hour: u16) {
        let db = Db::open(&self.dir.join("agents.db")).expect("agents.db");
        let grant = Grant {
            scope,
            chats,
            per_hour,
            // No hours: the test does not depend on the clock of the machine.
            window: None,
            // No waiting: the tests are not about the pace.
            humanize: false,
        };
        db.connect_agent(agent, agent, &grant, "token-de-prueba", 1_000)
            .expect("registered");
        std::fs::create_dir_all(self.dir.join("agents")).expect("agents folder");
        std::fs::write(
            self.dir.join("agents").join(format!("{agent}.token")),
            "token-de-prueba\n",
        )
        .expect("token file");
    }

    /// Forgets the token of an agent, as a disconnected one.
    fn drop_token(&self, agent: &str) {
        let path = self.dir.join("agents").join(format!("{agent}.token"));
        let _ = std::fs::remove_file(path);
    }
}

impl Drop for Profile {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// The client of one agent on this profile.
fn client(profile: &Profile, agent: &str) -> Client {
    Client::new(profile.dir.clone(), backend(), agent).expect("client")
}

fn chats_list() -> rpc_request::Request {
    rpc_request::Request::ListChats(ListChatsRequest {
        cursor: String::new(),
        limit: 20,
    })
}

fn messages_read(chat_id: &str) -> rpc_request::Request {
    rpc_request::Request::ListMessages(ListMessagesRequest {
        chat_id: chat_id.to_owned(),
        before_timestamp_ms: 0,
        before_message_id: String::new(),
        limit: 5,
    })
}

fn chat_info(chat_id: &str) -> rpc_request::Request {
    rpc_request::Request::GetChatInfo(GetChatInfoRequest {
        chat_id: chat_id.to_owned(),
    })
}

fn send(chat_id: &str) -> rpc_request::Request {
    rpc_request::Request::SendText(SendTextRequest {
        client_message_id: "3f0f7f0e-0000-4000-8000-000000000001".into(),
        chat_id: chat_id.to_owned(),
        text: "mensaje de prueba".into(),
        ..Default::default()
    })
}

#[test]
fn a_read_grant_reads_and_cannot_send() {
    let profile = Profile::new("read");
    profile.connect(
        "codex",
        Scope::Read,
        ChatFilter::only(vec!["5491100000000@s.whatsapp.net".into()]),
        20,
    );
    let mut client = client(&profile, "codex");
    let chats = client
        .call(chats_list(), Duration::from_secs(30))
        .expect("the list of chats answers");
    let whatfast_protocol::rpc_response::Result::ListChats(chats) = chats else {
        panic!("expected the list of chats");
    };
    // The grant names one chat and the list is trimmed to it.
    assert_eq!(chats.chats.len(), 1, "the list is the grant");
    assert_eq!(chats.chats[0].transport_jid, "5491100000000@s.whatsapp.net");

    client
        .call(
            messages_read("5491100000000@s.whatsapp.net"),
            Duration::from_secs(30),
        )
        .expect("the messages of the chat answer");

    let error = client
        .call(
            messages_read("120363169271016023@g.us"),
            Duration::from_secs(30),
        )
        .expect_err("the group is outside the grant");
    assert_eq!(error.code(), "forbidden");

    let error = client
        .call(
            send("5491100000000@s.whatsapp.net"),
            Duration::from_secs(30),
        )
        .expect_err("a read grant does not send");
    assert_eq!(error.code(), "forbidden");
    client.shutdown();
}

#[test]
fn a_wrong_token_is_turned_away() {
    let profile = Profile::new("token");
    profile.connect("codex", Scope::Read, ChatFilter::all(), 20);
    std::fs::write(
        profile.dir.join("agents").join("codex.token"),
        "otro-token\n",
    )
    .expect("token file");
    let mut client = client(&profile, "codex");
    let error = client
        .call(chats_list(), Duration::from_secs(30))
        .expect_err("the token does not match");
    assert!(
        matches!(error.code(), "unauthorized" | "handshake_required"),
        "unexpected code {}",
        error.code()
    );
    client.shutdown();
}

#[test]
fn an_agent_the_profile_does_not_know_is_turned_away() {
    let profile = Profile::new("unknown");
    profile.connect("codex", Scope::Read, ChatFilter::all(), 20);
    // `cursor` has no row and no token: the profile has an agent, so a harness
    // that names itself without being connected is not the user.
    let mut client = client(&profile, "cursor");
    let error = client
        .call(chats_list(), Duration::from_secs(30))
        .expect_err("an unknown agent cannot read");
    assert!(
        matches!(error.code(), "unauthorized" | "handshake_required"),
        "unexpected code {}",
        error.code()
    );
    client.shutdown();
}

#[test]
fn the_chat_info_of_a_trimmed_chat_still_answers() {
    let profile = Profile::new("info");
    profile.connect(
        "codex",
        Scope::Read,
        ChatFilter::only(vec!["5491100000000@s.whatsapp.net".into()]),
        20,
    );
    let mut client = client(&profile, "codex");
    client
        .call(
            chat_info("5491100000000@s.whatsapp.net"),
            Duration::from_secs(30),
        )
        .expect("the chat of the grant answers");
    let error = client
        .call(
            chat_info("120363169271016023@g.us"),
            Duration::from_secs(30),
        )
        .expect_err("the group is outside the grant");
    assert_eq!(error.code(), "forbidden");
    client.shutdown();
}

#[test]
fn a_revoked_agent_does_not_connect() {
    let profile = Profile::new("revoked");
    profile.connect("codex", Scope::Send, ChatFilter::all(), 20);
    let db = Db::open(&profile.dir.join("agents.db")).expect("agents.db");
    db.revoke("codex", 2_000).expect("revoked");
    let mut client = client(&profile, "codex");
    let error = client
        .call(chats_list(), Duration::from_secs(30))
        .expect_err("a revoked agent is out");
    assert!(
        matches!(error.code(), "unauthorized" | "handshake_required"),
        "unexpected code {}",
        error.code()
    );
    client.shutdown();
}

/// A profile with no agents registered still serves the user: nothing to add
/// here, the guard treats the harness as the user who started it.
#[test]
fn a_profile_without_agents_still_serves_the_user() {
    let profile = Profile::new("bootstrap");
    // No `agents.db` row at all: the harness is the user who started it, which
    // is what a fresh install does before the screen of agents exists.
    let mut client = client(&profile, "probar");
    let chats = client
        .call(chats_list(), Duration::from_secs(30))
        .expect("the user reads");
    let whatfast_protocol::rpc_response::Result::ListChats(chats) = chats else {
        panic!("expected the list of chats");
    };
    assert_eq!(chats.chats.len(), 2);
    client.shutdown();
    profile.drop_token("probar");
}

#[test]
fn a_draft_waits_for_the_user() {
    let profile = Profile::new("draft");
    profile.connect(
        "codex",
        Scope::Draft,
        ChatFilter::only(vec!["5491100000000@s.whatsapp.net".into()]),
        20,
    );
    let mut client = client(&profile, "codex");
    // The tool layer: `message_send` in a chat without the send scope leaves a
    // draft instead of failing.
    let sent = whatfast_mcp::tools::call(
        &mut client,
        "message_send",
        &serde_json::json!({
            "chat_id": "5491100000000@s.whatsapp.net",
            "text": "hola, te paso el precio"
        }),
    )
    .expect("the message waits as a draft");
    let draft_id = sent["draft_id"].as_i64().expect("a draft id");
    assert_eq!(sent["state"], "proposed");

    let listed = whatfast_mcp::tools::call(&mut client, "drafts_list", &serde_json::json!({}))
        .expect("the tray lists");
    assert_eq!(listed["count"], 1);
    assert_eq!(listed["drafts"][0]["draft_id"], draft_id);
    assert_eq!(
        listed["drafts"][0]["chat_id"],
        "5491100000000@s.whatsapp.net"
    );

    let decided = whatfast_mcp::tools::call(
        &mut client,
        "draft_decide",
        &serde_json::json!({ "draft_id": draft_id, "send": true }),
    )
    .expect("the user approves");
    assert_eq!(decided["state"], "approved");

    // An approved draft is out of the open tray.
    let open = whatfast_mcp::tools::call(
        &mut client,
        "drafts_list",
        &serde_json::json!({ "state": "proposed" }),
    )
    .expect("the tray lists");
    assert_eq!(open["count"], 0);
    client.shutdown();
}

#[test]
fn a_draft_of_another_agent_is_not_decided() {
    let profile = Profile::new("ajeno");
    profile.connect("codex", Scope::Draft, ChatFilter::all(), 20);
    profile.connect("cursor", Scope::Draft, ChatFilter::all(), 20);
    let db = Db::open(&profile.dir.join("agents.db")).expect("agents.db");
    let id = db
        .add_draft(&whatfast_auto::Draft {
            id: 0,
            agent_id: "codex".into(),
            chat_id: "5491100000000@s.whatsapp.net".into(),
            reply_to: None,
            text: "del otro".into(),
            reason: None,
            confidence: None,
            state: whatfast_auto::DraftState::Proposed,
            at_ms: 1_000,
        })
        .expect("draft");
    let mut client = client(&profile, "cursor");
    let error = whatfast_mcp::tools::call(
        &mut client,
        "draft_decide",
        &serde_json::json!({ "draft_id": id, "send": false }),
    )
    .expect_err("another agent's draft is not mine");
    assert!(
        error.contains("otro agente") || error.contains("forbidden"),
        "{error}"
    );
    client.shutdown();
}

#[test]
fn a_draft_outside_the_grant_is_refused() {
    let profile = Profile::new("fuera");
    profile.connect(
        "codex",
        Scope::Draft,
        ChatFilter::only(vec!["5491100000000@s.whatsapp.net".into()]),
        20,
    );
    let mut client = client(&profile, "codex");
    let error = whatfast_mcp::tools::call(
        &mut client,
        "draft_propose",
        &serde_json::json!({
            "chat_id": "120363169271016023@g.us",
            "text": "hola grupo"
        }),
    )
    .expect_err("the group is outside the grant");
    assert!(error.contains("forbidden"), "{error}");
    client.shutdown();
}
