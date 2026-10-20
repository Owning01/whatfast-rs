//! Port of `tauri/tests/unread.test.ts`.
//!
//! Every case of the oracle suite has its own test here, with the same name
//! translated to snake_case.

use whatfast_core::unread::{optimistic_unread_count, should_restore_optimistic_unread};

#[test]
fn partial_message_windows_retain_the_backend_unread_count() {
    assert_eq!(optimistic_unread_count(7, true), 7);
}

#[test]
fn a_read_through_the_newest_message_can_clear_the_badge_optimistically() {
    assert_eq!(optimistic_unread_count(7, false), 0);
}

#[test]
fn a_matching_authoritative_upsert_prevents_rollback_after_a_lost_response() {
    assert!(!should_restore_optimistic_unread(Some(0), 0, 4, 5));
    assert!(should_restore_optimistic_unread(Some(0), 0, 4, 4));
    assert!(!should_restore_optimistic_unread(Some(2), 0, 4, 4));
}
