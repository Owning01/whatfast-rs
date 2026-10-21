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
//! - runs once with the runtime locale of the machine (`es-AR`) and once with
//!   the locale forced to `en-US`.
//!
//! The port takes the locale and the offset of the time zone as parameters
//! (the crate has no ICU and no time zone database), so both captures are
//! asserted here.

use whatfast_core::message_format::{
    AttachmentContent, ChatKind, ContactContent, ContactsContent, DateLocale, ImageContent,
    LocalTimestamp, LocationContent, MessageContent, PollContent, TextContent, UnsupportedContent,
    chat_subtitle, chat_subtitle_with_prefix, connection_label, day_key, format_bytes,
    format_chat_time, format_countdown, format_day, format_duration, format_time, hue_for,
    initials, message_text, safe_http_url,
};
// The projections the formatters work on. The canonical `Chat` and `Message`
// live in `whatfast_core::types`; these are the lighter shapes the captured
// values were recorded against.
use whatfast_core::types::{ChatSummary as Chat, MessageSummary as Message};

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

    // capture: `formatTime`, `TZ=UTC`, locale `es-AR`.
    assert_eq!(format_time(utc(1), DateLocale::EsAr), "12:00 a. m.");
    assert_eq!(format_time(at(-3_600_000), DateLocale::EsAr), "6:10 p. m.");
    assert_eq!(format_time(at(0), DateLocale::EsAr), "7:10 p. m.");
    assert_eq!(
        format_time(utc(1_700_000_000_000), DateLocale::EsAr),
        "10:13 p. m.",
    );
    assert_eq!(
        format_time(utc(1_735_689_600_000), DateLocale::EsAr),
        "12:00 a. m.",
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
        format_chat_time(at(-3_600_000), utc(NOW), DateLocale::EnUs),
        "6:10 PM",
    );
    assert_eq!(
        format_chat_time(at(-1_800_000), utc(NOW), DateLocale::EnUs),
        "6:40 PM",
    );
    assert_eq!(
        format_chat_time(at(0), utc(NOW), DateLocale::EnUs),
        "7:10 PM",
    );
    assert_eq!(
        format_chat_time(at(3_600_000), utc(NOW), DateLocale::EnUs),
        "8:10 PM",
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

    // capture: `formatChatTime`, `TZ=UTC`, locale `es-AR`.
    assert_eq!(
        format_chat_time(at(-3_600_000), utc(NOW), DateLocale::EsAr),
        "6:10 p. m.",
    );
    assert_eq!(
        format_chat_time(at(0), utc(NOW), DateLocale::EsAr),
        "7:10 p. m.",
    );
    assert_eq!(
        format_chat_time(at(-DAY), utc(NOW), DateLocale::EsAr),
        "Yesterday",
    );
    assert_eq!(
        format_chat_time(at(-2 * DAY), utc(NOW), DateLocale::EsAr),
        "dom",
    );
    assert_eq!(
        format_chat_time(at(-3 * DAY), utc(NOW), DateLocale::EsAr),
        "sáb",
    );
    assert_eq!(
        format_chat_time(at(-6 * DAY), utc(NOW), DateLocale::EsAr),
        "mié",
    );
    assert_eq!(
        format_chat_time(at(-8 * DAY), utc(NOW), DateLocale::EsAr),
        "12/10/26",
    );
    assert_eq!(
        format_chat_time(at(-40 * DAY), utc(NOW), DateLocale::EsAr),
        "10/09/26",
    );
    assert_eq!(
        format_chat_time(at(-365 * DAY), utc(NOW), DateLocale::EsAr),
        "20/10/25",
    );
    assert_eq!(
        format_chat_time(utc(1_700_000_000_000), utc(NOW), DateLocale::EsAr),
        "14/11/23",
    );
    assert_eq!(
        format_chat_time(utc(1_735_689_600_000), utc(NOW), DateLocale::EsAr),
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

    // capture: `formatDay`, `TZ=UTC`, locale `es-AR`.
    assert_eq!(
        format_day(utc(0), utc(NOW), DateLocale::EsAr),
        "jueves, 1 de enero",
    );
    assert_eq!(
        format_day(at(-3_600_000), utc(NOW), DateLocale::EsAr),
        "Today",
    );
    assert_eq!(
        format_day(at(-DAY), utc(NOW), DateLocale::EsAr),
        "Yesterday"
    );
    assert_eq!(
        format_day(at(-2 * DAY), utc(NOW), DateLocale::EsAr),
        "domingo, 18 de octubre",
    );
    assert_eq!(
        format_day(at(-3 * DAY), utc(NOW), DateLocale::EsAr),
        "sábado, 17 de octubre",
    );
    assert_eq!(
        format_day(at(-6 * DAY), utc(NOW), DateLocale::EsAr),
        "miércoles, 14 de octubre",
    );
    assert_eq!(
        format_day(at(-8 * DAY), utc(NOW), DateLocale::EsAr),
        "lunes, 12 de octubre",
    );
    assert_eq!(
        format_day(at(-40 * DAY), utc(NOW), DateLocale::EsAr),
        "jueves, 10 de septiembre",
    );
    assert_eq!(
        format_day(at(-365 * DAY), utc(NOW), DateLocale::EsAr),
        "lunes, 20 de octubre",
    );
    assert_eq!(
        format_day(utc(1_700_000_000_000), utc(NOW), DateLocale::EsAr),
        "martes, 14 de noviembre",
    );
    assert_eq!(
        format_day(utc(1_735_689_600_000), utc(NOW), DateLocale::EsAr),
        "miércoles, 1 de enero",
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

// ------------------------------------------------------------- the message

fn message(content: Option<MessageContent>) -> Message {
    Message {
        id: "m1".to_string(),
        chat_id: "chat".to_string(),
        sender_id: "owner".to_string(),
        sender_name: "Owner".to_string(),
        from_me: false,
        timestamp_ms: 1,
        status: 3,
        edited: false,
        revoked: false,
        expires_at_ms: 0,
        sender_phone_number: String::new(),
        sender_avatar_path: String::new(),
        reactions: Vec::new(),
        reply_to_message_id: String::new(),
        reply_to_chat_id: String::new(),
        content,
    }
}

fn text_content(text: &str) -> MessageContent {
    MessageContent::Text(TextContent {
        text: text.to_string(),
        link_preview: None,
    })
}

fn image_content(caption: &str, sticker: bool) -> MessageContent {
    MessageContent::Image(ImageContent {
        caption: caption.to_string(),
        mime_type: "image/png".to_string(),
        local_path: String::new(),
        width: 0,
        height: 0,
        file_size: 0,
        downloadable: false,
        sticker,
        animated: false,
        thumbnail_path: String::new(),
        jpeg_thumbnail: Vec::new(),
    })
}

fn attachment_content(caption: &str, file_name: &str, kind: &str) -> MessageContent {
    MessageContent::Attachment(AttachmentContent {
        kind: kind.to_string(),
        caption: caption.to_string(),
        mime_type: String::new(),
        file_name: file_name.to_string(),
        local_path: String::new(),
        file_size: 0,
        width: 0,
        height: 0,
        duration_seconds: 0.0,
        animated: false,
        voice_note: false,
        downloadable: false,
    })
}

fn contacts_content(names: &[&str]) -> MessageContent {
    MessageContent::Contacts(ContactsContent {
        contacts: names
            .iter()
            .map(|name| ContactContent {
                display_name: (*name).to_string(),
                vcard: String::new(),
            })
            .collect(),
    })
}

fn location_content(name: &str, address: &str) -> MessageContent {
    MessageContent::Location(LocationContent {
        latitude: 0.0,
        longitude: 0.0,
        name: name.to_string(),
        address: address.to_string(),
        url: String::new(),
        live: false,
    })
}

fn poll_content() -> MessageContent {
    MessageContent::Poll(PollContent {
        question: "Lunch?".to_string(),
        options: Vec::new(),
        selectable_options_count: 2,
        total_voters: 0,
    })
}

fn unsupported_content(type_name: &str, fallback_text: &str) -> MessageContent {
    MessageContent::Unsupported(UnsupportedContent {
        type_name: type_name.to_string(),
        fallback_text: fallback_text.to_string(),
    })
}

#[test]
fn the_text_of_a_message_follows_the_captured_values() {
    // capture: `messageText`.
    let mut revoked = message(Some(text_content("hi")));
    revoked.revoked = true;
    assert_eq!(message_text(&revoked), "This message was deleted");
    assert_eq!(message_text(&message(None)), "Message");
    assert_eq!(
        message_text(&message(Some(text_content("hello there")))),
        "hello there",
    );
    assert_eq!(message_text(&message(Some(text_content("")))), "");
    assert_eq!(
        message_text(&message(Some(image_content("a caption", false)))),
        "a caption",
    );
    assert_eq!(
        message_text(&message(Some(image_content("", true)))),
        "Sticker",
    );
    assert_eq!(
        message_text(&message(Some(image_content("", false)))),
        "Photo",
    );
    assert_eq!(
        message_text(&message(Some(attachment_content(
            "the caption",
            "",
            "document"
        )))),
        "the caption",
    );
    assert_eq!(
        message_text(&message(Some(attachment_content(
            "",
            "report.pdf",
            "document"
        )))),
        "report.pdf",
    );
    assert_eq!(
        message_text(&message(Some(attachment_content("", "", "video")))),
        "video",
    );
    assert_eq!(
        message_text(&message(Some(attachment_content("", "", "")))),
        "Attachment",
    );
    assert_eq!(
        message_text(&message(Some(contacts_content(&["Ada"])))),
        "Ada",
    );
    assert_eq!(
        message_text(&message(Some(contacts_content(&[""])))),
        "Contact",
    );
    assert_eq!(
        message_text(&message(Some(contacts_content(&["Ada", "Bob", "Cid"])))),
        "3 contacts",
    );
    assert_eq!(
        message_text(&message(Some(contacts_content(&[])))),
        "0 contacts"
    );
    assert_eq!(
        message_text(&message(Some(location_content("The Plaza", "Somewhere")))),
        "The Plaza",
    );
    assert_eq!(
        message_text(&message(Some(location_content("", "Somewhere")))),
        "Somewhere",
    );
    assert_eq!(
        message_text(&message(Some(location_content("", "")))),
        "Location",
    );
    assert_eq!(message_text(&message(Some(poll_content()))), "Poll: Lunch?");
    assert_eq!(
        message_text(&message(Some(unsupported_content(
            "e2e_notification",
            "unsupported text"
        )))),
        "unsupported text",
    );
    assert_eq!(
        message_text(&message(Some(unsupported_content("e2e_notification", "")))),
        "e2e_notification",
    );
    assert_eq!(
        message_text(&message(Some(unsupported_content("", "")))),
        "Message",
    );
}

// --------------------------------------------------------------- the chat

fn chat(preview: &str, phone: &str, from_me: bool) -> Chat {
    Chat {
        id: "c1".to_string(),
        kind: ChatKind::Direct,
        title: "Ada".to_string(),
        avatar_path: String::new(),
        last_message_preview: preview.to_string(),
        last_message_timestamp_ms: 1,
        unread_count: 0,
        muted: false,
        pinned: false,
        archived: false,
        phone_number: phone.to_string(),
        contact_name: String::new(),
        push_name: String::new(),
        business_name: String::new(),
        last_message_from_me: from_me,
        transport_jid: String::new(),
    }
}

#[test]
fn the_subtitle_of_a_chat_follows_the_captured_values() {
    // capture: `chatSubtitle` and `chatSubtitle(chat, "Tu: ")`.
    assert_eq!(
        chat_subtitle(&chat("last preview", "+1 555", false)),
        "last preview"
    );
    assert_eq!(
        chat_subtitle(&chat("last preview", "+1 555", true)),
        "You: last preview"
    );
    assert_eq!(chat_subtitle(&chat("", "+1 555", true)), "+1 555");
    assert_eq!(chat_subtitle(&chat("", "+1 555", false)), "+1 555");
    assert_eq!(chat_subtitle(&chat("", "", false)), "No messages yet");
    assert_eq!(
        chat_subtitle_with_prefix(&chat("last preview", "+1 555", true), "Tu: "),
        "Tu: last preview",
    );
    assert_eq!(
        chat_subtitle_with_prefix(&chat("last preview", "+1 555", false), "Tu: "),
        "last preview",
    );
    assert_eq!(
        chat_subtitle_with_prefix(&chat("", "", false), "Tu: "),
        "No messages yet",
    );
}

// ------------------------------------------------------------ the avatars

#[test]
fn the_initials_of_a_name_follow_the_captured_values() {
    // capture: `initials`.
    assert_eq!(initials(""), "?");
    assert_eq!(initials(" "), "?");
    assert_eq!(initials("Ada Lovelace"), "AL");
    assert_eq!(initials("  ada   lovelace  "), "AL");
    assert_eq!(initials("ada"), "A");
    assert_eq!(initials("Ada"), "A");
    assert_eq!(initials("123"), "1");
    assert_eq!(initials("42 7"), "47");
    assert_eq!(initials("José María"), "JM");
    assert_eq!(initials("Ångström Ek"), "ÅE");
    assert_eq!(initials("😀 emoji"), "E");
    assert_eq!(initials("!!!"), "?");
    assert_eq!(initials("3com"), "3");
    assert_eq!(initials("ß straße"), "SSS");
    assert_eq!(initials("a-b c_d"), "AC");
    assert_eq!(initials("ada lovelace phd"), "AP");
}

#[test]
fn the_hue_of_a_name_follows_the_captured_values() {
    // capture: `hueFor`.
    assert_eq!(hue_for(""), 61);
    assert_eq!(hue_for("a"), 276);
    assert_eq!(hue_for("hello"), 243);
    assert_eq!(hue_for("Ada Lovelace"), 250);
    assert_eq!(hue_for("😀"), 344);
    assert_eq!(hue_for("0123456789"), 214);
    assert_eq!(hue_for("the quick brown fox"), 266);
}

// ------------------------------------------------------------- the sizes

#[test]
fn the_size_of_a_file_follows_the_captured_values() {
    // capture: `formatBytes`.
    assert_eq!(format_bytes(0.0), "");
    assert_eq!(format_bytes(1.0), "1 B");
    assert_eq!(format_bytes(512.0), "512 B");
    assert_eq!(format_bytes(1023.0), "1023 B");
    assert_eq!(format_bytes(1024.0), "1.0 KB");
    assert_eq!(format_bytes(1536.0), "1.5 KB");
    assert_eq!(format_bytes(10_240.0), "10 KB");
    assert_eq!(format_bytes(102_400.0), "100 KB");
    assert_eq!(format_bytes(1_048_576.0), "1.0 MB");
    assert_eq!(format_bytes(1_073_741_824.0), "1.0 GB");
    assert_eq!(format_bytes(1_099_511_627_776.0), "1024 GB");
    assert_eq!(format_bytes(-5.0), "-5 B");
}

#[test]
fn the_duration_of_a_voice_note_follows_the_captured_values() {
    // capture: `formatDuration`.
    assert_eq!(format_duration(0.0), "");
    assert_eq!(format_duration(1.0), "0:01");
    assert_eq!(format_duration(9.0), "0:09");
    assert_eq!(format_duration(59.0), "0:59");
    assert_eq!(format_duration(59.5), "0:59.5");
    assert_eq!(format_duration(60.0), "1:00");
    assert_eq!(format_duration(61.0), "1:01");
    assert_eq!(format_duration(90.0), "1:30");
    assert_eq!(format_duration(599.0), "9:59");
    assert_eq!(format_duration(3600.0), "1:00:00");
    assert_eq!(format_duration(3661.0), "1:01:01");
    assert_eq!(format_duration(7322.0), "2:02:02");
    assert_eq!(format_duration(86_400.0), "24:00:00");
}

#[test]
fn the_countdown_of_a_disappearing_message_follows_the_captured_values() {
    // capture: `formatCountdown`.
    assert_eq!(format_countdown(0.0), "0m");
    assert_eq!(format_countdown(-1.0), "0m");
    assert_eq!(format_countdown(-5000.0), "0m");
    assert_eq!(format_countdown(1.0), "<1m");
    assert_eq!(format_countdown(1000.0), "<1m");
    assert_eq!(format_countdown(59_999.0), "<1m");
    assert_eq!(format_countdown(60_000.0), "1m");
    assert_eq!(format_countdown(61_000.0), "1m");
    assert_eq!(format_countdown(3_599_000.0), "59m");
    assert_eq!(format_countdown(3_600_000.0), "1h");
    assert_eq!(format_countdown(3_660_000.0), "1h");
    assert_eq!(format_countdown(7_199_999.0), "1h");
    assert_eq!(format_countdown(7_200_000.0), "2h");
    assert_eq!(format_countdown(86_399_999.0), "23h");
    assert_eq!(format_countdown(86_400_000.0), "1d");
    assert_eq!(format_countdown(259_200_000.0), "3d");
}

#[test]
fn the_label_of_a_connection_follows_the_captured_values() {
    // capture: `connectionLabel`.
    assert_eq!(connection_label(-1), "Unknown");
    assert_eq!(connection_label(0), "Starting");
    assert_eq!(connection_label(1), "Starting");
    assert_eq!(connection_label(2), "Pairing");
    assert_eq!(connection_label(3), "Connecting");
    assert_eq!(connection_label(4), "Connected");
    assert_eq!(connection_label(5), "Reconnecting");
    assert_eq!(connection_label(6), "Offline");
    assert_eq!(connection_label(7), "Logged out");
    assert_eq!(connection_label(8), "Connection failed");
    assert_eq!(connection_label(9), "Unknown");
    assert_eq!(connection_label(100), "Unknown");
}

#[test]
fn the_safe_http_url_of_a_value_follows_the_captured_values() {
    // capture: `safeHttpUrl`.
    assert_eq!(
        safe_http_url("https://example.com").as_deref(),
        Some("https://example.com/")
    );
    assert_eq!(
        safe_http_url("http://example.com").as_deref(),
        Some("http://example.com/")
    );
    assert_eq!(
        safe_http_url("https://example.com/").as_deref(),
        Some("https://example.com/")
    );
    assert_eq!(
        safe_http_url("https://example.com/path").as_deref(),
        Some("https://example.com/path")
    );
    assert_eq!(
        safe_http_url("https://example.com/path?query=1").as_deref(),
        Some("https://example.com/path?query=1")
    );
    assert_eq!(
        safe_http_url("https://example.com/path?query=1#frag").as_deref(),
        Some("https://example.com/path?query=1#frag")
    );
    assert_eq!(
        safe_http_url("https://example.com/path#frag").as_deref(),
        Some("https://example.com/path#frag")
    );
    assert_eq!(
        safe_http_url("https://EXAMPLE.com/Path").as_deref(),
        Some("https://example.com/Path")
    );
    assert_eq!(
        safe_http_url("https://example.com:8443/x").as_deref(),
        Some("https://example.com:8443/x")
    );
    assert_eq!(
        safe_http_url("https://example.com:443/x").as_deref(),
        Some("https://example.com/x")
    );
    assert_eq!(
        safe_http_url("http://example.com:80/x").as_deref(),
        Some("http://example.com/x")
    );
    assert_eq!(
        safe_http_url("https://user:pass@example.com/x").as_deref(),
        Some("https://user:pass@example.com/x")
    );
    assert_eq!(
        safe_http_url("https://example.com/a b").as_deref(),
        Some("https://example.com/a%20b")
    );
    assert_eq!(safe_http_url("javascript:alert(1)"), None);
    assert_eq!(safe_http_url("javascript:alert('x')"), None);
    assert_eq!(safe_http_url("data:text/html,<b>x</b>"), None);
    assert_eq!(safe_http_url("file:///etc/passwd"), None);
    assert_eq!(safe_http_url("ftp://example.com/x"), None);
    assert_eq!(safe_http_url("mailto:someone@example.com"), None);
    assert_eq!(safe_http_url("not a url"), None);
    assert_eq!(safe_http_url(""), None);
    assert_eq!(safe_http_url("   "), None);
    assert_eq!(safe_http_url("https://"), None);
    assert_eq!(
        safe_http_url("https://example.com/a/../b").as_deref(),
        Some("https://example.com/b")
    );
    assert_eq!(
        safe_http_url("https://example.com/a/./b").as_deref(),
        Some("https://example.com/a/b")
    );
}
