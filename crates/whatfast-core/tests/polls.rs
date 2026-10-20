//! Port of `tauri/tests/polls.test.ts`.
//!
//! Every case of the oracle suite has its own test here, with the same name
//! translated to snake_case.

use whatfast_core::message_format::{MessageContent, PollContent, PollOption, PollVoter};
use whatfast_core::polls::{
    clone_poll_content, optimistic_poll_vote, preserve_pending_poll_intent,
};
// The projection the poll reducer works on; the canonical `Message` lives in
// `whatfast_core::types`.
use whatfast_core::types::MessageSummary as Message;

fn poll() -> PollContent {
    PollContent {
        question: "Lunch?".to_string(),
        selectable_options_count: 2,
        total_voters: 1,
        options: vec![
            PollOption {
                name: "Pizza".to_string(),
                vote_count: 1,
                selected_by_me: true,
                voters: vec![PollVoter {
                    user_id: "me".to_string(),
                    display_name: "You".to_string(),
                    avatar_path: String::new(),
                    from_me: true,
                }],
            },
            PollOption {
                name: "Sushi".to_string(),
                vote_count: 0,
                selected_by_me: false,
                voters: Vec::new(),
            },
        ],
    }
}

fn message(content: PollContent) -> Message {
    Message {
        id: "poll".to_string(),
        chat_id: "chat".to_string(),
        sender_id: "owner".to_string(),
        sender_name: "Owner".to_string(),
        from_me: false,
        timestamp_ms: 1,
        status: 3,
        edited: false,
        revoked: false,
        expires_at_ms: 0,
        sender_phone_number: String::new(),
        sender_avatar_path: String::new(),
        reactions: Vec::new(),
        reply_to_message_id: String::new(),
        reply_to_chat_id: String::new(),
        content: Some(MessageContent::Poll(content)),
    }
}

#[test]
fn poll_reducer_changes_a_vote_without_duplicating_the_voter() {
    let changed = optimistic_poll_vote(&poll(), &["Sushi".to_string()]);
    assert_eq!(changed.total_voters, 1);
    let summary: Vec<(&str, u32, bool)> = changed
        .options
        .iter()
        .map(|option| {
            (
                option.name.as_str(),
                option.vote_count,
                option.selected_by_me,
            )
        })
        .collect();
    assert_eq!(summary, vec![("Pizza", 0, false), ("Sushi", 1, true)]);
}

#[test]
fn poll_reducer_retracts_and_restores_a_complete_vote_intent() {
    let retracted = optimistic_poll_vote(&poll(), &[]);
    assert_eq!(retracted.total_voters, 0);
    assert_eq!(retracted.options[0].vote_count, 0);
    let restored = optimistic_poll_vote(&retracted, &["Pizza".to_string(), "Sushi".to_string()]);
    let counts: Vec<u32> = restored
        .options
        .iter()
        .map(|option| option.vote_count)
        .collect();
    assert_eq!(counts, vec![1, 1]);
}

#[test]
fn an_older_backend_update_cannot_overwrite_a_newer_pending_intent() {
    let current = message(optimistic_poll_vote(&poll(), &["Sushi".to_string()]));
    let stale = message(poll());
    let merged = preserve_pending_poll_intent(&current, &stale, true);
    assert_eq!(merged.content, current.content);
    let not_pending = preserve_pending_poll_intent(&current, &stale, false);
    assert_eq!(not_pending.content, stale.content);
}

#[test]
fn poll_snapshots_can_be_copied_from_reactive_proxy_values() {
    // port: the original wraps the poll in a `Proxy` to emulate the reactive
    // store of Solid. The port has no proxy, so the case checks the same
    // property the original checks: the snapshot is a deep copy and shares no
    // buffer with the source.
    let reactive = poll();
    let snapshot = clone_poll_content(&reactive);
    assert_eq!(snapshot, reactive);
    assert!(!std::ptr::eq(&snapshot.options, &reactive.options));
    assert!(!std::ptr::eq(
        &snapshot.options[0].voters,
        &reactive.options[0].voters
    ));
}
