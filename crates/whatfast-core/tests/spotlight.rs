//! Port of the frozen suite `tauri/tests/spotlight.test.ts`.

use whatfast_core::spotlight::{
    Chat, ChatKind, SpotlightChatMatch, SpotlightChatUsage, SpotlightUsage, rank_spotlight_chats,
};

/// port: `Date.UTC(2026, 6, 24, 12)`.
const NOW: i64 = 1_784_894_400_000;

fn direct(id: &str, title: &str) -> Chat {
    Chat {
        id: id.to_string(),
        kind: ChatKind::Direct,
        title: title.to_string(),
        avatar_path: String::new(),
        last_message_preview: String::new(),
        last_message_timestamp_ms: NOW - 30 * 86_400_000,
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

fn usage_of(opens: u32, last_opened_at: i64) -> SpotlightChatUsage {
    SpotlightChatUsage {
        opens,
        last_opened_at,
    }
}

fn ids(matches: &[SpotlightChatMatch]) -> Vec<String> {
    matches.iter().map(|entry| entry.chat.id.clone()).collect()
}

#[test]
fn empty_spotlight_puts_frequently_opened_chats_ahead_of_merely_recent_ones() {
    let mut recent = direct("recent", "Recent");
    recent.last_message_timestamp_ms = NOW - 60_000;
    let chats = vec![recent, direct("frequent", "Frequent")];

    let mut usage = SpotlightUsage::new();
    usage.insert("frequent".to_string(), usage_of(20, NOW - 86_400_000));

    assert_eq!(
        ids(&rank_spotlight_chats(&chats, "", &usage, NOW, 12)),
        vec!["frequent".to_string(), "recent".to_string()]
    );
}

#[test]
fn text_relevance_dominates_affinity_while_frequency_breaks_equal_matches() {
    let mut other = direct("other", "Amaya");
    other.contact_name = "Maya".to_string();
    let chats = vec![
        direct("exact", "Maya"),
        direct("prefix", "Maya Thomas"),
        other,
    ];

    let mut usage = SpotlightUsage::new();
    usage.insert("prefix".to_string(), usage_of(50, NOW));
    usage.insert("other".to_string(), usage_of(100, NOW));

    assert_eq!(
        ids(&rank_spotlight_chats(&chats, "maya", &usage, NOW, 12)),
        vec![
            "exact".to_string(),
            "prefix".to_string(),
            "other".to_string()
        ]
    );
}

#[test]
fn aliases_phone_numbers_multiple_terms_and_accents_are_searchable() {
    let mut joao = direct("joao", "João Silva");
    joao.phone_number = "+91 98765 43210".to_string();
    joao.business_name = "North Star".to_string();
    let chats = vec![joao, direct("other", "Someone Else")];

    assert_eq!(
        rank_spotlight_chats(&chats, "joao", &SpotlightUsage::new(), NOW, 12)
            .first()
            .map(|entry| entry.chat.id.clone()),
        Some("joao".to_string())
    );
    assert_eq!(
        rank_spotlight_chats(&chats, "north star", &SpotlightUsage::new(), NOW, 12)
            .first()
            .map(|entry| entry.chat.id.clone()),
        Some("joao".to_string())
    );
    assert_eq!(
        rank_spotlight_chats(&chats, "98765", &SpotlightUsage::new(), NOW, 12)
            .first()
            .map(|entry| entry.chat.id.clone()),
        Some("joao".to_string())
    );
}

#[test]
fn archived_chats_stay_out_of_instant_spotlight_results() {
    let mut archived = direct("archived", "Archived");
    archived.archived = true;
    let chats = vec![direct("visible", "Visible"), archived];

    assert_eq!(
        ids(&rank_spotlight_chats(
            &chats,
            "",
            &SpotlightUsage::new(),
            NOW,
            12
        )),
        vec!["visible".to_string()]
    );
}
