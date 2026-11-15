//! Characterisation of `tauri/src/lib/format.ts`.
//!
//! The oracle has no suite for this module, so every expectation below was
//! captured from the oracle module itself with a temporary Node harness (kept
//! outside the repository). The harness:
//!
//! - imports `tauri/src/lib/format.ts` untouched (the oracle repository is
//!   never modified);
//! - freezes `new Date()` at `1792523400000` (2026-10-20T19:10:00Z), which is
//!   what `formatChatTime` and `formatDay` read as "now";
//! - runs with `TZ=UTC`, so the local time zone of the capture is UTC;
//! - runs with the locale forced to `en-US`.
//!
//! The port takes the locale and the offset of the time zone as parameters
//! (the crate has no ICU and no time zone database), so the capture is
//! asserted here.

use whatfast_core::message_format::{
    DateLocale, LocalTimestamp, day_key, format_chat_time, format_day, format_time,
};

/// The frozen "now" of the capture: 2026-10-20T19:10:00Z.
const NOW: i64 = 1_792_523_400_000;
const DAY: i64 = 86_400_000;
/// The capture ran with `TZ=UTC`.
const UTC: i32 = 0;

fn utc(epoch_ms: i64) -> LocalTimestamp {
    LocalTimestamp::new(epoch_ms, UTC)
}

fn at(offset_ms: i64) -> LocalTimestamp {
    utc(NOW + offset_ms)
}

// ------------------------------------------------------------- the clock

#[test]
fn the_time_of_a_day_follows_the_captured_format_of_the_locale() {
    // capture: `formatTime`, `TZ=UTC`.
    assert_eq!(format_time(utc(0), DateLocale::EnUs), "");
    assert_eq!(format_time(utc(1), DateLocale::EnUs), "12:00 AM");
    assert_eq!(format_time(at(-3_600_000), DateLocale::EnUs), "6:10 PM");
    assert_eq!(format_time(at(-1_800_000), DateLocale::EnUs), "6:40 PM");
    assert_eq!(format_time(at(0), DateLocale::EnUs), "7:10 PM");
    assert_eq!(format_time(at(3_600_000), DateLocale::EnUs), "8:10 PM");
    assert_eq!(
        format_time(utc(1_700_000_000_000), DateLocale::EnUs),
        "10:13 PM",
    );
    assert_eq!(
        format_time(utc(1_735_689_600_000), DateLocale::EnUs),
        "12:00 AM",
    );
}

#[test]
fn the_time_of_a_day_follows_the_offset_of_the_time_zone() {
    // capture: `formatTime`, `TZ=America/Buenos_Aires` (UTC-03:00), locale
    // `en-US`.
    let buenos_aires = -180;
    assert_eq!(
        format_time(LocalTimestamp::new(NOW, buenos_aires), DateLocale::EnUs),
        "4:10 PM",
    );
    assert_eq!(
        format_time(LocalTimestamp::new(1, buenos_aires), DateLocale::EnUs),
        "9:00 PM",
    );
    assert_eq!(
        format_time(
            LocalTimestamp::new(1_700_000_000_000, buenos_aires),
            DateLocale::EnUs
        ),
        "7:13 PM",
    );
}

#[test]
fn the_chat_time_of_a_day_follows_the_captured_format_of_the_locale() {
    // capture: `formatChatTime`, `TZ=UTC`, locale `en-US`.
    assert_eq!(format_chat_time(utc(0), utc(NOW), DateLocale::EnUs), "");
    assert_eq!(
        format_chat_time(utc(1), utc(NOW), DateLocale::EnUs),
        "01/01/70",
    );
    assert_eq!(
        format_chat_time(at(-DAY), utc(NOW), DateLocale::EnUs),
        "Yesterday",
    );
    assert_eq!(
        format_chat_time(at(-2 * DAY), utc(NOW), DateLocale::EnUs),
        "Sun",
    );
    assert_eq!(
        format_chat_time(at(-3 * DAY), utc(NOW), DateLocale::EnUs),
        "Sat",
    );
    assert_eq!(
        format_chat_time(at(-6 * DAY), utc(NOW), DateLocale::EnUs),
        "Wed",
    );
    assert_eq!(
        format_chat_time(at(-8 * DAY), utc(NOW), DateLocale::EnUs),
        "10/12/26",
    );
    assert_eq!(
        format_chat_time(at(-40 * DAY), utc(NOW), DateLocale::EnUs),
        "09/10/26",
    );
    assert_eq!(
        format_chat_time(at(-365 * DAY), utc(NOW), DateLocale::EnUs),
        "10/20/25",
    );
    assert_eq!(
        format_chat_time(utc(1_700_000_000_000), utc(NOW), DateLocale::EnUs),
        "11/14/23",
    );
    assert_eq!(
        format_chat_time(utc(1_735_689_600_000), utc(NOW), DateLocale::EnUs),
        "01/01/25",
    );
}

#[test]
fn the_day_of_a_message_follows_the_captured_format_of_the_locale() {
    // capture: `formatDay`, `TZ=UTC`, locale `en-US`.
    assert_eq!(
        format_day(utc(0), utc(NOW), DateLocale::EnUs),
        "Thursday, January 1"
    );
    assert_eq!(
        format_day(utc(1), utc(NOW), DateLocale::EnUs),
        "Thursday, January 1"
    );
    assert_eq!(
        format_day(at(-3_600_000), utc(NOW), DateLocale::EnUs),
        "Today"
    );
    assert_eq!(format_day(at(0), utc(NOW), DateLocale::EnUs), "Today");
    assert_eq!(
        format_day(at(3_600_000), utc(NOW), DateLocale::EnUs),
        "Today"
    );
    assert_eq!(
        format_day(at(-DAY), utc(NOW), DateLocale::EnUs),
        "Yesterday"
    );
    assert_eq!(
        format_day(at(-2 * DAY), utc(NOW), DateLocale::EnUs),
        "Sunday, October 18",
    );
    assert_eq!(
        format_day(at(-3 * DAY), utc(NOW), DateLocale::EnUs),
        "Saturday, October 17",
    );
    assert_eq!(
        format_day(at(-6 * DAY), utc(NOW), DateLocale::EnUs),
        "Wednesday, October 14",
    );
    assert_eq!(
        format_day(at(-8 * DAY), utc(NOW), DateLocale::EnUs),
        "Monday, October 12",
    );
    assert_eq!(
        format_day(at(-40 * DAY), utc(NOW), DateLocale::EnUs),
        "Thursday, September 10",
    );
    assert_eq!(
        format_day(at(-365 * DAY), utc(NOW), DateLocale::EnUs),
        "Monday, October 20",
    );
    assert_eq!(
        format_day(utc(1_700_000_000_000), utc(NOW), DateLocale::EnUs),
        "Tuesday, November 14",
    );
    assert_eq!(
        format_day(utc(1_735_689_600_000), utc(NOW), DateLocale::EnUs),
        "Wednesday, January 1",
    );
}

#[test]
fn the_day_key_keeps_the_zero_based_month_of_the_original() {
    // capture: `dayKey`, `TZ=UTC`.
    assert_eq!(day_key(utc(0)), "1970-0-1");
    assert_eq!(day_key(utc(1)), "1970-0-1");
    assert_eq!(day_key(at(0)), "2026-9-20");
    assert_eq!(day_key(at(-DAY)), "2026-9-19");
    assert_eq!(day_key(at(-2 * DAY)), "2026-9-18");
    assert_eq!(day_key(at(-3 * DAY)), "2026-9-17");
    assert_eq!(day_key(at(-6 * DAY)), "2026-9-14");
    assert_eq!(day_key(at(-8 * DAY)), "2026-9-12");
    assert_eq!(day_key(at(-40 * DAY)), "2026-8-10");
    assert_eq!(day_key(at(-365 * DAY)), "2025-9-20");
    assert_eq!(day_key(utc(1_700_000_000_000)), "2023-10-14");
    assert_eq!(day_key(utc(1_735_689_600_000)), "2025-0-1");
}
