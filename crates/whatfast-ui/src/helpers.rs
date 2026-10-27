//! The free helpers of the drawing layer.
//!
//! port: taken from the origin's `desktop/src/main.rs`, where they sat next to
//! the `render_*` methods of the GPUI client. Same names, same behaviour, same
//! constants: the drawing code (`render.rs`) only paints, and every format
//! that does not belong to the drawing lives here.

use std::path::PathBuf;
use std::rc::Rc;
use whatfast_i18n as i18n;

use gpui::{
    App, FontWeight, ObjectFit, ParentElement as _, Pixels, Styled as _, StyledImage as _, canvas,
    div, img, point, px, rgb, size,
};
use gpui_component::{ActiveTheme as _, Colorize as _, Icon, IconName, Sizable as _};
use qrcode::{Color, QrCode};

use whatfast_model::model::message_text;

use crate::local_time::local_offset_minutes;
use whatfast_protocol::{ConnectionState, ImageContent, Message, MessageStatus};

/// The box an image occupies inside a message bubble.
///
/// port: the origin scaled the picture down to a square of at most 220 points
/// for stickers and 320 points for photographs, keeping the aspect ratio.
pub(crate) fn image_render_size(image: &ImageContent) -> (Pixels, Pixels) {
    let max_edge = if image.sticker { 220. } else { 320. };
    if image.width == 0 || image.height == 0 {
        return if image.sticker {
            (px(max_edge), px(max_edge))
        } else {
            (px(300.), px(220.))
        };
    }
    let source_width = image.width as f32;
    let source_height = image.height as f32;
    let scale = (max_edge / source_width).min(max_edge / source_height);
    (px(source_width * scale), px(source_height * scale))
}

/// The pixel box of the picture inside the viewer card.
///
/// The card is a fixed rectangle of the window; the picture has to fit inside
/// it whole, keeping its aspect ratio. The message usually carries the size;
/// when it does not (an old row, a sticker), the file header answers, and a
/// square that fills the box is the last resort. Never larger than the box:
/// the card clips, and a picture painted past its box came out cut in half.
pub(crate) fn viewer_picture_size(
    viewer: &crate::app::ImageViewer,
    available_width: Pixels,
    available_height: Pixels,
) -> (Pixels, Pixels) {
    let ceiling = available_width.min(available_height);
    let (source_width, source_height) = match (viewer.width, viewer.height) {
        (width, height) if width > 0 && height > 0 => (width as f32, height as f32),
        _ => match picture_dimensions(PathBuf::from(&viewer.path).as_path()) {
            Some((width, height)) => (width as f32, height as f32),
            None => return (ceiling, ceiling),
        },
    };
    let available_width = f32::from(available_width);
    let available_height = f32::from(available_height);
    let scale = (available_width / source_width).min(available_height / source_height);
    (
        px((source_width * scale).max(1.)),
        px((source_height * scale).max(1.)),
    )
}

/// The pixel size of a picture, read from its header: PNG, JPEG or WebP.
/// Answers `None` for anything else or for a file that cannot be read.
fn picture_dimensions(path: &std::path::Path) -> Option<(u32, u32)> {
    let bytes = std::fs::read(path).ok()?;
    png_dimensions(&bytes)
        .or_else(|| jpeg_dimensions(&bytes))
        .or_else(|| webp_dimensions(&bytes))
}

/// The IHDR of a PNG: width and height big-endian right after the signature.
fn png_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 24 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" {
        return None;
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
    let height = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
    (width > 0 && height > 0).then_some((width, height))
}

/// The first SOF marker of a JPEG: the frame carries the height and width.
fn jpeg_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 4 || bytes[0] != 0xFF || bytes[1] != 0xD8 {
        return None;
    }
    let mut index = 2;
    while index + 8 < bytes.len() {
        if bytes[index] != 0xFF {
            index += 1;
            continue;
        }
        let marker = bytes[index + 1];
        // SOF0..SOF15, except DHT (C4), JPG (C8) and DAC (CC).
        if (0xC0..=0xCF).contains(&marker) && marker != 0xC4 && marker != 0xC8 && marker != 0xCC {
            let height = u32::from(u16::from_be_bytes([bytes[index + 5], bytes[index + 6]]));
            let width = u32::from(u16::from_be_bytes([bytes[index + 7], bytes[index + 8]]));
            return (width > 0 && height > 0).then_some((width, height));
        }
        let length = usize::from(u16::from_be_bytes([bytes[index + 2], bytes[index + 3]]));
        index += 2 + length;
    }
    None
}

/// The canvas of a WebP: the lossy VP8, the lossless VP8L or the VP8X header.
fn webp_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 30 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WEBP" {
        return None;
    }
    match &bytes[12..16] {
        b"VP8 " => {
            let width = u32::from(u16::from_le_bytes([bytes[26], bytes[27]]) & 0x3FFF);
            let height = u32::from(u16::from_le_bytes([bytes[28], bytes[29]]) & 0x3FFF);
            (width > 0 && height > 0).then_some((width, height))
        }
        b"VP8L" => {
            let bits = u32::from_le_bytes(bytes[21..25].try_into().ok()?);
            let width = (bits & 0x3FFF) + 1;
            let height = ((bits >> 14) & 0x3FFF) + 1;
            Some((width, height))
        }
        b"VP8X" => {
            let width = 1 + u32::from_le_bytes([bytes[24], bytes[25], bytes[26], 0]);
            let height = 1 + u32::from_le_bytes([bytes[27], bytes[28], bytes[29], 0]);
            (width > 0 && height > 0).then_some((width, height))
        }
        _ => None,
    }
}

/// Whether the bubble has to ask the backend for the image.
///
/// A picture needs the download when it has no thumbnail yet. An animated
/// sticker needs it even when the thumbnail is here: the full WebP is the
/// file that moves, and the thumbnail is a single frame.
pub(crate) fn image_needs_download(image: &ImageContent) -> bool {
    image.downloadable
        && (image.thumbnail_path.is_empty() || (image.animated && image.local_path.is_empty()))
}

/// The files that the bubble of an image tries, in order.
///
/// A picture is normally painted from the thumbnail that the backend keeps
/// for the rows. An animated sticker has to be painted from the full file:
/// the thumbnail is a single frame, and GPUI only plays the frames when it
/// reads the animated WebP itself. The caller keeps the first path that is
/// really in the disk.
pub(crate) fn image_source_candidates<'a>(
    image: &ImageContent,
    row_path: &'a str,
    viewer_path: &'a str,
) -> [&'a str; 2] {
    if image.animated && !viewer_path.is_empty() {
        [viewer_path, row_path]
    } else {
        [row_path, viewer_path]
    }
}

/// Collapses every run of whitespace — including the newlines carried by
/// multi-line messages — into single spaces, so a title or preview always
/// renders on one line. `truncate()` only sets `white-space: nowrap`, which
/// leaves hard `\n` breaks intact and would otherwise overflow fixed-height
/// rows and collide with the row below.
pub(crate) fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Up to two uppercase initials for an avatar, or `None` when the name has no
/// alphabetic characters to draw from (e.g. a bare phone number) — the caller
/// then falls back to a glyph so nothing renders off-centre or truncated.
pub(crate) fn avatar_initials(name: &str) -> Option<String> {
    let words: Vec<&str> = name.split_whitespace().collect();
    let mut initials: String = words
        .iter()
        .filter_map(|word| word.chars().find(|character| character.is_alphabetic()))
        .take(2)
        .collect();
    if initials.chars().count() < 2
        && let Some(word) = words
            .iter()
            .find(|word| word.chars().any(char::is_alphabetic))
    {
        initials = word
            .chars()
            .filter(|character| character.is_alphabetic())
            .take(2)
            .collect();
    }
    (!initials.is_empty()).then(|| initials.to_uppercase())
}

/// Stable hue in the 0..1 range derived from an identity key, so each
/// participant keeps the same colour across their avatar and name label.
pub(crate) fn identity_hue(key: &str) -> f32 {
    (gpui::hash(&key) % 24) as f32 * 15.0 / 360.0
}

/// A round avatar that always centres its content: the cached photo when we
/// have one, otherwise tinted initials, otherwise a person glyph. Replaces
/// the stock component so contactless senders no longer show off-centre
/// initials.
///
/// port: the `Avatar` of the origin (`tauri/src/components/Avatar.tsx`): a
/// group without a photo draws the glyph of the people, not the initials.
pub(crate) fn avatar_element(name: &str, avatar_path: &str, size: Pixels, cx: &App) -> gpui::Div {
    avatar_element_of(name, avatar_path, size, false, cx)
}

/// The avatar of a conversation or of a participant.
pub(crate) fn avatar_element_of(
    name: &str,
    avatar_path: &str,
    size: Pixels,
    group: bool,
    cx: &App,
) -> gpui::Div {
    let base = div()
        .flex_shrink_0()
        .size(size)
        .rounded_full()
        .overflow_hidden()
        .flex()
        .items_center()
        .justify_center();
    if !avatar_path.is_empty() {
        return base.child(
            img(PathBuf::from(avatar_path.to_owned()))
                .size(size)
                .rounded_full()
                .object_fit(ObjectFit::Cover),
        );
    }
    if group {
        return base.bg(cx.theme().secondary).child(
            Icon::new(IconName::User)
                .with_size(size * 0.48)
                .text_color(cx.theme().muted_foreground),
        );
    }
    match avatar_initials(name) {
        Some(initials) => {
            let color = cx.theme().blue.hue(identity_hue(name));
            base.bg(color.opacity(0.18))
                .text_color(color)
                .text_size(size * 0.4)
                .line_height(size)
                .font_weight(FontWeight::SEMIBOLD)
                .child(initials)
        }
        None => base
            .bg(cx.theme().secondary)
            .text_color(cx.theme().muted_foreground)
            .child(
                Icon::new(IconName::User)
                    .with_size(size * 0.55)
                    .text_color(cx.theme().muted_foreground),
            ),
    }
}

/// Civil-date conversion (Howard Hinnant's algorithm). Renders a date like
/// "12 Apr 2024" for the group "created" caption without a date dependency.
///
/// The date is the one of the machine: the offset of the zone of the instant
/// goes in first, so a group created at 22:30 in Buenos Aires says the day it
/// was created and not the next one, which is the day it has in UTC.
pub(crate) fn format_epoch_date(timestamp_ms: i64) -> String {
    let timestamp_ms = timestamp_ms + i64::from(local_offset_minutes(timestamp_ms)) * 60_000;
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let days = timestamp_ms.div_euclid(86_400_000);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{day} {} {year}", MONTHS[(month - 1) as usize])
}

/// The human label of a disappearing-messages timer.
pub(crate) fn format_disappearing_timer(seconds: u32) -> String {
    match seconds {
        0 => i18n::t("Off").into(),
        86_400 => i18n::t("24 hours").into(),
        604_800 => i18n::t("7 days").into(),
        7_776_000 => i18n::t("90 days").into(),
        s if s % 86_400 == 0 => i18n::t("{n} days").replace("{n}", &(s / 86_400).to_string()),
        s if s % 3_600 == 0 => i18n::t("{n} hours").replace("{n}", &(s / 3_600).to_string()),
        s if s % 60 == 0 => i18n::t("{n} minutes").replace("{n}", &(s / 60).to_string()),
        s => i18n::t("{n} seconds").replace("{n}", &s.to_string()),
    }
}

/// The label of the connection state shown under the chat title.
pub(crate) fn connection_label(connection: ConnectionState) -> &'static str {
    i18n::t(match connection {
        ConnectionState::Connected => "online",
        ConnectionState::Reconnecting => "reconnecting…",
        ConnectionState::Offline => "offline",
        ConnectionState::Connecting => "connecting…",
        _ => "local client",
    })
}

/// The glyph that the reference draws for the delivery state of a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StatusGlyph {
    /// The message is on its way: the clock of the reference.
    Clock,
    /// The server took it: one tick.
    Single,
    /// It reached the phone of the receiver: two ticks.
    Double,
    /// The receiver read it: the two ticks, in blue.
    Read,
    /// It did not go out: the warning of the reference.
    Failed,
}

/// The glyph of the delivery state of an outgoing message.
///
/// `None` is a state that draws nothing: the messages of the other side and
/// the ones the backend did not report.
///
/// port: the oracle wrote the state as a word (`sent`, `delivered`, `read`)
/// under the bubble. The reference draws the ticks of WhatsApp instead, and
/// the native client follows the reference.
pub(crate) fn message_status_glyph(status: MessageStatus) -> Option<StatusGlyph> {
    match status {
        MessageStatus::Pending => Some(StatusGlyph::Clock),
        MessageStatus::Sent => Some(StatusGlyph::Single),
        MessageStatus::Delivered => Some(StatusGlyph::Double),
        MessageStatus::Read => Some(StatusGlyph::Read),
        MessageStatus::Failed => Some(StatusGlyph::Failed),
        _ => None,
    }
}

/// A one-line preview of a message, used by the quoted-message blocks.
pub(crate) fn message_preview(message: &Message) -> String {
    let flattened = message_text(message)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if flattened.is_empty() {
        return "Media message".to_string();
    }
    let mut characters = flattened.chars();
    let preview = characters.by_ref().take(96).collect::<String>();
    if characters.next().is_some() {
        format!("{preview}…")
    } else {
        preview
    }
}

/// The QR code of the pairing screen, painted module by module.
pub(crate) fn qr_canvas(code: QrCode) -> gpui::Div {
    let width = code.width();
    let modules = Rc::new(
        code.into_colors()
            .into_iter()
            .map(|color| color == Color::Dark)
            .collect::<Vec<_>>(),
    );
    div()
        .size(px(240.))
        .p_3()
        .rounded_lg()
        .bg(rgb(0xffffff))
        .child(
            canvas(
                move |bounds, _, _| bounds,
                move |bounds, _, window, _| {
                    let cell = bounds.size.width / width as f32;
                    for row in 0..width {
                        for col in 0..width {
                            if modules[row * width + col] {
                                window.paint_quad(gpui::PaintQuad {
                                    bounds: gpui::Bounds {
                                        origin: bounds.origin
                                            + point(cell * col as f32, cell * row as f32),
                                        size: size(cell, cell),
                                    },
                                    corner_radii: gpui::Corners::default(),
                                    background: rgb(0x111b21).into(),
                                    border_widths: gpui::Edges::default(),
                                    border_color: gpui::transparent_black(),
                                    border_style: gpui::BorderStyle::default(),
                                });
                            }
                        }
                    }
                },
            )
            .size_full(),
        )
}

#[cfg(test)]
mod tests {
    use gpui::px;
    use whatfast_protocol::{ImageContent, MessageStatus};

    use super::{
        StatusGlyph, image_needs_download, image_source_candidates, jpeg_dimensions,
        message_status_glyph, png_dimensions, viewer_picture_size, webp_dimensions,
    };

    /// Un visor con el tamaño que trae el mensaje.
    fn viewer(width: u32, height: u32) -> crate::app::ImageViewer {
        crate::app::ImageViewer {
            chat_id: "chat".into(),
            message_id: "message".into(),
            path: String::new(),
            caption: String::new(),
            sticker: false,
            width,
            height,
        }
    }

    #[test]
    fn a_tall_picture_fits_the_box_whole_instead_of_being_cropped() {
        // La foto del reporte: vertical 360x1080 en una caja de 431x222. La
        // caja manda el alto y el ancho sale de la proporcion, nunca al reves:
        // con la imagen estirada al ancho de la caja se recortaba al medio.
        let (width, height) = viewer_picture_size(&viewer(360, 1080), px(431.), px(222.));
        assert_eq!(height, px(222.));
        assert!((f32::from(width) - 74.).abs() < 0.5, "ancho {width:?}");
    }

    #[test]
    fn a_wide_picture_fits_the_box_by_its_width() {
        let (width, height) = viewer_picture_size(&viewer(1920, 1080), px(400.), px(400.));
        assert_eq!(width, px(400.));
        assert!((f32::from(height) - 225.).abs() < 0.5, "alto {height:?}");
    }

    #[test]
    fn a_picture_never_paints_past_its_box() {
        for (width, height) in [(360u32, 1080u32), (1080, 360), (900, 900), (1, 4000)] {
            let (drawn_width, drawn_height) =
                viewer_picture_size(&viewer(width, height), px(500.), px(300.));
            assert!(drawn_width <= px(500.), "{width}x{height} de ancho");
            assert!(drawn_height <= px(300.), "{width}x{height} de alto");
        }
    }

    #[test]
    fn a_picture_without_a_size_falls_back_to_the_square_of_the_box() {
        let (width, height) = viewer_picture_size(&viewer(0, 0), px(500.), px(300.));
        assert_eq!((width, height), (px(300.), px(300.)));
    }

    #[test]
    fn a_png_header_answers_its_size() {
        let mut png = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        png.extend_from_slice(&[0, 0, 0, 13]);
        png.extend_from_slice(b"IHDR");
        png.extend_from_slice(&360u32.to_be_bytes());
        png.extend_from_slice(&1080u32.to_be_bytes());
        assert_eq!(png_dimensions(&png), Some((360, 1080)));
    }

    #[test]
    fn a_jpeg_header_answers_its_size() {
        let mut jpeg = vec![0xFF, 0xD8];
        // Un segmento cualquiera antes del SOF, como trae un JPEG real.
        jpeg.extend_from_slice(&[0xFF, 0xE0, 0x00, 0x04, 0x00, 0x00]);
        // SOF0: longitud, precision, alto, ancho.
        jpeg.extend_from_slice(&[0xFF, 0xC0, 0x00, 0x11, 0x08]);
        jpeg.extend_from_slice(&240u16.to_be_bytes());
        jpeg.extend_from_slice(&320u16.to_be_bytes());
        assert_eq!(jpeg_dimensions(&jpeg), Some((320, 240)));
    }

    #[test]
    fn a_webp_header_answers_its_size() {
        let mut webp = b"RIFF".to_vec();
        webp.extend_from_slice(&[0, 0, 0, 0]);
        webp.extend_from_slice(b"WEBPVP8X");
        webp.extend_from_slice(&[0; 8]);
        // El lienzo guarda el ancho y el alto menos uno, en tres bytes.
        webp.extend_from_slice(&[0x67, 0x01, 0x00]);
        webp.extend_from_slice(&[0x37, 0x01, 0x00]);
        assert_eq!(webp_dimensions(&webp), Some((360, 312)));
    }

    #[test]
    fn an_animated_sticker_asks_for_the_full_file_even_with_a_thumbnail() {
        let animated = ImageContent {
            sticker: true,
            animated: true,
            local_path: String::new(),
            thumbnail_path: "nota-thumb.webp".into(),
            downloadable: true,
            ..Default::default()
        };
        assert!(image_needs_download(&animated));
        // The same sticker with the full file in the disk is done.
        let cached = ImageContent {
            local_path: "nota.webp".into(),
            ..animated
        };
        assert!(!image_needs_download(&cached));
        // A still picture with its thumbnail does not download again.
        let still = ImageContent {
            sticker: false,
            animated: false,
            local_path: String::new(),
            thumbnail_path: "foto-thumb.jpg".into(),
            downloadable: true,
            ..Default::default()
        };
        assert!(!image_needs_download(&still));
        // One without any cached file does.
        let empty = ImageContent {
            thumbnail_path: String::new(),
            downloadable: true,
            ..Default::default()
        };
        assert!(image_needs_download(&empty));
    }

    #[test]
    fn an_animated_sticker_is_painted_from_the_full_file() {
        let animated = ImageContent {
            sticker: true,
            animated: true,
            local_path: "nota.webp".into(),
            thumbnail_path: "nota-thumb.webp".into(),
            ..Default::default()
        };
        assert_eq!(
            image_source_candidates(&animated, "nota-thumb.webp", "nota.webp"),
            ["nota.webp", "nota-thumb.webp"]
        );
        // A picture without movement keeps the small thumbnail for the row.
        let still = ImageContent {
            local_path: "foto.jpg".into(),
            thumbnail_path: "foto-thumb.jpg".into(),
            ..Default::default()
        };
        assert_eq!(
            image_source_candidates(&still, "foto-thumb.jpg", "foto.jpg"),
            ["foto-thumb.jpg", "foto.jpg"]
        );
    }

    #[test]
    fn every_state_of_a_message_has_its_glyph() {
        assert_eq!(
            message_status_glyph(MessageStatus::Pending),
            Some(StatusGlyph::Clock)
        );
        assert_eq!(
            message_status_glyph(MessageStatus::Sent),
            Some(StatusGlyph::Single)
        );
        assert_eq!(
            message_status_glyph(MessageStatus::Delivered),
            Some(StatusGlyph::Double)
        );
        assert_eq!(
            message_status_glyph(MessageStatus::Read),
            Some(StatusGlyph::Read)
        );
        assert_eq!(
            message_status_glyph(MessageStatus::Failed),
            Some(StatusGlyph::Failed)
        );
    }

    #[test]
    fn a_state_without_a_glyph_draws_nothing() {
        assert_eq!(message_status_glyph(MessageStatus::Unspecified), None);
    }
}
