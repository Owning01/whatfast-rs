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
                (i + 1) * 10,
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
    assert_eq!(around.len(), 21);
    assert_eq!(around.first().unwrap().id, "m00");
    assert_eq!(around.last().unwrap().id, "m20");
    assert!(around.iter().any(|row| row.id == "m10"));
    // Exactly ten older rows fit in the page, so there is no older page left.
    assert!(!has_older && has_newer);

    let (after, _) = store.messages_after("a@x", 110, "m10", 5).unwrap();
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

#[test]
fn chats_page_hides_status_and_newsletters_and_closes_the_cursor() {
    let (_dir, store) = temp_store("chats-hidden");
    for (jid, at) in [
        ("a@x", 100),
        ("b@x", 300),
        ("c@x", 200),
        ("status@broadcast", 400),
        ("canal@newsletter", 500),
    ] {
        store.ensure_chat(jid, jid).unwrap();
        store
            .insert_message(&message(jid, &format!("m-{jid}"), at, false, "hi"))
            .unwrap();
    }
    let (page, total, cursor) = store.chats_page(0, "", 10).unwrap();
    assert_eq!(page.len(), 3);
    assert!(
        page.iter()
            .all(|chat| chat.preferred_jid != "status@broadcast")
    );
    assert!(
        page.iter()
            .all(|chat| !chat.preferred_jid.ends_with("@newsletter"))
    );
    // The Go page hid those rows but counted every stored conversation.
    assert_eq!(total, 5);
    assert!(cursor.is_empty(), "a complete page has no next cursor");

    let (first, _total, cursor) = store.chats_page(0, "", 2).unwrap();
    assert_eq!(first.len(), 2);
    assert_eq!(first[0].jid, "b@x");
    assert!(!cursor.is_empty());
    let (at, jid) = cursor.split_once(':').unwrap();
    let (second, _total, last_cursor) = store.chats_page(at.parse().unwrap(), jid, 2).unwrap();
    assert_eq!(second.len(), 1);
    assert_eq!(second[0].jid, "a@x");
    assert!(
        last_cursor.is_empty(),
        "the last page must not advertise another page"
    );
}

#[test]
fn insert_message_merges_redeliveries_like_the_go_reducer() {
    let (_dir, store) = temp_store("merge-reducer");
    store.ensure_chat("a@x", "a@x").unwrap();
    let mut current = message("a@x", "m1", 1_000, false, "new caption");
    current.kind = "image".into();
    current.status = 4;
    current.edited_at = 1_100;
    current.image_caption = "new caption".into();
    current.image_local_path = "/cache/a.jpg".into();
    current.image_width = 640;
    current.image_media_key = vec![1, 2];
    assert!(store.insert_message(&current).unwrap());

    // A stale history copy carries a later event time and no media: it must
    // not move the row, downgrade the receipt or erase the descriptor.
    let mut stale = message("a@x", "m1", 5_000, false, "old caption");
    stale.kind = "image".into();
    stale.status = 3;
    assert!(!store.insert_message(&stale).unwrap());

    let stored = store.message("a@x", "m1").unwrap().unwrap();
    assert_eq!(stored.timestamp, 1_000, "the original position must stay");
    assert_eq!(stored.text, "new caption");
    assert_eq!(stored.status, 4, "a read receipt must not go backwards");
    assert_eq!(stored.image_local_path, "/cache/a.jpg");
    assert_eq!(stored.image_media_key, vec![1, 2]);
    assert_eq!(stored.image_width, 640);

    let mut edited = current.clone();
    edited.text = "edited caption".into();
    edited.edited_at = 1_500;
    assert!(!store.insert_message(&edited).unwrap());
    let stored = store.message("a@x", "m1").unwrap().unwrap();
    assert_eq!(stored.text, "edited caption");
    assert_eq!(stored.timestamp, 1_000);
    assert_eq!(
        store.chat("a@x").unwrap().unwrap().last_message_text,
        "edited caption"
    );

    let mut revoked = edited.clone();
    revoked.revoked = true;
    revoked.kind = "revoked".into();
    revoked.edited_at = 1_600;
    revoked.text = "Message deleted".into();
    assert!(!store.insert_message(&revoked).unwrap());
    let stored = store.message("a@x", "m1").unwrap().unwrap();
    assert!(stored.revoked);
    assert_eq!(stored.kind, "revoked");
    assert_eq!(stored.timestamp, 1_000);
}

#[test]
fn insert_message_counts_unread_once_per_new_incoming_row() {
    let (_dir, store) = temp_store("unread-once");
    store.ensure_chat("a@x", "a@x").unwrap();
    let mut incoming = message("a@x", "m1", 10, false, "hi");
    incoming.unread = true;
    assert!(store.insert_message(&incoming).unwrap());
    assert!(!store.insert_message(&incoming).unwrap());
    assert_eq!(store.chat("a@x").unwrap().unwrap().unread_count, 1);

    // An outgoing row never counts, even if a caller flags it as unread.
    let mut outgoing = message("a@x", "m2", 20, true, "mine");
    outgoing.unread = true;
    store.insert_message(&outgoing).unwrap();
    assert_eq!(store.chat("a@x").unwrap().unwrap().unread_count, 1);

    store.mark_chat_read("a@x").unwrap();
    assert_eq!(store.chat("a@x").unwrap().unwrap().unread_count, 0);
}

#[test]
fn set_message_status_follows_the_go_transition_table() {
    let (_dir, store) = temp_store("status");
    store.ensure_chat("a@x", "a@x").unwrap();
    let mut pending = message("a@x", "m1", 10, true, "sending");
    pending.status = 1;
    store.insert_message(&pending).unwrap();

    assert!(store.set_message_status("a@x", "m1", 5).unwrap());
    assert!(!store.set_message_status("a@x", "m1", 5).unwrap());
    assert!(store.set_message_status("a@x", "m1", 2).unwrap());
    assert!(store.set_message_status("a@x", "m1", 4).unwrap());
    // A replayed failure cannot downgrade a read receipt.
    assert!(!store.set_message_status("a@x", "m1", 5).unwrap());
    // Nor can a late delivered receipt move it either.
    assert!(!store.set_message_status("a@x", "m1", 3).unwrap());
    assert_eq!(store.message("a@x", "m1").unwrap().unwrap().status, 4);
    assert!(!store.set_message_status("a@x", "missing", 4).unwrap());
}

#[test]
fn reactions_keep_a_tombstone_and_collapse_self_aliases() {
    let (_dir, store) = temp_store("reaction-rules");
    store.ensure_chat("a@x", "a@x").unwrap();
    store
        .insert_message(&message("a@x", "m1", 10, false, "hi"))
        .unwrap();
    let peer = whatfast_wa::db::ReactionRow {
        chat_jid: "a@x".into(),
        message_id: "m1".into(),
        sender_jid: "peer@x".into(),
        emoji: "👍".into(),
        timestamp: 20,
        from_me: false,
    };
    store.set_reaction(&peer).unwrap();
    let stale = whatfast_wa::db::ReactionRow {
        emoji: "👎".into(),
        timestamp: 10,
        ..peer.clone()
    };
    store.set_reaction(&stale).unwrap();
    let stored = store.message("a@x", "m1").unwrap().unwrap();
    assert_eq!(stored.reactions.len(), 1);
    assert_eq!(stored.reactions[0].emoji, "👍");

    let removal = whatfast_wa::db::ReactionRow {
        emoji: String::new(),
        timestamp: 30,
        ..peer.clone()
    };
    store.set_reaction(&removal).unwrap();
    assert!(
        store
            .message("a@x", "m1")
            .unwrap()
            .unwrap()
            .reactions
            .is_empty()
    );
    // The tombstone keeps the stale replay from resurrecting the reaction.
    store.set_reaction(&stale).unwrap();
    assert!(
        store
            .message("a@x", "m1")
            .unwrap()
            .unwrap()
            .reactions
            .is_empty()
    );

    let pn = whatfast_wa::db::ReactionRow {
        chat_jid: "a@x".into(),
        message_id: "m1".into(),
        sender_jid: "15551234@s.whatsapp.net".into(),
        emoji: "👍".into(),
        timestamp: 2_000,
        from_me: true,
    };
    store.set_reaction(&pn).unwrap();
    let lid = whatfast_wa::db::ReactionRow {
        sender_jid: "12345@lid".into(),
        emoji: "❤️".into(),
        timestamp: 3_000,
        ..pn.clone()
    };
    store.set_reaction(&lid).unwrap();
    let stale_self = whatfast_wa::db::ReactionRow {
        emoji: "😂".into(),
        timestamp: 1_000,
        ..pn.clone()
    };
    store.set_reaction(&stale_self).unwrap();
    let stored = store.message("a@x", "m1").unwrap().unwrap();
    assert_eq!(stored.reactions.len(), 1);
    assert_eq!(stored.reactions[0].sender_jid, "12345@lid");
    assert_eq!(stored.reactions[0].emoji, "❤️");
}

#[test]
fn messages_around_builds_the_go_centered_window() {
    let (_dir, store) = temp_store("around-window");
    store.ensure_chat("a@x", "a@x").unwrap();
    for index in 0..100 {
        store
            .insert_message(&message(
                "a@x",
                &format!("m{index:03}"),
                index + 1,
                false,
                "t",
            ))
            .unwrap();
    }
    let (window, has_older, has_newer) = store.messages_around("a@x", "m050", 25).unwrap().unwrap();
    assert_eq!(window.len(), 51);
    assert_eq!(window[0].id, "m025");
    assert_eq!(window[25].id, "m050");
    assert_eq!(window[50].id, "m075");
    assert!(has_older && has_newer);

    // The oracle clamps a side limit above 50 down to its 25 default.
    let (clamped, ..) = store.messages_around("a@x", "m050", 60).unwrap().unwrap();
    assert_eq!(clamped.len(), 51);
    assert!(
        store
            .messages_around("a@x", "missing", 25)
            .unwrap()
            .is_none()
    );
}

#[test]
fn edit_updates_the_chat_preview_but_a_revoked_row_ignores_it() {
    let (_dir, store) = temp_store("edit-preview");
    store.ensure_chat("a@x", "a@x").unwrap();
    store
        .insert_message(&message("a@x", "m1", 10, false, "original"))
        .unwrap();
    store
        .insert_message(&message("a@x", "m2", 20, false, "newest"))
        .unwrap();

    store.mark_message_edited("a@x", "m1", "edited").unwrap();
    let edited = store.message("a@x", "m1").unwrap().unwrap();
    assert_eq!(edited.text, "edited");
    assert!(edited.edited_at > 0);
    assert_eq!(
        store.chat("a@x").unwrap().unwrap().last_message_text,
        "newest",
        "editing an older message must not replace the preview"
    );

    store
        .mark_message_edited("a@x", "m2", "newest edited")
        .unwrap();
    assert_eq!(
        store.chat("a@x").unwrap().unwrap().last_message_text,
        "newest edited"
    );

    store.mark_message_revoked("a@x", "m2").unwrap();
    store.mark_message_edited("a@x", "m2", "late edit").unwrap();
    let revoked = store.message("a@x", "m2").unwrap().unwrap();
    assert!(revoked.revoked);
    assert_eq!(revoked.kind, "revoked");
    assert_eq!(
        revoked.text, "newest edited",
        "a revoked row never accepts new text"
    );
}

#[test]
fn chat_flags_create_the_conversation_before_the_first_message() {
    let (_dir, store) = temp_store("flags-first");
    store
        .set_chat_flag("blocked@x", whatfast_wa::db::ChatFlag::Blocked, true)
        .unwrap();
    let chat = store.chat("blocked@x").unwrap().unwrap();
    assert!(chat.blocked);
    store.set_chat_mute("muted@x", 1_234).unwrap();
    let chat = store.chat("muted@x").unwrap().unwrap();
    assert_eq!(chat.muted_until, 1_234);

    // The first message must not clear the local preference.
    store
        .insert_message(&message("blocked@x", "m1", 10, false, "hello"))
        .unwrap();
    assert!(store.chat("blocked@x").unwrap().unwrap().blocked);
}

#[test]
fn search_groups_ranks_fuzzy_matches_beyond_the_first_page() {
    let (_dir, store) = temp_store("search-groups");
    for index in 0..501 {
        let jid = format!("g{index}@g.us");
        store.ensure_chat(&jid, &jid).unwrap();
    }
    store
        .write(|conn| {
            for index in 0..501 {
                let jid = format!("g{index}@g.us");
                let (name, at) = if index == 0 {
                    ("Equipo de Rust".to_owned(), 1)
                } else {
                    (format!("Group {index}"), 1_000 + index)
                };
                conn.execute(
                    "UPDATE chats SET name=?1, last_message_at=?2 WHERE jid=?3",
                    (name, at, jid),
                )?;
            }
            Ok(())
        })
        .unwrap();

    // A typo still matches, and the oldest group is past the first chat page.
    let hits = store.search_groups("eqipo", 6).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].jid, "g0@g.us");
    assert_eq!(hits[0].name, "Equipo de Rust");
}

#[test]
fn search_messages_matches_typos_captions_and_file_names() {
    let (_dir, store) = temp_store("search-messages");
    store.ensure_chat("team@g.us", "team@g.us").unwrap();
    store.ensure_chat("docs@g.us", "docs@g.us").unwrap();
    store
        .insert_message(&message(
            "team@g.us",
            "meeting",
            10,
            false,
            "Quarterly meeting notes",
        ))
        .unwrap();
    let mut document = message("docs@g.us", "document", 11, false, "Document");
    document.kind = "document".into();
    document.media_file_name = "roadmap-final.pdf".into();
    store.insert_message(&document).unwrap();
    let mut location = message("team@g.us", "location", 12, false, "Location");
    location.kind = "location".into();
    location.location_name = "Cubbon Park".into();
    store.insert_message(&location).unwrap();
    let mut revoked = message("team@g.us", "revoked", 13, false, "planning secret");
    revoked.revoked = true;
    store.insert_message(&revoked).unwrap();
    store
        .insert_message(&message("team@g.us", "planning", 14, false, "planning"))
        .unwrap();

    let typo = store.search_messages("meting", 20).unwrap();
    assert_eq!(typo[0].1.id, "meeting");
    let file = store.search_messages("roadmp", 20).unwrap();
    assert_eq!(file[0].1.id, "document");
    let place = store.search_messages("cubbon", 20).unwrap();
    assert_eq!(place[0].1.id, "location");
    let planning = store.search_messages("planning", 20).unwrap();
    assert_eq!(planning.len(), 1, "a revoked row is not a search result");
    assert_eq!(planning[0].1.id, "planning");

    // Zero means the Go default of twenty, and a short needle is ignored.
    assert!(store.search_messages("pl", 0).unwrap().is_empty());
    assert!(store.search_messages("planning", 0).unwrap().len() <= 20);
}

#[test]
fn newsletters_list_newest_first_and_resubscribe_moves_them() {
    let (_dir, store) = temp_store("newsletters");
    store
        .upsert_newsletter("a@newsletter", "Channel A", false)
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(5));
    store
        .upsert_newsletter("b@newsletter", "Channel B", false)
        .unwrap();
    let list = store.list_newsletters().unwrap();
    assert_eq!(list[0].jid, "b@newsletter");

    std::thread::sleep(std::time::Duration::from_millis(5));
    store
        .upsert_newsletter("a@newsletter", "Channel A renamed", false)
        .unwrap();
    let list = store.list_newsletters().unwrap();
    assert_eq!(list[0].jid, "a@newsletter");
    assert_eq!(list[0].name, "Channel A renamed");

    store.remove_newsletter("b@newsletter").unwrap();
    let list = store.list_newsletters().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].jid, "a@newsletter");
}

#[test]
fn poll_upsert_refreshes_the_definition() {
    let (_dir, store) = temp_store("poll");
    store.ensure_chat("a@x", "a@x").unwrap();
    let mut row = message("a@x", "m1", 10, false, "poll");
    row.kind = "poll".into();
    store.insert_message(&row).unwrap();
    store
        .upsert_poll("a@x", "m1", "First?", &["A".into(), "B".into()], 1)
        .unwrap();
    store
        .upsert_poll(
            "a@x",
            "m1",
            "Second?",
            &["A".into(), "C".into(), "D".into()],
            2,
        )
        .unwrap();
    let (question, selectable, options) = store
        .read(|conn| {
            conn.query_row(
                "SELECT question,selectable_count,options_json FROM polls
                 WHERE chat_jid='a@x' AND message_id='m1'",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
        })
        .unwrap();
    assert_eq!(question, "Second?");
    assert_eq!(selectable, 2);
    assert!(options.contains('C') && options.contains('D'));
}

#[test]
fn merge_chats_keeps_the_richer_row_and_recomputes_the_chat() {
    let (_dir, store) = temp_store("merge");
    store.ensure_chat("winner@x", "winner@x").unwrap();
    store.ensure_chat("loser@x", "loser@x").unwrap();
    let mut winner = message("winner@x", "m1", 100, false, "winner text");
    winner.status = 4;
    store.insert_message(&winner).unwrap();
    let mut loser = message("loser@x", "m1", 100, false, "loser text");
    loser.status = 2;
    loser.image_mime = "image/jpeg".into();
    loser.image_local_path = "/cache/loser.jpg".into();
    store.insert_message(&loser).unwrap();
    let mut unread = message("loser@x", "m2", 200, false, "unread");
    unread.unread = true;
    store.insert_message(&unread).unwrap();
    store.adopt_address("alias@x", "loser@x").unwrap();
    store
        .set_chat_flag("loser@x", whatfast_wa::db::ChatFlag::Archived, true)
        .unwrap();

    store.merge_chats("winner@x", "loser@x").unwrap();

    // The loser row is gone; its jid now resolves to the winner.
    assert!(
        store
            .read(
                |conn| conn.query_row("SELECT jid FROM chats WHERE jid='loser@x'", [], |row| {
                    row.get::<_, String>(0)
                })
            )
            .is_err()
    );
    assert_eq!(store.resolve_chat_id("loser@x").unwrap(), "winner@x");
    assert_eq!(store.resolve_chat_id("alias@x").unwrap(), "winner@x");
    let merged = store.message("winner@x", "m1").unwrap().unwrap();
    assert_eq!(merged.text, "loser text");
    assert_eq!(merged.status, 4);
    assert_eq!(merged.image_local_path, "/cache/loser.jpg");
    let moved = store.message("winner@x", "m2").unwrap().unwrap();
    assert_eq!(moved.chat_jid, "winner@x");
    let chat = store.chat("winner@x").unwrap().unwrap();
    assert_eq!(chat.last_message_id, "m2");
    assert_eq!(chat.unread_count, 1);
    // The loser was the newer conversation, so its archive state wins.
    assert!(chat.archived);
}

#[test]
fn sticker_lookup_skips_revoked_rows() {
    let (_dir, store) = temp_store("sticker");
    store.ensure_chat("a@x", "a@x").unwrap();
    let hash = vec![0x11u8; 32];
    let mut first = message("a@x", "s1", 100, false, "");
    first.kind = "sticker".into();
    first.image_file_sha256 = hash.clone();
    store.insert_message(&first).unwrap();
    let mut second = message("a@x", "s2", 200, false, "");
    second.kind = "sticker".into();
    second.image_file_sha256 = hash;
    store.insert_message(&second).unwrap();
    store.mark_message_revoked("a@x", "s2").unwrap();

    let found = store.message_by_sticker_id(&"11".repeat(32)).unwrap();
    assert_eq!(found, Some(("a@x".to_owned(), "s1".to_owned())));
}

#[test]
fn resolve_chat_prefers_a_live_chat_over_a_stale_redirect() {
    let (_dir, store) = temp_store("resolve");
    store.ensure_chat("live@x", "live@x").unwrap();
    store.ensure_chat("other@x", "other@x").unwrap();
    store.ensure_chat("canon@x", "alias@x").unwrap();
    store
        .write(|conn| {
            conn.execute(
                "INSERT INTO chat_redirects(old_chat_jid,new_chat_jid) VALUES('live@x','other@x')",
                [],
            )?;
            Ok(())
        })
        .unwrap();
    assert_eq!(store.resolve_chat_id("live@x").unwrap(), "live@x");
    assert_eq!(store.resolve_chat_id("alias@x").unwrap(), "canon@x");
    assert_eq!(store.resolve_chat_id("unknown@x").unwrap(), "unknown@x");
}

#[test]
fn local_path_updates_are_kind_scoped() {
    let (_dir, store) = temp_store("local-path");
    store.ensure_chat("a@x", "a@x").unwrap();
    store
        .insert_message(&message("a@x", "text", 10, false, "hi"))
        .unwrap();
    let mut image = message("a@x", "image", 20, false, "");
    image.kind = "image".into();
    store.insert_message(&image).unwrap();
    let mut audio = message("a@x", "audio", 30, false, "");
    audio.kind = "audio".into();
    store.insert_message(&audio).unwrap();

    assert!(store.set_image_local_path("a@x", "text", "/x").is_err());
    assert!(store.set_image_local_path("a@x", "audio", "/x").is_err());
    store.set_image_local_path("a@x", "image", "/img").unwrap();
    store.set_media_local_path("a@x", "audio", "/aud").unwrap();
    assert!(store.set_media_local_path("a@x", "image", "/x").is_err());
    assert_eq!(
        store
            .message("a@x", "image")
            .unwrap()
            .unwrap()
            .image_local_path,
        "/img"
    );
    assert_eq!(
        store
            .message("a@x", "audio")
            .unwrap()
            .unwrap()
            .media_local_path,
        "/aud"
    );
}

#[test]
fn messages_after_requires_a_full_cursor() {
    let (_dir, store) = temp_store("after-cursor");
    store.ensure_chat("a@x", "a@x").unwrap();
    for (id, at) in [("m1", 10), ("m2", 20), ("m3", 30)] {
        store
            .insert_message(&message("a@x", id, at, false, "hi"))
            .unwrap();
    }
    assert!(store.messages_after("a@x", 0, "", 10).is_err());
    assert!(store.messages_after("a@x", 10, "", 10).is_err());

    // A page that exactly fills the requested size is not a partial page.
    let (rows, has_more) = store.messages_after("a@x", 10, "m1", 2).unwrap();
    assert_eq!(rows.len(), 2);
    assert!(!has_more, "an exact page has nothing more after it");
    let (rows, has_more) = store.messages_after("a@x", 10, "m1", 1).unwrap();
    assert_eq!(rows.len(), 1);
    assert!(has_more);
    let (rows, has_more) = store.messages_after("a@x", 30, "m3", 10).unwrap();
    assert!(rows.is_empty());
    assert!(!has_more);
}

#[test]
fn insert_message_stamps_missing_timestamps_and_the_health_row() {
    let (_dir, store) = temp_store("stamp");
    store.ensure_chat("a@x", "a@x").unwrap();
    let mut unstamped = message("a@x", "m1", 0, false, "no time");
    unstamped.kind = "text".into();
    assert!(store.insert_message(&unstamped).unwrap());
    let stored = store.message("a@x", "m1").unwrap().unwrap();
    assert!(stored.timestamp > 0, "a zero timestamp is stamped on write");
    assert_eq!(
        store.chat("a@x").unwrap().unwrap().last_message_at,
        stored.timestamp
    );
    let persisted: i64 = store
        .read(|conn| {
            conn.query_row(
                "SELECT last_message_persisted_at FROM storage_health WHERE singleton=1",
                [],
                |row| row.get(0),
            )
        })
        .unwrap();
    assert!(persisted > 0);
}

#[test]
fn page_limits_clamp_like_the_go_sidecar() {
    let (_dir, store) = temp_store("clamp");
    store.ensure_chat("a@x", "a@x").unwrap();
    for index in 1..=210 {
        store
            .insert_message(&message("a@x", &format!("m{index:03}"), index, false, "t"))
            .unwrap();
    }
    // Zero asks for the default page of fifty, and nothing exceeds two
    // hundred: the Go `clampLimit` bounds every message page.
    assert_eq!(store.messages_before("a@x", 0, "", 0).unwrap().len(), 50);
    assert_eq!(store.messages_before("a@x", 0, "", 300).unwrap().len(), 200);
    assert_eq!(store.messages_before("a@x", 0, "", 200).unwrap().len(), 200);
}

#[test]
fn revoke_through_the_reducer_updates_the_preview_and_survives_replay() {
    let (_dir, store) = temp_store("revoke-preview");
    store.ensure_chat("a@x", "a@x").unwrap();
    store
        .insert_message(&message("a@x", "m1", 10, false, "original"))
        .unwrap();
    let mut revoked = message("a@x", "m1", 10, false, "Message deleted");
    revoked.revoked = true;
    revoked.kind = "revoked".into();
    revoked.edited_at = 30;
    assert!(!store.insert_message(&revoked).unwrap());
    let chat = store.chat("a@x").unwrap().unwrap();
    assert_eq!(chat.last_message_text, "Message deleted");
    assert_eq!(
        chat.last_message_at, 10,
        "a revoke must not move the pointer"
    );

    // A stale replay of the pre-revoke row cannot resurrect its text.
    assert!(
        !store
            .insert_message(&message("a@x", "m1", 10, false, "original"))
            .unwrap()
    );
    let stored = store.message("a@x", "m1").unwrap().unwrap();
    assert!(stored.revoked);
    assert_eq!(stored.text, "Message deleted");
    assert_eq!(
        store.chat("a@x").unwrap().unwrap().last_message_text,
        "Message deleted"
    );
}

#[test]
fn reaction_pseudo_messages_never_become_rows() {
    let (_dir, store) = temp_store("reaction-row");
    store.ensure_chat("a@x", "a@x").unwrap();
    let mut pseudo = message("a@x", "r1", 10, false, "");
    pseudo.kind = "reaction".into();
    // The Go reducer only made sure the chat existed; the event went to the
    // reactions table, never to the message history.
    assert!(!store.insert_message(&pseudo).unwrap());
    assert!(store.messages_before("a@x", 0, "", 50).unwrap().is_empty());
    assert!(store.message("a@x", "r1").unwrap().is_none());
}

#[test]
fn chats_cursor_breaks_timestamp_ties_on_jid_desc() {
    let (_dir, store) = temp_store("chat-ties");
    for jid in ["a@x", "b@x", "c@x"] {
        store.ensure_chat(jid, jid).unwrap();
        store
            .insert_message(&message(jid, "m", 100, false, "hi"))
            .unwrap();
    }
    let (first, _total, cursor) = store.chats_page(0, "", 2).unwrap();
    assert_eq!(first[0].jid, "c@x");
    assert_eq!(first[1].jid, "b@x");
    let (at, jid) = cursor.split_once(':').unwrap();
    assert_eq!((at, jid), ("100", "b@x"));
    let (second, _total, last) = store.chats_page(100, "b@x", 2).unwrap();
    assert_eq!(second.len(), 1);
    assert_eq!(second[0].jid, "a@x");
    assert!(last.is_empty());
}

// ---------------------------------------------------------------------------
// History coverage, overview, integrity, polls and legacy replays
// ---------------------------------------------------------------------------

#[test]
fn history_coverage_starts_without_an_anchor_and_tracks_requests() {
    use whatfast_wa::db::HistoryCoverageData;
    let (_dir, store) = temp_store("coverage");
    store
        .ensure_chat("c@s.whatsapp.net", "c@s.whatsapp.net")
        .unwrap();
    // No message: there is nothing to anchor a request to.
    let empty = store.history_coverage("c@s.whatsapp.net").unwrap();
    assert_eq!(empty.state, "no_anchor");
    assert_eq!(empty.local_message_count, 0);
    store
        .insert_message(&message("c@s.whatsapp.net", "m1", 1000, false, "hi"))
        .unwrap();
    let fresh = store.history_coverage("c@s.whatsapp.net").unwrap();
    assert_eq!(fresh.local_message_count, 1);
    assert_eq!(fresh.oldest_message_timestamp, 1000);
    assert_eq!(fresh.state, "unknown");
    // A request round-trips through the store and bumps the revision.
    let requesting = store
        .save_history_coverage(HistoryCoverageData {
            state: "requesting".into(),
            last_requested_count: 50,
            last_requested_at: 2000,
            detail: "Requesting 50 older messages from WhatsApp".into(),
            ..fresh
        })
        .unwrap();
    assert_eq!(requesting.revision, 1);
    assert_eq!(requesting.state, "requesting");
    assert_eq!(requesting.last_requested_count, 50);
    // An exhausted chat with received messages is one the phone may extend.
    let exhausted = store
        .save_history_coverage(HistoryCoverageData {
            state: "exhausted".into(),
            last_received_count: 3,
            ..requesting
        })
        .unwrap();
    assert_eq!(exhausted.state, "more_available");
    // A zero-message answer stays terminal.
    let terminal = store
        .save_history_coverage(HistoryCoverageData {
            state: "exhausted".into(),
            last_received_count: 0,
            ..exhausted
        })
        .unwrap();
    assert_eq!(terminal.state, "exhausted");
}

#[test]
fn history_overview_counts_messages_and_chats() {
    let (_dir, store) = temp_store("overview");
    for (chat, count) in [("a@s.whatsapp.net", 2), ("b@s.whatsapp.net", 1)] {
        store.ensure_chat(chat, chat).unwrap();
        for index in 0..count {
            store
                .insert_message(&message(
                    chat,
                    &format!("m{index}"),
                    100 + index,
                    false,
                    "hi",
                ))
                .unwrap();
        }
    }
    store
        .save_history_coverage(whatfast_wa::db::HistoryCoverageData {
            chat_id: "a@s.whatsapp.net".into(),
            state: "exhausted".into(),
            detail: "done".into(),
            ..Default::default()
        })
        .unwrap();
    let overview = store.history_overview().unwrap();
    assert_eq!(overview.local_message_count, 3);
    assert_eq!(overview.chats_with_messages, 2);
    assert_eq!(overview.chats_checked, 1);
    assert_eq!(overview.chats_unknown, 1);
    assert_eq!(overview.chats_without_anchor, 0);
}

#[test]
fn oldest_message_row_anchors_the_history_request() {
    let (_dir, store) = temp_store("oldest");
    store
        .ensure_chat("c@s.whatsapp.net", "c@s.whatsapp.net")
        .unwrap();
    assert!(
        store
            .oldest_message_row("c@s.whatsapp.net")
            .unwrap()
            .is_none()
    );
    for (id, ts) in [("m2", 200), ("m1", 100), ("m3", 100)] {
        store
            .insert_message(&message("c@s.whatsapp.net", id, ts, false, "hi"))
            .unwrap();
    }
    let anchor = store
        .oldest_message_row("c@s.whatsapp.net")
        .unwrap()
        .unwrap();
    // The oldest timestamp wins; ties break by id, the store's page order.
    assert_eq!(anchor.id, "m1");
    assert_eq!(anchor.timestamp, 100);
}

#[test]
fn integrity_status_is_clean_and_repair_rebuilds_summaries() {
    let (_dir, store) = temp_store("integrity");
    store
        .ensure_chat("c@s.whatsapp.net", "c@s.whatsapp.net")
        .unwrap();
    store
        .insert_message(&message("c@s.whatsapp.net", "m1", 1000, false, "hi"))
        .unwrap();
    let status = store.integrity_status().unwrap();
    assert!(status.database_ok);
    assert_eq!(status.inconsistent_chats, 0);
    assert_eq!(status.pending_inbound, 0);
    assert!(status.last_message_persisted_at > 0);
    let repaired = store.repair_local_cache().unwrap();
    assert!(repaired.database_ok);
    assert_eq!(repaired.inconsistent_chats, 0);
    assert!(repaired.last_reconciliation_at > 0);
    let chat = store.chat("c@s.whatsapp.net").unwrap().unwrap();
    assert_eq!(chat.last_message_id, "m1");
    assert_eq!(chat.last_message_text, "hi");
}

#[test]
fn poll_meta_and_votes_round_trip() {
    let (_dir, store) = temp_store("pollmeta");
    store.ensure_chat("g@g.us", "g@g.us").unwrap();
    let mut poll = message("g@g.us", "p1", 1000, true, "Lunch?");
    poll.kind = "poll".into();
    store.insert_message(&poll).unwrap();
    store
        .upsert_poll(
            "g@g.us",
            "p1",
            "Lunch?",
            &["Pizza".into(), "Sushi".into()],
            1,
        )
        .unwrap();
    // Without the key a vote has nothing to encrypt with.
    let bare = store.poll_meta("g@g.us", "p1").unwrap().unwrap();
    assert!(bare.message_secret.is_empty());
    store
        .set_poll_meta("g@g.us", "p1", "me@s.whatsapp.net", &[7u8; 32])
        .unwrap();
    let meta = store.poll_meta("g@g.us", "p1").unwrap().unwrap();
    assert_eq!(meta.creator_jid, "me@s.whatsapp.net");
    assert_eq!(meta.message_secret, vec![7u8; 32]);
    store
        .apply_poll_vote(
            "g@g.us",
            "p1",
            "me@s.whatsapp.net",
            &["Sushi".into()],
            2000,
            true,
        )
        .unwrap();
    // A stale write never replaces a newer vote.
    store
        .apply_poll_vote(
            "g@g.us",
            "p1",
            "me@s.whatsapp.net",
            &["Pizza".into()],
            1000,
            true,
        )
        .unwrap();
    let snapshots = store.poll_snapshots("g@g.us", &["p1".into()]).unwrap();
    assert_eq!(snapshots.len(), 1);
    let sushi = snapshots[0]
        .options
        .iter()
        .find(|option| option.name == "Sushi")
        .unwrap();
    assert_eq!(sushi.votes, 1);
    assert!(sushi.selected_by_me);
}

#[test]
fn legacy_replays_reserve_request_and_complete() {
    let (_dir, store) = temp_store("legacy");
    store
        .ensure_chat("c@s.whatsapp.net", "c@s.whatsapp.net")
        .unwrap();
    assert!(
        store
            .pending_legacy_replays("c@s.whatsapp.net", 16)
            .unwrap()
            .is_empty()
    );
    store
        .record_legacy_replay(
            "c@s.whatsapp.net",
            "c@s.whatsapp.net",
            "ev1",
            "peer@s.whatsapp.net",
            1000,
            false,
            5,
        )
        .unwrap();
    // A second record of the same event is a duplicate, not a second row.
    store
        .record_legacy_replay(
            "c@s.whatsapp.net",
            "c@s.whatsapp.net",
            "ev1",
            "peer@s.whatsapp.net",
            1000,
            false,
            5,
        )
        .unwrap();
    let pending = store
        .pending_legacy_replays("c@s.whatsapp.net", 16)
        .unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].attempts, 0);
    store
        .mark_legacy_replay_requested("c@s.whatsapp.net", "ev1", "repair-1")
        .unwrap();
    let requested = store
        .pending_legacy_replays("c@s.whatsapp.net", 16)
        .unwrap();
    assert_eq!(requested.len(), 1);
    assert_eq!(requested[0].attempts, 1);
    store
        .complete_legacy_replay("c@s.whatsapp.net", "ev1")
        .unwrap();
    assert!(
        store
            .pending_legacy_replays("c@s.whatsapp.net", 16)
            .unwrap()
            .is_empty()
    );
}

/// Marking a chat read heals a counter inflated without flagged rows: the
/// cached total recomputes even when no row was dirty, so a badge stuck by
/// an older build clears the next time the chat opens.
#[test]
fn mark_read_ids_heals_an_inflated_counter() {
    let (_dir, store) = temp_store("heal");
    store
        .ensure_chat("c@s.whatsapp.net", "c@s.whatsapp.net")
        .unwrap();
    store
        .insert_message(&message("c@s.whatsapp.net", "m1", 1000, false, "hi"))
        .unwrap();
    // Simulate the drift: rows cleared without the counter.
    store
        .read(|conn| {
            conn.execute(
                "UPDATE messages SET unread=0 WHERE chat_jid='c@s.whatsapp.net'",
                [],
            )
            .unwrap();
            Ok::<_, rusqlite::Error>(())
        })
        .unwrap();
    // Empty ids: nothing to mark, but the counter must come down.
    store.mark_read_ids("c@s.whatsapp.net", &[]).unwrap();
    let chat = store.chat("c@s.whatsapp.net").unwrap().unwrap();
    assert_eq!(chat.unread_count, 0);
}

/// A message for a conversation the store never saw lands instead of
/// vanishing: the insert adopts the chat and retries once.
#[test]
fn insert_message_adopts_a_missing_chat() {
    let (_dir, store) = temp_store("adopt");
    let mut row = message("new@s.whatsapp.net", "m1", 1000, false, "hi");
    row.unread = true;
    assert!(store.insert_message(&row).unwrap());
    let chat = store.chat("new@s.whatsapp.net").unwrap().unwrap();
    assert_eq!(chat.unread_count, 1);
    assert_eq!(chat.last_message_id, "m1");
}
