//! Port of `tauri/tests/chat-merge.test.ts`.
//!
//! Every case of the oracle suite has its own test here, with the same name
//! translated to snake_case.

use std::collections::BTreeMap;

use whatfast_core::chat_merge::clear_merged_poll_votes;

#[test]
fn chat_merge_cancels_old_poll_intents_without_losing_canonical_work() {
    let pending: BTreeMap<String, bool> = BTreeMap::from([
        ("old:a".to_string(), true),
        ("new:a".to_string(), false),
        ("new:b".to_string(), true),
    ]);
    let cleared = clear_merged_poll_votes(&pending, "old");
    let expected: BTreeMap<String, bool> =
        BTreeMap::from([("new:a".to_string(), false), ("new:b".to_string(), true)]);
    assert_eq!(cleared, expected);
}
