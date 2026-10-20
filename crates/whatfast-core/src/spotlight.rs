//! Port of `tauri/src/lib/spotlight.ts` (SolidJS frontend, frozen oracle).
//!
//! The ranking of the chats and the normalization of the query are pure and
//! live here. The affinity the launcher remembers per chat lives in
//! `localStorage` in the original: the access to the store belongs to the UI
//! crate, so this module offers the pure parts ([`parse_spotlight_usage`],
//! [`serialize_spotlight_usage`], [`record_spotlight_chat_use`] and
//! [`merge_spotlight_chat_usage`]), the same way `state/app-helpers.ts` was
//! split.
//!
//! Fidelity notes (the odd bits are deliberate, they are the oracle's):
//! - `normalize` is `NFKD` + "drop the diacritics" + lowercase + trim. The
//!   crate takes no new dependency, so the decomposition of the Latin blocks
//!   is implemented by hand ([`strip_diacritics`]).
//! - the tie break of the ranking is `localeCompare`, the ICU collation of
//!   the runtime and not the byte order ([`locale_compare`]).
//! - the affinity map is a plain object in the original: its insertion order
//!   decides which entries survive the trim when two of them were opened the
//!   same millisecond, so the port keeps the insertion order.

use std::cmp::Ordering;

use serde::Serialize;

pub use crate::types::{Chat, ChatKind};

/// The `localStorage` key the affinity lives under.
pub const SPOTLIGHT_STORAGE_KEY: &str = "whatfast-spotlight-affinity-v1";

/// The affinity map keeps at most this many chats.
pub const MAX_TRACKED_CHATS: usize = 250;

/// The counter of opens of one chat is capped at this value.
pub const MAX_OPEN_COUNT: u32 = 10_000;

/// port: the day in milliseconds the original hardcodes.
const DAY_MS: f64 = 86_400_000.0;

/// port: the kind of chat that gets the direct bonus.
const DIRECT_CHAT_KIND: ChatKind = ChatKind::Direct;

/// How many times the user opened a chat, and when.
///
/// port: the serde names are the ones of the payload the original stores.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpotlightChatUsage {
    pub opens: u32,
    pub last_opened_at: i64,
}

/// The affinity the launcher remembers per chat.
///
/// port: the original is a `Record<string, SpotlightChatUsage>`, a plain
/// object: the insertion order is part of the behavior, so the port keeps it
/// instead of using a sorted map.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SpotlightUsage {
    entries: Vec<(String, SpotlightChatUsage)>,
}

impl SpotlightUsage {
    /// An empty map.
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// How many chats are tracked.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the map holds nothing.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The usage of one chat.
    pub fn get(&self, chat_id: &str) -> Option<&SpotlightChatUsage> {
        self.entries
            .iter()
            .find(|(id, _)| id == chat_id)
            .map(|(_, usage)| usage)
    }

    /// The usage of one chat, copied.
    pub fn get_copy(&self, chat_id: &str) -> Option<SpotlightChatUsage> {
        self.get(chat_id).copied()
    }

    /// Remember the usage of one chat.
    ///
    /// port: overwriting an existing key keeps its position in the object.
    pub fn insert(&mut self, chat_id: String, usage: SpotlightChatUsage) {
        match self.entries.iter_mut().find(|(id, _)| id == &chat_id) {
            Some(entry) => entry.1 = usage,
            None => self.entries.push((chat_id, usage)),
        }
    }

    /// Forget the usage of one chat.
    pub fn remove(&mut self, chat_id: &str) -> Option<SpotlightChatUsage> {
        let index = self.entries.iter().position(|(id, _)| id == chat_id)?;
        Some(self.entries.remove(index).1)
    }

    /// The tracked chats, in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &SpotlightChatUsage)> {
        self.entries.iter().map(|(id, usage)| (id.as_str(), usage))
    }
}

/// One ranked chat.
#[derive(Clone, Debug, PartialEq)]
pub struct SpotlightChatMatch {
    pub chat: Chat,
    pub score: f64,
    pub usage: Option<SpotlightChatUsage>,
}

/// Rank the already-loaded chats for the instant part of Spotlight.
///
/// Text relevance dominates while a query is present. Within equally relevant
/// matches, the chats the user opens repeatedly rank ahead of the merely
/// recent ones. With an empty query the same affinity score becomes the
/// primary ordering, so the launcher opens with useful people instead of an
/// arbitrary alphabetical list.
///
/// port: the original takes the instant (`Date.now()`) and the limit as
/// parameters with defaults; the port takes both, so the ranking is testable
/// without a clock.
pub fn rank_spotlight_chats(
    chats: &[Chat],
    query: &str,
    usage: &SpotlightUsage,
    now: i64,
    limit: usize,
) -> Vec<SpotlightChatMatch> {
    // port: `if (!Number.isInteger(limit) || limit <= 0) return []`. The port
    // takes an unsigned integer, so the integer check is structural and only
    // the empty limit is left.
    if limit == 0 {
        return Vec::new();
    }
    let normalized_query = normalize(js_trim(query));
    let terms: Vec<String> = normalized_query
        .split_whitespace()
        .map(str::to_string)
        .collect();

    let mut matches: Vec<SpotlightChatMatch> = Vec::new();
    for chat in chats {
        if chat.archived {
            continue;
        }
        let text_score = spotlight_text_score(chat, &normalized_query, &terms);
        if !normalized_query.is_empty() && text_score < 0.0 {
            continue;
        }
        let chat_usage = usage.get_copy(&chat.id);
        let score = text_score * 10_000.0 + affinity_score(chat, chat_usage.as_ref(), now);
        matches.push(SpotlightChatMatch {
            chat: chat.clone(),
            usage: chat_usage,
            score,
        });
    }

    matches.sort_by(|left, right| {
        let by_score = match right.score.partial_cmp(&left.score) {
            Some(ordering) => ordering,
            None => Ordering::Equal,
        };
        by_score
            .then_with(|| {
                right
                    .chat
                    .last_message_timestamp_ms
                    .cmp(&left.chat.last_message_timestamp_ms)
            })
            .then_with(|| locale_compare(&left.chat.id, &right.chat.id))
    });
    matches.truncate(limit);
    matches
}

/// Read the affinity out of the raw payload stored in `localStorage`.
///
/// port: `None` is the "key missing" case (the original parses `"{}"` then),
/// a broken payload yields an empty map and the entries that do not carry
/// the two numbers are dropped.
pub fn parse_spotlight_usage(raw: Option<&str>) -> SpotlightUsage {
    let Some(raw) = raw else {
        return SpotlightUsage::new();
    };
    let Ok(parsed) = serde_json::from_str::<OrderedObject>(raw) else {
        return SpotlightUsage::new();
    };
    // port: the original rejects anything that is not a plain object, and
    // `Array.isArray(parsed)` rejects the arrays.
    if parsed.is_array {
        return SpotlightUsage::new();
    }

    let mut usage = SpotlightUsage::new();
    for (chat_id, value) in parsed.entries {
        if chat_id.is_empty() {
            continue;
        }
        let Some(candidate) = value.as_object() else {
            continue;
        };
        let Some(opens) = candidate.get("opens").and_then(serde_json::Value::as_f64) else {
            continue;
        };
        if !opens.is_finite() {
            continue;
        }
        let Some(last_opened_at) = candidate
            .get("lastOpenedAt")
            .and_then(serde_json::Value::as_f64)
        else {
            continue;
        };
        if !last_opened_at.is_finite() {
            continue;
        }
        usage.insert(
            chat_id,
            SpotlightChatUsage {
                opens: clamp_opens(opens),
                last_opened_at: last_opened_at.max(0.0) as i64,
            },
        );
    }
    usage
}

/// A JSON value that keeps the order of the keys of its object.
///
/// port: the original reads the affinity with `Object.entries`, which keeps
/// the order of the document. The map of `serde_json` sorts its keys, so the
/// port reads the object with a visitor that keeps the order of the input.
struct OrderedObject {
    entries: Vec<(String, serde_json::Value)>,
    is_array: bool,
}

impl<'de> serde::Deserialize<'de> for OrderedObject {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct ObjectVisitor;

        impl<'de> serde::de::Visitor<'de> for ObjectVisitor {
            type Value = OrderedObject;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a JSON object")
            }

            fn visit_map<A>(self, mut access: A) -> Result<OrderedObject, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                let mut entries = Vec::new();
                while let Some((key, value)) = access.next_entry::<String, serde_json::Value>()? {
                    entries.push((key, value));
                }
                Ok(OrderedObject {
                    entries,
                    is_array: false,
                })
            }

            fn visit_seq<A>(self, mut access: A) -> Result<OrderedObject, A::Error>
            where
                A: serde::de::SeqAccess<'de>,
            {
                while access.next_element::<serde_json::Value>()?.is_some() {}
                Ok(OrderedObject {
                    entries: Vec::new(),
                    is_array: true,
                })
            }
        }

        deserializer.deserialize_any(ObjectVisitor)
    }
}

/// Serialize the affinity for `localStorage`.
pub fn serialize_spotlight_usage(usage: &SpotlightUsage) -> String {
    let mut serialized = String::from("{");
    for (index, (chat_id, value)) in usage.iter().enumerate() {
        if index > 0 {
            serialized.push(',');
        }
        serialized.push_str(&serde_json::to_string(chat_id).unwrap_or_default());
        serialized.push(':');
        serialized.push_str(&serde_json::to_string(value).unwrap_or_default());
    }
    serialized.push('}');
    serialized
}

/// Count one more open of a chat.
///
/// port: the original writes the result back to `localStorage`; the storage
/// effect belongs to the UI crate, the decision does not.
pub fn record_spotlight_chat_use(chat_id: &str, now: i64, usage: &mut SpotlightUsage) {
    if chat_id.is_empty() {
        return;
    }
    let previous = match usage.get(chat_id) {
        Some(previous) => previous.opens,
        None => 0,
    };
    usage.insert(
        chat_id.to_string(),
        SpotlightChatUsage {
            opens: previous.saturating_add(1).min(MAX_OPEN_COUNT),
            last_opened_at: now,
        },
    );
    *usage = trim_usage(usage.clone());
}

/// Move the affinity of a chat to its new id after a JID merge.
///
/// port: the original writes the result back to `localStorage`; the storage
/// effect belongs to the UI crate, the decision does not.
pub fn merge_spotlight_chat_usage(old_id: &str, new_id: &str, usage: &mut SpotlightUsage) {
    if old_id.is_empty() || new_id.is_empty() || old_id == new_id {
        return;
    }
    let Some(old_usage) = usage.get_copy(old_id) else {
        return;
    };
    let new_usage = usage.get_copy(new_id);
    let opens = match new_usage {
        Some(new_usage) => old_usage.opens.saturating_add(new_usage.opens),
        None => old_usage.opens,
    };
    let last_opened_at = match new_usage {
        Some(new_usage) => old_usage.last_opened_at.max(new_usage.last_opened_at),
        None => old_usage.last_opened_at,
    };
    usage.insert(
        new_id.to_string(),
        SpotlightChatUsage {
            opens: opens.min(MAX_OPEN_COUNT),
            last_opened_at,
        },
    );
    usage.remove(old_id);
    *usage = trim_usage(usage.clone());
}

/// The text score of one chat against the query.
///
/// port: `-1` means "the chat does not match".
fn spotlight_text_score(chat: &Chat, query: &str, terms: &[String]) -> f64 {
    if query.is_empty() {
        return 0.0;
    }
    // port: `chat.title || chat.phoneNumber` is falsy based: an empty title
    // falls through to the phone number.
    let title = normalize(if chat.title.is_empty() {
        &chat.phone_number
    } else {
        &chat.title
    });
    let aliases: Vec<String> = [
        chat.contact_name.as_str(),
        chat.push_name.as_str(),
        chat.business_name.as_str(),
        chat.phone_number.as_str(),
    ]
    .iter()
    .map(|field| normalize(field))
    .filter(|field| !field.is_empty())
    .collect();

    let mut fields = vec![title.clone()];
    fields.extend(aliases.iter().cloned());
    if !terms
        .iter()
        .all(|term| fields.iter().any(|field| field.contains(term.as_str())))
    {
        return -1.0;
    }

    if title == query {
        return 1_000.0;
    }
    if title.starts_with(query) {
        return 900.0;
    }
    if title.split_whitespace().any(|word| word.starts_with(query)) {
        return 825.0;
    }
    if title.contains(query) {
        return 750.0;
    }
    if aliases.iter().any(|field| field == query) {
        return 700.0;
    }
    if aliases.iter().any(|field| field.starts_with(query)) {
        return 650.0;
    }
    if aliases.iter().any(|field| field.contains(query)) {
        return 600.0;
    }
    500.0
}

/// The affinity score of one chat.
fn affinity_score(chat: &Chat, usage: Option<&SpotlightChatUsage>, now: i64) -> f64 {
    let opens = match usage {
        Some(usage) => usage.opens,
        None => 0,
    };
    let frequency = 175.0 * (f64::from(opens) + 1.0).log2();
    let frequency = frequency.min(620.0);

    let last_opened_at = match usage {
        Some(usage) => usage.last_opened_at,
        None => 0,
    };
    let opened_age_days = (now - last_opened_at).max(0) as f64 / DAY_MS;
    let opened_recency = match usage {
        Some(_) => (280.0 - opened_age_days * 12.0).max(0.0),
        None => 0.0,
    };

    let activity_age_days = (now - chat.last_message_timestamp_ms).max(0) as f64 / DAY_MS;
    let activity_recency = if chat.last_message_timestamp_ms != 0 {
        (180.0 - activity_age_days * 5.0).max(0.0)
    } else {
        0.0
    };

    let direct_bonus = if chat.kind == DIRECT_CHAT_KIND {
        35.0
    } else {
        0.0
    };
    let pinned_bonus = if chat.pinned { 65.0 } else { 0.0 };

    frequency + opened_recency + activity_recency + direct_bonus + pinned_bonus
}

/// port: `Math.max(0, Math.min(MAX_OPEN_COUNT, Math.floor(opens)))`.
fn clamp_opens(opens: f64) -> u32 {
    let floored = opens.floor();
    if floored <= 0.0 {
        return 0;
    }
    if floored >= f64::from(MAX_OPEN_COUNT) {
        return MAX_OPEN_COUNT;
    }
    floored as u32
}

/// port: the newest affinities win the room of the map.
fn trim_usage(mut usage: SpotlightUsage) -> SpotlightUsage {
    // port: `Array.prototype.sort` is stable, so two entries opened the same
    // millisecond keep their insertion order.
    usage
        .entries
        .sort_by_key(|entry| std::cmp::Reverse(entry.1.last_opened_at));
    if usage.entries.len() > MAX_TRACKED_CHATS {
        usage.entries.truncate(MAX_TRACKED_CHATS);
    }
    usage
}

/// port: `value.normalize("NFKD").replace(/\p{Diacritic}/gu, "").toLocaleLowerCase().trim()`.
///
/// The crate takes no new dependency, so the `NFKD` decomposition is done by
/// hand for the Latin blocks (the Latin-1 Supplement and the Latin Extended-A
/// that the names of the chats use). The letters without a decomposition
/// (`ß`, `ø`, `æ`, `Đ`, `ł`…) are kept as they are, exactly like the
/// original does.
fn normalize(value: &str) -> String {
    strip_diacritics(value).to_lowercase().trim().to_string()
}

/// port: the `NFKD` decomposition of the Latin blocks, without the combining
/// marks the original drops.
fn strip_diacritics(value: &str) -> String {
    let mut stripped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' => stripped.push('a'),
            'À' | 'Á' | 'Â' | 'Ã' | 'Ä' | 'Å' | 'Ā' | 'Ă' | 'Ą' => stripped.push('A'),
            'ç' | 'ć' | 'ĉ' | 'ċ' | 'č' => stripped.push('c'),
            'Ç' | 'Ć' | 'Ĉ' | 'Ċ' | 'Č' => stripped.push('C'),
            'ď' => stripped.push('d'),
            'Ď' => stripped.push('D'),
            'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' => stripped.push('e'),
            'È' | 'É' | 'Ê' | 'Ë' | 'Ē' | 'Ĕ' | 'Ė' | 'Ę' | 'Ě' => stripped.push('E'),
            'ì' | 'í' | 'î' | 'ï' | 'ĩ' | 'ī' | 'ĭ' | 'į' => stripped.push('i'),
            'Ì' | 'Í' | 'Î' | 'Ï' | 'Ĩ' | 'Ī' | 'Ĭ' | 'Į' => stripped.push('I'),
            'ñ' | 'ń' | 'ņ' | 'ň' => stripped.push('n'),
            'Ñ' | 'Ń' | 'Ņ' | 'Ň' => stripped.push('N'),
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ō' | 'ŏ' | 'ő' => stripped.push('o'),
            'Ò' | 'Ó' | 'Ô' | 'Õ' | 'Ö' | 'Ō' | 'Ŏ' | 'Ő' => stripped.push('O'),
            'ř' => stripped.push('r'),
            'Ř' => stripped.push('R'),
            'ś' | 'ŝ' | 'ş' | 'š' => stripped.push('s'),
            'Ś' | 'Ŝ' | 'Ş' | 'Š' => stripped.push('S'),
            'ţ' | 'ť' | 'ŧ' => stripped.push('t'),
            'Ţ' | 'Ť' | 'Ŧ' => stripped.push('T'),
            'ù' | 'ú' | 'û' | 'ü' | 'ũ' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' => {
                stripped.push('u')
            }
            'Ù' | 'Ú' | 'Û' | 'Ü' | 'Ũ' | 'Ū' | 'Ŭ' | 'Ů' | 'Ű' | 'Ų' => {
                stripped.push('U')
            }
            'ŵ' => stripped.push('w'),
            'Ŵ' => stripped.push('W'),
            'ý' | 'ŷ' => stripped.push('y'),
            'Ý' | 'Ÿ' | 'Ŷ' => stripped.push('Y'),
            'ź' | 'ż' | 'ž' => stripped.push('z'),
            'Ź' | 'Ż' | 'Ž' => stripped.push('Z'),
            // port: the combining marks the original drops with
            // `/\p{Diacritic}/gu`.
            '\u{300}'..='\u{36f}'
            | '\u{1ab0}'..='\u{1aff}'
            | '\u{1dc0}'..='\u{1dff}'
            | '\u{20d0}'..='\u{20ff}'
            | '\u{fe20}'..='\u{fe2f}' => {}
            other => stripped.push(other),
        }
    }
    stripped
}

/// port: `String.prototype.localeCompare` uses the ICU collation of the
/// runtime, which is not the byte order: `"a"` sorts before `"A"` and before
/// `"B"`. The port reproduces the two levels that matter for ids (the
/// case-insensitive primary weight and the lowercase before uppercase as the
/// tertiary weight) and falls back to the byte order for the rest.
fn locale_compare(left: &str, right: &str) -> Ordering {
    let left_primary = left.to_lowercase();
    let right_primary = right.to_lowercase();
    match left_primary.cmp(&right_primary) {
        Ordering::Equal => {
            for (left_char, right_char) in left.chars().zip(right.chars()) {
                let left_lower = left_char.is_lowercase();
                let right_lower = right_char.is_lowercase();
                if left_lower != right_lower {
                    return if left_lower {
                        Ordering::Less
                    } else {
                        Ordering::Greater
                    };
                }
            }
            left.cmp(right)
        }
        other => other,
    }
}

/// port: `String.prototype.trim`.
fn js_trim(value: &str) -> &str {
    value.trim()
}
