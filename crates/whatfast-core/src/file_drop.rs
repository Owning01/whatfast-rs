//! Port of `tauri/src/lib/file-drop.ts`.
//!
//! The original classifies a batch of dropped paths into one WhatsApp send
//! mode.

/// How the batch is going to be sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileSendMode {
    Image,
    Sticker,
    Attachment,
}

impl FileSendMode {
    /// The string the original returns in the `mode` field.
    pub fn as_str(self) -> &'static str {
        match self {
            FileSendMode::Image => "image",
            FileSendMode::Sticker => "sticker",
            FileSendMode::Attachment => "attachment",
        }
    }
}

/// The kind of attachment the backend is asked to build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum AttachmentKind {
    Unspecified = 0,
    Document = 1,
    Video = 2,
    Audio = 3,
}

/// One confirmed batch of dropped files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DroppedFileBatch {
    pub mode: FileSendMode,
    pub attachment_kind: AttachmentKind,
}

const IMAGE_EXTENSIONS: [&str; 6] = ["avif", "gif", "jpeg", "jpg", "png", "webp"];
const VIDEO_EXTENSIONS: [&str; 5] = ["m4v", "mkv", "mov", "mp4", "webm"];
const AUDIO_EXTENSIONS: [&str; 7] = ["aac", "flac", "m4a", "mp3", "ogg", "opus", "wav"];

const DOCUMENT_KIND: AttachmentKind = AttachmentKind::Document;
const VIDEO_KIND: AttachmentKind = AttachmentKind::Video;
const AUDIO_KIND: AttachmentKind = AttachmentKind::Audio;

/// port: the original lowercases with `toLocaleLowerCase()`; the port uses the
/// locale independent `to_lowercase()` because the extensions it compares are
/// ASCII.
fn extension(path: &str) -> String {
    // port: `path.split(/[\\/]/).at(-1)`.
    let name = path.split(['\\', '/']).next_back().unwrap_or("");
    match name.rfind('.') {
        // port: `separator > 0`, a dot at the start of the name is not an
        // extension.
        Some(separator) if separator > 0 => name[separator + 1..].to_lowercase(),
        _ => String::new(),
    }
}

/// Keep one WhatsApp send mode per confirmed batch. Homogeneous media drops
/// retain their native photo/video/audio treatment; mixed drops remain a
/// single reviewable batch and are sent as documents without guessing.
pub fn classify_dropped_files(paths: &[String]) -> DroppedFileBatch {
    let extensions: Vec<String> = paths.iter().map(|path| extension(path)).collect();

    // port: the original checks `extensions.length > 0` before every
    // `every(...)`, so an empty batch is a document batch.
    if !extensions.is_empty()
        && extensions
            .iter()
            .all(|value| IMAGE_EXTENSIONS.contains(&value.as_str()))
    {
        return DroppedFileBatch {
            mode: FileSendMode::Image,
            attachment_kind: DOCUMENT_KIND,
        };
    }
    if !extensions.is_empty()
        && extensions
            .iter()
            .all(|value| VIDEO_EXTENSIONS.contains(&value.as_str()))
    {
        return DroppedFileBatch {
            mode: FileSendMode::Attachment,
            attachment_kind: VIDEO_KIND,
        };
    }
    if !extensions.is_empty()
        && extensions
            .iter()
            .all(|value| AUDIO_EXTENSIONS.contains(&value.as_str()))
    {
        return DroppedFileBatch {
            mode: FileSendMode::Attachment,
            attachment_kind: AUDIO_KIND,
        };
    }
    DroppedFileBatch {
        mode: FileSendMode::Attachment,
        attachment_kind: DOCUMENT_KIND,
    }
}
