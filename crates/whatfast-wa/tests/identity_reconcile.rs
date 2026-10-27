//! The identity reconciliation against twin-chat profiles: the same person
//! under a hidden-identity jid and a phone-number jid folds into one chat,
//! the phone's agenda names take over the titles, and the fold is announced
//! once.

use std::sync::Arc;

use whatfast_protocol::backend_event::Event;
use whatfast_wa::db::{ChatFlag, MessageRow, Store};
use whatfast_wa::identity::Identity;
use whatfast_wa::ingest::{history_conversations, store_conversation};
use whatsapp_rust::waproto::buffa::MessageField;
use whatsapp_rust::waproto::whatsapp as wa;

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

/// The events one pass left on the channel, with their payloads unwrapped.
fn take_events(receiver: &async_channel::Receiver<whatfast_protocol::BackendEvent>) -> Vec<Event> {
    let mut out = Vec::new();
    while let Ok(event) = receiver.try_recv() {
        if let Some(event) = event.event {
            out.push(event);
        }
    }
    out
}

/// The merged pairs one pass announced, oldest first.
fn merges(events: &[Event]) -> Vec<(String, String)> {
    events
        .iter()
        .filter_map(|event| match event {
            Event::ChatMerged(merged) => {
                Some((merged.old_chat_id.clone(), merged.new_chat_id.clone()))
            }
            _ => None,
        })
        .collect()
}

/// One stored text message, ready for an insert.
fn text_message(chat_jid: &str, id: &str, timestamp: i64, text: &str) -> MessageRow {
    MessageRow {
        id: id.into(),
        chat_jid: chat_jid.into(),
        timestamp,
        text: text.into(),
        ..MessageRow::default()
    }
}

/// One history-sync conversation with one text message, the way the chunk
/// carries it: the proto is the input of `history_conversations`, so a test
/// never pins the digested struct's field list.
fn history_with_one_message(jid: &str, id: &str, timestamp_seconds: u64) -> wa::HistorySync {
    let mut key = wa::MessageKey::default();
    key.remote_jid = Some(jid.to_owned());
    key.from_me = Some(false);
    key.id = Some(id.to_owned());
    key.participant = Some(jid.to_owned());

    let mut message = wa::Message::default();
    message.conversation = Some("hola desde el historial".into());

    let mut info = wa::WebMessageInfo::default();
    info.key = MessageField::some(key);
    info.message = MessageField::some(message);
    info.message_timestamp = Some(timestamp_seconds);

    let mut entry = wa::HistorySyncMsg::default();
    entry.message = MessageField::some(info);

    let mut conversation = wa::Conversation::default();
    conversation.id = jid.to_owned();
    conversation.unread_count = Some(1);
    conversation.end_of_history_transfer = Some(false);
    conversation.messages = vec![entry];

    let mut sync = wa::HistorySync::default();
    sync.sync_type = wa::history_sync::HistorySyncType::FULL;
    sync.conversations = vec![conversation];
    sync
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
        .insert_message(&text_message(phone_chat, "old", 1_000, "the oldest"))
        .unwrap();
    store.ensure_chat(lid_chat, lid_chat).unwrap();
    store
        .insert_message(&text_message(lid_chat, "new", 2_000, "the newest"))
        .unwrap();
    // The store the reconcile pass takes is the same one; an Arc over it.
    let shared = Arc::new(Store::open(&dir.join("chats.db")).unwrap());
    let (events, drain) = async_channel::unbounded();
    whatfast_wa::reconcile::reconcile(&shared, &identity, &events).await;

    // The fold is announced once, keyed by the pair that retired.
    let announced = merges(&take_events(&drain));
    assert_eq!(
        announced,
        vec![(lid_chat.to_string(), phone_chat.to_string())]
    );

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
    // The retired address follows the fold, so a late write cannot rebuild
    // the twin from it.
    assert_eq!(store.resolve_chat_id(lid_chat).unwrap(), phone_chat);
}

#[tokio::test]
async fn a_chat_row_left_behind_by_an_earlier_fold_converges() {
    let dir = profile_with_identity("leftover");
    let store = Arc::new(Store::open(&dir.join("chats.db")).unwrap());
    let identity = Identity::open(&dir);
    let phone_chat = "5493816041868@s.whatsapp.net";
    let lid_chat = "162040542929138@lid";
    store.ensure_chat(phone_chat, phone_chat).unwrap();
    // An earlier session had no map and keyed the person by the hidden
    // identity; a later write moved the alias and never folded the row.
    store.ensure_chat(lid_chat, lid_chat).unwrap();
    store
        .insert_message(&text_message(lid_chat, "stranded", 2_000, "stranded"))
        .unwrap();
    assert_eq!(
        store.adopt_address(lid_chat, phone_chat).unwrap(),
        Some((lid_chat.to_string(), phone_chat.to_string()))
    );

    let (events, drain) = async_channel::unbounded();
    whatfast_wa::reconcile::reconcile(&store, &identity, &events).await;

    // The address table no longer mentions the twin, so only the row itself
    // can still tell the pass there is something to fold.
    assert!(
        store.chat(lid_chat).unwrap().is_none(),
        "the stranded row folded"
    );
    let moved = store.message(phone_chat, "stranded").unwrap();
    assert!(moved.is_some(), "its message moved to the phone chat");
    assert_eq!(
        merges(&take_events(&drain)),
        vec![(lid_chat.to_string(), phone_chat.to_string())]
    );
}

#[tokio::test]
async fn a_second_pass_announces_nothing_and_moves_nothing() {
    let dir = profile_with_identity("idempotent");
    let store = Arc::new(Store::open(&dir.join("chats.db")).unwrap());
    let identity = Identity::open(&dir);
    let phone_chat = "5493816041868@s.whatsapp.net";
    let lid_chat = "162040542929138@lid";
    store.ensure_chat(phone_chat, phone_chat).unwrap();
    store.ensure_chat(lid_chat, lid_chat).unwrap();
    store
        .insert_message(&text_message(lid_chat, "m1", 2_000, "twice"))
        .unwrap();

    // The passes overlap in production: Connected, every finished history
    // and every contact update spawn one.
    let (events, drain) = async_channel::unbounded();
    whatfast_wa::reconcile::reconcile(&store, &identity, &events).await;
    let first = take_events(&drain);
    whatfast_wa::reconcile::reconcile(&store, &identity, &events).await;
    let second = take_events(&drain);

    assert_eq!(merges(&first).len(), 1);
    assert!(
        second.is_empty(),
        "the settled profile announced {second:?}"
    );
    assert_eq!(
        store.message(phone_chat, "m1").unwrap().unwrap().text,
        "twice"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_passes_over_the_same_pair_announce_one_merge() {
    let dir = profile_with_identity("race");
    let store = Arc::new(Store::open(&dir.join("chats.db")).unwrap());
    let identity = Arc::new(Identity::open(&dir));
    let phone_chat = "5493816041868@s.whatsapp.net";
    let lid_chat = "162040542929138@lid";
    store.ensure_chat(phone_chat, phone_chat).unwrap();
    store.ensure_chat(lid_chat, lid_chat).unwrap();
    store
        .insert_message(&text_message(lid_chat, "racing", 2_000, "racing"))
        .unwrap();

    let (events, drain) = async_channel::unbounded();
    let mut tasks = Vec::new();
    for _ in 0..2 {
        let store = Arc::clone(&store);
        let identity = Arc::clone(&identity);
        let events = events.clone();
        tasks.push(tokio::spawn(async move {
            whatfast_wa::reconcile::reconcile(&store, &identity, &events).await;
        }));
    }
    for task in tasks {
        task.await.unwrap();
    }

    // One row retired, so exactly one announcement; the loser may not come
    // back and the message may not be duplicated.
    let announced = merges(&take_events(&drain));
    assert_eq!(
        announced,
        vec![(lid_chat.to_string(), phone_chat.to_string())]
    );
    assert!(store.chat(lid_chat).unwrap().is_none());
    assert_eq!(
        store
            .read(|conn| conn.query_row(
                "SELECT count(*) FROM messages WHERE id='racing'",
                [],
                |row| row.get::<_, i64>(0),
            ))
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn the_agenda_never_erases_a_name_it_already_learned() {
    let dir = profile_with_identity("erase");
    let store = Arc::new(Store::open(&dir.join("chats.db")).unwrap());
    let identity = Identity::open(&dir);
    let phone_chat = "5493816041868@s.whatsapp.net";
    store.ensure_chat(phone_chat, phone_chat).unwrap();
    // The saved name came with an earlier sync; this one only carries the
    // push name.
    store
        .learn_contact(phone_chat, "Julio Bonkosky", "", "")
        .unwrap();

    let (events, _drain) = async_channel::unbounded();
    whatfast_wa::reconcile::reconcile(&store, &identity, &events).await;

    let (name, push, business, _) = store.contact(phone_chat).unwrap().unwrap();
    assert_eq!(name, "Julio Bonkosky");
    assert_eq!(push, "Julio");
    assert_eq!(business, "");
    let chat = store.chat(phone_chat).unwrap().unwrap();
    assert_eq!(chat.contact_name, "Julio Bonkosky");
    assert_eq!(whatfast_wa::wire::wire_chat(&chat).title, "Julio Bonkosky");
}

#[tokio::test]
async fn the_title_precedence_is_agenda_then_business_then_push() {
    let dir = profile_with_identity("precedence");
    let session = rusqlite::Connection::open(dir.join("session.db")).unwrap();
    session
        .execute_batch(
            "INSERT INTO whatsmeow_contacts(their_jid, full_name, first_name, push_name, business_name)
                 VALUES('5493815000001@s.whatsapp.net','Nombre Completo','Nombre','Push','Negocio');
             INSERT INTO whatsmeow_contacts(their_jid, first_name, push_name)
                 VALUES('5493815000002@s.whatsapp.net','Solo Nombre','Push Dos');
             INSERT INTO whatsmeow_contacts(their_jid, push_name, business_name)
                 VALUES('5493815000003@s.whatsapp.net','Push Tres','Negocio Tres');
             INSERT INTO whatsmeow_contacts(their_jid, push_name)
                 VALUES('5493815000004@s.whatsapp.net','Push Cuatro');",
        )
        .unwrap();
    let store = Arc::new(Store::open(&dir.join("chats.db")).unwrap());
    let identity = Identity::open(&dir);
    let titles = [
        ("5493815000001@s.whatsapp.net", "Nombre Completo"),
        ("5493815000002@s.whatsapp.net", "Solo Nombre"),
        ("5493815000003@s.whatsapp.net", "Negocio Tres"),
        ("5493815000004@s.whatsapp.net", "Push Cuatro"),
    ];
    for (jid, _) in titles {
        store.ensure_chat(jid, jid).unwrap();
    }
    let (events, _drain) = async_channel::unbounded();
    whatfast_wa::reconcile::reconcile(&store, &identity, &events).await;

    for (jid, title) in titles {
        let chat = store.chat(jid).unwrap().expect("the chat row");
        assert_eq!(
            whatfast_wa::wire::wire_chat(&chat).title,
            title,
            "title of {jid}"
        );
    }
}

#[tokio::test]
async fn the_history_lands_in_the_canonical_chat_from_the_start() {
    let dir = profile_with_identity("history");
    let store = Store::open(&dir.join("chats.db")).unwrap();
    let identity = Identity::open(&dir);

    // The history keys this person by the hidden identity; the map knows
    // the phone number.
    let parsed = history_with_one_message("162040542929138@lid", "hist", 3);
    let conversations = history_conversations(&parsed);
    assert_eq!(conversations.len(), 1);
    let (event, stored) = store_conversation(&store, &identity, &conversations[0]).unwrap();
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
        Event::ChatUpserted(chat) => {
            assert_eq!(
                chat.chat.expect("the chat payload").id,
                "5493816041868@s.whatsapp.net"
            );
        }
        other => panic!("expected a chat upsert, got {other:?}"),
    }
}

#[tokio::test]
async fn the_fold_carries_the_display_state_of_the_retired_twin() {
    let dir = profile_with_identity("flags");
    let store = Arc::new(Store::open(&dir.join("chats.db")).unwrap());
    let identity = Identity::open(&dir);
    let phone_chat = "5493816041868@s.whatsapp.net";
    let lid_chat = "162040542929138@lid";
    // The phone chat is the old side and owns the block flag; the twin is the
    // newest conversation and carries the archive, the mute and the pin.
    store.ensure_chat(phone_chat, phone_chat).unwrap();
    store
        .insert_message(&text_message(phone_chat, "old", 1_000, "old"))
        .unwrap();
    store.ensure_chat(lid_chat, lid_chat).unwrap();
    store
        .insert_message(&text_message(lid_chat, "new", 5_000, "new"))
        .unwrap();
    store
        .set_chat_flag(lid_chat, ChatFlag::Archived, true)
        .unwrap();
    store
        .set_chat_flag(phone_chat, ChatFlag::Blocked, true)
        .unwrap();
    store
        .read(|conn| {
            conn.execute(
                "UPDATE chats SET muted_until=9000, pinned=1 WHERE jid=?1",
                [lid_chat],
            )?;
            Ok(())
        })
        .unwrap();

    let (events, _drain) = async_channel::unbounded();
    whatfast_wa::reconcile::reconcile(&store, &identity, &events).await;

    let chat = store.chat(phone_chat).unwrap().unwrap();
    // The newest side owns the archive state, mute and block take the
    // maximum, and the pointer follows the moved message.
    assert!(chat.archived);
    assert_eq!(chat.muted_until, 9000);
    assert!(chat.blocked);
    assert_eq!(chat.last_message_id, "new");
    assert_eq!(chat.last_message_at, 5_000);
    // The pin flag is not part of the Go merge: the winner keeps its own.
    assert!(!chat.pinned);
}

/// The reconciliation over a real profile copy. Ignored by default:
/// point WHATFAST_WA_PROFILE at a directory holding chats.db and
/// session.db and run with --ignored --nocapture.
#[tokio::test]
#[ignore = "needs WHATFAST_WA_PROFILE"]
async fn the_real_profile_folds() {
    let dir = std::path::PathBuf::from(
        std::env::var("WHATFAST_WA_PROFILE").expect("WHATFAST_WA_PROFILE"),
    );
    let store = Arc::new(Store::open(&dir.join("chats.db")).unwrap());
    let identity = Identity::open(&dir);

    let count = |sql: &str| -> i64 {
        store
            .read(|conn| conn.query_row(sql, [], |row| row.get::<_, i64>(0)))
            .unwrap()
    };
    let chats_before = count("SELECT count(*) FROM chats");
    let lid_before = count("SELECT count(*) FROM chats WHERE jid LIKE '%@lid'");
    let phone_before = count("SELECT count(*) FROM chats WHERE jid LIKE '%@s.whatsapp.net'");
    let groups = count("SELECT count(*) FROM chats WHERE jid LIKE '%@g.us'");
    let messages_before = count("SELECT count(*) FROM messages");
    let lid_messages_before = count("SELECT count(*) FROM messages WHERE chat_jid LIKE '%@lid'");
    let reactions_before = count("SELECT count(*) FROM reactions");
    let redirects_before = count("SELECT count(*) FROM chat_redirects");
    let addresses_before = count("SELECT count(*) FROM chat_addresses");
    let named_before = count(
        "SELECT count(*) FROM chats c JOIN contacts k ON k.jid=c.preferred_jid
         WHERE k.contact_name<>''",
    );

    // The pairs the phone knows, and the pairs this profile already folded
    // on earlier runs (one redirect each).
    let contacts = identity.phone_contacts();
    let known_pairs = contacts
        .iter()
        .filter(|contact| identity.aliases(&contact.jid).len() > 1)
        .count();

    let (events, drain) = async_channel::unbounded();
    whatfast_wa::reconcile::reconcile(&store, &identity, &events).await;
    let first_pass = take_events(&drain);

    let chats_after = count("SELECT count(*) FROM chats");
    let lid_after = count("SELECT count(*) FROM chats WHERE jid LIKE '%@lid'");
    let messages_after = count("SELECT count(*) FROM messages");
    let lid_messages_after = count("SELECT count(*) FROM messages WHERE chat_jid LIKE '%@lid'");
    let reactions_after = count("SELECT count(*) FROM reactions");
    let redirects_after = count("SELECT count(*) FROM chat_redirects");
    let named_after = count(
        "SELECT count(*) FROM chats c JOIN contacts k ON k.jid=c.preferred_jid
         WHERE k.contact_name<>''",
    );

    // A second pass over the settled profile is a no-op.
    whatfast_wa::reconcile::reconcile(&store, &identity, &events).await;
    let second_pass = take_events(&drain);

    // Every hidden-identity chat this profile already retired still maps,
    // through the port's own reader, to the phone chat it was folded into.
    let retired = store
        .read(|conn| {
            let mut statement =
                conn.prepare("SELECT old_chat_jid,new_chat_jid FROM chat_redirects")?;
            let rows = statement
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .unwrap();
    let disagreements = retired
        .iter()
        .filter(|(old, new)| !identity.aliases(old).iter().any(|alias| alias == new))
        .count();

    // Pending divergence, reported not asserted: the chat's address resolves
    // to the newest alias, and the fold makes the hidden identity the newest,
    // so the wire phone number falls back to the lid digits instead of the
    // phone number the Go prints. See the audit report.
    let twin_address = count(
        "SELECT count(*) FROM chats c WHERE c.jid LIKE '%@s.whatsapp.net'
         AND COALESCE((SELECT a.jid FROM chat_addresses a WHERE a.chat_jid=c.jid
                       ORDER BY a.last_seen_at DESC LIMIT 1), c.preferred_jid) LIKE '%@lid'",
    );
    let twin_phone = count(
        "SELECT count(*) FROM chats c LEFT JOIN contacts k ON k.jid=c.preferred_jid
         WHERE c.jid LIKE '%@s.whatsapp.net' AND COALESCE(k.phone_number,'')=''
         AND COALESCE((SELECT a.jid FROM chat_addresses a WHERE a.chat_jid=c.jid
                       ORDER BY a.last_seen_at DESC LIMIT 1), c.preferred_jid) LIKE '%@lid'",
    );

    println!(
        "contacts: {} ({} with a known twin); chats: {} -> {} (phone {}, lid {} -> {}, groups {})",
        contacts.len(),
        known_pairs,
        chats_before,
        chats_after,
        phone_before,
        lid_before,
        lid_after,
        groups,
    );
    println!(
        "folds: {} announced, {} chat rows retired; messages moved out of @lid: {}",
        merges(&first_pass).len(),
        chats_before - chats_after,
        lid_messages_before - lid_messages_after,
    );
    println!(
        "messages: {} -> {}; reactions: {} -> {}; addresses: {} -> {}",
        messages_before,
        messages_after,
        reactions_before,
        reactions_after,
        addresses_before,
        count("SELECT count(*) FROM chat_addresses"),
    );
    println!(
        "redirects: {} -> {} ({} agree with the live map); chats with an agenda name: {} -> {}",
        redirects_before,
        redirects_after,
        retired.len() - disagreements,
        named_before,
        named_after,
    );
    println!(
        "second pass: {} events ({} merges)",
        second_pass.len(),
        merges(&second_pass).len()
    );
    println!(
        "pending: {} chats resolve their address to the hidden identity, {} of them print the lid digits as the phone number",
        twin_address, twin_phone,
    );

    assert_eq!(
        disagreements, 0,
        "a retired lid no longer maps to the chat it folded into"
    );

    // Zero data loss: the fold moves rows, never drops them.
    assert_eq!(
        messages_after, messages_before,
        "messages survived the fold"
    );
    assert_eq!(reactions_after, reactions_before, "reactions survived");
    // Every retired row is reachable through its redirect, and every address
    // still points at a live chat.
    assert_eq!(
        count(
            "SELECT count(*) FROM chat_redirects r
             WHERE NOT EXISTS(SELECT 1 FROM chats c WHERE c.jid=r.new_chat_jid)"
        ),
        0,
        "no redirect points at a retired chat"
    );
    assert_eq!(
        count(
            "SELECT count(*) FROM chat_addresses a
             WHERE NOT EXISTS(SELECT 1 FROM chats c WHERE c.jid=a.chat_jid)"
        ),
        0,
        "no address points at a retired chat"
    );
    assert_eq!(
        count(
            "SELECT count(*) FROM chat_redirects r
             WHERE EXISTS(SELECT 1 FROM chats c WHERE c.jid=r.old_chat_jid)"
        ),
        0,
        "no retired jid came back as a live chat"
    );
    // Nothing is left to announce once the profile settled.
    assert!(
        second_pass.is_empty(),
        "the settled profile kept announcing {second_pass:?}"
    );
}
