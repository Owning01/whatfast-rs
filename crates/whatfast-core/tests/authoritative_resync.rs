//! Rust port of `tauri/tests/authoritative-resync.test.ts` (frozen oracle).
//!
//! Every case of the TypeScript suite is here with the same name translated
//! to snake_case and the same assertions.
//!
//! port: the function of the oracle lives in `tauri/src/state/app-helpers.ts`,
//! which another batch of the migration already ported to
//! `whatfast_core::app_helpers`. This module re-exports it instead of
//! repeating the logic, and the case goes through that public path.

use std::collections::HashMap;

use whatfast_core::authoritative_resync::{Chat, ChatKind, reconcile_authoritative_chats};

fn chat(id: &str, timestamp: i64, preview: &str) -> Chat {
    Chat {
        id: id.to_string(),
        kind: ChatKind::Direct,
        title: id.to_string(),
        avatar_path: String::new(),
        last_message_preview: preview.to_string(),
        last_message_timestamp_ms: timestamp,
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

#[test]
fn authoritative_chat_resync_deletes_stale_rows_while_preserving_racing_live_upserts() {
    let stale = chat("stale", 10, "stale");
    let unchanged = chat("unchanged", 20, "old");
    let raced = chat("raced", 30, "live");
    let result = reconcile_authoritative_chats(
        &[chat("unchanged", 40, "authoritative")],
        &[stale, unchanged, raced],
        &HashMap::from([
            ("stale".to_string(), 1u32),
            ("unchanged".to_string(), 2u32),
            ("raced".to_string(), 3u32),
        ]),
        &HashMap::from([
            ("stale".to_string(), 1u32),
            ("unchanged".to_string(), 2u32),
            ("raced".to_string(), 4u32),
        ]),
    );

    assert_eq!(
        result
            .iter()
            .map(|chat| chat.id.clone())
            .collect::<Vec<String>>(),
        vec!["unchanged".to_string(), "raced".to_string()]
    );
    assert_eq!(
        result
            .iter()
            .find(|chat| chat.id == "unchanged")
            .map(|chat| chat.last_message_preview.clone()),
        Some("authoritative".to_string())
    );
    assert_eq!(
        result
            .iter()
            .find(|chat| chat.id == "raced")
            .map(|chat| chat.last_message_preview.clone()),
        Some("live".to_string())
    );
}
