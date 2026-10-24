//! The store's own unit tests: paging, unread counters, reactions, and the
//! last-message pointer, exercised on a temporary database.

fn temp_store(label: &str) -> (std::path::PathBuf, whatfast_wa::db::Store) {
    let dir = std::env::temp_dir().join(format!(
        "whatfast-wa-store-{label}-{}-{}",
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

fn message(
    chat: &str,
    id: &str,
    ts: i64,
    from_me: bool,
    text: &str,
) -> whatfast_wa::db::MessageRow {
    whatfast_wa::db::MessageRow {
        id: id.into(),
        chat_jid: chat.into(),
        transport_jid: chat.into(),
        sender_jid: String::new(),
        text: text.into(),
        timestamp: ts,
        from_me,
        status: 2,
        kind: "text".into(),
        ..Default::default()
    }
}

#[test]
fn chats_page_orders_newest_first_and_the_cursor_walks_back() {
    let (_dir, store) = temp_store("page");
    for (jid, at) in [("a@x", 100), ("b@x", 300), ("c@x", 200)] {
        store.ensure_chat(jid, jid).unwrap();
        store
            .insert_message(&message(jid, &format!("m{at}"), at, false, "hi"))
            .unwrap();
    }
    let (page, total, cursor) = store.chats_page(0, "", 2).unwrap();
    assert_eq!(total, 3);
    assert_eq!(page.len(), 2);
    assert_eq!(page[0].jid, "b@x");
    assert_eq!(page[1].jid, "c@x");
    let (page2, _total, cursor2) = store.chats_page(200, "c@x", 2).unwrap();
    assert_eq!(page2.len(), 1);
    assert_eq!(page2[0].jid, "a@x");
    assert!(!cursor.is_empty());
    assert!(cursor2.ends_with("a@x") || page2[0].last_message_at == 100);
}

#[test]
fn insert_message_updates_the_last_message_pointer_only_when_new() {
    let (_dir, store) = temp_store("pointer");
    store.ensure_chat("a@x", "a@x").unwrap();
    assert!(
        store
            .insert_message(&message("a@x", "m1", 100, false, "first"))
            .unwrap()
    );
    // A replay of the same row is a no-op.
    assert!(
        !store
            .insert_message(&message("a@x", "m1", 100, false, "first"))
            .unwrap()
    );
    store
        .insert_message(&message("a@x", "m2", 90, true, "older from me"))
        .unwrap();
    let chat = store.chat("a@x").unwrap().unwrap();
    assert_eq!(chat.last_message_id, "m1");
    assert_eq!(chat.last_message_at, 100);
    assert!(!chat.last_message_from_me);
}

#[test]
fn messages_around_and_before_slice_the_history() {
    let (_dir, store) = temp_store("slice");
    store.ensure_chat("a@x", "a@x").unwrap();
    for i in 0..30 {
        store
            .insert_message(&message(
                "a@x",
                &format!("m{i:02}"),
                i * 10,
                i % 2 == 0,
                "t",
            ))
            .unwrap();
    }
    let newest = store.messages_before("a@x", 0, "", 10).unwrap();
    assert_eq!(newest.len(), 10);
    // Oldest-to-newest is the rendering contract, so the newest row is last.
    assert_eq!(newest.first().unwrap().id, "m20");
    assert_eq!(newest.last().unwrap().id, "m29");

    let (around, has_older, has_newer) = store.messages_around("a@x", "m10", 10).unwrap().unwrap();
    assert!(around.iter().any(|row| row.id == "m10"));
    assert!(has_older && has_newer);

    let (after, _) = store.messages_after("a@x", 100, "m10", 5).unwrap();
    assert_eq!(after.first().unwrap().id, "m11");
}

#[test]
fn reactions_upsert_remove_and_join_back() {
    let (_dir, store) = temp_store("reactions");
    store.ensure_chat("a@x", "a@x").unwrap();
    store
        .insert_message(&message("a@x", "m1", 10, false, "hi"))
        .unwrap();
    let reaction = whatfast_wa::db::ReactionRow {
        chat_jid: "a@x".into(),
        message_id: "m1".into(),
        sender_jid: "peer@x".into(),
        emoji: "👍".into(),
        timestamp: 20,
        from_me: false,
    };
    store.set_reaction(&reaction).unwrap();
    let stored = store.message("a@x", "m1").unwrap().unwrap();
    assert_eq!(stored.reactions.len(), 1);
    assert_eq!(stored.reactions[0].emoji, "👍");

    let removal = whatfast_wa::db::ReactionRow {
        emoji: String::new(),
        ..reaction.clone()
    };
    store.set_reaction(&removal).unwrap();
    let stored = store.message("a@x", "m1").unwrap().unwrap();
    assert!(stored.reactions.is_empty());
}

#[test]
fn mark_read_clears_unread_rows_and_the_chat_counter() {
    let (_dir, store) = temp_store("read");
    store.ensure_chat("a@x", "a@x").unwrap();
    let mut incoming = message("a@x", "m1", 10, false, "hi");
    incoming.unread = true;
    store.insert_message(&incoming).unwrap();
    store
        .write(|conn| {
            conn.execute("UPDATE chats SET unread_count=1 WHERE jid='a@x'", [])?;
            Ok(())
        })
        .unwrap();
    let chat = store.chat("a@x").unwrap().unwrap();
    assert_eq!(chat.unread_count, 1);
    store.mark_chat_read("a@x").unwrap();
    let chat = store.chat("a@x").unwrap().unwrap();
    assert_eq!(chat.unread_count, 0);
    let stored = store.message("a@x", "m1").unwrap().unwrap();
    assert!(!stored.unread);
}
