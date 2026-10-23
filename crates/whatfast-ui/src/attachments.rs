//! The card of the attachments of the messages.
//!
//! port: the oracle draws the attachments with the component
//! `AttachmentMessage` of `tauri/src/components/MessageBubble.tsx`, and the
//! shape of the card comes from its stylesheet (`tauri/src/styles.css`):
//! `.attachment-card`, `.attachment-icon`, `.attachment-meta` and
//! `.media-download-error`.
//!
//! The oracle plays the video and the audio inside the bubble, with a
//! `<video>` and with the player of the voice note. The native client has no
//! webview and no player of its own, so the file goes to the program of the
//! system, the same way the oracle opens the documents with
//! `bridge.openMediaPath`.
//!
//! The three icons of the oracle (`FileText`, `FileAudio` and `FileVideo`)
//! are the ones of `lucide`, and the set of Whatfast does not have them: the
//! document uses the page of `gpui_component`, the audio the microphone and
//! the video the camera of the set of Whatfast.

// The card is painted by `crate::render`, and the drawing of the messages
// lives in the branch of the other agent: until the two halves are joined,
// the compiler calls every piece of this module dead.
#![allow(dead_code)]

use gpui::prelude::FluentBuilder as _;
use gpui::{
    App, Context, FontWeight, InteractiveElement as _, ParentElement as _, Pixels,
    StatefulInteractiveElement as _, Styled as _, div, px,
};
use gpui_component::{ActiveTheme as _, Icon, IconName, Sizable as _, h_flex, v_flex};
use whatfast_i18n as i18n;

use whatfast_model::model::message_text;
use whatfast_protocol as proto;

use crate::app::Whatfast;
use crate::icons::WhatfastIcon;

/// The width of the card of an attachment, before the scale.
///
/// port: the width of `.attachment-card` is `min(300px * var(--scale), 55vw)`.
const CARD_WIDTH: f32 = 300.;

/// The side of the circle of the icon of the card, before the scale.
///
/// port: the width and the height of `.attachment-icon`.
const ICON_SIZE: f32 = 40.;

/// The side of the glyph of the icon of the card, before the scale.
///
/// port: the icons of the oracle are painted with a size of twenty three
/// pixels (`<FileText size={23} />`).
const GLYPH_SIZE: f32 = 23.;

/// The side of the icon of the download, before the scale.
///
/// port: the arrow of the download of the oracle is painted with a size of
/// twenty pixels (`<Download size={20} />`).
const DOWNLOAD_SIZE: f32 = 20.;

/// The space between the pieces of the card, before the scale.
///
/// port: the gap of `.attachment-card`.
const CARD_GAP: f32 = 10.;

/// The padding of the card, before the scale.
///
/// port: the padding of `.attachment-card`.
const CARD_PADDING: f32 = 10.;

/// The size of the name of the file.
///
/// port: the name of the file is the `<strong>` of `.attachment-meta`, and it
/// has the size of the text of the bubble.
const NAME_SIZE: f32 = 16.;

/// The size of the line of the details of the file, before the scale.
///
/// port: the line of the details of the file is the `<span>` of
/// `.attachment-meta`, and it has a size of eleven and a half pixels.
const DETAILS_SIZE: f32 = 11.5;

/// The size of the line of the error, before the scale.
///
/// port: the line of the error is `.media-download-error`, and it has a size
/// of eleven and a half pixels.
const ERROR_SIZE: f32 = 11.5;

/// The height of the card, for the measure of the messages.
///
/// port: the card of the oracle is as tall as the circle of its icon, plus
/// the padding and the border of the stylesheet. The line of the error adds
/// one more line under the card.
fn card_height(scale: f32, failed: bool) -> Pixels {
    let card = px((ICON_SIZE + CARD_PADDING * 2.) * scale + 2.);
    if failed {
        card + px((ERROR_SIZE + 6.) * scale)
    } else {
        card
    }
}

/// The height of the player of a voice note, for the measure of the messages.
fn voice_note_height(scale: f32) -> Pixels {
    px(ICON_SIZE * scale + 2.)
}

/// Whether the note can play here: it is a voice note and its file is in the
/// disk.
///
/// A note that was not downloaded yet keeps the card of the attachment, with
/// the arrow that asks for the file.
fn is_playable_voice_note(attachment: &proto::AttachmentContent) -> bool {
    attachment.voice_note
        && !attachment.local_path.is_empty()
        && std::path::Path::new(&attachment.local_path).is_file()
}

/// The chip of the speed of the note: `1x`, `1.5x` or `2x`.
fn format_speed(speed: f32) -> String {
    if (speed - speed.round()).abs() < 0.01 {
        format!("{}x", speed.round() as u32)
    } else {
        format!("{speed}x")
    }
}

/// The clock of the note: `0:07`, also when the sound did not start yet.
///
/// The details of the attachment leave the zero out, because there it means
/// "no duration"; the player always shows the minute and the second.
pub(crate) fn format_clock(seconds: f32) -> String {
    let total = seconds.max(0.0).round() as u32;
    format!("{}:{:02}", total / 60, total % 60)
}

/// The size of a file, in the readable form of the oracle.
///
/// port: `formatBytes` of `tauri/src/lib/format.ts`: the bytes go to the
/// first unit that holds them, and the number has a decimal only when it is
/// smaller than ten.
pub(crate) fn format_bytes(bytes: u64) -> String {
    if bytes == 0 {
        return String::new();
    }
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if value >= 10.0 || unit == 0 {
        format!("{:.0} {}", value, UNITS[unit])
    } else {
        format!("{:.1} {}", value, UNITS[unit])
    }
}

/// The duration of a file, in the readable form of the oracle.
///
/// port: `formatDuration` of `tauri/src/lib/format.ts`: the minutes and the
/// seconds, and the hours when the file is longer than an hour.
pub(crate) fn format_duration(seconds: u32) -> String {
    if seconds == 0 {
        return String::new();
    }
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    let rest = seconds % 60;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{rest:02}")
    } else {
        format!("{minutes}:{rest:02}")
    }
}

/// The name of one attachment, when the message has no name of its own.
///
/// port: `attachmentLabel` of `tauri/src/components/MessageBubble.tsx`.
pub(crate) fn attachment_label(attachment: &proto::AttachmentContent) -> &'static str {
    if attachment.voice_note {
        return i18n::t("Voice message");
    }
    i18n::t(match attachment.kind.as_str() {
        "video" if attachment.animated => "GIF",
        "video" => "Video",
        "audio" => "Audio",
        _ => "Document",
    })
}

/// The name that the card of one attachment shows.
///
/// port: the oracle shows the name of the file, and the kind of the file
/// when the message has no name.
fn attachment_name(attachment: &proto::AttachmentContent) -> String {
    let name = attachment.file_name.trim();
    if name.is_empty() {
        attachment_label(attachment).to_string()
    } else {
        name.to_string()
    }
}

/// The line of the details of one attachment: the size, the duration and the
/// type of the file.
///
/// port: the oracle joins the three pieces with a middle dot and leaves out
/// the ones that are empty.
fn attachment_details(attachment: &proto::AttachmentContent) -> String {
    [
        format_bytes(attachment.file_size),
        format_duration(attachment.duration_seconds),
        attachment.mime_type.trim().to_string(),
    ]
    .into_iter()
    .filter(|piece| !piece.is_empty())
    .collect::<Vec<_>>()
    .join(" · ")
}

/// The text of the bubble of one message.
///
/// port: the oracle draws the name of the file inside the card and the
/// caption of the attachment under it, as the text of the message. The
/// native client has the same two pieces: the card carries the name and the
/// bubble carries the caption.
pub(crate) fn bubble_text(message: &proto::Message) -> &str {
    match message.content.as_ref() {
        Some(proto::message::Content::Attachment(attachment)) => attachment.caption.as_str(),
        _ => message_text(message),
    }
}

/// The circle of the icon of the card, with the glyph of the kind of the
/// file.
fn attachment_icon(attachment: &proto::AttachmentContent, scale: f32, cx: &mut App) -> gpui::Div {
    // port: the oracle paints the icon of the kind of the file: the page for
    // a document, the speaker for an audio and the film for a video.
    let glyph = match attachment.kind.as_str() {
        "audio" => Icon::new(WhatfastIcon::Mic),
        "video" => Icon::new(WhatfastIcon::Video),
        _ => Icon::new(IconName::File),
    };
    // port: the circle of the icon has the soft color of the accent as its
    // background and the accent as its color.
    div()
        .flex_none()
        .size(px(ICON_SIZE * scale))
        .rounded_full()
        .bg(cx.theme().selection)
        .flex()
        .items_center()
        .justify_center()
        .child(
            glyph
                .with_size(px(GLYPH_SIZE * scale))
                .text_color(cx.theme().accent),
        )
}

impl Whatfast {
    /// The height that the card of the attachment of one message adds to the
    /// bubble, for the measure of the messages.
    ///
    /// It is zero for the messages that have no attachment.
    pub(crate) fn attachment_height(&self, message: &proto::Message) -> Pixels {
        let Some(proto::message::Content::Attachment(attachment)) = message.content.as_ref() else {
            return px(0.);
        };
        // The player of the voice note has a height of its own.
        if is_playable_voice_note(attachment) {
            return voice_note_height(self.ui_scale);
        }
        let failed = self
            .attachment_failures
            .contains_key(&(message.chat_id.clone(), message.id.clone()));
        card_height(self.ui_scale, failed)
    }

    /// The bubble of a voice note: the chip of the speed, the button of the
    /// play, the wave and the length, the same way the reference paints it.
    ///
    /// The sound plays here, inside the conversation: the file never goes to
    /// the player of the system.
    pub(crate) fn render_voice_note(
        &self,
        index: usize,
        message: &proto::Message,
        attachment: &proto::AttachmentContent,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let scale = self.ui_scale;
        let key = (message.chat_id.clone(), message.id.clone());
        let on = self.voice.is_on(&key);
        let playing = on && !self.voice.paused;
        let position = if on { self.voice.position() } else { 0.0 };
        // The length of the file wins over the one of the header: the
        // decoder knows how much sound there really is.
        let duration = if on && self.voice.duration > 0.0 {
            self.voice.duration
        } else {
            attachment.duration_seconds as f32
        };
        let progress = if duration > 0.0 {
            (position / duration).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let wave = self
            .voice
            .waves
            .get(&key)
            .map(|wave| wave.as_ref().clone())
            .unwrap_or_else(|| vec![0.45; crate::audio::WAVE_BARS]);
        let played = (progress * wave.len() as f32).floor() as usize;
        let played_color = cx.theme().foreground;
        let rest_color = cx.theme().foreground.opacity(0.35);
        let chip = cx.theme().foreground.alpha(0.08);

        let play_chat = message.chat_id.clone();
        let play_message = message.id.clone();
        let play_path = attachment.local_path.clone();
        let play = move |this: &mut Whatfast,
                         _: &gpui::ClickEvent,
                         window: &mut gpui::Window,
                         cx: &mut Context<Whatfast>| {
            this.toggle_voice_note(&play_chat, &play_message, &play_path, window, cx);
        };

        h_flex()
            .items_center()
            .gap(px(10.0 * scale))
            .w(px(CARD_WIDTH * scale))
            .max_w_full()
            .min_w_0()
            .h(px(ICON_SIZE * scale))
            .border_1()
            .border_color(gpui::transparent_black())
            .child(
                // El chip de la velocidad: 1x, 1.5x y 2x.
                div()
                    .id(("voice-speed", index))
                    .flex_none()
                    .w(px(46.0 * scale))
                    .h(px(28.0 * scale))
                    .rounded_full()
                    .bg(chip)
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(12.0 * scale))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(played_color)
                    .cursor_pointer()
                    .hover(move |style| style.bg(played_color.opacity(0.10)))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.cycle_voice_speed(cx);
                    }))
                    .child(format_speed(self.voice.speed)),
            )
            .child(
                div()
                    .id(("voice-play", index))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(26.0 * scale))
                    .cursor_pointer()
                    .on_click(cx.listener(play.clone()))
                    .child(
                        Icon::new(if playing {
                            WhatfastIcon::Pause
                        } else {
                            WhatfastIcon::Play
                        })
                        .with_size(px(22.0 * scale))
                        .text_color(played_color),
                    ),
            )
            .child(
                h_flex()
                    .id(("voice-wave", index))
                    .flex_1()
                    .min_w_0()
                    .items_center()
                    .gap(px(2.0 * scale))
                    .h_full()
                    .cursor_pointer()
                    .on_click(cx.listener(play))
                    .children(wave.iter().enumerate().map(|(bar, height)| {
                        let tall = (6.0 + 24.0 * height) * scale;
                        div()
                            .flex_none()
                            .w(px(3.0 * scale))
                            .h(px(tall))
                            .rounded_full()
                            .bg(if bar < played {
                                played_color
                            } else {
                                rest_color
                            })
                    })),
            )
            .child(
                div()
                    .flex_none()
                    .text_size(px(DETAILS_SIZE * scale))
                    .text_color(rest_color)
                    .child(format_clock(if on { position } else { duration })),
            )
    }

    /// The card of one attachment: the circle of the icon of its kind, the
    /// name of the file, the line with the size, the duration and the type,
    /// and the arrow of the download when the file is not in the disk yet.
    ///
    /// Under the card, the line of the error appears when the download
    /// failed, and it asks for the file again when it is pinned.
    ///
    /// The caption of the attachment is NOT here: it goes under the card, as
    /// the text of the bubble, with [`bubble_text`].
    pub(crate) fn render_attachment_card(
        &self,
        index: usize,
        message: &proto::Message,
        attachment: &proto::AttachmentContent,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        // La nota de voz que ya esta en el disco se reproduce aca adentro,
        // como en la referencia, y no en el programa del sistema.
        if is_playable_voice_note(attachment) {
            return self.render_voice_note(index, message, attachment, cx);
        }
        let scale = self.ui_scale;
        let name = attachment_name(attachment);
        let details = attachment_details(attachment);
        let failure = self
            .attachment_failures
            .get(&(message.chat_id.clone(), message.id.clone()))
            .cloned();
        let downloaded = !attachment.local_path.is_empty();
        // The two listeners of the block need their own copy of the ids.
        let card_chat_id = message.chat_id.clone();
        let card_message_id = message.id.clone();
        let save_chat_id = message.chat_id.clone();
        let save_message_id = message.id.clone();
        let error_chat_id = message.chat_id.clone();
        let error_message_id = message.id.clone();
        // port: the background of the card is the color of the text of the
        // theme with five percent of alpha (`color-mix(in srgb, var(--fg)
        // 5%, transparent)`), and the border is the border of the theme.
        let background = cx.theme().foreground.alpha(0.05);
        // The hovered card and the pinned card take the colors of the theme:
        // the one of the hovered elements and the one of the active ones.
        let hovered = cx.theme().muted;
        let pinned = cx.theme().list_active;
        let muted = cx.theme().muted_foreground;
        let danger = cx.theme().danger;

        v_flex()
            .min_w_0()
            .child(
                h_flex()
                    .id(("attachment-card", index))
                    .items_center()
                    .gap(px(CARD_GAP * scale))
                    .w(px(CARD_WIDTH * scale))
                    .max_w_full()
                    .min_w_0()
                    .px(px(CARD_PADDING * scale))
                    .py(px(CARD_PADDING * scale))
                    .rounded_md()
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(background)
                    .hover(move |style| style.bg(hovered))
                    .active(move |style| style.bg(pinned))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.open_message_attachment(
                            card_chat_id.clone(),
                            card_message_id.clone(),
                            cx,
                        );
                        cx.notify();
                    }))
                    .child(attachment_icon(attachment, scale, cx))
                    .child(
                        v_flex()
                            .min_w_0()
                            .flex_1()
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(NAME_SIZE * scale))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(name),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(DETAILS_SIZE * scale))
                                    .text_color(muted)
                                    .child(details),
                            ),
                    )
                    .when(downloaded || attachment.downloadable, |card| {
                        // The arrow always downloads the file into the folder
                        // of the downloads, without opening it: a document, a
                        // video, a PDF or any other file. When the file is
                        // not in the disk yet it is fetched first.
                        card.child(
                            div()
                                .id(("attachment-save", index))
                                .p(px(4. * scale))
                                .rounded_md()
                                .cursor_pointer()
                                .hover(move |style| style.bg(hovered))
                                .child(
                                    Icon::new(WhatfastIcon::Download)
                                        .with_size(px(DOWNLOAD_SIZE * scale))
                                        .text_color(muted),
                                )
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    cx.stop_propagation();
                                    this.save_message_attachment(
                                        save_chat_id.clone(),
                                        save_message_id.clone(),
                                        window,
                                        cx,
                                    );
                                })),
                        )
                    }),
            )
            .when_some(failure, |block, failure| {
                block.child(
                    h_flex()
                        .id(("attachment-error", index))
                        .items_center()
                        .gap(px(6. * scale))
                        .w(px(CARD_WIDTH * scale))
                        .max_w_full()
                        .mt(px(4. * scale))
                        .text_size(px(ERROR_SIZE * scale))
                        .text_color(danger)
                        .cursor_pointer()
                        .child(
                            Icon::new(WhatfastIcon::Warning)
                                .with_size(px(14. * scale))
                                .text_color(danger),
                        )
                        .child(failure)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            // port: the oracle asks for the file again with
                            // the same action of the card (`openAttachment`).
                            this.open_message_attachment(
                                error_chat_id.clone(),
                                error_message_id.clone(),
                                cx,
                            );
                            cx.notify();
                        })),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use whatfast_protocol::AttachmentContent;

    use super::{
        attachment_details, attachment_label, attachment_name, bubble_text, format_bytes,
        format_clock, format_duration, format_speed, is_playable_voice_note, voice_note_height,
    };

    /// An attachment with the values that a test needs.
    fn attachment(kind: &str) -> AttachmentContent {
        AttachmentContent {
            kind: kind.into(),
            ..Default::default()
        }
    }

    /// A message with the content that a test needs.
    fn message_with(content: whatfast_protocol::message::Content) -> whatfast_protocol::Message {
        whatfast_protocol::Message {
            id: "1".into(),
            chat_id: "chat".into(),
            content: Some(content),
            ..Default::default()
        }
    }

    #[test]
    fn the_size_of_a_file_goes_to_the_first_unit_that_holds_it() {
        assert_eq!(format_bytes(0), "");
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1024), "1.0 KB");
        assert_eq!(format_bytes(1536), "1.5 KB");
        assert_eq!(format_bytes(10 * 1024), "10 KB");
        assert_eq!(format_bytes(1024 * 1024), "1.0 MB");
        assert_eq!(format_bytes(3 * 1024 * 1024 * 1024), "3.0 GB");
    }

    #[test]
    fn the_duration_of_a_file_is_minutes_and_seconds() {
        assert_eq!(format_duration(0), "");
        assert_eq!(format_duration(9), "0:09");
        assert_eq!(format_duration(75), "1:15");
        assert_eq!(format_duration(3600), "1:00:00");
        assert_eq!(format_duration(3725), "1:02:05");
    }

    #[test]
    fn the_kind_of_a_file_is_the_name_of_the_card() {
        assert_eq!(attachment_label(&attachment("document")), "Document");
        assert_eq!(attachment_label(&attachment("audio")), "Audio");
        assert_eq!(attachment_label(&attachment("video")), "Video");

        let mut voice = attachment("audio");
        voice.voice_note = true;
        assert_eq!(attachment_label(&voice), "Voice message");

        let mut gif = attachment("video");
        gif.animated = true;
        assert_eq!(attachment_label(&gif), "GIF");
    }

    #[test]
    fn the_name_of_the_card_is_the_name_of_the_file() {
        let mut file = attachment("document");
        file.file_name = "agenda.pdf".into();
        assert_eq!(attachment_name(&file), "agenda.pdf");
        assert_eq!(attachment_name(&attachment("document")), "Document");
    }

    #[test]
    fn the_details_of_the_card_join_the_pieces_that_are_there() {
        let mut file = attachment("document");
        file.file_size = 2048;
        file.mime_type = "application/pdf".into();
        assert_eq!(attachment_details(&file), "2.0 KB · application/pdf");

        let mut voice = attachment("audio");
        voice.voice_note = true;
        voice.duration_seconds = 12;
        assert_eq!(attachment_details(&voice), "0:12");
    }

    #[test]
    fn the_text_of_the_bubble_of_an_attachment_is_its_caption() {
        let mut file = attachment("document");
        file.file_name = "agenda.pdf".into();
        file.caption = "the agenda of the meeting".into();
        let message = message_with(whatfast_protocol::message::Content::Attachment(file));
        assert_eq!(bubble_text(&message), "the agenda of the meeting");

        // Without a caption the bubble has no text: the name of the file is
        // inside the card.
        let mut bare = attachment("document");
        bare.file_name = "agenda.pdf".into();
        let message = message_with(whatfast_protocol::message::Content::Attachment(bare));
        assert_eq!(bubble_text(&message), "");

        // The other messages keep the text they had.
        let message = message_with(whatfast_protocol::message::Content::Text(
            whatfast_protocol::TextContent {
                text: "hello".into(),
                ..Default::default()
            },
        ));
        assert_eq!(bubble_text(&message), "hello");
    }

    #[test]
    fn the_chip_of_the_speed_shows_the_three_steps() {
        assert_eq!(format_speed(1.0), "1x");
        assert_eq!(format_speed(1.5), "1.5x");
        assert_eq!(format_speed(2.0), "2x");
    }

    #[test]
    fn the_clock_of_the_note_always_has_the_minute() {
        assert_eq!(format_clock(0.0), "0:00");
        assert_eq!(format_clock(7.4), "0:07");
        assert_eq!(format_clock(75.6), "1:16");
    }

    #[test]
    fn a_note_without_its_file_keeps_the_card_of_the_attachment() {
        let mut voice = attachment("audio");
        voice.voice_note = true;
        // Without a path the file is not in the disk.
        assert!(!is_playable_voice_note(&voice));

        // Any other audio is not a note, even with its file.
        let mut song = attachment("audio");
        song.local_path = "C:/musica/cancion.wav".into();
        assert!(!is_playable_voice_note(&song));
    }

    #[test]
    fn the_player_of_the_note_is_the_row_of_the_wave() {
        // The row of the wave (forty points) plus the border of the card.
        assert_eq!(voice_note_height(1.0), gpui::px(42.0));
        assert_eq!(voice_note_height(2.0), gpui::px(82.0));
    }
}
