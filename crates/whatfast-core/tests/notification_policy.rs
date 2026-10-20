//! Rust port of `tauri/tests/notifications.test.ts` (frozen oracle).
//!
//! Every case of the TypeScript suite is here with the same name translated
//! to snake_case and the same assertions.

use whatfast_core::app_helpers::{
    Chat, ChatKind, Message, MessageContent, MessageStatus, TextContent,
};
use whatfast_core::notification_policy::{
    NotificationPolicyInput, is_chat_actually_visible, notification_body, notification_title,
    should_notify,
};
use whatfast_core::workspace::Pane;

fn message() -> Message {
    Message {
        id: "message-1".to_string(),
        chat_id: "chat-1".to_string(),
        sender_id: String::new(),
        sender_name: "Alice".to_string(),
        from_me: false,
        timestamp_ms: 0,
        status: MessageStatus::Unspecified,
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
        content: Some(MessageContent::Text(TextContent {
            text: "Hello\nfrom   Alice".to_string(),
            link_preview: None,
        })),
    }
}

fn chat(title: &str) -> Chat {
    Chat {
        id: "chat-1".to_string(),
        kind: ChatKind::Direct,
        title: title.to_string(),
        avatar_path: String::new(),
        last_message_preview: String::new(),
        last_message_timestamp_ms: 0,
        unread_count: 0,
        muted: false,
        pinned: false,
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

fn policy(
    enabled: bool,
    visible: bool,
    chat_visible: bool,
    muted: bool,
    incoming: bool,
) -> NotificationPolicyInput {
    NotificationPolicyInput {
        enabled,
        visible,
        chat_visible,
        muted,
        incoming,
    }
}

#[test]
fn notification_policy_suppresses_foreground_muted_outgoing_and_disabled_messages() {
    assert!(should_notify(&policy(true, false, false, false, true)));
    assert!(!should_notify(&policy(true, true, true, false, true)));
    assert!(!should_notify(&policy(true, false, false, true, true)));
    assert!(!should_notify(&policy(true, false, false, false, false)));
    assert!(!should_notify(&policy(false, false, false, false, true)));
}

#[test]
fn responsive_split_panes_only_expose_the_focused_conversation() {
    let panes = vec![
        Pane {
            id: "pane-1".to_string(),
            tab_chat_ids: vec!["alpha".to_string()],
            active_chat_id: "alpha".to_string(),
        },
        Pane {
            id: "pane-2".to_string(),
            tab_chat_ids: vec!["beta".to_string()],
            active_chat_id: "beta".to_string(),
        },
    ];
    assert!(is_chat_actually_visible(&panes, "pane-1", "beta", false));
    assert!(is_chat_actually_visible(&panes, "pane-1", "alpha", true));
    assert!(!is_chat_actually_visible(&panes, "pane-1", "beta", true));
}

#[test]
fn notification_text_obeys_the_privacy_preview_preference() {
    assert_eq!(notification_body(&message(), true), "Hello from Alice");
    assert_eq!(notification_body(&message(), false), "New message");
    assert_eq!(notification_title(None, &message()), "Alice");
    assert_eq!(
        notification_title(Some(&chat("Project room")), &message()),
        "Project room"
    );
}
