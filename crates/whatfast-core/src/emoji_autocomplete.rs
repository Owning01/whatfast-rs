//! Port of `tauri/src/lib/emoji-autocomplete.ts` (SolidJS frontend, frozen oracle).
//!
//! The module finds the colon shortcode that is being edited, ranks the
//! catalog of the picker for it and replaces the token when a suggestion is
//! accepted.
//!
//! port: the strings of JavaScript are UTF-16 and the strings of Rust are
//! UTF-8, so the offsets of the caret are the trap of this module (RULEBOOK
//! 3.1, decision D3):
//!
//! - the original works with the offsets of the UTF-16 code units of the
//!   draft; the port works with the offsets of the UTF-8 bytes of the draft.
//! - every shortcode character is ASCII, so the walk that finds the colon
//!   stops in the same place in both encodings. The query is ASCII too, so
//!   its length is the same in both.
//! - the caret of the replacement is the length of the emoji in the encoding
//!   of the port: the original counts the UTF-16 code units of the emoji and
//!   the port counts its UTF-8 bytes.
//! - the frozen suite of the oracle asserts the offsets in UTF-16 units, so
//!   the expectations of the Rust suite are translated (see
//!   `tests/emoji_autocomplete.rs`).

/// The longest query the module accepts.
pub const MAX_QUERY_LENGTH: usize = 48;

/// A row of the catalog of the picker: the emoji and its keywords.
pub type EmojiCatalogEntry = (String, String);

/// The colon shortcode that is being edited.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActiveEmojiShortcode {
    /// Inclusive offset of the leading colon, in UTF-8 bytes.
    pub start: usize,
    /// Exclusive offset of the complete token, including the text after the
    /// caret, in UTF-8 bytes.
    pub end: usize,
    /// Caret offset used to derive the query, in UTF-8 bytes.
    pub caret: usize,
    /// Text between the leading colon and the caret, normalized to lowercase.
    pub query: String,
    /// Complete token that will be replaced when a suggestion is accepted.
    pub token: String,
}

/// A suggestion of the picker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmojiSuggestion {
    pub emoji: String,
    /// Best matching keyword, useful as the human readable suggestion label.
    pub label: String,
    pub keywords: Vec<String>,
}

/// The active shortcode with the suggestions that match it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmojiAutocomplete {
    /// The shortcode that is being edited.
    ///
    /// port: the original names this field `match`, a reserved word in Rust.
    pub match_: ActiveEmojiShortcode,
    pub suggestions: Vec<EmojiSuggestion>,
}

/// The draft after accepting a suggestion, with the caret to restore.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmojiAutocompleteReplacement {
    pub text: String,
    /// Collapsed selection position immediately after the inserted emoji, in
    /// UTF-8 bytes.
    pub caret: usize,
}

/// Finds the colon shortcode currently being edited at `caret`.
///
/// A shortcode must begin at the start of the draft or after whitespace or
/// opening punctuation. Its body is deliberately ASCII compatible with
/// familiar emoji shortcodes. These constraints prevent false positives in
/// URLs, times, and words such as `https://`, `12:30` and `namespace:value`.
pub fn find_active_emoji_shortcode(text: &str, caret: usize) -> Option<ActiveEmojiShortcode> {
    // port: the original rejects a caret that is not an integer, is negative
    // or is past the end of the draft. A `usize` cannot be negative, and the
    // port also rejects a caret that is not the boundary of a character, the
    // original counted UTF-16 code units and could land in the middle of a
    // surrogate pair.
    if caret > text.len() || !text.is_char_boundary(caret) {
        return None;
    }

    let bytes = text.as_bytes();
    // port: the original walks the UTF-16 code units backwards from the
    // caret. The port walks the bytes backwards: every shortcode character
    // is ASCII, so both walks stop at the same character.
    let mut cursor = caret;
    while cursor > 0 && is_shortcode_character(bytes[cursor - 1]) {
        cursor -= 1;
    }
    if cursor == 0 {
        return None;
    }
    let start = cursor - 1;
    if bytes[start] != b':' {
        return None;
    }

    if start > 0 {
        // port: the original reads the character before the colon.
        if let Some(boundary) = text[..start].chars().next_back() {
            if !is_token_boundary(boundary) {
                return None;
            }
        }
    }

    // port: the query only holds shortcode characters, which are ASCII, so
    // its length in bytes is its length in UTF-16 code units.
    let query = &text[start + 1..caret];
    if query.len() > MAX_QUERY_LENGTH {
        return None;
    }

    let mut end = caret;
    while end < bytes.len() && is_shortcode_character(bytes[end]) {
        end += 1;
    }

    Some(ActiveEmojiShortcode {
        start,
        end,
        caret,
        query: query.to_lowercase(),
        token: text[start..end].to_string(),
    })
}

/// Ranks the picker catalog for a shortcode query, with the default limit.
pub fn rank_emoji_suggestions(query: &str, entries: &[EmojiCatalogEntry]) -> Vec<EmojiSuggestion> {
    rank_emoji_suggestions_with_limit(query, entries, 8)
}

/// Ranks the picker catalog for a shortcode query.
///
/// Exact keyword matches come first, followed by word prefix matches and then
/// substring matches. Catalog order breaks ties, so results remain stable.
pub fn rank_emoji_suggestions_with_limit(
    query: &str,
    entries: &[EmojiCatalogEntry],
    limit: usize,
) -> Vec<EmojiSuggestion> {
    // port: the original returns nothing when the limit is not a positive
    // integer. A `usize` cannot be negative, so only the zero remains.
    if limit == 0 {
        return Vec::new();
    }

    let normalized_query = trim_js_whitespace(query).to_lowercase();
    let mut ranked: Vec<(EmojiSuggestion, i32, usize)> = Vec::new();
    let mut seen: Vec<&str> = Vec::new();

    for (emoji, keyword_text) in entries {
        if seen.contains(&emoji.as_str()) {
            continue;
        }

        let keywords: Vec<String> = split_js_whitespace(&keyword_text.to_lowercase())
            .into_iter()
            .map(str::to_string)
            .collect();
        let Some((keyword, score)) = best_keyword_match(&normalized_query, &keywords) else {
            continue;
        };

        seen.push(emoji.as_str());
        ranked.push((
            EmojiSuggestion {
                emoji: emoji.clone(),
                label: keyword,
                keywords,
            },
            score,
            ranked.len(),
        ));
    }

    // port: the original sorts by the score and then by the order of the
    // catalog, and slices the first `limit` entries.
    ranked.sort_by(|left, right| left.1.cmp(&right.1).then(left.2.cmp(&right.2)));
    ranked
        .into_iter()
        .take(limit)
        .map(|(suggestion, _, _)| suggestion)
        .collect()
}

/// Resolves the active token and ranked suggestions in one integration call,
/// with the default limit.
pub fn get_emoji_autocomplete(
    text: &str,
    caret: usize,
    entries: &[EmojiCatalogEntry],
) -> Option<EmojiAutocomplete> {
    get_emoji_autocomplete_with_limit(text, caret, entries, 8)
}

/// Resolves the active token and ranked suggestions in one integration call.
pub fn get_emoji_autocomplete_with_limit(
    text: &str,
    caret: usize,
    entries: &[EmojiCatalogEntry],
    limit: usize,
) -> Option<EmojiAutocomplete> {
    let matched = find_active_emoji_shortcode(text, caret)?;
    let suggestions = rank_emoji_suggestions_with_limit(&matched.query, entries, limit);
    Some(EmojiAutocomplete {
        match_: matched,
        suggestions,
    })
}

/// Replaces exactly the matched token, including any token characters after the
/// caret, and returns the draft plus the collapsed caret position to restore.
pub fn apply_emoji_suggestion(
    text: &str,
    matched: &ActiveEmojiShortcode,
    emoji: &str,
) -> EmojiAutocompleteReplacement {
    // port: the original checks that the offsets are in range and that the
    // token is still the text of the draft. A `usize` cannot be negative, and
    // the port also checks that the offsets are the boundary of a character.
    let usable = !emoji.is_empty()
        && matched.end >= matched.start
        && matched.end <= text.len()
        && text.is_char_boundary(matched.start)
        && text.is_char_boundary(matched.end)
        && text[matched.start..matched.end] == matched.token;
    if !usable {
        return EmojiAutocompleteReplacement {
            text: text.to_string(),
            caret: matched.caret.min(text.len()),
        };
    }

    let mut next = String::with_capacity(text.len() + emoji.len());
    next.push_str(&text[..matched.start]);
    next.push_str(emoji);
    next.push_str(&text[matched.end..]);
    EmojiAutocompleteReplacement {
        text: next,
        // port: the original adds the length of the emoji in UTF-16 code
        // units; the port adds its length in UTF-8 bytes.
        caret: matched.start + emoji.len(),
    }
}

/// The best keyword of the entry for the query, and its score.
fn best_keyword_match(query: &str, keywords: &[String]) -> Option<(String, i32)> {
    if keywords.is_empty() {
        return None;
    }
    if query.is_empty() {
        return Some((keywords[0].clone(), 3));
    }

    if let Some(exact) = keywords.iter().find(|keyword| keyword.as_str() == query) {
        return Some((exact.clone(), 0));
    }

    if let Some(prefix) = keywords.iter().find(|keyword| keyword.starts_with(query)) {
        return Some((prefix.clone(), 1));
    }

    if let Some(substring) = keywords.iter().find(|keyword| keyword.contains(query)) {
        return Some((substring.clone(), 2));
    }

    None
}

/// port: `/[A-Za-z0-9_+-]/`.
fn is_shortcode_character(value: u8) -> bool {
    value.is_ascii_alphanumeric() || value == b'_' || value == b'+' || value == b'-'
}

/// port: `/[\s([{'"\u2018\u201c]/`.
fn is_token_boundary(value: char) -> bool {
    is_js_whitespace(value)
        || matches!(
            value,
            '(' | '[' | '{' | '\'' | '"' | '\u{2018}' | '\u{201C}'
        )
}

/// port: the `\s` of the regular expressions of the original.
fn is_js_whitespace(value: char) -> bool {
    matches!(
        value,
        '\u{0009}'..='\u{000D}'
            | '\u{0020}'
            | '\u{00A0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200A}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202F}'
            | '\u{205F}'
            | '\u{3000}'
            | '\u{FEFF}'
    )
}

/// port: `String.prototype.trim`.
fn trim_js_whitespace(value: &str) -> &str {
    value.trim_matches(is_js_whitespace)
}

/// port: `value.split(/\s+/).filter(Boolean)`.
fn split_js_whitespace(value: &str) -> Vec<&str> {
    value
        .split(is_js_whitespace)
        .filter(|part| !part.is_empty())
        .collect()
}
