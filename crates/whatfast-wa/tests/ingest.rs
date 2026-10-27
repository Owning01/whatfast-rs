//! The ingestion's own tests: the live reducer the store drives (unread,
//! forward-only preview, edits and revokes), the history walk, receipts and
//! presence. Each one pins a rule the Go sidecar's oracle enforced.

fn temp_store(label: &str) -> (std::path::PathBuf, whatfast_wa::db::Store) {
    let dir = std::env::temp_dir().join(format!(
        "whatfast-wa-ingest-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let store = whatfast_wa::db::Store::open(&dir.join("chats.db")).unwrap();
    (dir, store)
}

use std::sync::Arc;

use wacore::types::events::{BatchOrigin, ChatPresenceUpdate, Event, InboundMessage, MessageBatch};
use wacore::types::message::{MessageInfo, MessageSource};
use wacore::types::presence::{ChatActivity, ReceiptType};
use whatfast_protocol::MessageStatus;
use whatfast_wa::db::Store;
use whatfast_wa::identity::Identity;
use whatsapp_rust::waproto::buffa::MessageField;
use whatsapp_rust::waproto::whatsapp as wa;

fn info(id: &str, from_me: bool, timestamp_ms: i64) -> MessageInfo {
    MessageInfo {
        source: MessageSource {
            chat: "120363000000000001@g.us".parse().unwrap(),
            sender: "5491100000001@s.whatsapp.net".parse().unwrap(),
            is_from_me: from_me,
            is_group: true,
            ..Default::default()
        },
        id: id.into(),
        timestamp: whatsapp_rust::chrono::DateTime::from_timestamp_millis(timestamp_ms).unwrap(),
        ..Default::default()
    }
}

fn text_message(text: &str) -> wa::Message {
    let mut message = wa::Message::default();
    message.conversation = Some(text.to_owned());
    message
}

fn inbound(id: &str, text: &str, from_me: bool, timestamp_ms: i64) -> InboundMessage {
    InboundMessage::builder()
        .message(Arc::new(text_message(text)))
        .info(Arc::new(info(id, from_me, timestamp_ms)))
        .build()
}

fn event_of(messages: Vec<InboundMessage>) -> Event {
    Event::Messages(
        MessageBatch::builder()
            .messages(messages.into())
            .origin(BatchOrigin::Live)
            .build(),
    )
}

#[test]
fn a_live_inbound_message_grows_the_unread_counter_and_points_the_chat() {
    let (dir, store) = temp_store("unread");
    let identity = Identity::open(&dir);
    let (events, _ingested) = whatfast_wa::ingest::apply_event(
        &store,
        &identity,
        &event_of(vec![inbound("m1", "hello", false, 1_700_000_000_000)]),
    );
    let chat = store
        .chat("120363000000000001@g.us")
        .unwrap()
        .expect("the chat exists");
    assert_eq!(chat.unread_count, 1);
    assert_eq!(chat.last_message_id, "m1");
    assert_eq!(chat.last_message_text, "hello");
    assert_eq!(chat.last_message_at, 1_700_000_000_000);
    let stored = store
        .message("120363000000000001@g.us", "m1")
        .unwrap()
        .expect("the row exists");
    assert_eq!(stored.kind, "text");
    assert_eq!(stored.text, "hello");
    assert!(stored.unread);
    // One chat event and one message event.
    assert_eq!(events.len(), 2);
}

#[test]
fn an_out_of_order_message_does_not_drag_the_preview_or_the_pointer_back() {
    let (dir, store) = temp_store("order");
    let identity = Identity::open(&dir);
    whatfast_wa::ingest::apply_event(
        &store,
        &identity,
        &event_of(vec![inbound("new", "the newest", false, 2_000)]),
    );
    whatfast_wa::ingest::apply_event(
        &store,
        &identity,
        &event_of(vec![inbound("old", "a late arrival", false, 1_000)]),
    );
    let chat = store.chat("120363000000000001@g.us").unwrap().unwrap();
    assert_eq!(chat.last_message_id, "new");
    assert_eq!(chat.last_message_text, "the newest");
    assert_eq!(chat.last_message_at, 2_000);
    // The late row still counts as one more unread message.
    assert_eq!(chat.unread_count, 2);
    let late = store
        .message("120363000000000001@g.us", "old")
        .unwrap()
        .expect("the late row landed");
    assert_eq!(late.timestamp, 1_000);
}

#[test]
fn a_replayed_row_grows_the_counter_once() {
    let (dir, store) = temp_store("replay");
    let identity = Identity::open(&dir);
    for _ in 0..2 {
        whatfast_wa::ingest::apply_event(
            &store,
            &identity,
            &event_of(vec![inbound("m1", "hello", false, 1_700_000_000_000)]),
        );
    }
    let chat = store.chat("120363000000000001@g.us").unwrap().unwrap();
    assert_eq!(chat.unread_count, 1);
}

#[test]
fn our_own_message_never_grows_the_counter() {
    let (dir, store) = temp_store("mine");
    let identity = Identity::open(&dir);
    whatfast_wa::ingest::apply_event(
        &store,
        &identity,
        &event_of(vec![inbound("m1", "mine", true, 1_700_000_000_000)]),
    );
    let chat = store.chat("120363000000000001@g.us").unwrap().unwrap();
    assert_eq!(chat.unread_count, 0);
    assert_eq!(chat.last_message_id, "m1");
}

#[test]
fn an_edit_event_reaches_the_desktop() {
    let (dir, store) = temp_store("edit");
    let identity = Identity::open(&dir);
    whatfast_wa::ingest::apply_event(
        &store,
        &identity,
        &event_of(vec![inbound("m1", "first", false, 1_700_000_000_000)]),
    );

    let mut edited = wa::Message::default();
    edited.conversation = Some("corrected".into());
    let mut key = wa::MessageKey::default();
    key.id = Some("m1".into());
    let mut protocol = wa::message::ProtocolMessage::default();
    protocol.r#type = Some(wa::message::protocol_message::Type::MessageEdit);
    protocol.key = MessageField::some(key);
    protocol.timestamp_ms = Some(1_700_000_000_500);
    protocol.edited_message = MessageField::some(edited);
    let mut wrapper = wa::Message::default();
    wrapper.protocol_message = MessageField::some(protocol);
    let mut information = info("edit-event", false, 1_700_000_000_500);
    information.edit = wacore::types::message::EditAttribute::MessageEdit;
    let envelope = InboundMessage::builder()
        .message(Arc::new(wrapper))
        .info(Arc::new(information))
        .build();

    let (events, _ingested) =
        whatfast_wa::ingest::apply_event(&store, &identity, &event_of(vec![envelope]));
    assert!(
        events.iter().any(|event| matches!(
            event.event,
            Some(whatfast_protocol::backend_event::Event::MessageUpserted(_))
        )),
        "a merged edit still has to be announced"
    );
    let stored = store
        .message("120363000000000001@g.us", "m1")
        .unwrap()
        .expect("the original row");
    assert_eq!(stored.text, "corrected");
    assert_eq!(stored.edited_at, 1_700_000_000_500);
    // The edit did not move the timeline.
    assert_eq!(stored.timestamp, 1_700_000_000_000);
}

#[test]
fn a_signal_only_envelope_leaves_no_row() {
    let (dir, store) = temp_store("passive");
    let identity = Identity::open(&dir);
    let mut poll_vote = wa::Message::default();
    poll_vote.poll_update_message = MessageField::some(Default::default());
    let envelope = InboundMessage::builder()
        .message(Arc::new(poll_vote))
        .info(Arc::new(info("vote", false, 1_700_000_000_000)))
        .build();
    let (events, _ingested) =
        whatfast_wa::ingest::apply_event(&store, &identity, &event_of(vec![envelope]));
    assert!(events.is_empty());
    assert!(
        store
            .message("120363000000000001@g.us", "vote")
            .unwrap()
            .is_none()
    );
}

fn history_sync(conversation: wa::Conversation) -> wa::HistorySync {
    let mut parsed = wa::HistorySync::default();
    parsed.conversations = vec![conversation];
    parsed
}

fn history_message(id: &str, text: &str, at_ms: i64) -> wa::HistorySyncMsg {
    let mut key = wa::MessageKey::default();
    key.remote_jid = Some("120363000000000001@g.us".into());
    key.id = Some(id.into());
    key.from_me = Some(false);
    key.participant = Some("5491100000001@s.whatsapp.net".into());
    let mut web = wa::WebMessageInfo::default();
    web.key = MessageField::some(key);
    web.message = MessageField::some(text_message(text));
    web.message_timestamp = Some(at_ms as u64 / 1000);
    let mut entry = wa::HistorySyncMsg::default();
    entry.message = MessageField::some(web);
    entry
}

#[test]
fn the_history_lands_the_group_name_the_archive_flag_and_the_rows() {
    let (dir, store) = temp_store("history");
    let identity = Identity::open(&dir);
    let mut conversation = wa::Conversation::default();
    conversation.id = "120363000000000001@g.us".into();
    conversation.name = Some("The group".into());
    conversation.archived = Some(true);
    conversation.unread_count = Some(7);
    conversation.messages = vec![history_message("h1", "from history", 1_700_000_000_000)];
    let parsed = history_sync(conversation);

    whatfast_wa::ingest::learn_history_contacts(&store, &parsed);
    let conversations = whatfast_wa::ingest::history_conversations(&parsed);
    assert_eq!(conversations.len(), 1);
    for conversation in &conversations {
        whatfast_wa::ingest::store_conversation(&store, &identity, conversation).unwrap();
    }

    let chat = store.chat("120363000000000001@g.us").unwrap().unwrap();
    assert_eq!(chat.name, "The group");
    assert!(chat.archived);
    // The oracle ignores the conversation's unreadCount.
    assert_eq!(chat.unread_count, 0);
    assert_eq!(chat.last_message_id, "h1");
    assert_eq!(chat.last_message_text, "from history");
    let stored = store
        .message("120363000000000001@g.us", "h1")
        .unwrap()
        .expect("the history row");
    assert!(!stored.unread);
    assert_eq!(stored.timestamp, 1_700_000_000_000);
}

#[test]
fn the_history_aggregates_land_on_the_parent_message() {
    let (dir, store) = temp_store("history-reactions");
    let mut entry = history_message("h1", "reacted", 1_700_000_000_000);
    let mut key = wa::MessageKey::default();
    key.participant = Some("5491100000002@s.whatsapp.net".into());
    key.from_me = Some(false);
    let mut reaction = wa::Reaction::default();
    reaction.key = MessageField::some(key);
    reaction.text = Some("❤️".into());
    reaction.sender_timestamp_ms = Some(1_700_000_000_900);
    if let Some(web) = entry.message.as_option_mut() {
        web.reactions = vec![reaction];
    }
    let mut conversation = wa::Conversation::default();
    conversation.id = "120363000000000001@g.us".into();
    conversation.messages = vec![entry];
    let parsed = history_sync(conversation);

    whatfast_wa::ingest::learn_history_contacts(&store, &parsed);
    let conversations = whatfast_wa::ingest::history_conversations(&parsed);
    for conversation in &conversations {
        whatfast_wa::ingest::store_conversation(&store, &Identity::open(&dir), conversation)
            .unwrap();
    }
    let reactions = store
        .read(|conn| {
            let mut statement = conn.prepare(
                "SELECT sender_jid,emoji,timestamp FROM reactions WHERE chat_jid=?1 AND message_id=?2",
            )?;
            let rows = statement
                .query_map(
                    rusqlite::params!["120363000000000001@g.us", "h1"],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, i64>(2)?,
                        ))
                    },
                )?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .unwrap();
    assert_eq!(
        reactions,
        vec![(
            "5491100000002@s.whatsapp.net".to_string(),
            "❤️".to_string(),
            1_700_000_000_900
        )]
    );
}

#[test]
fn a_reaction_keeps_its_own_time_and_one_row_per_sender() {
    let (dir, store) = temp_store("reaction");
    let identity = Identity::open(&dir);
    let chat = "120363000000000001@g.us";
    let player = |text: &str, stamp: i64, from_me: bool| {
        let mut key = wa::MessageKey::default();
        key.id = Some("target".into());
        let mut reaction = wa::message::ReactionMessage::default();
        reaction.key = MessageField::some(key);
        reaction.text = Some(text.to_owned());
        reaction.sender_timestamp_ms = Some(stamp);
        let mut content = wa::Message::default();
        content.reaction_message = MessageField::some(reaction);
        let envelope = InboundMessage::builder()
            .message(Arc::new(content))
            .info(Arc::new(info("reaction-event", from_me, 1_000)))
            .build();
        whatfast_wa::ingest::apply_event(&store, &identity, &event_of(vec![envelope]))
    };

    let (events, _) = player("❤️", 5_000, false);
    assert_eq!(events.len(), 1);
    match events[0].event.as_ref().unwrap() {
        whatfast_protocol::backend_event::Event::ReactionUpdated(updated) => {
            assert!(!updated.removed);
            assert_eq!(updated.reaction.as_ref().unwrap().timestamp_ms, 5_000);
        }
        other => panic!("expected a reaction, got {other:?}"),
    }
    // The reaction created the conversation before any message arrived.
    assert!(store.chat(chat).unwrap().is_some());

    // An empty emoji removes it; the row keeps the tombstone.
    let (events, _) = player("", 6_000, false);
    match events[0].event.as_ref().unwrap() {
        whatfast_protocol::backend_event::Event::ReactionUpdated(updated) => {
            assert!(updated.removed);
        }
        other => panic!("expected a reaction, got {other:?}"),
    }
    let reactions = store
        .read(|conn| {
            let mut statement = conn.prepare(
                "SELECT sender_jid,emoji,timestamp FROM reactions WHERE chat_jid=?1 AND message_id=?2",
            )?;
            let rows = statement
                .query_map(rusqlite::params![chat, "target"], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .unwrap();
    assert_eq!(
        reactions,
        vec![(
            "5491100000001@s.whatsapp.net".to_string(),
            String::new(),
            6_000
        )]
    );
}

fn receipt(r#type: ReceiptType, from_me: bool, ids: &[&str]) -> Event {
    let receipt = wacore::types::events::Receipt::builder()
        .source(MessageSource {
            chat: "120363000000000001@g.us".parse().unwrap(),
            sender: "5491100000001@s.whatsapp.net".parse().unwrap(),
            is_from_me: from_me,
            is_group: true,
            ..Default::default()
        })
        .message_ids(ids.iter().map(|id| (*id).into()).collect())
        .timestamp(
            whatsapp_rust::chrono::DateTime::from_timestamp_millis(1_700_000_001_000).unwrap(),
        )
        .r#type(r#type)
        .offline(false)
        .build();
    Event::Receipt(receipt)
}

fn store_mine(store: &Store) {
    use whatfast_wa::db::MessageRow;
    store.ensure_chat("120363000000000001@g.us", "").unwrap();
    store
        .insert_message(&MessageRow {
            id: "out".into(),
            chat_jid: "120363000000000001@g.us".into(),
            transport_jid: "120363000000000001@g.us".into(),
            text: "sent".into(),
            timestamp: 1_700_000_000_000,
            from_me: true,
            status: MessageStatus::Sent as i64,
            kind: "text".into(),
            ..MessageRow::default()
        })
        .unwrap();
}

#[test]
fn a_read_receipt_moves_the_status_and_a_played_one_reports_a_delivery() {
    let (dir, store) = temp_store("receipt");
    let identity = Identity::open(&dir);
    store_mine(&store);

    let (events, _) = whatfast_wa::ingest::apply_event(
        &store,
        &identity,
        &receipt(ReceiptType::Read, false, &["out"]),
    );
    assert_eq!(events.len(), 1);
    let stored = store
        .message("120363000000000001@g.us", "out")
        .unwrap()
        .unwrap();
    assert_eq!(stored.status, MessageStatus::Read as i64);

    let (events, _) = whatfast_wa::ingest::apply_event(
        &store,
        &identity,
        &receipt(ReceiptType::Played, false, &["out"]),
    );
    assert_eq!(events.len(), 1);
    match events[0].event.as_ref().unwrap() {
        whatfast_protocol::backend_event::Event::ReceiptUpdated(updated) => {
            assert_eq!(updated.status, MessageStatus::Delivered as i32);
        }
        other => panic!("expected a receipt, got {other:?}"),
    }
    // A played receipt never walks a read row backwards.
    let stored = store
        .message("120363000000000001@g.us", "out")
        .unwrap()
        .unwrap();
    assert_eq!(stored.status, MessageStatus::Read as i64);
}

#[test]
fn a_read_from_another_device_is_not_a_peer_status() {
    let (dir, store) = temp_store("receipt-self");
    let identity = Identity::open(&dir);
    store_mine(&store);
    let (events, _) = whatfast_wa::ingest::apply_event(
        &store,
        &identity,
        &receipt(ReceiptType::ReadSelf, true, &["out"]),
    );
    assert!(events.is_empty());
    let stored = store
        .message("120363000000000001@g.us", "out")
        .unwrap()
        .unwrap();
    assert_eq!(stored.status, MessageStatus::Sent as i64);
}

fn presence(from_me: bool, state: ChatActivity) -> Event {
    Event::ChatPresence(
        ChatPresenceUpdate::builder()
            .source(MessageSource {
                chat: "120363000000000001@g.us".parse().unwrap(),
                sender: "5491100000001@s.whatsapp.net".parse().unwrap(),
                is_from_me: from_me,
                is_group: true,
                ..Default::default()
            })
            .state(state)
            .build(),
    )
}

#[test]
fn typing_and_recording_reach_the_chat_and_our_own_state_does_not() {
    let (dir, store) = temp_store("presence");
    let identity = Identity::open(&dir);
    let (events, _) =
        whatfast_wa::ingest::apply_event(&store, &identity, &presence(false, ChatActivity::Typing));
    assert_eq!(events.len(), 1);
    match events[0].event.as_ref().unwrap() {
        whatfast_protocol::backend_event::Event::TypingChanged(changed) => {
            assert_eq!(changed.chat_id, "120363000000000001@g.us");
            assert!(changed.typing);
            assert!(!changed.recording);
        }
        other => panic!("expected typing, got {other:?}"),
    }

    let (events, _) = whatfast_wa::ingest::apply_event(
        &store,
        &identity,
        &presence(false, ChatActivity::RecordingAudio),
    );
    match events[0].event.as_ref().unwrap() {
        whatfast_protocol::backend_event::Event::TypingChanged(changed) => {
            assert!(changed.typing);
            assert!(changed.recording);
        }
        other => panic!("expected typing, got {other:?}"),
    }

    let (events, _) =
        whatfast_wa::ingest::apply_event(&store, &identity, &presence(true, ChatActivity::Typing));
    assert!(events.is_empty());
}
