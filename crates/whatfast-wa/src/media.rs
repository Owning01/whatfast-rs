//! Outgoing media: images today; attachments and stickers ride the same
//! upload path as their waves land.
//!
//! The upload encrypts and stores the blob on WhatsApp's CDN through the
//! library; the local row keeps the source path the desktop picked, so the
//! conversation renders the image from disk with zero re-download.
//!
//! Incoming media rides the same folder in the other direction: the desktop
//! asks for one attachment or image and this module downloads, decrypts, and
//! caches the plaintext file, then the row points at it.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use wacore::download::MediaType;
use whatsapp_rust::Jid;
use whatsapp_rust::download::{DownloadLimits, DownloadParams};
use whatsapp_rust::upload::UploadOptions;
use whatsapp_rust::waproto::whatsapp as wa;

use crate::db::{self, MessageRow, Store};
use crate::meta::hash_hex;
use crate::send::{chat_jid, internal, refresh_chat_pointer, stored_request};
use crate::session::Session;

/// The download ceiling, the same 2 GiB the Go sidecar enforced.
const MAX_MEDIA_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// Sends a local image with an optional caption.
#[expect(clippy::too_many_arguments)]
pub async fn send_image(
    session: &Session,
    store: &Arc<Store>,
    client_message_id: &str,
    chat_id: &str,
    image_path: &str,
    caption: &str,
    reply_to_message_id: &str,
    reply_to_chat_id: &str,
) -> Result<MessageRow, (String, String, bool)> {
    if let Some((_chat, message_id)) = stored_request(store, client_message_id)
        && let Some(row) = store.message(chat_id, &message_id).map_err(internal)?
    {
        return Ok(row);
    }
    let bytes = std::fs::read(image_path).map_err(|error| {
        (
            "invalid_argument".into(),
            format!("reading the image: {error}"),
            false,
        )
    })?;
    let mime = sniff_mime(&bytes);
    let (width, height) = sniff_dimensions(&bytes, &mime);
    let response = session
        .client
        .upload(bytes, MediaType::Image, UploadOptions::new())
        .await
        .map_err(|error| ("upload_failed".into(), format!("{error}"), true))?;

    let mut image = wa::message::ImageMessage::default();
    image.url = Some(response.url.clone());
    image.mimetype = Some(mime.clone());
    image.caption = Some(caption.to_owned());
    image.direct_path = Some(response.direct_path.clone());
    image.media_key = Some(response.media_key.to_vec());
    image.file_sha256 = Some(response.file_sha256.to_vec());
    image.file_enc_sha256 = Some(response.file_enc_sha256.to_vec());
    image.file_length = Some(response.file_length);
    image.width = Some(width);
    image.height = Some(height);
    let mut message = wa::Message::default();
    message.image_message = whatsapp_rust::waproto::buffa::MessageField::some(image);
    if !caption.is_empty() && !reply_to_message_id.is_empty() {
        let reply_chat = if reply_to_chat_id.is_empty() {
            chat_id.to_owned()
        } else {
            reply_to_chat_id.to_owned()
        };
        if let Some(mut image) = message.image_message.as_option().cloned() {
            let mut context = wa::ContextInfo::default();
            context.stanza_id = Some(reply_to_message_id.to_owned());
            context.remote_jid = Some(reply_chat);
            image.context_info = whatsapp_rust::waproto::buffa::MessageField::some(context);
            message.image_message = whatsapp_rust::waproto::buffa::MessageField::some(image);
        }
    }
    deliver_media(
        session,
        store,
        client_message_id,
        chat_id,
        message,
        MediaKind {
            kind: "image",
            caption: caption.to_owned(),
            mime,
            file_name: String::new(),
            local_path: image_path.to_owned(),
            direct_path: response.direct_path.clone(),
            media_key: response.media_key.to_vec(),
            file_sha256: response.file_sha256.to_vec(),
            file_enc_sha256: response.file_enc_sha256.to_vec(),
            size: response.file_length,
            width,
            height,
            duration: 0,
            voice: false,
            animated: false,
        },
        reply_to_message_id,
        reply_to_chat_id,
    )
    .await
}

/// The media-shaped columns an outgoing attachment carries.
pub struct MediaKind {
    pub kind: &'static str,
    pub caption: String,
    pub mime: String,
    pub file_name: String,
    pub local_path: String,
    pub direct_path: String,
    pub media_key: Vec<u8>,
    pub file_sha256: Vec<u8>,
    pub file_enc_sha256: Vec<u8>,
    pub size: u64,
    pub width: u32,
    pub height: u32,
    pub duration: u32,
    pub voice: bool,
    pub animated: bool,
}

/// Uploads, sends, stores, and answers one media message. The shared body
/// of every media send.
#[expect(clippy::too_many_arguments)]
async fn deliver_media(
    session: &Session,
    store: &Arc<Store>,
    client_message_id: &str,
    chat_id: &str,
    message: wa::Message,
    media: MediaKind,
    reply_to_message_id: &str,
    reply_to_chat_id: &str,
) -> Result<MessageRow, (String, String, bool)> {
    let jid: Jid =
        chat_jid(chat_id).map_err(|message| ("invalid_argument".into(), message, false))?;
    let sent = session
        .client
        .send_message(jid, message)
        .await
        .map_err(|error| ("send_failed".into(), format!("{error}"), true))?;
    let message_id = sent.message_id.to_string();
    store
        .write(|conn| {
            conn.execute(
                "INSERT OR IGNORE INTO outgoing_requests(client_request_id,chat_jid,text,message_id,created_at)
                 VALUES(?1,?2,?3,?4,?5)",
                rusqlite::params![
                    client_message_id,
                    chat_id,
                    media.caption,
                    message_id,
                    db::now_ms()
                ],
            )?;
            Ok(())
        })
        .map_err(internal)?;
    let row = MessageRow {
        id: message_id,
        chat_jid: chat_id.to_owned(),
        transport_jid: chat_id.to_owned(),
        sender_jid: String::new(),
        text: String::new(),
        timestamp: db::now_ms(),
        from_me: true,
        status: whatfast_protocol::MessageStatus::Sent as i64,
        kind: media.kind.to_owned(),
        reply_to_id: reply_to_message_id.to_owned(),
        reply_to_chat_id: reply_to_chat_id.to_owned(),
        image_mime: media.mime.clone(),
        image_caption: media.caption.clone(),
        image_local_path: media.local_path.clone(),
        image_direct_path: media.direct_path.clone(),
        image_media_key: media.media_key.clone(),
        image_file_sha256: media.file_sha256.clone(),
        image_file_enc_sha256: media.file_enc_sha256.clone(),
        image_width: media.width as i64,
        image_height: media.height as i64,
        image_size: media.size as i64,
        image_animated: media.animated,
        media_mime: media.mime,
        media_file_name: media.file_name,
        media_local_path: media.local_path,
        media_direct_path: media.direct_path,
        media_media_key: media.media_key,
        media_file_sha256: media.file_sha256,
        media_file_enc_sha256: media.file_enc_sha256,
        media_width: media.width as i64,
        media_height: media.height as i64,
        media_size: media.size as i64,
        media_duration: media.duration as i64,
        media_voice: media.voice,
        media_animated: media.animated,
        media_caption: media.caption,
        ..MessageRow::default()
    };
    store.insert_message(&row).map_err(internal)?;
    refresh_chat_pointer(store, chat_id).map_err(internal)?;
    Ok(row)
}

/// The mime of a media blob by its first bytes; unknown falls back to a
/// generic stream so the upload still happens.
fn sniff_mime(bytes: &[u8]) -> String {
    match bytes.first_chunk::<12>() {
        Some([0x89, b'P', b'N', b'G', ..]) => "image/png",
        Some([0xFF, 0xD8, 0xFF, ..]) => "image/jpeg",
        Some([b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P']) => "image/webp",
        Some([b'G', b'I', b'F', b'8', ..]) => "image/gif",
        _ => "application/octet-stream",
    }
    .to_owned()
}

/// The pixel size of a PNG or JPEG; unknown formats answer zero and the
/// peer's client scales on its own.
fn sniff_dimensions(bytes: &[u8], mime: &str) -> (u32, u32) {
    match mime {
        "image/png" => {
            // IHDR: 8-byte signature, 4 length, 4 "IHDR", then width/height.
            if bytes.len() >= 24 {
                let width = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
                let height = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
                return (width, height);
            }
            (0, 0)
        }
        "image/jpeg" => {
            let mut offset = 2;
            while offset + 9 < bytes.len() {
                if bytes[offset] != 0xFF {
                    break;
                }
                let marker = bytes[offset + 1];
                let length = u16::from_be_bytes([bytes[offset + 2], bytes[offset + 3]]) as usize;
                // SOF0..SOF15 minus the progressive DHT marker carry the size.
                if (0xC0..=0xCF).contains(&marker) && marker != 0xC4 && marker != 0xC8 {
                    if offset + 9 <= bytes.len() {
                        let height =
                            u16::from_be_bytes([bytes[offset + 5], bytes[offset + 6]]) as u32;
                        let width =
                            u16::from_be_bytes([bytes[offset + 7], bytes[offset + 8]]) as u32;
                        return (width, height);
                    }
                    break;
                }
                offset += 2 + length;
            }
            (0, 0)
        }
        _ => (0, 0),
    }
}

/// Sends a document, a video, or an audio/voice note from a local file.
#[expect(clippy::too_many_arguments)]
pub async fn send_attachment(
    session: &Session,
    store: &Arc<Store>,
    client_message_id: &str,
    chat_id: &str,
    file_path: &str,
    kind: whatfast_protocol::AttachmentKind,
    caption: &str,
    reply_to_message_id: &str,
    reply_to_chat_id: &str,
    voice_note: bool,
) -> Result<MessageRow, (String, String, bool)> {
    if let Some((_chat, message_id)) = stored_request(store, client_message_id)
        && let Some(row) = store.message(chat_id, &message_id).map_err(internal)?
    {
        return Ok(row);
    }
    let bytes = std::fs::read(file_path).map_err(|error| {
        (
            "invalid_argument".into(),
            format!("reading the attachment: {error}"),
            false,
        )
    })?;
    let file_name = std::path::Path::new(file_path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let (media_type, wire_kind, mime) = match kind {
        whatfast_protocol::AttachmentKind::Video => (
            MediaType::Video,
            "video",
            sniff_mime(&bytes).replace("application/octet-stream", "video/mp4"),
        ),
        whatfast_protocol::AttachmentKind::Audio => (
            MediaType::Audio,
            "audio",
            sniff_mime(&bytes).replace("application/octet-stream", "audio/ogg"),
        ),
        _ => (MediaType::Document, "document", sniff_mime(&bytes)),
    };
    let response = session
        .client
        .upload(bytes, media_type, UploadOptions::new())
        .await
        .map_err(|error| ("upload_failed".into(), format!("{error}"), true))?;

    let mut message = wa::Message::default();
    match wire_kind {
        "video" => {
            let mut video = wa::message::VideoMessage::default();
            video.url = Some(response.url.clone());
            video.mimetype = Some(mime.clone());
            video.caption = Some(caption.to_owned());
            video.direct_path = Some(response.direct_path.clone());
            video.media_key = Some(response.media_key.to_vec());
            video.file_sha256 = Some(response.file_sha256.to_vec());
            video.file_enc_sha256 = Some(response.file_enc_sha256.to_vec());
            video.file_length = Some(response.file_length);
            message.video_message = whatsapp_rust::waproto::buffa::MessageField::some(video);
        }
        "audio" => {
            let mut audio = wa::message::AudioMessage::default();
            audio.url = Some(response.url.clone());
            audio.mimetype = Some(mime.clone());
            audio.direct_path = Some(response.direct_path.clone());
            audio.media_key = Some(response.media_key.to_vec());
            audio.file_sha256 = Some(response.file_sha256.to_vec());
            audio.file_enc_sha256 = Some(response.file_enc_sha256.to_vec());
            audio.file_length = Some(response.file_length);
            audio.ptt = Some(voice_note);
            message.audio_message = whatsapp_rust::waproto::buffa::MessageField::some(audio);
        }
        _ => {
            let mut document = wa::message::DocumentMessage::default();
            document.url = Some(response.url.clone());
            document.mimetype = Some(mime.clone());
            document.title = Some(file_name.clone());
            document.file_name = Some(file_name.clone());
            document.caption = Some(caption.to_owned());
            document.direct_path = Some(response.direct_path.clone());
            document.media_key = Some(response.media_key.to_vec());
            document.file_sha256 = Some(response.file_sha256.to_vec());
            document.file_enc_sha256 = Some(response.file_enc_sha256.to_vec());
            document.file_length = Some(response.file_length);
            message.document_message = whatsapp_rust::waproto::buffa::MessageField::some(document);
        }
    }
    deliver_media(
        session,
        store,
        client_message_id,
        chat_id,
        message,
        MediaKind {
            kind: wire_kind,
            caption: caption.to_owned(),
            mime,
            file_name,
            local_path: file_path.to_owned(),
            direct_path: response.direct_path.clone(),
            media_key: response.media_key.to_vec(),
            file_sha256: response.file_sha256.to_vec(),
            file_enc_sha256: response.file_enc_sha256.to_vec(),
            size: response.file_length,
            width: 0,
            height: 0,
            duration: 0,
            voice: voice_note && wire_kind == "audio",
            animated: false,
        },
        reply_to_message_id,
        reply_to_chat_id,
    )
    .await
}

/// The sticker path: an already-made WebP the desktop picked or made.
pub async fn send_sticker(
    session: &Session,
    store: &Arc<Store>,
    client_message_id: &str,
    chat_id: &str,
    webp_data: &[u8],
    reply_to_message_id: &str,
    reply_to_chat_id: &str,
) -> Result<MessageRow, (String, String, bool)> {
    if let Some((_chat, message_id)) = stored_request(store, client_message_id)
        && let Some(row) = store.message(chat_id, &message_id).map_err(internal)?
    {
        return Ok(row);
    }
    let response = session
        .client
        .upload(webp_data.to_vec(), MediaType::Sticker, UploadOptions::new())
        .await
        .map_err(|error| ("upload_failed".into(), format!("{error}"), true))?;
    let (width, height, animated) = sniff_webp(webp_data);
    let mut sticker = wa::message::StickerMessage::default();
    sticker.url = Some(response.url.clone());
    sticker.mimetype = Some("image/webp".into());
    sticker.direct_path = Some(response.direct_path.clone());
    sticker.media_key = Some(response.media_key.to_vec());
    sticker.file_sha256 = Some(response.file_sha256.to_vec());
    sticker.file_enc_sha256 = Some(response.file_enc_sha256.to_vec());
    sticker.file_length = Some(response.file_length);
    sticker.width = Some(width);
    sticker.height = Some(height);
    sticker.is_animated = Some(animated);
    let mut message = wa::Message::default();
    message.sticker_message = whatsapp_rust::waproto::buffa::MessageField::some(sticker);
    deliver_media(
        session,
        store,
        client_message_id,
        chat_id,
        message,
        MediaKind {
            kind: "sticker",
            caption: String::new(),
            mime: "image/webp".into(),
            file_name: String::new(),
            local_path: String::new(),
            direct_path: response.direct_path.clone(),
            media_key: response.media_key.to_vec(),
            file_sha256: response.file_sha256.to_vec(),
            file_enc_sha256: response.file_enc_sha256.to_vec(),
            size: response.file_length,
            width,
            height,
            duration: 0,
            voice: false,
            animated,
        },
        reply_to_message_id,
        reply_to_chat_id,
    )
    .await
}

/// The WebP dimensions from the VP8X chunk; plain VP8/VP8L answer 512, the
/// square WhatsApp normalizes to, and `animated` reads the ALPH/ANIM flag.
fn sniff_webp(bytes: &[u8]) -> (u32, u32, bool) {
    let Some(header) = bytes.first_chunk::<30>() else {
        return (0, 0, false);
    };
    if &header[8..12] != b"WEBP" {
        return (0, 0, false);
    }
    match &header[12..16] {
        b"VP8X" => {
            let width = (1 + u32::from(header[24]))
                | (u32::from(header[25]) << 8)
                | (u32::from(header[26]) << 16);
            let height = (1 + u32::from(header[27]))
                | (u32::from(header[28]) << 8)
                | (u32::from(header[29]) << 16);
            let animated = header[20] & 0x02 != 0;
            (width, height, animated)
        }
        _ => (512, 512, false),
    }
}

/// The cache path of one message's media: a stable hash of the ids plus the
/// extension that the file name or the mime type suggests.
///
/// port: the Go sidecar hashed `chat\0message` with sha256 for the same
/// purpose; the FNV-1a key of the avatars is enough here, because the cache
/// regenerates and nothing shares it across profiles.
pub(crate) fn cached_media_path(
    media_dir: &Path,
    chat_id: &str,
    message_id: &str,
    mime: &str,
    file_name: &str,
) -> PathBuf {
    let mut key = Vec::with_capacity(chat_id.len() + message_id.len() + 1);
    key.extend_from_slice(chat_id.as_bytes());
    key.push(0);
    key.extend_from_slice(message_id.as_bytes());
    media_dir.join(format!(
        "media-{}{}",
        hash_hex(&key),
        cache_extension(mime, file_name)
    ))
}

/// The extension of one cached media file, from the file name first and the
/// mime type second; empty when neither names one the cache trusts.
fn cache_extension(mime: &str, file_name: &str) -> String {
    const KNOWN: &[&str] = &[
        "3gpp", "aac", "amr", "apk", "avi", "csv", "doc", "docx", "gif", "jpeg", "jpg", "m4a",
        "mkv", "mov", "mp3", "mp4", "oga", "ogg", "opus", "pdf", "png", "ppt", "pptx", "rar",
        "txt", "wav", "webm", "webp", "xls", "xlsx", "zip",
    ];
    if let Some(dot) = file_name.rfind('.') {
        let candidate = file_name[dot + 1..].to_ascii_lowercase();
        if KNOWN.contains(&candidate.as_str()) {
            return format!(".{candidate}");
        }
    }
    let head = mime.split(';').next().unwrap_or("").trim();
    let mapped = match head {
        "video/mp4" | "video/quicktime" => "mp4",
        "video/webm" => "webm",
        "video/x-matroska" => "mkv",
        "video/3gpp" => "3gpp",
        "audio/ogg" => "ogg",
        "audio/mpeg" => "mp3",
        "audio/mp4" => "m4a",
        "audio/aac" => "aac",
        "audio/amr" => "amr",
        "audio/wav" | "audio/x-wav" => "wav",
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/webp" => "webp",
        "application/pdf" => "pdf",
        "application/zip" => "zip",
        "text/plain" => "txt",
        _ => "",
    };
    if mapped.is_empty() {
        String::new()
    } else {
        format!(".{mapped}")
    }
}

/// Which columns of the row carry the media, and which CDN type decrypts it.
struct MediaSource {
    mime: String,
    file_name: String,
    local_path: String,
    direct_path: String,
    media_key: Vec<u8>,
    file_sha256: Vec<u8>,
    file_enc_sha256: Vec<u8>,
    file_size: u64,
    media_type: MediaType,
}

impl MediaSource {
    /// The media columns of one row, or nothing when the kind carries no
    /// downloadable media.
    fn of(row: &MessageRow) -> Option<Self> {
        let media_type = match row.kind.as_str() {
            "video" => MediaType::Video,
            "audio" => MediaType::Audio,
            "document" => MediaType::Document,
            "image" | "sticker" => MediaType::Image,
            _ => return None,
        };
        Some(if row.kind == "image" || row.kind == "sticker" {
            Self {
                mime: row.image_mime.clone(),
                file_name: String::new(),
                local_path: row.image_local_path.clone(),
                direct_path: row.image_direct_path.clone(),
                media_key: row.image_media_key.clone(),
                file_sha256: row.image_file_sha256.clone(),
                file_enc_sha256: row.image_file_enc_sha256.clone(),
                file_size: row.image_size.max(0) as u64,
                media_type: if row.kind == "sticker" {
                    MediaType::Sticker
                } else {
                    MediaType::Image
                },
            }
        } else {
            Self {
                mime: row.media_mime.clone(),
                file_name: row.media_file_name.clone(),
                local_path: row.media_local_path.clone(),
                direct_path: row.media_direct_path.clone(),
                media_key: row.media_media_key.clone(),
                file_sha256: row.media_file_sha256.clone(),
                file_enc_sha256: row.media_file_enc_sha256.clone(),
                file_size: row.media_size.max(0) as u64,
                media_type,
            }
        })
    }

    /// The local file when it is already valid: a real file whose size, when
    /// the row declares one, matches.
    fn cached(&self) -> Option<String> {
        if self.local_path.is_empty() {
            return None;
        }
        let size = std::fs::metadata(&self.local_path).ok()?.len();
        if size == 0 || (self.file_size > 0 && size != self.file_size) {
            return None;
        }
        Some(self.local_path.clone())
    }
}

/// Downloads one message's media into the cache and answers the file path.
///
/// A message whose file is already cached answers it straight away; the
/// download decrypts straight into a temporary file that is renamed into
/// place, so a failed attempt never leaves a half file behind the final
/// name.
///
/// The Go sidecar also recovered expired CDN references through app-state
/// before retrying; this port answers an honest `not_found` instead until
/// that wave lands.
pub(crate) async fn fetch_message_media(
    session: &Session,
    store: &Arc<Store>,
    media_dir: &Path,
    chat_id: &str,
    message_id: &str,
) -> Result<String, (String, String, bool)> {
    let row = store
        .message(chat_id, message_id)
        .map_err(internal)?
        .ok_or_else(|| ("not_found".into(), "message is not stored".into(), false))?;
    if row.revoked {
        return Err(("not_found".into(), "attachment was deleted".into(), false));
    }
    let chat_id = row.chat_jid.clone();
    let source = MediaSource::of(&row).ok_or_else(|| {
        (
            "invalid_argument".into(),
            "message is not an attachment".into(),
            false,
        )
    })?;
    if source.file_size > MAX_MEDIA_BYTES {
        return Err((
            "failed_precondition".into(),
            "attachment exceeds the 2 GiB download limit".into(),
            false,
        ));
    }

    if let Some(path) = source.cached() {
        return Ok(path);
    }
    let cache = cached_media_path(
        media_dir,
        &chat_id,
        message_id,
        &source.mime,
        &source.file_name,
    );
    if let Ok(size) = std::fs::metadata(&cache).map(|meta| meta.len())
        && size > 0
        && (source.file_size == 0 || size == source.file_size)
    {
        let path = cache.to_string_lossy().into_owned();
        set_media_path(store, &row, &path)?;
        return Ok(path);
    }

    if source.direct_path.is_empty() || source.media_key.len() != 32 {
        return Err((
            "not_found".into(),
            "the media descriptor of this message is missing".into(),
            false,
        ));
    }
    let params = DownloadParams::encrypted(
        source.direct_path.clone(),
        &source.media_key,
        &source.file_sha256,
        &source.file_enc_sha256,
        source.file_size,
        source.media_type,
    )
    .map_err(|error| {
        (
            "not_found".into(),
            format!("media descriptor: {error}"),
            false,
        )
    })?;

    std::fs::create_dir_all(media_dir).map_err(|error| {
        (
            "internal".into(),
            format!("creating the media cache: {error}"),
            false,
        )
    })?;
    let temporary = media_dir.join(format!(
        ".media-{}.tmp",
        hash_hex(cache.to_string_lossy().as_bytes())
    ));
    let download = async {
        let file = std::fs::File::create(&temporary).map_err(|error| {
            (
                "internal".into(),
                format!("creating the temp file: {error}"),
                false,
            )
        })?;
        // The writer travels by value and comes back inside the result.
        let file = session
            .client
            .download_to_writer(&params, file, DownloadLimits::default())
            .await
            .map_err(|error| {
                (
                    "unavailable".into(),
                    format!("media download: {error}"),
                    true,
                )
            })?;
        file.sync_all().map_err(|error| {
            (
                "internal".into(),
                format!("flushing the download: {error}"),
                false,
            )
        })?;
        Ok(())
    };
    if let Err(error) = download.await {
        let _ = std::fs::remove_file(&temporary);
        return Err(error);
    }
    // Windows refuses a rename onto an existing name: the cache entry only
    // exists here when its size check failed, so clear it first.
    let _ = std::fs::remove_file(&cache);
    std::fs::rename(&temporary, &cache).map_err(|error| {
        (
            "internal".into(),
            format!("moving the download into the cache: {error}"),
            false,
        )
    })?;
    let path = cache.to_string_lossy().into_owned();
    set_media_path(store, &row, &path)?;
    Ok(path)
}

/// Points the row at the cache file, on the column set its kind uses.
fn set_media_path(
    store: &Store,
    row: &MessageRow,
    path: &str,
) -> Result<(), (String, String, bool)> {
    let changed = if row.kind == "image" || row.kind == "sticker" {
        store.set_image_local_path(&row.chat_jid, &row.id, path)
    } else {
        store.set_media_local_path(&row.chat_jid, &row.id, path)
    };
    changed.map(|_| ()).map_err(internal)
}
