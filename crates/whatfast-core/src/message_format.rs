//! Port of `tauri/src/lib/format.ts`.
//!
//! The original renders the timestamps of the sidebar and the conversation,
//! the text of a message, the initials of an avatar, the colour of an avatar,
//! the size of a file, the length of a voice note and the connection label.
//!
//! port: three things the original takes from the runtime cannot be ported
//! as they are, and the port injects them instead:
//!
//! - `new Date()` (the "now" of `formatChatTime` and `formatDay`) becomes the
//!   `now` parameter, as RULEBOOK 1.4 asks.
//! - `Intl.DateTimeFormat(undefined, ...)` follows the locale of the runtime.
//!   The crate has no ICU, so the port takes the locale as a parameter and
//!   implements the two locales the app ships (`en` and `es`).
//! - The local time zone needs a time zone database, which `std` does not
//!   have. The port takes the offset from UTC in minutes, so the caller
//!   resolves it (the caller is the only one that can, it knows the operating
//!   system). The offset is a fixed one: a zone with daylight saving time
//!   needs the caller to resolve the offset of the exact instant.
//! - `t("message.deleted")` follows the locale of the UI. The port keeps the
//!   English text of the default locale of the original.
//!
//! The characterisation values of every function below come from the oracle
//! module itself, see `tests/message_format.rs`.
//!
//! port: the model the original imports from `tauri/src/lib/types.ts` lives in
//! [`crate::types`]. The formatters read a handful of fields of the chat and
//! of the message, so they work with the projections
//! [`crate::types::ChatSummary`] and [`crate::types::MessageSummary`]; the
//! module exports them under the names `Chat` and `Message` to keep the public
//! path of the module stable.

pub use crate::types::{
    AttachmentContent, ChatKind, ContactContent, ContactsContent, ImageContent, LinkPreview,
    LocationContent, MessageContent, PollContent, PollOption, PollVoter, Reaction, TextContent,
    UnsupportedContent,
};
// port: the projections the formatters read. The canonical DTOs of the app are
// `crate::types::Chat` and `crate::types::Message`.
pub use crate::types::{ChatSummary as Chat, MessageSummary as Message};

// -------------------------------------------------------------- the clock

/// The locale the date formatters render with.
///
/// port: the original leaves the locale to the runtime of `Intl`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DateLocale {
    /// `en-US`: `7:10 PM`, `Sun`, `10/12/26`, `Sunday, October 12`.
    #[default]
    EnUs,
    /// `es-AR`: `7:10 p. m.`, `dom`, `12/10/26`, `domingo, 12 de octubre`.
    EsAr,
}

/// An instant plus the offset of the time zone it has to be rendered in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalTimestamp {
    /// Milliseconds since the epoch, UTC.
    pub epoch_ms: i64,
    /// Minutes east of UTC of the time zone of the renderer.
    pub utc_offset_minutes: i32,
}

impl LocalTimestamp {
    /// An instant rendered with the given offset.
    pub fn new(epoch_ms: i64, utc_offset_minutes: i32) -> Self {
        LocalTimestamp {
            epoch_ms,
            utc_offset_minutes,
        }
    }
}

/// The calendar fields of an instant, already in the local time zone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CivilDateTime {
    year: i64,
    /// 1 to 12.
    month: u32,
    /// 1 to 31.
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
    /// 0 is Sunday.
    weekday: u32,
}

/// Days since 1970-01-01 of a calendar date.
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let shifted_year = if month <= 2 { year - 1 } else { year };
    let era = if shifted_year >= 0 {
        shifted_year
    } else {
        shifted_year - 399
    }
    .div_euclid(400);
    let year_of_era = shifted_year - era * 400;
    let month = i64::from(month);
    let day_of_year =
        (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// The calendar date of a number of days since 1970-01-01.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    }
    .div_euclid(146_097);
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = if month <= 2 { year + 1 } else { year };
    (year, month as u32, day as u32)
}

/// The calendar fields of an instant in the given time zone.
fn civil_from_epoch(epoch_ms: i64, utc_offset_minutes: i32) -> CivilDateTime {
    let local_ms = epoch_ms + i64::from(utc_offset_minutes) * 60_000;
    let days = local_ms.div_euclid(86_400_000);
    let millis_of_day = local_ms.rem_euclid(86_400_000);
    let (year, month, day) = civil_from_days(days);
    CivilDateTime {
        year,
        month,
        day,
        hour: (millis_of_day / 3_600_000) as u32,
        minute: (millis_of_day / 60_000 % 60) as u32,
        second: (millis_of_day / 1_000 % 60) as u32,
        // 1970-01-01 was a Thursday.
        weekday: (days + 4).rem_euclid(7) as u32,
    }
}

/// The day before the given one, in the same time zone.
fn previous_day(value: &CivilDateTime) -> CivilDateTime {
    let days = days_from_civil(value.year, value.month, value.day) - 1;
    let (year, month, day) = civil_from_days(days);
    CivilDateTime {
        year,
        month,
        day,
        ..*value
    }
}

/// port: the `sameDay` helper of the original.
fn same_day(left: &CivilDateTime, right: &CivilDateTime) -> bool {
    left.year == right.year && left.month == right.month && left.day == right.day
}

const EN_US_WEEKDAYS_SHORT: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const EN_US_WEEKDAYS_LONG: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];
const EN_US_MONTHS_LONG: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

const ES_AR_WEEKDAYS_SHORT: [&str; 7] = ["dom", "lun", "mar", "mié", "jue", "vie", "sáb"];
const ES_AR_WEEKDAYS_LONG: [&str; 7] = [
    "domingo",
    "lunes",
    "martes",
    "miércoles",
    "jueves",
    "viernes",
    "sábado",
];
const ES_AR_MONTHS_LONG: [&str; 12] = [
    "enero",
    "febrero",
    "marzo",
    "abril",
    "mayo",
    "junio",
    "julio",
    "agosto",
    "septiembre",
    "octubre",
    "noviembre",
    "diciembre",
];

/// port: `timeFormatter`, `hour: "numeric"` and `minute: "2-digit"`.
fn format_time_of_day(value: &CivilDateTime, locale: DateLocale) -> String {
    let hour = match value.hour % 12 {
        0 => 12,
        other => other,
    };
    let minute = format!("{:02}", value.minute);
    match locale {
        DateLocale::EnUs => {
            let period = if value.hour < 12 { "AM" } else { "PM" };
            format!("{hour}:{minute} {period}")
        }
        DateLocale::EsAr => {
            let period = if value.hour < 12 { "a. m." } else { "p. m." };
            format!("{hour}:{minute} {period}")
        }
    }
}

/// port: `weekdayFormatter`, `weekday: "short"`.
fn format_weekday_short(weekday: u32, locale: DateLocale) -> String {
    let index = (weekday as usize) % 7;
    match locale {
        DateLocale::EnUs => EN_US_WEEKDAYS_SHORT[index].to_string(),
        DateLocale::EsAr => ES_AR_WEEKDAYS_SHORT[index].to_string(),
    }
}

/// port: `weekdayFormatter`, `weekday: "long"`.
fn format_weekday_long(weekday: u32, locale: DateLocale) -> String {
    let index = (weekday as usize) % 7;
    match locale {
        DateLocale::EnUs => EN_US_WEEKDAYS_LONG[index].to_string(),
        DateLocale::EsAr => ES_AR_WEEKDAYS_LONG[index].to_string(),
    }
}

/// port: `shortDateFormatter`, `day: "2-digit"`, `month: "2-digit"` and
/// `year: "2-digit"`.
fn format_short_date(value: &CivilDateTime, locale: DateLocale) -> String {
    let year = value.year.rem_euclid(100);
    match locale {
        DateLocale::EnUs => format!("{:02}/{:02}/{:02}", value.month, value.day, year),
        DateLocale::EsAr => format!("{:02}/{:02}/{:02}", value.day, value.month, year),
    }
}

/// port: `dayFormatter`, `weekday: "long"`, `day: "numeric"` and
/// `month: "long"`.
fn format_day_long(value: &CivilDateTime, locale: DateLocale) -> String {
    let weekday = format_weekday_long(value.weekday, locale);
    let month = match locale {
        DateLocale::EnUs => EN_US_MONTHS_LONG[month_index(value.month)],
        DateLocale::EsAr => ES_AR_MONTHS_LONG[month_index(value.month)],
    };
    match locale {
        DateLocale::EnUs => format!("{weekday}, {month} {}", value.day),
        DateLocale::EsAr => format!("{weekday}, {} de {month}", value.day),
    }
}

// ---------------------------------------------------------- the formatters

/// port: `formatTime`.
pub fn format_time(value: LocalTimestamp, locale: DateLocale) -> String {
    // port: `!timestampMs` is true for 0.
    if value.epoch_ms == 0 {
        return String::new();
    }
    format_time_of_day(
        &civil_from_epoch(value.epoch_ms, value.utc_offset_minutes),
        locale,
    )
}

/// port: `formatChatTime`.
pub fn format_chat_time(value: LocalTimestamp, now: LocalTimestamp, locale: DateLocale) -> String {
    if value.epoch_ms == 0 {
        return String::new();
    }
    let value_civil = civil_from_epoch(value.epoch_ms, value.utc_offset_minutes);
    let now_civil = civil_from_epoch(now.epoch_ms, now.utc_offset_minutes);
    if same_day(&value_civil, &now_civil) {
        return format_time_of_day(&value_civil, locale);
    }
    if same_day(&value_civil, &previous_day(&now_civil)) {
        // port: the original hardcodes "Yesterday" here.
        return "Yesterday".to_string();
    }
    let age = now.epoch_ms - value.epoch_ms;
    if age < 7 * 86_400_000 {
        return format_weekday_short(value_civil.weekday, locale);
    }
    format_short_date(&value_civil, locale)
}

/// port: `formatDay`.
pub fn format_day(value: LocalTimestamp, now: LocalTimestamp, locale: DateLocale) -> String {
    let value_civil = civil_from_epoch(value.epoch_ms, value.utc_offset_minutes);
    let now_civil = civil_from_epoch(now.epoch_ms, now.utc_offset_minutes);
    if same_day(&value_civil, &now_civil) {
        return "Today".to_string();
    }
    if same_day(&value_civil, &previous_day(&now_civil)) {
        return "Yesterday".to_string();
    }
    format_day_long(&value_civil, locale)
}

/// port: `dayKey`.
///
/// port: the original uses `getMonth()`, which is zero based, so the month of
/// the key is zero based too.
pub fn day_key(value: LocalTimestamp) -> String {
    let civil = civil_from_epoch(value.epoch_ms, value.utc_offset_minutes);
    format!("{}-{}-{}", civil.year, civil.month - 1, civil.day)
}

/// port: `messageText`.
///
/// port: the original reads `t("message.deleted")`, which follows the locale
/// of the UI; the port keeps the text of the default locale of the original.
pub fn message_text(message: &Message) -> String {
    if message.revoked {
        return "This message was deleted".to_string();
    }
    let Some(content) = &message.content else {
        return "Message".to_string();
    };
    match content {
        MessageContent::Text(text) => text.text.clone(),
        MessageContent::Image(image) => {
            if !image.caption.is_empty() {
                image.caption.clone()
            } else if image.sticker {
                "Sticker".to_string()
            } else {
                "Photo".to_string()
            }
        }
        MessageContent::Attachment(attachment) => {
            // port: `||` in JS is "the string is empty".
            if !attachment.caption.is_empty() {
                attachment.caption.clone()
            } else if !attachment.file_name.is_empty() {
                attachment.file_name.clone()
            } else if !attachment.kind.is_empty() {
                attachment.kind.clone()
            } else {
                "Attachment".to_string()
            }
        }
        MessageContent::Contacts(contacts) => {
            let count = contacts.contacts.len();
            if count == 1 {
                // port: `contacts[0]?.displayName || "Contact"`.
                match contacts.contacts.first() {
                    Some(contact) if !contact.display_name.is_empty() => {
                        contact.display_name.clone()
                    }
                    _ => "Contact".to_string(),
                }
            } else {
                format!("{count} contacts")
            }
        }
        MessageContent::Location(location) => {
            if !location.name.is_empty() {
                location.name.clone()
            } else if !location.address.is_empty() {
                location.address.clone()
            } else {
                "Location".to_string()
            }
        }
        MessageContent::Poll(poll) => format!("Poll: {}", poll.question),
        MessageContent::Unsupported(unsupported) => {
            if !unsupported.fallback_text.is_empty() {
                unsupported.fallback_text.clone()
            } else if !unsupported.type_name.is_empty() {
                unsupported.type_name.clone()
            } else {
                "Message".to_string()
            }
        }
    }
}

/// port: `chatSubtitle` with its default prefix.
pub fn chat_subtitle(chat: &Chat) -> String {
    chat_subtitle_with_prefix(chat, "You: ")
}

/// port: `chatSubtitle`.
pub fn chat_subtitle_with_prefix(chat: &Chat, you_prefix: &str) -> String {
    // port: `||` in JS is "the string is empty".
    let preview = if !chat.last_message_preview.is_empty() {
        chat.last_message_preview.clone()
    } else if !chat.phone_number.is_empty() {
        chat.phone_number.clone()
    } else {
        "No messages yet".to_string()
    };
    if !chat.last_message_from_me || chat.last_message_preview.is_empty() {
        return preview;
    }
    format!("{you_prefix}{preview}")
}

/// port: `initials`.
///
/// port: the original takes the first code point of the first and of the last
/// word, and keeps at most two initials.
pub fn initials(name: &str) -> String {
    let words: Vec<&str> = name
        .trim_matches(is_js_whitespace)
        .split(is_js_whitespace)
        // port: `/[\p{L}\p{N}]/u` keeps the words with a letter or a number.
        .filter(|word| word.chars().any(is_letter_or_number))
        .collect();
    if words.is_empty() {
        return "?".to_string();
    }
    // port: `Array.from(word)[0]` is the first code point of the word.
    let first = words[0].chars().next().unwrap_or('?');
    let last = if words.len() > 1 {
        words[words.len() - 1].chars().next()
    } else {
        None
    };
    let mut initials = String::new();
    initials.push(first);
    if let Some(last) = last {
        initials.push(last);
    }
    initials.to_uppercase()
}

/// port: `hueFor`, the FNV-1a hash of the name.
///
/// port: the original hashes UTF-16 code units and keeps the hash in the
/// 32 bit signed range, so the port does the same.
pub fn hue_for(value: &str) -> u32 {
    // port: the original starts with the number 2166136261, which JavaScript
    // keeps as a double until the first bitwise operator truncates it to a
    // signed 32 bit integer. The port keeps the same two steps, so the hash of
    // an empty name is the hash of the initial value.
    let mut hash: i64 = 2_166_136_261;
    for unit in value.encode_utf16() {
        // port: `^` converts both operands to a signed 32 bit integer.
        let truncated = (hash as i32) ^ i32::from(unit);
        // port: `Math.imul` multiplies in the signed 32 bit range.
        hash = i64::from(truncated.wrapping_mul(16_777_619));
    }
    // port: `Math.abs(hash) % 360`; the hash fits in an i64 even when it is
    // the most negative 32 bit integer.
    (hash.abs() % 360) as u32
}

/// port: `formatBytes`.
pub fn format_bytes(bytes: f64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    // port: `!bytes` is true for 0 and for NaN.
    if bytes == 0.0 || bytes.is_nan() {
        return String::new();
    }
    let mut value = bytes;
    let mut unit = 0usize;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    let text = if value >= 10.0 || unit == 0 {
        js_to_fixed(value, 0)
    } else {
        js_to_fixed(value, 1)
    };
    format!("{text} {}", UNITS[unit])
}

/// port: `formatDuration`.
pub fn format_duration(seconds: f64) -> String {
    if seconds == 0.0 || seconds.is_nan() {
        return String::new();
    }
    let hours = (seconds / 3600.0).floor();
    let minutes = ((seconds % 3600.0) / 60.0).floor();
    let rest = seconds % 60.0;
    if hours > 0.0 {
        format!(
            "{}:{}:{}",
            js_number_to_string(hours),
            pad_start_two(&js_number_to_string(minutes)),
            pad_start_two(&js_number_to_string(rest))
        )
    } else {
        format!(
            "{}:{}",
            js_number_to_string(minutes),
            pad_start_two(&js_number_to_string(rest))
        )
    }
}

/// port: `formatCountdown`, the remaining lifetime of a disappearing message.
pub fn format_countdown(milliseconds: f64) -> String {
    if milliseconds <= 0.0 {
        return "0m".to_string();
    }
    let minutes = (milliseconds / 60_000.0).floor();
    if minutes < 1.0 {
        return "<1m".to_string();
    }
    if minutes < 60.0 {
        return format!("{}m", js_number_to_string(minutes));
    }
    let hours = (minutes / 60.0).floor();
    if hours < 24.0 {
        return format!("{}h", js_number_to_string(hours));
    }
    format!("{}d", js_number_to_string((hours / 24.0).floor()))
}

/// port: `connectionLabel`.
pub fn connection_label(connection: i32) -> String {
    match connection {
        0 | 1 => "Starting",
        2 => "Pairing",
        3 => "Connecting",
        4 => "Connected",
        5 => "Reconnecting",
        6 => "Offline",
        7 => "Logged out",
        8 => "Connection failed",
        _ => "Unknown",
    }
    .to_string()
}

/// port: `safeHttpUrl`.
///
/// port: the original builds a `URL` and keeps it only when the protocol is
/// `http` or `https`. The port parses the URL with a reduced WHATWG parser
/// (see `parse_url`): it covers the scheme, the authority, the path with its
/// dot segments, the query and the fragment, and it does not cover the
/// internationalised hosts nor the Windows drive letters of the `file`
/// scheme.
pub fn safe_http_url(value: &str) -> Option<String> {
    let url = parse_url(value)?;
    if url.scheme == "http" || url.scheme == "https" {
        Some(url.href)
    } else {
        None
    }
}

// ------------------------------------------------------------- the helpers

/// port: `String(number)`.
///
/// port: JavaScript switches to the exponent form for very big or very small
/// numbers; the port prints them in full.
fn js_number_to_string(value: f64) -> String {
    if value.is_nan() {
        return "NaN".to_string();
    }
    if value.is_infinite() {
        return if value > 0.0 {
            "Infinity".to_string()
        } else {
            "-Infinity".to_string()
        };
    }
    if value == 0.0 {
        return "0".to_string();
    }
    format!("{value}")
}

/// port: `Number.prototype.toFixed`.
///
/// port: JavaScript rounds the halves towards +Infinity and keeps the sign of
/// the value even when the result is zero.
fn js_to_fixed(value: f64, digits: u32) -> String {
    if !value.is_finite() {
        return js_number_to_string(value);
    }
    let factor = 10f64.powi(digits as i32);
    // port: `n` is the integer that makes `n / 10^f - value` as close to zero
    // as possible; a tie picks the larger `n`.
    let rounded = (value * factor + 0.5).floor();
    let sign = if value < 0.0 { "-" } else { "" };
    let mut text = format!("{}", rounded.abs() as u64);
    if digits == 0 {
        return format!("{sign}{text}");
    }
    if text.len() as u32 <= digits {
        let padding = digits as usize - text.len();
        text = format!("{}{}", "0".repeat(padding), text);
    }
    let split = text.len() - digits as usize;
    let (integer, fraction) = text.split_at(split);
    format!("{sign}{integer}.{fraction}")
}

/// port: `String.prototype.padStart(2, "0")`.
fn pad_start_two(value: &str) -> String {
    if value.chars().count() >= 2 {
        return value.to_string();
    }
    format!("0{value}")
}

/// port: the `\s` of the regular expressions of the original and the white
/// space of `String.prototype.trim`.
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

/// port: `/[\p{L}\p{N}]/u`, the letters and the numbers of Unicode.
///
/// port: Rust's `is_alphabetic` is the Unicode `Alphabetic` property, which
/// also accepts the number letters (`Nl`) and some marks; the original only
/// accepts the `L` category.
fn is_letter_or_number(value: char) -> bool {
    value.is_alphabetic() || value.is_numeric()
}

// -------------------------------------------------------------- the parser

/// A URL the port could parse.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedUrl {
    scheme: String,
    href: String,
}

const SPECIAL_SCHEMES: [&str; 6] = ["ftp", "file", "http", "https", "ws", "wss"];

fn is_special_scheme(scheme: &str) -> bool {
    SPECIAL_SCHEMES.contains(&scheme)
}

fn default_port(scheme: &str) -> Option<u32> {
    match scheme {
        "ftp" => Some(21),
        "http" | "ws" => Some(80),
        "https" | "wss" => Some(443),
        _ => None,
    }
}

/// port: `new URL(value)`.
fn parse_url(input: &str) -> Option<ParsedUrl> {
    // port: strip the leading and the trailing C0 controls and spaces.
    let trimmed = input.trim_matches(|value: char| value <= ' ');
    // port: remove every ASCII tab, line feed and carriage return.
    let cleaned: String = trimmed
        .chars()
        .filter(|value| !matches!(value, '\t' | '\n' | '\r'))
        .collect();

    let colon = cleaned.find(':')?;
    let scheme_source = &cleaned[..colon];
    let mut scheme_chars = scheme_source.chars();
    let first = scheme_chars.next()?;
    if !first.is_ascii_alphabetic() {
        return None;
    }
    if !scheme_chars.all(|value| value.is_ascii_alphanumeric() || matches!(value, '+' | '-' | '.'))
    {
        return None;
    }
    let scheme = scheme_source.to_ascii_lowercase();
    let rest = &cleaned[colon + 1..];

    if is_special_scheme(&scheme) {
        parse_special_url(&scheme, rest)
    } else {
        // port: an opaque path.
        let path = percent_encode(rest, in_c0_control_set);
        Some(ParsedUrl {
            scheme: scheme.clone(),
            href: format!("{scheme}:{path}"),
        })
    }
}

/// port: the special scheme branch of the URL parser.
fn parse_special_url(scheme: &str, rest: &str) -> Option<ParsedUrl> {
    // port: the special authority ignore slashes state.
    let after_slashes = rest.trim_start_matches(['/', '\\']);
    let authority_end = after_slashes
        .find(['/', '?', '#', '\\'])
        .unwrap_or(after_slashes.len());
    let authority = &after_slashes[..authority_end];
    let after_authority = &after_slashes[authority_end..];

    // port: the last `@` of the authority opens the host.
    let (userinfo, host_port) = match authority.rfind('@') {
        Some(index) => (Some(&authority[..index]), &authority[index + 1..]),
        None => (None, authority),
    };

    let (host, port) = parse_host_and_port(host_port)?;
    if host.is_empty() {
        // port: a special scheme needs a host.
        return None;
    }

    let (before_fragment, fragment) = split_at_first(after_authority, '#');
    let (path_source, query) = split_at_first(before_fragment, '?');

    let mut path = path_state(path_source);
    if path.is_empty() {
        path = "/".to_string();
    }

    let mut href = format!("{scheme}://");
    if let Some(userinfo) = userinfo {
        href.push_str(&encode_userinfo(userinfo));
        href.push('@');
    }
    href.push_str(&host);
    if let Some(port) = port {
        if Some(port) != default_port(scheme) {
            href.push(':');
            href.push_str(&port.to_string());
        }
    }
    href.push_str(&path);
    if let Some(query) = query {
        href.push('?');
        href.push_str(&percent_encode(query, in_query_set));
    }
    if let Some(fragment) = fragment {
        href.push('#');
        href.push_str(&percent_encode(fragment, in_fragment_set));
    }

    Some(ParsedUrl {
        scheme: scheme.to_string(),
        href,
    })
}

/// port: the host and the port of the authority.
fn parse_host_and_port(value: &str) -> Option<(String, Option<u32>)> {
    if let Some(after_bracket) = value.strip_prefix('[') {
        let end = after_bracket.find(']')?;
        let host = format!("[{}]", &after_bracket[..end]);
        let after = &after_bracket[end + 1..];
        let port = match after.strip_prefix(':') {
            Some(port) if !port.is_empty() => Some(parse_port(port)?),
            Some(_) => None,
            None => {
                if after.is_empty() {
                    None
                } else {
                    return None;
                }
            }
        };
        return Some((host, port));
    }
    let (host, port) = match value.rfind(':') {
        Some(index) => {
            let port_source = &value[index + 1..];
            let port = if port_source.is_empty() {
                None
            } else {
                Some(parse_port(port_source)?)
            };
            (&value[..index], port)
        }
        None => (value, None),
    };
    if host.chars().any(|character| {
        matches!(
            character,
            '\u{0000}'
                | '\u{0009}'
                | '\u{000A}'
                | '\u{000D}'
                | ' '
                | '#'
                | '/'
                | ':'
                | '?'
                | '@'
                | '['
                | '\\'
                | ']'
                | '^'
                | '|'
        )
    }) {
        return None;
    }
    Some((host.to_ascii_lowercase(), port))
}

fn parse_port(value: &str) -> Option<u32> {
    if !value.chars().all(|digit| digit.is_ascii_digit()) {
        return None;
    }
    value.parse::<u32>().ok()
}

/// port: the userinfo of the authority, split into its user and its password.
fn encode_userinfo(userinfo: &str) -> String {
    match userinfo.split_once(':') {
        Some((user, password)) => format!(
            "{}:{}",
            percent_encode(user, in_userinfo_set),
            percent_encode(password, in_userinfo_set)
        ),
        None => percent_encode(userinfo, in_userinfo_set),
    }
}

/// port: the path state of the URL parser, with its dot segments.
fn path_state(path: &str) -> String {
    let mut segments: Vec<String> = Vec::new();
    let mut buffer = String::new();
    let mut characters = path.chars();
    loop {
        let current = characters.next();
        let is_delimiter = match current {
            None => true,
            Some(value) => matches!(value, '/' | '?' | '#' | '\\'),
        };
        if is_delimiter {
            let is_slash = matches!(current, Some('/')) || matches!(current, Some('\\'));
            if is_double_dot_segment(&buffer) {
                shorten_path(&mut segments);
                if !is_slash {
                    segments.push(String::new());
                }
            } else if is_single_dot_segment(&buffer) {
                if !is_slash {
                    segments.push(String::new());
                }
            } else {
                segments.push(buffer.clone());
            }
            buffer.clear();
            if current.is_none() {
                break;
            }
        } else if let Some(value) = current {
            buffer.push_str(&percent_encode_char(value, in_path_set));
        }
    }
    segments.join("/")
}

/// port: the shorten a path step of the URL parser.
fn shorten_path(segments: &mut Vec<String>) {
    if segments.len() > 1 || (segments.len() == 1 && segments[0].is_empty()) {
        segments.pop();
    }
}

fn is_double_dot_segment(buffer: &str) -> bool {
    matches!(
        buffer.to_ascii_lowercase().as_str(),
        ".." | ".%2e" | "%2e." | "%2e%2e"
    )
}

fn is_single_dot_segment(buffer: &str) -> bool {
    matches!(buffer.to_ascii_lowercase().as_str(), "." | "%2e")
}

fn split_at_first(value: &str, separator: char) -> (&str, Option<&str>) {
    match value.find(separator) {
        Some(index) => (&value[..index], Some(&value[index + 1..])),
        None => (value, None),
    }
}

/// port: the C0 control percent encode set.
fn in_c0_control_set(value: char) -> bool {
    value <= '\u{001F}' || value > '\u{007E}'
}

/// port: the query percent encode set.
fn in_query_set(value: char) -> bool {
    in_c0_control_set(value) || matches!(value, ' ' | '"' | '#' | '<' | '>')
}

/// The index of a month in the tables of the long month names.
fn month_index(month: u32) -> usize {
    (month as usize).saturating_sub(1).min(11)
}

/// port: the path percent encode set.
fn in_path_set(value: char) -> bool {
    in_query_set(value) || matches!(value, '?' | '`' | '{' | '}')
}

/// port: the fragment percent encode set.
fn in_fragment_set(value: char) -> bool {
    in_c0_control_set(value) || matches!(value, ' ' | '"' | '<' | '>' | '`')
}

/// port: the userinfo percent encode set.
fn in_userinfo_set(value: char) -> bool {
    in_path_set(value)
        || matches!(
            value,
            '/' | ':' | ';' | '=' | '@' | '[' | '\\' | ']' | '^' | '|'
        )
}

fn percent_encode(value: &str, needs: fn(char) -> bool) -> String {
    let mut encoded = String::with_capacity(value.len());
    for character in value.chars() {
        encoded.push_str(&percent_encode_char(character, needs));
    }
    encoded
}

fn percent_encode_char(value: char, needs: fn(char) -> bool) -> String {
    if value.is_ascii() && !needs(value) {
        return value.to_string();
    }
    let mut encoded = String::with_capacity(3);
    for byte in value.to_string().as_bytes() {
        encoded.push_str(&format!("%{byte:02X}"));
    }
    encoded
}
