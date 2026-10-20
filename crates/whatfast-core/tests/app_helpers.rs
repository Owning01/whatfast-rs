//! Characterization tests for `tauri/src/state/app-helpers.ts`.
//!
//! The oracle has no test suite for this module, so the expectations below
//! were dumped from the frozen TypeScript module itself: a temporary Node
//! script imported `tauri/src/state/app-helpers.ts` (read-only, the oracle
//! repo was not touched), ran every function with the cases below and printed
//! the exact outputs that became the expectations below.

use std::collections::HashMap;

use whatfast_core::app_helpers::{
    AttachmentContent, Chat, ChatKind, Draft, DropSide, ImageContent, Mention, Message,
    MessageContent, MessageStatus, TextContent, clone_draft, draft_is_empty, encode_mentions,
    media_key, merge_chats, merge_messages, parse_recent_chats, reconcile_authoritative_chats,
    remember_recent_chat, serialize_recent_chats, sort_chats, sort_messages, trim_messages,
    upsert_sorted_message, upsert_sorted_message_with_index,
};

fn chat(id: &str, pinned: bool, last_message_timestamp_ms: i64) -> Chat {
    Chat {
        id: id.to_string(),
        kind: ChatKind::Direct,
        title: format!("title-{id}"),
        avatar_path: String::new(),
        last_message_preview: String::new(),
        last_message_timestamp_ms,
        unread_count: 0,
        muted: false,
        pinned,
        archived: false,
        favorite: None,
        blocked: None,
        mute_until_ms: None,
        phone_number: String::new(),
        contact_name: String::new(),
        push_name: String::new(),
        business_name: String::new(),
        last_message_from_me: false,
        transport_jid: String::new(),
    }
}

fn text_content(text: &str) -> MessageContent {
    MessageContent::Text(TextContent {
        text: text.to_string(),
        link_preview: None,
    })
}

fn image_content(
    caption: &str,
    local_path: &str,
    thumbnail_path: &str,
    jpeg_thumbnail: Vec<u8>,
) -> MessageContent {
    MessageContent::Image(ImageContent {
        caption: caption.to_string(),
        mime_type: "image/jpeg".to_string(),
        local_path: local_path.to_string(),
        width: 10,
        height: 10,
        file_size: 100,
        downloadable: true,
        sticker: false,
        animated: false,
        thumbnail_path: thumbnail_path.to_string(),
        jpeg_thumbnail,
    })
}

fn attachment_content(caption: &str, local_path: &str) -> MessageContent {
    MessageContent::Attachment(AttachmentContent {
        kind: "document".to_string(),
        caption: caption.to_string(),
        mime_type: "application/pdf".to_string(),
        file_name: "a.pdf".to_string(),
        local_path: local_path.to_string(),
        file_size: 10,
        width: 0,
        height: 0,
        duration_seconds: 0.0,
        animated: false,
        voice_note: false,
        downloadable: true,
    })
}

fn message(id: &str, timestamp_ms: i64, content: MessageContent) -> Message {
    Message {
        id: id.to_string(),
        chat_id: "chat-1".to_string(),
        sender_id: "sender-1".to_string(),
        sender_name: "Alice".to_string(),
        from_me: false,
        timestamp_ms,
        status: MessageStatus::Delivered,
        edited: false,
        revoked: false,
        expires_at_ms: 0,
        sender_phone_number: String::new(),
        sender_avatar_path: String::new(),
        reactions: Vec::new(),
        reply_to_message_id: String::new(),
        reply_to_chat_id: String::new(),
        forwarding_score: 0,
        mention_texts: Vec::new(),
        content: Some(content),
    }
}

fn ids(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| value.to_string()).collect()
}

fn text_of(message: &Message) -> String {
    match message.content.as_ref() {
        Some(MessageContent::Text(text)) => text.text.clone(),
        _ => String::new(),
    }
}

fn mention(display_name: &str, jid: &str) -> Mention {
    Mention {
        display_name: display_name.to_string(),
        jid: jid.to_string(),
    }
}

#[test]
fn sort_chats_puts_pinned_first_then_the_newest_then_the_id() {
    let sorted = sort_chats(&[
        chat("b", false, 100),
        chat("a", true, 50),
        chat("c", false, 300),
        chat("d", true, 300),
    ]);
    assert_eq!(
        sorted
            .iter()
            .map(|chat| chat.id.clone())
            .collect::<Vec<String>>(),
        ids(&["d", "a", "c", "b"])
    );
}

#[test]
fn sort_chats_breaks_the_tie_with_the_locale_collation_of_the_ids() {
    let sorted = sort_chats(&[
        chat("B", false, 100),
        chat("a", false, 100),
        chat("A", false, 100),
        chat("b", false, 100),
    ]);
    assert_eq!(
        sorted
            .iter()
            .map(|chat| chat.id.clone())
            .collect::<Vec<String>>(),
        ids(&["a", "A", "b", "B"])
    );
}

#[test]
fn sort_chats_breaks_a_full_tie_with_the_id() {
    let sorted = sort_chats(&[
        chat("z", false, 10),
        chat("y", false, 10),
        chat("x", false, 10),
    ]);
    assert_eq!(
        sorted
            .iter()
            .map(|chat| chat.id.clone())
            .collect::<Vec<String>>(),
        ids(&["x", "y", "z"])
    );
}

#[test]
fn merge_chats_lets_the_incoming_chat_win_and_keeps_the_first_seen_order() {
    let mut existing_a = chat("a", false, 100);
    existing_a.title = "old-a".to_string();
    existing_a.unread_count = 3;
    let mut existing_b = chat("b", false, 100);
    existing_b.title = "old-b".to_string();
    existing_b.unread_count = 7;
    let mut incoming_a = chat("a", false, 200);
    incoming_a.title = "new-a".to_string();
    let mut incoming_c = chat("c", false, 300);
    incoming_c.unread_count = 1;

    let merged = merge_chats(
        &[existing_a.clone(), existing_b.clone()],
        &[incoming_a, incoming_c],
    );
    assert_eq!(
        merged
            .iter()
            .map(|chat| (
                chat.id.clone(),
                chat.title.clone(),
                chat.unread_count,
                chat.last_message_timestamp_ms
            ))
            .collect::<Vec<(String, String, u32, i64)>>(),
        vec![
            ("a".to_string(), "new-a".to_string(), 0, 200),
            ("b".to_string(), "old-b".to_string(), 7, 100),
            ("c".to_string(), "title-c".to_string(), 1, 300),
        ]
    );
}

#[test]
fn merge_chats_keeps_the_previous_optional_flags_when_the_incoming_chat_has_none() {
    let mut existing = chat("a", false, 100);
    existing.favorite = Some(true);
    existing.blocked = Some(false);
    let merged = merge_chats(&[existing], &[chat("a", false, 200)]);
    assert_eq!(merged[0].favorite, Some(true));
    assert_eq!(merged[0].blocked, Some(false));
}

#[test]
fn reconcile_authoritative_chats_keeps_the_local_copy_of_a_chat_whose_revision_moved() {
    let merged = reconcile_authoritative_chats(
        &[chat("a", false, 100), chat("b", false, 200)],
        &[chat("a", false, 999), chat("b", false, 999)],
        &HashMap::from([("a".to_string(), 1u32)]),
        &HashMap::from([("a".to_string(), 2u32)]),
    );
    assert_eq!(
        merged
            .iter()
            .map(|chat| (chat.id.clone(), chat.last_message_timestamp_ms))
            .collect::<Vec<(String, i64)>>(),
        vec![("a".to_string(), 999), ("b".to_string(), 200)]
    );
}

#[test]
fn merge_messages_merges_by_id_and_sorts_the_window() {
    let existing = vec![
        message("m1", 100, text_content("one")),
        message("m2", 200, text_content("two")),
    ];
    let merged = merge_messages(
        &existing,
        &[
            message("m0", 50, text_content("zero")),
            message("m2", 200, text_content("two-updated")),
        ],
    );
    assert_eq!(
        merged
            .iter()
            .map(|message| (message.id.clone(), message.timestamp_ms, text_of(message)))
            .collect::<Vec<(String, i64, String)>>(),
        vec![
            ("m0".to_string(), 50, "zero".to_string()),
            ("m1".to_string(), 100, "one".to_string()),
            ("m2".to_string(), 200, "two-updated".to_string()),
        ]
    );
}

#[test]
fn merge_messages_keeps_the_local_image_path_the_backend_does_not_know() {
    let merged = merge_messages(
        &[message(
            "m1",
            100,
            image_content("", "C:/local/a.jpg", "C:/local/a-thumb.jpg", vec![1, 2]),
        )],
        &[message(
            "m1",
            100,
            image_content("fresh", "", "", Vec::new()),
        )],
    );
    assert_eq!(
        merged[0].content,
        Some(MessageContent::Image(ImageContent {
            caption: "fresh".to_string(),
            mime_type: "image/jpeg".to_string(),
            local_path: "C:/local/a.jpg".to_string(),
            width: 10,
            height: 10,
            file_size: 100,
            downloadable: true,
            sticker: false,
            animated: false,
            thumbnail_path: "C:/local/a-thumb.jpg".to_string(),
            jpeg_thumbnail: vec![1, 2],
        }))
    );
}

#[test]
fn merge_messages_keeps_the_local_attachment_path() {
    let merged = merge_messages(
        &[message("m1", 100, attachment_content("", "C:/local/a.pdf"))],
        &[message("m1", 100, attachment_content("fresh", ""))],
    );
    assert_eq!(
        merged[0].content,
        Some(MessageContent::Attachment(AttachmentContent {
            kind: "document".to_string(),
            caption: "fresh".to_string(),
            mime_type: "application/pdf".to_string(),
            file_name: "a.pdf".to_string(),
            local_path: "C:/local/a.pdf".to_string(),
            file_size: 10,
            width: 0,
            height: 0,
            duration_seconds: 0.0,
            animated: false,
            voice_note: false,
            downloadable: true,
        }))
    );
}

#[test]
fn merge_messages_drops_the_media_when_the_incoming_message_is_text() {
    let merged = merge_messages(
        &[message(
            "m1",
            100,
            image_content("", "C:/local/a.jpg", "", Vec::new()),
        )],
        &[message("m1", 100, text_content("plain"))],
    );
    assert_eq!(merged[0].content, Some(text_content("plain")));
}

#[test]
fn sort_messages_orders_by_timestamp_and_then_by_id() {
    let sorted = sort_messages(&[
        message("b", 100, text_content("b")),
        message("a", 100, text_content("a")),
        message("c", 50, text_content("c")),
    ]);
    assert_eq!(
        sorted
            .iter()
            .map(|message| message.id.clone())
            .collect::<Vec<String>>(),
        ids(&["c", "a", "b"])
    );
}

#[test]
fn upsert_sorted_message_appends_a_newer_message_at_the_end() {
    let existing = vec![
        message("m1", 100, text_content("one")),
        message("m2", 200, text_content("two")),
    ];
    let next = upsert_sorted_message(&existing, &message("m3", 300, text_content("three")));
    assert_eq!(
        next.iter()
            .map(|message| message.id.clone())
            .collect::<Vec<String>>(),
        ids(&["m1", "m2", "m3"])
    );
}

#[test]
fn upsert_sorted_message_inserts_a_message_in_the_middle() {
    let existing = vec![
        message("m1", 100, text_content("one")),
        message("m2", 200, text_content("two")),
    ];
    let next = upsert_sorted_message(&existing, &message("m15", 150, text_content("mid")));
    assert_eq!(
        next.iter()
            .map(|message| message.id.clone())
            .collect::<Vec<String>>(),
        ids(&["m1", "m15", "m2"])
    );
}

#[test]
fn upsert_sorted_message_inserts_a_message_at_the_beginning() {
    let existing = vec![
        message("m1", 100, text_content("one")),
        message("m2", 200, text_content("two")),
    ];
    let next = upsert_sorted_message(&existing, &message("m05", 50, text_content("first")));
    assert_eq!(
        next.iter()
            .map(|message| message.id.clone())
            .collect::<Vec<String>>(),
        ids(&["m05", "m1", "m2"])
    );
}

#[test]
fn upsert_sorted_message_replaces_an_existing_message_in_place() {
    let existing = vec![
        message("m1", 100, text_content("one")),
        message("m2", 200, text_content("two")),
    ];
    let next = upsert_sorted_message(&existing, &message("m1", 100, text_content("one-updated")));
    assert_eq!(
        next.iter()
            .map(|message| (message.id.clone(), text_of(message)))
            .collect::<Vec<(String, String)>>(),
        vec![
            ("m1".to_string(), "one-updated".to_string()),
            ("m2".to_string(), "two".to_string()),
        ]
    );
}

#[test]
fn upsert_sorted_message_resorts_an_existing_message_that_moved() {
    let existing = vec![
        message("m1", 100, text_content("one")),
        message("m2", 200, text_content("two")),
    ];
    let next = upsert_sorted_message(&existing, &message("m1", 500, text_content("moved")));
    assert_eq!(
        next.iter()
            .map(|message| message.id.clone())
            .collect::<Vec<String>>(),
        ids(&["m2", "m1"])
    );
}

#[test]
fn upsert_sorted_message_honours_an_explicit_existing_index() {
    let existing = vec![
        message("m1", 100, text_content("one")),
        message("m2", 200, text_content("two")),
    ];
    let next = upsert_sorted_message_with_index(
        &existing,
        &message("zzz", 100, text_content("explicit")),
        Some(1),
    );
    assert_eq!(
        next.iter()
            .map(|message| (message.id.clone(), text_of(message)))
            .collect::<Vec<(String, String)>>(),
        vec![
            ("m1".to_string(), "one".to_string()),
            ("zzz".to_string(), "explicit".to_string()),
        ]
    );
}

#[test]
fn trim_messages_leaves_a_window_under_the_cap_untouched() {
    let messages = vec![
        message("m1", 100, text_content("one")),
        message("m2", 200, text_content("two")),
    ];
    let trimmed = trim_messages(&messages, DropSide::Older);
    assert_eq!(
        trimmed
            .iter()
            .map(|message| message.id.clone())
            .collect::<Vec<String>>(),
        ids(&["m1", "m2"])
    );
}

#[test]
fn trim_messages_drops_the_oldest_window_when_it_drops_older() {
    let messages: Vec<Message> = (0..260)
        .map(|index| message(&format!("m{index}"), index, text_content("t")))
        .collect();
    let trimmed = trim_messages(&messages, DropSide::Older);
    assert_eq!(trimmed.len(), 250);
    assert_eq!(trimmed[0].id, "m10");
    assert_eq!(trimmed[1].id, "m11");
    assert_eq!(trimmed[248].id, "m258");
    assert_eq!(trimmed[249].id, "m259");
}

#[test]
fn trim_messages_drops_the_newest_window_when_it_drops_newer() {
    let messages: Vec<Message> = (0..260)
        .map(|index| message(&format!("m{index}"), index, text_content("t")))
        .collect();
    let trimmed = trim_messages(&messages, DropSide::Newer);
    assert_eq!(trimmed.len(), 250);
    assert_eq!(trimmed[0].id, "m0");
    assert_eq!(trimmed[1].id, "m1");
    assert_eq!(trimmed[248].id, "m248");
    assert_eq!(trimmed[249].id, "m249");
}

#[test]
fn encode_mentions_replaces_the_visible_names_with_the_bare_jid_user() {
    let encoded = encode_mentions(
        "hi @Alice and @Bob",
        &[
            mention("Alice", "111@s.whatsapp.net"),
            mention("Bob", "222@s.whatsapp.net"),
        ],
    );
    assert_eq!(encoded.text, "hi @111 and @222");
    assert_eq!(
        encoded.jids,
        ids(&["111@s.whatsapp.net", "222@s.whatsapp.net"])
    );
}

#[test]
fn encode_mentions_skips_a_mention_that_is_not_visible() {
    let encoded = encode_mentions(
        "hi @Alice",
        &[
            mention("Carol", "333@s.whatsapp.net"),
            mention("Alice", "111@s.whatsapp.net"),
        ],
    );
    assert_eq!(encoded.text, "hi @111");
    assert_eq!(encoded.jids, ids(&["111@s.whatsapp.net"]));
}

#[test]
fn encode_mentions_replaces_every_occurrence_of_the_visible_name() {
    let encoded = encode_mentions("@Alice @Alice", &[mention("Alice", "111@s.whatsapp.net")]);
    assert_eq!(encoded.text, "@111 @111");
    assert_eq!(encoded.jids, ids(&["111@s.whatsapp.net"]));
}

#[test]
fn media_key_joins_the_chat_and_the_message_with_a_nul() {
    assert_eq!(media_key("chat-1", "msg-1"), "chat-1\u{0}msg-1");
}

#[test]
fn read_recent_chats_drops_the_entries_that_are_not_strings() {
    assert_eq!(parse_recent_chats(None), Vec::<String>::new());
    assert_eq!(
        parse_recent_chats(Some(r#"["r1","r2"]"#)),
        ids(&["r1", "r2"])
    );
    assert_eq!(
        parse_recent_chats(Some(r#"["r1",7,null,"r2"]"#)),
        ids(&["r1", "r2"])
    );
    assert_eq!(
        parse_recent_chats(Some(r#"{"nope":true}"#)),
        Vec::<String>::new()
    );
    assert_eq!(parse_recent_chats(Some("not json")), Vec::<String>::new());
}

#[test]
fn write_recent_chats_serializes_the_list_for_the_storage() {
    assert_eq!(
        serialize_recent_chats(&ids(&["w1", "w2"])),
        r#"["w1","w2"]"#
    );
    assert_eq!(
        parse_recent_chats(Some(&serialize_recent_chats(&ids(&["w1", "w2"])))),
        ids(&["w1", "w2"])
    );
}

#[test]
fn remember_recent_chat_moves_a_known_chat_to_the_front() {
    assert_eq!(
        remember_recent_chat("r2", &ids(&["r1", "r2"])),
        ids(&["r2", "r1"])
    );
}

#[test]
fn remember_recent_chat_prepends_a_new_chat() {
    assert_eq!(
        remember_recent_chat("r3", &ids(&["r2", "r1"])),
        ids(&["r3", "r2", "r1"])
    );
}

#[test]
fn remember_recent_chat_caps_the_history_at_ten_entries() {
    assert_eq!(
        remember_recent_chat(
            "k",
            &ids(&["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"])
        ),
        ids(&["k", "a", "b", "c", "d", "e", "f", "g", "h", "i"])
    );
}

#[test]
fn clone_draft_copies_the_draft_and_its_mentions() {
    let draft = Draft {
        text: "hello".to_string(),
        reply_to_message_id: "m1".to_string(),
        reply_to_chat_id: "chat-1".to_string(),
        reply_preview_text: "preview".to_string(),
        reply_sender_name: "Alice".to_string(),
        editing_message_id: "m2".to_string(),
        mentions: vec![mention("Alice", "111@s.whatsapp.net")],
    };
    let cloned = clone_draft(&draft);
    assert_eq!(cloned, draft);
    assert_eq!(cloned.mentions, draft.mentions);
}

#[test]
fn draft_is_empty_only_accepts_a_draft_without_content() {
    let empty = Draft::default();
    assert!(draft_is_empty(Some(&empty)));
    assert!(!draft_is_empty(None));

    let with_text = Draft {
        text: "hello".to_string(),
        ..Draft::default()
    };
    assert!(!draft_is_empty(Some(&with_text)));

    let with_mentions = Draft {
        mentions: vec![mention("Alice", "111@s.whatsapp.net")],
        ..Draft::default()
    };
    assert!(!draft_is_empty(Some(&with_mentions)));
}
