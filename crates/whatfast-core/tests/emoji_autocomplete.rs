//! Rust port of `tauri/tests/emoji-autocomplete.test.ts` (frozen oracle).
//!
//! Every case of the TypeScript suite is here with the same name translated
//! to snake_case and the same assertions.
//!
//! port: the offsets of the oracle are UTF-16 code units and the offsets of
//! the port are UTF-8 bytes (RULEBOOK 3.1, decision D3). Every draft of the
//! suite is ASCII, so the offsets of the draft are the same in both
//! encodings. The two cases that assert a caret built from the length of an
//! emoji are the ones that change: the oracle counts the UTF-16 code units of
//! the emoji and the port counts its UTF-8 bytes. The expectations below
//! carry the conversion.

use whatfast_core::emoji_autocomplete::{
    EmojiCatalogEntry, apply_emoji_suggestion, find_active_emoji_shortcode, get_emoji_autocomplete,
    rank_emoji_suggestions, rank_emoji_suggestions_with_limit,
};

/// The catalog of the oracle, in the same order.
fn catalog() -> Vec<EmojiCatalogEntry> {
    vec![
        ("😄".to_string(), "grinning smiling eyes happy".to_string()),
        ("🙂".to_string(), "slightly smiling happy".to_string()),
        ("❤️".to_string(), "red heart love".to_string()),
        ("💛".to_string(), "yellow heart".to_string()),
        ("💔".to_string(), "broken heartbreak sad".to_string()),
        ("🔥".to_string(), "fire hot lit".to_string()),
        ("😄".to_string(), "duplicate should not appear".to_string()),
    ]
}

/// The emoji and the label of a suggestion, the shape the oracle asserts.
fn pairs(
    suggestions: &[whatfast_core::emoji_autocomplete::EmojiSuggestion],
) -> Vec<(String, String)> {
    suggestions
        .iter()
        .map(|suggestion| (suggestion.emoji.clone(), suggestion.label.clone()))
        .collect()
}

/// The emoji of a suggestion, the shape the oracle asserts.
fn glyphs(suggestions: &[whatfast_core::emoji_autocomplete::EmojiSuggestion]) -> Vec<String> {
    suggestions
        .iter()
        .map(|suggestion| suggestion.emoji.clone())
        .collect()
}

#[test]
fn finds_an_active_shortcode_at_the_start_or_a_sensible_token_boundary() {
    let at_start = find_active_emoji_shortcode(":smile", 6);
    assert_eq!(
        at_start,
        Some(whatfast_core::emoji_autocomplete::ActiveEmojiShortcode {
            start: 0,
            end: 6,
            caret: 6,
            query: "smile".to_string(),
            token: ":smile".to_string(),
        })
    );

    let text = "hello (:HEART world";
    let after_a_bracket = find_active_emoji_shortcode(text, 13);
    assert_eq!(
        after_a_bracket,
        Some(whatfast_core::emoji_autocomplete::ActiveEmojiShortcode {
            start: 7,
            end: 13,
            caret: 13,
            query: "heart".to_string(),
            token: ":HEART".to_string(),
        })
    );
}

#[test]
fn a_bare_colon_activates_autocomplete() {
    assert_eq!(
        find_active_emoji_shortcode("hello :", 7),
        Some(whatfast_core::emoji_autocomplete::ActiveEmojiShortcode {
            start: 6,
            end: 7,
            caret: 7,
            query: String::new(),
            token: ":".to_string(),
        })
    );
}

#[test]
fn ignores_urls_times_and_colons_embedded_in_words() {
    assert_eq!(find_active_emoji_shortcode("https://example.com", 6), None);
    assert_eq!(find_active_emoji_shortcode("meet at 12:30", 13), None);
    assert_eq!(find_active_emoji_shortcode("namespace:value", 15), None);
    assert_eq!(find_active_emoji_shortcode("word.:heart", 11), None);
}

#[test]
fn rejects_unsupported_shortcode_characters_and_invalid_carets() {
    assert_eq!(find_active_emoji_shortcode("hello :two words", 16), None);
    assert_eq!(find_active_emoji_shortcode("hello :smile", 99), None);
    assert_eq!(
        find_active_emoji_shortcode(&format!(":{}", "a".repeat(49)), 50),
        None
    );
    // port: the oracle also rejects a negative caret; a `usize` cannot be
    // negative, so that half of the case has no representation here.
}

#[test]
fn detects_and_replaces_the_whole_token_when_editing_in_its_middle() {
    let text = "before :smiley after";
    let matched = find_active_emoji_shortcode(text, 11);
    assert!(matched.is_some());
    let Some(matched) = matched else { return };
    assert_eq!(matched.query, "smi");
    assert_eq!(matched.token, ":smiley");

    let replacement = apply_emoji_suggestion(text, &matched, "😄");
    assert_eq!(replacement.text, "before 😄 after");
    // port: the oracle asserts 9, the caret in UTF-16 code units (7 + 2 for
    // "😄"); the port counts UTF-8 bytes, and "😄" is 4 bytes, so the caret
    // is 7 + 4 = 11.
    assert_eq!(replacement.caret, 11);
}

#[test]
fn replacement_preserves_all_surrounding_text_and_reports_a_utf_16_caret() {
    let text = "one :heart two";
    let matched = find_active_emoji_shortcode(text, 10);
    assert!(matched.is_some());
    let Some(matched) = matched else { return };
    let replacement = apply_emoji_suggestion(text, &matched, "❤️");
    assert_eq!(replacement.text, "one ❤️ two");
    // port: the oracle asserts 6, the caret in UTF-16 code units (4 + 2 for
    // "❤️"); the port counts UTF-8 bytes, and "❤️" is 6 bytes, so the
    // caret is 4 + 6 = 10.
    assert_eq!(replacement.caret, 10);
}

#[test]
fn replacement_refuses_a_stale_match_rather_than_overwriting_changed_text() {
    let matched = find_active_emoji_shortcode("say :heart", 10);
    assert!(matched.is_some());
    let Some(matched) = matched else { return };
    let replacement = apply_emoji_suggestion("say :smile", &matched, "❤️");
    assert_eq!(replacement.text, "say :smile");
    assert_eq!(replacement.caret, 10);
}

#[test]
fn ranks_exact_keyword_prefix_then_substring_matches_stably() {
    assert_eq!(
        pairs(&rank_emoji_suggestions("heart", &catalog())),
        vec![
            ("❤️".to_string(), "heart".to_string()),
            ("💛".to_string(), "heart".to_string()),
            ("💔".to_string(), "heartbreak".to_string()),
        ]
    );

    assert_eq!(
        pairs(&rank_emoji_suggestions("light", &catalog())),
        vec![("🙂".to_string(), "slightly".to_string())]
    );
}

#[test]
fn deduplicates_glyphs_honors_the_result_limit_and_handles_a_bare_query() {
    assert_eq!(
        glyphs(&rank_emoji_suggestions_with_limit("", &catalog(), 2)),
        vec!["😄".to_string(), "🙂".to_string()]
    );
    assert_eq!(
        glyphs(&rank_emoji_suggestions("smil", &catalog())),
        vec!["😄".to_string(), "🙂".to_string()]
    );
    assert_eq!(
        rank_emoji_suggestions_with_limit("heart", &catalog(), 0),
        vec![]
    );
}

#[test]
fn integration_helper_returns_the_active_match_with_ranked_suggestions() {
    let result = get_emoji_autocomplete("Love :hea today", 9, &catalog());
    assert!(result.is_some());
    let Some(result) = result else { return };
    assert_eq!(result.match_.token, ":hea");
    assert_eq!(
        glyphs(&result.suggestions),
        vec!["❤️".to_string(), "💛".to_string(), "💔".to_string()]
    );
    assert_eq!(get_emoji_autocomplete("https://", 6, &catalog()), None);
}
