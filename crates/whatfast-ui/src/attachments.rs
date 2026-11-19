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

use std::cell::Cell;
use std::rc::Rc;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    App, Bounds, Context, FontWeight, InteractiveElement as _, MouseButton, MouseDownEvent,
    ObjectFit, ParentElement as _, Pixels, StatefulInteractiveElement as _, Styled as _,
    StyledImage as _, canvas, div, img, px, relative, rgb, rgba,
};
use gpui_component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, h_flex, slider::Slider, tooltip::Tooltip,
    v_flex,
};
use whatfast_i18n as i18n;

use whatfast_model::model::{Store, message_text};
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

/// El alto de la fila de los datos del video, antes de la escala.
const VIDEO_META_HEIGHT: f32 = 40.;

/// El espacio entre la caja del video y su fila de datos, antes de la escala.
const VIDEO_META_GAP: f32 = 6.;

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

/// El alto del reproductor de video, para la medida de los mensajes: la caja
/// con la forma del archivo y la fila de los datos.
fn video_height(attachment: &proto::AttachmentContent, scale: f32) -> Pixels {
    let (_, height) = crate::video::video_box_size(attachment.width, attachment.height, scale);
    px(height + (VIDEO_META_HEIGHT + VIDEO_META_GAP) * scale + 2.)
}

/// Whether the note can play here: it is a voice note and its file is in the
/// disk.
///
/// A note that was not downloaded yet keeps the card of the attachment, with
/// the arrow that asks for the file.
fn is_playable_voice_note(store: &Store, attachment: &proto::AttachmentContent) -> bool {
    attachment.voice_note
        && !attachment.local_path.is_empty()
        && store.media_exists(&attachment.local_path)
}

/// Whether an audio still has to be asked for: the file is not on the disk,
/// so the player cannot draw its wave or play it.
pub(crate) fn audio_needs_download(
    store: &Store,
    attachment: &proto::AttachmentContent,
) -> bool {
    attachment.kind == "audio" && !store.media_exists(&attachment.local_path)
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

/// The second that one bar of the wave points at: the first bar is the
/// start of the note and the last one its end.
pub(crate) fn wave_seek_seconds(duration: f32, bar: usize, bars: usize) -> f32 {
    if bars <= 1 || duration <= 0.0 {
        return 0.0;
    }
    let bar = bar.min(bars - 1);
    duration * bar as f32 / (bars - 1) as f32
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
    // El MIME crudo ("application/octet-stream") no dice nada: se muestra la
    // extension legible, y el generico se omite.
    let mime = attachment.mime_type.trim().to_ascii_lowercase();
    let readable = match mime.as_str() {
        "" | "application/octet-stream" => String::new(),
        "application/zip" | "application/x-zip-compressed" | "multipart/x-zip" => {
            i18n::t("ZIP file").to_string()
        }
        "text/html" | "application/xhtml+xml" => "HTML".to_string(),
        "application/pdf" => "PDF".to_string(),
        "text/plain" => "TXT".to_string(),
        _ => attachment.mime_type.trim().to_string(),
    };
    [
        format_bytes(attachment.file_size),
        format_duration(attachment.duration_seconds),
        readable,
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
        if is_playable_voice_note(&self.store, attachment) {
            return voice_note_height(self.ui_scale);
        }
        // El video se mira en su caja, con los mandos debajo.
        if crate::video::is_video(attachment) {
            return video_height(attachment, self.ui_scale);
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
        // Cada barra de la onda lleva al segundo que representa: el primer
        // clic vuelve al inicio y el ultimo salta al final.
        let seek_chat = message.chat_id.clone();
        let seek_message = message.id.clone();
        let seek_path = attachment.local_path.clone();
        let bars = wave.len().max(1);

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
                    .h_full()
                    .cursor_pointer()
                    .on_click(cx.listener(play))
                    .children(wave.iter().enumerate().map(|(bar, height)| {
                        let tall = (6.0 + 24.0 * height) * scale;
                        let seconds = wave_seek_seconds(duration, bar, bars);
                        let seek_chat = seek_chat.clone();
                        let seek_message = seek_message.clone();
                        let seek_path = seek_path.clone();
                        // Cada barra ocupa su casilla entera: el clic no
                        // tiene que apuntarle a los tres puntos del dibujo.
                        div()
                            .id(format!("voice-bar-{index}-{bar}"))
                            .flex_none()
                            .w(px(5.0 * scale))
                            .h_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.seek_voice_note(
                                    &seek_chat,
                                    &seek_message,
                                    &seek_path,
                                    seconds,
                                    window,
                                    cx,
                                );
                                cx.stop_propagation();
                                cx.notify();
                            }))
                            .child(
                                div()
                                    .w(px(3.0 * scale))
                                    .h(px(tall))
                                    .rounded_full()
                                    .bg(if bar < played {
                                        played_color
                                    } else {
                                        rest_color
                                    }),
                            )
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
    /// El reproductor de un video: la caja con el cuadro que va saliendo, el
    /// boton de reproducir y la barra del tiempo.
    ///
    /// El archivo se mira aca adentro y nunca sale al reproductor del
    /// sistema. Un video que todavia no esta en el disco muestra la misma
    /// caja, y el primer boton de reproducir pide el archivo y arranca
    /// cuando llega.
    pub(crate) fn render_video(
        &self,
        index: usize,
        message: &proto::Message,
        attachment: &proto::AttachmentContent,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let (width, height) =
            crate::video::video_box_size(attachment.width, attachment.height, self.ui_scale);
        self.render_video_player(index, message, attachment, width, height, false, cx)
    }

    /// The player of the video of a status.
    ///
    /// The viewer of the statuses fills the window, so the box grows past the
    /// width of a bubble: it is the same player with more room.
    pub(crate) fn render_status_video(
        &self,
        index: usize,
        message: &proto::Message,
        attachment: &proto::AttachmentContent,
        max_width: f32,
        max_height: f32,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        // El reproductor suma la fila de los controles debajo del cuadro: el
        // tope del cuadro les deja ese lugar para que el video entero entre
        // en la ventana.
        let controls = (VIDEO_META_HEIGHT + VIDEO_META_GAP) * self.ui_scale + 2.;
        let frame_height = (max_height - controls).max(80. * self.ui_scale);
        let (width, height) = crate::video::status_box_size(
            attachment.width,
            attachment.height,
            self.ui_scale,
            max_width,
            frame_height,
        );
        self.render_video_player(index, message, attachment, width, height, true, cx)
    }

    /// La fila del volumen del reproductor de video: el boton que apaga el
    /// sonido y la barra que lo sube o lo baja.
    fn render_video_volume(
        &self,
        index: usize,
        scale: f32,
        muted: gpui::Hsla,
        hovered: gpui::Hsla,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let silent = self.video.muted || self.video.volume <= 0.0;
        h_flex()
            .mt(px(VIDEO_META_GAP * scale))
            .gap(px(CARD_GAP * scale))
            .h(px(VIDEO_META_HEIGHT * scale))
            .items_center()
            .child(
                div()
                    .id(("video-mute", index))
                    .p(px(4. * scale))
                    .rounded_md()
                    .cursor_pointer()
                    .hover(move |style| style.bg(hovered))
                    .tooltip({
                        let label: gpui::SharedString = if silent {
                            i18n::t("Unmute")
                        } else {
                            i18n::t("Mute")
                        }
                        .into();
                        move |window, cx| Tooltip::new(label.clone()).build(window, cx)
                    })
                    .child(
                        Icon::new(if silent {
                            WhatfastIcon::VolumeMute
                        } else {
                            WhatfastIcon::Volume
                        })
                        .with_size(px(DOWNLOAD_SIZE * scale))
                        .text_color(muted),
                    )
                    .on_click(cx.listener(|this, _, window, cx| {
                        cx.stop_propagation();
                        this.toggle_video_mute(window, cx);
                    })),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(Slider::new(&self.volume_slider).horizontal()),
            )
    }

    /// Interactive progress & scrub bar shared between the inline video player and fullscreen viewer.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn render_video_seek_bar(
        id: impl Into<gpui::ElementId>,
        key: crate::video::VideoKey,
        duration: f32,
        progress: f32,
        dragging: bool,
        scale: f32,
        track_bg: gpui::Hsla,
        fill_color: gpui::Hsla,
        large: bool,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let track_bounds: Rc<Cell<Option<Bounds<Pixels>>>> = Rc::new(Cell::new(None));
        let bounds_paint = track_bounds.clone();
        let bounds_down = track_bounds.clone();
        let thumb_size = if large { 13. } else { 10. } * scale;
        let track_h = if dragging {
            if large { 6. } else { 5. }
        } else if large {
            4.
        } else {
            3.5
        } * scale;
        let hit_h = if large { 20. } else { 16. } * scale;
        let progress = progress.clamp(0.0, 1.0);

        div()
            .id(id)
            .w_full()
            .h(px(hit_h))
            .px(px(thumb_size * 0.5))
            .flex()
            .items_center()
            .cursor_pointer()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    let Some(bounds) = bounds_down.get() else {
                        return;
                    };
                    if duration <= 0.0 {
                        return;
                    }
                    let width = f32::from(bounds.size.width).max(1.0);
                    let fraction = ((f32::from(event.position.x) - f32::from(bounds.origin.x))
                        / width)
                        .clamp(0.0, 1.0);
                    this.video_seek_dragging = Some(key.clone());
                    this.video_seek_drag_bounds = Some(bounds);
                    this.video_seek_drag_duration = duration;
                    this.seek_video(key.clone(), fraction * duration, true, window, cx);
                }),
            )
            .on_click(cx.listener(|_, _, _, cx| {
                cx.stop_propagation();
            }))
            .child(
                div()
                    .relative()
                    .w_full()
                    .h(px(track_h))
                    .rounded_full()
                    .bg(track_bg)
                    .child(
                        canvas(
                            |_, _, _| (),
                            move |bounds, _, _, _| {
                                bounds_paint.set(Some(bounds));
                            },
                        )
                        .absolute()
                        .inset_0()
                        .size_full(),
                    )
                    .child(
                        div()
                            .h_full()
                            .w(relative(progress))
                            .rounded_full()
                            .bg(fill_color),
                    )
                    .when(duration > 0.0, |track| {
                        track.child(
                            div()
                                .absolute()
                                .top(px((track_h - thumb_size) * 0.5))
                                .left(relative(progress))
                                .ml(px(-thumb_size * 0.5))
                                .size(px(thumb_size))
                                .rounded_full()
                                .bg(rgb(0xffffff))
                                .border_1()
                                .border_color(fill_color)
                                .shadow_xs(),
                        )
                    }),
            )
    }

    #[allow(clippy::too_many_arguments)]
    fn render_video_player(
        &self,
        index: usize,
        message: &proto::Message,
        attachment: &proto::AttachmentContent,
        width: f32,
        height: f32,
        on_dark: bool,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let scale = self.ui_scale;
        let key = (message.chat_id.clone(), message.id.clone());
        let playback = self.video.is_for(&key).then_some(&self.video);
        let playing = playback.is_some_and(|playback| playback.is_running());
        let waiting = playback.is_some_and(|playback| playback.waiting);
        let position = playback.map_or(0.0, |playback| playback.position);
        let duration = playback
            .map(|playback| playback.duration)
            .filter(|duration| *duration > 0.0)
            .unwrap_or(attachment.duration_seconds as f32);
        let progress = if duration > 0.0 {
            (position / duration).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let frame = playback.and_then(|playback| playback.frame.clone());
        let has_frame = frame.is_some();
        let error = playback.and_then(|playback| playback.error.clone());
        let play_chat_id = message.chat_id.clone();
        let play_message_id = message.id.clone();
        let save_chat_id = message.chat_id.clone();
        let save_message_id = message.id.clone();
        let expand_chat_id = message.chat_id.clone();
        let expand_message_id = message.id.clone();
        // The viewer of the statuses is always dark, whatever the theme of the
        // window: the colors of the card come with the background.
        let (muted, accent, hovered, danger, title_color) = if on_dark {
            (
                rgba(0xffffffb3).into(),
                rgba(0xffffffff).into(),
                rgba(0xffffff22).into(),
                rgba(0xff8a8aff).into(),
                rgba(0xffffffff).into(),
            )
        } else {
            (
                cx.theme().muted_foreground,
                cx.theme().accent,
                cx.theme().muted,
                cx.theme().danger,
                cx.theme().foreground,
            )
        };
        let downloading = waiting
            || (self.attachment_download_attempted.contains(&key)
                && !self.attachment_failures.contains_key(&key)
                && (attachment.local_path.is_empty()
                    || !self.store.media_exists(&attachment.local_path)));
        let title = if downloading {
            i18n::t("Downloading the video").to_string()
        } else {
            attachment_name(attachment)
        };

        v_flex()
            .min_w_0()
            .child(
                div()
                    .id(("video-player", index))
                    .relative()
                    .w(px(width))
                    .max_w_full()
                    .h(px(height))
                    .rounded_md()
                    .overflow_hidden()
                    .bg(rgb(0x0f0f0f))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if this.video_seek_suppress_click {
                            this.video_seek_suppress_click = false;
                            return;
                        }
                        this.toggle_video_playback(
                            play_chat_id.clone(),
                            play_message_id.clone(),
                            window,
                            cx,
                        );
                    }))
                    .when_some(frame, |player, frame| {
                        player.child(img(frame).w_full().h_full().object_fit(ObjectFit::Contain))
                    })
                    .when(!has_frame, |player| {
                        // Sin cuadro todavia: el icono del video, que es lo
                        // que se ve mientras se baja el archivo.
                        player.child(
                            div()
                                .absolute()
                                .inset_0()
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(
                                    Icon::new(WhatfastIcon::Video)
                                        .with_size(px(38. * scale))
                                        .text_color(muted),
                                ),
                        )
                    })
                    // El boton del medio: reproducir o pausar.
                    .child(
                        div()
                            .absolute()
                            .inset_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                div()
                                    .size(px(54. * scale))
                                    .rounded_full()
                                    .bg(rgba(0x000000a6))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(
                                        Icon::new(if playing {
                                            WhatfastIcon::Pause
                                        } else {
                                            WhatfastIcon::Play
                                        })
                                        .with_size(px(24. * scale))
                                        .text_color(rgb(0xffffff)),
                                    ),
                            ),
                    )
                    // La barra del tiempo interactiva, pegada al borde de abajo.
                    .child(
                        div()
                            .absolute()
                            .bottom_0()
                            .left_0()
                            .right_0()
                            .px(px(6. * scale))
                            .bg(rgba(0x00000066))
                            .child(Self::render_video_seek_bar(
                                ("video-seekbar", index),
                                key.clone(),
                                duration,
                                progress,
                                self.video_seek_dragging.as_ref() == Some(&key),
                                scale,
                                rgba(0xffffff44).into(),
                                accent,
                                false,
                                cx,
                            )),
                    ),
            )
            // La fila del volumen, mientras el video es el que suena.
            .when(playback.is_some(), |player| {
                player.child(self.render_video_volume(index, scale, muted, hovered, cx))
            })
            // La fila de los datos del archivo, con la flecha de la descarga.
            .child(
                h_flex()
                    .mt(px(VIDEO_META_GAP * scale))
                    .gap(px(CARD_GAP * scale))
                    .h(px(VIDEO_META_HEIGHT * scale))
                    .items_center()
                    .child(
                        v_flex()
                            .min_w_0()
                            .flex_1()
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(NAME_SIZE * scale))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(title_color)
                                    .child(title),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(DETAILS_SIZE * scale))
                                    .text_color(muted)
                                    .child(format!(
                                        "{} / {}",
                                        format_clock(position),
                                        format_clock(duration)
                                    )),
                            ),
                    )
                    .when(
                        !attachment.local_path.is_empty() || attachment.downloadable,
                        |row| {
                            row.child(
                                div()
                                    .id(("video-save", index))
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
                        },
                    )
                    // El boton que abre el video en el visor de la ventana.
                    .child(
                        div()
                            .id(("video-expand", index))
                            .p(px(4. * scale))
                            .rounded_md()
                            .cursor_pointer()
                            .hover(move |style| style.bg(hovered))
                            .child(
                                Icon::new(WhatfastIcon::Maximize)
                                    .with_size(px(DOWNLOAD_SIZE * scale))
                                    .text_color(muted),
                            )
                            .on_click(cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.open_video_fullscreen(
                                    expand_chat_id.clone(),
                                    expand_message_id.clone(),
                                    window,
                                    cx,
                                );
                            })),
                    ),
            )
            .when_some(error, |player, error| {
                player.child(
                    div()
                        .mt(px(4. * scale))
                        .text_size(px(ERROR_SIZE * scale))
                        .text_color(danger)
                        .child(error),
                )
            })
    }

    pub(crate) fn render_attachment_card(
        &self,
        index: usize,
        message: &proto::Message,
        attachment: &proto::AttachmentContent,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        // La nota de voz que ya esta en el disco se reproduce aca adentro,
        // como en la referencia, y no en el programa del sistema.
        if is_playable_voice_note(&self.store, attachment) {
            return self.render_voice_note(index, message, attachment, cx);
        }
        // El video se mira en la burbuja, con su reproductor: el archivo no
        // sale al programa del sistema.
        if crate::video::is_video(attachment) {
            return self.render_video(index, message, attachment, cx);
        }
        let scale = self.ui_scale;
        let name = attachment_name(attachment);
        let ext_badge: Option<(String, u32)> = std::path::Path::new(&attachment.file_name)
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_uppercase())
            .filter(|e| !e.is_empty() && e.len() <= 5)
            .map(|ext| {
                let color = match ext.as_str() {
                    "PDF" => 0xdc2626,
                    "ZIP" | "RAR" | "7Z" | "TAR" | "GZ" => 0xd97706,
                    "DOC" | "DOCX" | "ODT" => 0x2563eb,
                    "XLS" | "XLSX" | "CSV" => 0x16a34a,
                    "PPT" | "PPTX" => 0xea580c,
                    "MP3" | "WAV" | "OGG" | "FLAC" => 0x7c3aed,
                    _ => 0x4b5563,
                };
                (ext, color)
            });
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
                    .when_some(ext_badge, |row, (ext, badge_rgb)| {
                        row.child(
                            div()
                                .px_1p5()
                                .py_0p5()
                                .rounded_sm()
                                .bg(gpui::rgb(badge_rgb))
                                .text_xs()
                                .font_weight(gpui::FontWeight::BOLD)
                                .text_color(gpui::white())
                                .child(ext),
                        )
                    })
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

