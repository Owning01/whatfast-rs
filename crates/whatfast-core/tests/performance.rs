//! Rust port of `tauri/tests/performance.test.ts` (frozen oracle).
//!
//! Every case of the TypeScript suite is here with the same name translated
//! to snake_case and the same assertions.
//!
//! port: the suite also asserts the two helpers of
//! `tauri/src/lib/participant-roster.ts` (`participantRosterRows` and
//! `participantDomBudget`), which the port keeps in the module
//! `whatfast_core::performance`.

use whatfast_core::performance::{
    BoundedSet, ChatParticipant, Message, bound_window_around, index_messages,
    participant_dom_budget, participant_roster_rows,
};

fn message(id: &str, reply_to_message_id: &str, reply_to_chat_id: &str) -> Message {
    Message {
        id: id.to_string(),
        chat_id: "chat".to_string(),
        sender_id: "sender".to_string(),
        sender_name: "Sender".to_string(),
        from_me: false,
        timestamp_ms: 1,
        status: whatfast_core::types::MessageStatus::Unspecified,
        edited: false,
        revoked: false,
        expires_at_ms: 0,
        sender_phone_number: String::new(),
        sender_avatar_path: String::new(),
        reactions: Vec::new(),
        reply_to_message_id: reply_to_message_id.to_string(),
        reply_to_chat_id: reply_to_chat_id.to_string(),
        forwarding_score: 0,
        mention_texts: Vec::new(),
        content: None,
    }
}

fn participant(index: u32) -> ChatParticipant {
    ChatParticipant {
        participant_id: format!("member-{index}"),
        display_name: format!("Member {index}"),
        phone_number: index.to_string(),
        is_admin: index < 5,
        is_super_admin: index == 0,
        is_me: false,
    }
}

#[test]
fn bounded_set_evicts_the_least_recently_inserted_value() {
    let values = BoundedSet::new(3);
    assert!(values.is_some());
    let Some(mut values) = values else { return };

    assert_eq!(values.add("a".to_string()), None);
    values.add("b".to_string());
    values.add("c".to_string());
    values.add("a".to_string());

    assert_eq!(values.add("d".to_string()), Some("b".to_string()));
    assert_eq!(values.size(), 3);
    assert!(values.has(&"a".to_string()));
    assert!(!values.has(&"b".to_string()));
}

#[test]
fn bounded_set_rejects_invalid_capacities() {
    // port: the original throws a `RangeError` for a capacity that is not a
    // positive integer; the port returns `None` for the capacity of zero.
    assert!(BoundedSet::<String>::new(0).is_none());
    // port: the original also rejects a fractional capacity (1.5); a `usize`
    // cannot be fractional, so that half of the case has no representation
    // here.
}

#[test]
fn message_index_resolves_quotes_and_reply_metadata_in_one_pass() {
    let messages = vec![
        message("root", "", ""),
        message("reply-1", "root", ""),
        message("reply-2", "root", ""),
        message("nested", "reply-1", ""),
        message("private-reply", "group-message", "source-group"),
    ];
    let index = index_messages(&messages);

    assert_eq!(index.by_id.get("root"), Some(&messages[0]));
    assert_eq!(index.reply_count_by_id.get("root"), Some(&2));
    assert_eq!(
        index.first_reply_id_by_id.get("root").map(String::as_str),
        Some("reply-1")
    );
    assert_eq!(index.reply_count_by_id.get("reply-1"), Some(&1));
    assert!(!index.reply_count_by_id.contains_key("reply-2"));
    assert!(!index.reply_count_by_id.contains_key("group-message"));
}

#[test]
fn canonical_message_windows_stay_bounded_around_the_requested_target() {
    let values: Vec<u32> = (0..10).collect();

    let first = bound_window_around(&values, 5, 1);
    assert!(first.is_some());
    let Some(first) = first else { return };
    assert_eq!(first.items, vec![0, 1, 2, 3, 4]);
    assert!(!first.dropped_before);
    assert!(first.dropped_after);

    let last = bound_window_around(&values, 5, 8);
    assert!(last.is_some());
    let Some(last) = last else { return };
    assert_eq!(last.items, vec![5, 6, 7, 8, 9]);
    assert!(last.dropped_before);
    assert!(!last.dropped_after);

    let middle = bound_window_around(&values, 5, 5);
    assert!(middle.is_some());
    let Some(middle) = middle else { return };
    assert_eq!(middle.items, vec![3, 4, 5, 6, 7]);
    assert!(middle.dropped_before);
    assert!(middle.dropped_after);
}

#[test]
fn the_1_000_member_roster_keeps_its_mounted_dom_budget_bounded() {
    let participants: Vec<ChatParticipant> = (0..1_000).map(participant).collect();
    assert_eq!(participant_roster_rows(&participants, true).len(), 1_002);
    assert!(
        participant_dom_budget(720.0) <= 28.0,
        "720px viewport should mount at most 28 roster rows"
    );
    assert!(participant_dom_budget(720.0) < participants.len() as f64 / 30.0);
}
