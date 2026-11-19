//! Port of `tauri/src/lib/format.ts`.
//!
//! The original renders the timestamps of the sidebar and the conversation.
//!
//! port: three things the original takes from the runtime cannot be ported
//! as they are, and the port injects them instead:
//!
//! - `new Date()` (the "now" of `formatChatTime` and `formatDay`) becomes the
//!   `now` parameter, as RULEBOOK 1.4 asks.
//! - `Intl.DateTimeFormat(undefined, ...)` follows the locale of the runtime.
//!   The crate has no ICU, so the port takes the locale as a parameter and
//!   implements the locale the app ships (`en`).
//! - The local time zone needs a time zone database, which `std` does not
//!   have. The port takes the offset from UTC in minutes, so the caller
//!   resolves it (the caller is the only one that can, it knows the operating
//!   system). The offset is a fixed one: a zone with daylight saving time
//!   needs the caller to resolve the offset of the exact instant.
//!
//! The characterisation values of every function below come from the oracle
//! module itself, see `tests/message_format.rs`.

pub use crate::types::{
    AttachmentContent, ChatKind, ContactContent, ContactsContent, ImageContent, LinkPreview,
    LocationContent, MessageContent, PollContent, PollOption, PollVoter, Reaction, TextContent,
    UnsupportedContent,
};

// -------------------------------------------------------------- the clock

/// The locale the date formatters render with.
///
/// port: the original leaves the locale to the runtime of `Intl`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DateLocale {
    /// `en-US`: `7:10 PM`, `Sun`, `10/12/26`, `Sunday, October 12`.
    #[default]
    EnUs,
    /// `es-ES`: `19:10`, `lun`, `12/10/26`, `lunes, 12 de octubre`.
    EsEs,
    /// `zh-CN`: `19:10`, `周一`, `26/10/12`, `10月12日星期一`.
    ZhCn,
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
const ES_ES_WEEKDAYS_SHORT: [&str; 7] = ["dom", "lun", "mar", "mié", "jue", "vie", "sáb"];
const ES_ES_WEEKDAYS_LONG: [&str; 7] = [
    "domingo",
    "lunes",
    "martes",
    "miércoles",
    "jueves",
    "viernes",
    "sábado",
];
const ES_ES_MONTHS_LONG: [&str; 12] = [
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
const ZH_CN_WEEKDAYS_SHORT: [&str; 7] = ["周日", "周一", "周二", "周三", "周四", "周五", "周六"];
const ZH_CN_WEEKDAYS_LONG: [&str; 7] = [
    "星期日",
    "星期一",
    "星期二",
    "星期三",
    "星期四",
    "星期五",
    "星期六",
];
const ZH_CN_MONTHS_LONG: [&str; 12] = [
    "1月", "2月", "3月", "4月", "5月", "6月", "7月", "8月", "9月", "10月", "11月", "12月",
];

/// port: `timeFormatter`, `hour: "numeric"` and `minute: "2-digit"`.
fn format_time_of_day(value: &CivilDateTime, locale: DateLocale) -> String {
    let minute = format!("{:02}", value.minute);
    match locale {
        DateLocale::EnUs => {
            let hour = match value.hour % 12 {
                0 => 12,
                other => other,
            };
            let period = if value.hour < 12 { "AM" } else { "PM" };
            format!("{hour}:{minute} {period}")
        }
        // El reloj de veinticuatro horas es el de la burbuja y el de la
        // referencia en español y en chino.
        DateLocale::EsEs | DateLocale::ZhCn => format!("{:02}:{}", value.hour, minute),
    }
}

/// port: `weekdayFormatter`, `weekday: "short"`.
fn format_weekday_short(weekday: u32, locale: DateLocale) -> String {
    let index = (weekday as usize) % 7;
    match locale {
        DateLocale::EnUs => EN_US_WEEKDAYS_SHORT[index].to_string(),
        DateLocale::EsEs => ES_ES_WEEKDAYS_SHORT[index].to_string(),
        DateLocale::ZhCn => ZH_CN_WEEKDAYS_SHORT[index].to_string(),
    }
}

/// port: `weekdayFormatter`, `weekday: "long"`.
fn format_weekday_long(weekday: u32, locale: DateLocale) -> String {
    let index = (weekday as usize) % 7;
    match locale {
        DateLocale::EnUs => EN_US_WEEKDAYS_LONG[index].to_string(),
        DateLocale::EsEs => ES_ES_WEEKDAYS_LONG[index].to_string(),
        DateLocale::ZhCn => ZH_CN_WEEKDAYS_LONG[index].to_string(),
    }
}

/// port: `shortDateFormatter`, `day: "2-digit"`, `month: "2-digit"` and
/// `year: "2-digit"`.
fn format_short_date(value: &CivilDateTime, locale: DateLocale) -> String {
    let year = value.year.rem_euclid(100);
    match locale {
        DateLocale::EnUs => format!("{:02}/{:02}/{:02}", value.month, value.day, year),
        DateLocale::EsEs => format!("{:02}/{:02}/{:02}", value.day, value.month, year),
        DateLocale::ZhCn => format!("{:02}/{:02}/{:02}", year, value.month, value.day),
    }
}

/// port: `dayFormatter`, `weekday: "long"`, `day: "numeric"` and
/// `month: "long"`.
fn format_day_long(value: &CivilDateTime, locale: DateLocale) -> String {
    let weekday = format_weekday_long(value.weekday, locale);
    let month = match locale {
        DateLocale::EnUs => EN_US_MONTHS_LONG[month_index(value.month)],
        DateLocale::EsEs => ES_ES_MONTHS_LONG[month_index(value.month)],
        DateLocale::ZhCn => ZH_CN_MONTHS_LONG[month_index(value.month)],
    };
    match locale {
        DateLocale::EnUs => format!("{weekday}, {month} {}", value.day),
        DateLocale::EsEs => format!("{weekday}, {} de {month}", value.day),
        DateLocale::ZhCn => format!("{month}{}日{weekday}", value.day),
    }
}

/// The word for "today" in the language of the interface.
fn today_label(locale: DateLocale) -> &'static str {
    match locale {
        DateLocale::EnUs => "Today",
        DateLocale::EsEs => "Hoy",
        DateLocale::ZhCn => "今天",
    }
}

/// The word for "yesterday" in the language of the interface.
fn yesterday_label(locale: DateLocale) -> &'static str {
    match locale {
        DateLocale::EnUs => "Yesterday",
        DateLocale::EsEs => "Ayer",
        DateLocale::ZhCn => "昨天",
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
        return yesterday_label(locale).to_string();
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
        return today_label(locale).to_string();
    }
    if same_day(&value_civil, &previous_day(&now_civil)) {
        return yesterday_label(locale).to_string();
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

/// The index of a month in the tables of the long month names.
fn month_index(month: u32) -> usize {
    (month as usize).saturating_sub(1).min(11)
}


