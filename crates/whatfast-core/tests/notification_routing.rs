//! Port of the frozen suite `tauri/tests/notification-routing.test.ts`.
//!
//! The oracle routes the activations with promises; the port hands the
//! targets over one at a time ([`next_target`]) and the caller reports that
//! the route finished ([`route_completed`]), so the order and the
//! deduplication are the same.

use std::cell::Cell;

use whatfast_core::notification_routing::{
    NotificationActivationQueue, NotificationPermissionGate, NotificationTarget,
    NotificationTargetAvailability, notification_target_availability,
};

fn activation(chat_id: &str, message_id: &str) -> serde_json::Value {
    serde_json::json!({ "chatId": chat_id, "messageId": message_id })
}

fn routed(target: &NotificationTarget) -> String {
    format!("{}/{}", target.chat_id, target.message_id)
}

#[test]
fn activation_waits_for_bootstrap_and_duplicate_native_deliveries_route_once() {
    let mut queue = NotificationActivationQueue::new();
    assert!(queue.enqueue(&activation("chat-1", "message-1")));
    assert!(!queue.enqueue(&activation("chat-1", "message-1")));
    assert_eq!(queue.next_target(), None);

    queue.mark_ready();
    let first = queue.next_target().map(|target| routed(&target));
    queue.route_completed();
    assert_eq!(first, Some("chat-1/message-1".to_string()));
    assert!(!queue.enqueue(&activation("chat-1", "message-1")));
}

#[test]
fn activation_routing_pauses_across_a_backend_restart_until_the_fresh_epoch_is_ready() {
    let mut queue = NotificationActivationQueue::new();
    queue.mark_ready();
    queue.mark_not_ready();
    assert!(queue.enqueue(&activation("chat-2", "message-2")));
    assert_eq!(queue.next_target(), None);

    queue.mark_ready();
    let routed = queue.next_target().map(|target| routed(&target));
    queue.route_completed();
    assert_eq!(routed, Some("chat-2/message-2".to_string()));
}

#[test]
fn queued_activation_follows_a_chat_merge_and_remains_ordered() {
    let mut queue = NotificationActivationQueue::new();
    assert!(queue.enqueue(&activation("old", "first")));
    assert!(queue.enqueue(&activation("new", "second")));
    queue.merge_chat_id("old", "new");
    queue.mark_ready();

    let first = queue.next_target().map(|target| routed(&target));
    queue.route_completed();
    let second = queue.next_target().map(|target| routed(&target));
    queue.route_completed();

    assert_eq!(first, Some("new/first".to_string()));
    assert_eq!(second, Some("new/second".to_string()));
}

#[test]
fn a_chat_merge_deduplicates_a_canonical_target_queued_during_routing() {
    let mut queue = NotificationActivationQueue::new();
    queue.mark_ready();
    assert!(queue.enqueue(&activation("old", "message")));
    // port: the original starts the drain inside `enqueue`, so the first
    // activation is on its way before the second one arrives.
    let routed_first = queue.next_target().map(|target| routed(&target));
    assert!(queue.enqueue(&activation("new", "message")));
    queue.merge_chat_id("old", "new");
    queue.route_completed();

    assert_eq!(routed_first, Some("old/message".to_string()));
    assert_eq!(queue.next_target(), None);
}

#[test]
fn invalid_activation_payloads_are_ignored() {
    let mut queue = NotificationActivationQueue::new();
    assert!(!queue.enqueue(&serde_json::Value::Null));
    assert!(!queue.enqueue(&activation("", "message")));
    assert!(!queue.enqueue(&serde_json::json!({ "chatId": "chat" })));
}

#[test]
fn missing_chats_and_deleted_messages_have_explicit_fallback_outcomes() {
    assert_eq!(
        notification_target_availability(false, false),
        NotificationTargetAvailability::MissingChat
    );
    assert_eq!(
        notification_target_availability(true, false),
        NotificationTargetAvailability::MissingMessage
    );
    assert_eq!(
        notification_target_availability(true, true),
        NotificationTargetAvailability::Available
    );
}

#[test]
fn permission_denial_is_cached_until_the_user_explicitly_retries() {
    let mut gate = NotificationPermissionGate::new();
    let attempts = Cell::new(0u32);
    let denied = || {
        attempts.set(attempts.get() + 1);
        false
    };

    assert!(!gate.check(denied, false));
    assert!(!gate.check(denied, false));
    assert_eq!(attempts.get(), 1);
    assert!(gate.check(|| true, true));
    assert_eq!(attempts.get(), 1);
}
