//! The identity reconciliation against a twin-chats profile: the same
//! person under a hidden-identity jid and a phone-number jid folds into one
//! chat, and the phone's agenda names take over the titles.

use std::sync::Arc;

use whatfast_wa::db::MessageRow;
use whatfast_wa::db::Store;
use whatfast_wa::identity::Identity;
use whatfast_wa::ingest::{HistoryConversation, store_conversation};

/// A profile directory with a fork-shaped session.db: the lid map and the
/// contacts table the fork maintains.
fn profile_with_identity(label: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "whatfast-wa-identity-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let session = rusqlite::Connection::open(dir.join("session.db")).unwrap();
    session
        .execute_batch(
            "CREATE TABLE whatsmeow_contacts(our_jid TEXT, their_jid TEXT, first_name TEXT,
                 full_name TEXT, push_name TEXT, business_name TEXT, redacted_phone TEXT);
             CREATE TABLE whatsmeow_lid_map(lid TEXT, pn TEXT);
             CREATE TABLE lid_pn_mapping(lid TEXT, phone_number TEXT);
             INSERT INTO whatsmeow_lid_map VALUES('162040542929138','5493816041868');
             INSERT INTO whatsmeow_contacts(their_jid, full_name, push_name)
                 VALUES('5493816041868@s.whatsapp.net','Julio Bonkosky','Julio');
             INSERT INTO whatsmeow_contacts(their_jid, full_name, push_name)
                 VALUES('5493816999999@s.whatsapp.net','','Cosi Solo Push');",
        )
        .unwrap();
    dir
}

#[tokio::test]
async fn twins_fold_into_the_phone_chat_and_the_agenda_wins() {
    let dir = profile_with_identity("fold");
    let store = Store::open(&dir.join("chats.db")).unwrap();
    let identity = Identity::open(&dir);

    // The history keyed the person by the phone; a live message later
    // arrived under the hidden identity.
    let phone_chat = "5493816041868@s.whatsapp.net";
    let lid_chat = "162040542929138@lid";
    store.ensure_chat(phone_chat, phone_chat).unwrap();
    store
        .insert_message(&MessageRow {
            id: "old".into(),
            chat_jid: phone_chat.into(),
            timestamp: 1_000,
            ..MessageRow::default()
        })
        .unwrap();
    store.ensure_chat(lid_chat, lid_chat).unwrap();
    store
        .insert_message(&MessageRow {
            id: "new".into(),
            chat_jid: lid_chat.into(),
            timestamp: 2_000,
            ..MessageRow::default()
        })
        .unwrap();
    // The store the reconcile pass takes is the same one; an Arc over it.
    let shared = Arc::new(Store::open(&dir.join("chats.db")).unwrap());
    let (events, _drain) = async_channel::unbounded();
    whatfast_wa::reconcile::reconcile(&shared, &identity, &events).await;

    // The second connection sees the folded profile.
    let store = Store::open(&dir.join("chats.db")).unwrap();
    assert!(
        store.chat(lid_chat).unwrap().is_none(),
        "the lid twin is gone"
    );
    let chat = store
        .chat(phone_chat)
        .unwrap()
        .expect("the phone chat lives");
    // Both messages live under the one chat, in order.
    let window = store
        .messages_around(phone_chat, "new", 10)
        .unwrap()
        .expect("the anchor message");
    assert_eq!(window.0.len(), 2);
    assert_eq!(window.0[0].id, "old");
    assert_eq!(window.0[1].id, "new");
    // The agenda name beats the push name in the title.
    assert_eq!(
        whatfast_wa::wire::wire_chat(&chat).title,
        "Julio Bonkosky".to_string()
    );
}

#[tokio::test]
async fn the_history_lands_in_the_canonical_chat_from_the_start() {
    let dir = profile_with_identity("history");
    let store = Store::open(&dir.join("chats.db")).unwrap();
    let identity = Identity::open(&dir);

    // The history keys this person by the hidden identity; the map knows
    // the phone number.
    let conversation = HistoryConversation {
        jid: "162040542929138@lid".into(),
        unread: 1,
        end_of_history_transfer: false,
        messages: vec![MessageRow {
            id: "hist".into(),
            chat_jid: "162040542929138@lid".into(),
            timestamp: 3_000,
            ..MessageRow::default()
        }],
    };
    let (event, stored) = store_conversation(&store, &identity, &conversation).unwrap();
    assert_eq!(stored, 1);
    assert!(store.chat("162040542929138@lid").unwrap().is_none());
    assert!(
        store
            .chat("5493816041868@s.whatsapp.net")
            .unwrap()
            .is_some()
    );
    // The event already names the canonical chat.
    let chat_event = event.event.unwrap();
    match chat_event {
        whatfast_protocol::backend_event::Event::ChatUpserted(chat) => {
            assert_eq!(
                chat.chat.expect("the chat payload").id,
                "5493816041868@s.whatsapp.net"
            );
        }
        other => panic!("expected a chat upsert, got {other:?}"),
    }
}

/// The reconciliation over a real profile copy. Ignored by default:
/// point WHATFAST_WA_PROFILE at a directory holding chats.db and
/// session.db and run with --ignored.
#[tokio::test]
#[ignore = "needs WHATFAST_WA_PROFILE"]
async fn the_real_profile_folds() {
    let dir = std::path::PathBuf::from(
        std::env::var("WHATFAST_WA_PROFILE").expect("WHATFAST_WA_PROFILE"),
    );
    let store = Arc::new(Store::open(&dir.join("chats.db")).unwrap());
    let identity = Identity::open(&dir);
    let before_lid = store.lid_chats().unwrap().len();
    let (events, drain) = async_channel::unbounded();
    tokio::spawn(async move { while drain.recv().await.is_ok() {} });
    whatfast_wa::reconcile::reconcile(&store, &identity, &events).await;
    let after_lid = store.lid_chats().unwrap().len();
    let named = store
        .read(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM chats c JOIN contacts ct ON ct.jid=c.jid
                 WHERE ct.contact_name<>''",
                [],
                |row| row.get::<_, i64>(0),
            )
        })
        .unwrap();
    println!("lid chats: {before_lid} -> {after_lid}; chats with an agenda name: {named}");
}
