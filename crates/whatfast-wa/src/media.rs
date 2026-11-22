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
use std::sync::atomic::{AtomicI64, Ordering};

use wacore::download::{Downloadable, MediaType};
use whatsapp_rust::Jid;
use whatsapp_rust::download::{DownloadLimits, DownloadParams, DownloadWriter};
use whatsapp_rust::upload::UploadOptions;
use whatsapp_rust::waproto::whatsapp as wa;

use crate::db::{self, MessageRow, Store};
use crate::meta::hash_hex;
use crate::send::{chat_jid, internal, refresh_chat_pointer, reply_context};
use crate::session::Session;

/// The download ceiling, the same 2 GiB the Go sidecar enforced.
const MAX_MEDIA_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// How often a running download checks that bytes are still arriving.
const DOWNLOAD_PROGRESS_POLL: std::time::Duration = std::time::Duration::from_secs(5);

/// A download that receives nothing for this long is abandoned, so a dead
/// connection cannot keep the request loop waiting.
const DOWNLOAD_STALL_MS: i64 = 45_000;

/// The outgoing image ceiling (`maxImageBytes`) and the safe decode bounds
/// `safeImageConfig` enforced before an upload.
const MAX_OUTGOING_IMAGE_BYTES: usize = 32 * 1024 * 1024;
const MAX_IMAGE_EDGE: u32 = 8192;
const MAX_IMAGE_PIXELS: u64 = 16_000_000;

/// Reads the file of an outgoing message, off the async thread.
///
/// The upload of the library takes the whole file as one buffer, so the read
/// cannot stream into the request. It can at least stay off the async thread,
/// where a slow disk would stall the bridge.
async fn read_upload(
    path: &str,
    what: &'static str,
) -> Result<Vec<u8>, (String, String, bool)> {
    let path = path.to_owned();
    tokio::task::spawn_blocking(move || std::fs::read(&path))
        .await
        .map_err(|error| {
            (
                "internal".to_string(),
                format!("the read of the {what} was interrupted: {error}"),
                true,
            )
        })?
        .map_err(|error| {
            (
                "invalid_argument".to_string(),
                format!("reading the {what}: {error}"),
                false,
            )
        })
}

/// The size of a local file, read off the async thread: the caller refuses
/// an oversized upload before it allocates the buffer.
async fn upload_size(path: &str) -> Result<u64, (String, String, bool)> {
    let path = path.to_owned();
    tokio::task::spawn_blocking(move || std::fs::metadata(&path).map(|meta| meta.len()))
        .await
        .map_err(|error| {
            (
                "internal".to_string(),
                format!("the size of the file could not be read: {error}"),
                true,
            )
        })?
        .map_err(|error| {
            (
                "invalid_argument".to_string(),
                format!("reading the file: {error}"),
                false,
            )
        })
}

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
    let size = upload_size(image_path).await?;
    if size == 0 || size > MAX_OUTGOING_IMAGE_BYTES as u64 {
        return Err((
            "invalid_argument".into(),
            format!(
                "image must be a regular file between 1 byte and {} MiB",
                MAX_OUTGOING_IMAGE_BYTES / (1024 * 1024)
            ),
            false,
        ));
    }
    let bytes = read_upload(image_path, "image").await?;
    let fingerprint = format!("image\u{0}{caption}\u{0}{}", hash_hex(&bytes));
    if let Some(row) =
        crate::send::stored_outgoing(store, client_message_id, chat_id, &fingerprint)?
    {
        return Ok(row);
    }
    let mime = outgoing_image_mime(&bytes)?;
    let (width, height) = sniff_dimensions(&bytes, &mime);
    check_image_bounds(width, height)?;
    let context = reply_context(store, chat_id, reply_to_chat_id, reply_to_message_id)?;
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
    if let Some(context) = context {
        image.context_info = whatsapp_rust::waproto::buffa::MessageField::some(context);
    }
    let mut message = wa::Message::default();
    message.image_message = whatsapp_rust::waproto::buffa::MessageField::some(image);
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
        &fingerprint,
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

/// The text the local row of an outgoing attachment keeps when the file has
/// no caption of its own.
///
/// port: `attachmentFallback`. The chat list and the bubble read this column,
/// so an empty name shows as an empty message until the echo arrives.
fn attachment_fallback(kind: &str, caption: &str, file_name: &str, voice: bool) -> String {
    if !caption.is_empty() {
        return caption.to_owned();
    }
    match kind {
        "document" if !file_name.is_empty() => file_name.to_owned(),
        "document" => "📄 Document".into(),
        "video" => "🎬 Video".into(),
        "audio" if voice => "🎤 Voice message".into(),
        "audio" => "🎵 Audio".into(),
        _ => "Attachment".into(),
    }
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
    fingerprint: &str,
) -> Result<MessageRow, (String, String, bool)> {
    let jid: Jid =
        chat_jid(chat_id).map_err(|message| ("invalid_argument".into(), message, false))?;
    let reply_chat_id = if reply_to_message_id.is_empty() {
        String::new()
    } else if reply_to_chat_id.is_empty() {
        chat_id.to_owned()
    } else {
        reply_to_chat_id.to_owned()
    };
    let row = MessageRow {
        chat_jid: chat_id.to_owned(),
        transport_jid: chat_id.to_owned(),
        sender_jid: crate::send::own_sender_jid(session),
        text: attachment_fallback(
            media.kind,
            &media.caption,
            &media.file_name,
            media.voice,
        ),
        timestamp: db::now_ms(),
        from_me: true,
        kind: media.kind.to_owned(),
        reply_to_id: reply_to_message_id.to_owned(),
        reply_to_chat_id: reply_chat_id,
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
    let (row, already_reserved) = crate::send::reserve_outgoing(
        session,
        store,
        client_message_id,
        chat_id,
        fingerprint,
        row,
    )?;
    if already_reserved {
        return Ok(row);
    }
    session
        .client
        .send(whatsapp_rust::SendRequest::new(&jid, message).with_options(
            whatsapp_rust::SendOptions::default().with_message_id(
                whatsapp_rust::MessageId::new(row.id.as_str()).map_err(|error| {
                    ("internal".into(), format!("bad message id: {error}"), false)
                })?,
            ),
        ))
        .await
        .map_err(|error| {
            let _ = store.set_message_status(
                chat_id,
                &row.id,
                whatfast_protocol::MessageStatus::Failed as i64,
            );
            ("send_failed".into(), format!("{error}"), true)
        })?;
    // The upload and the stanza are done; a failure to write the status back
    // is not a failed send. Returning an error here made the desktop offer
    // the file again, and the phone got it twice.
    if let Err(error) = store.set_message_status(
        chat_id,
        &row.id,
        whatfast_protocol::MessageStatus::Sent as i64,
    ) {
        tracing::error!(chat = %chat_id, id = %row.id, %error, "the sent status could not be stored");
    }
    let mut row = row;
    row.status = whatfast_protocol::MessageStatus::Sent as i64;
    if let Err(error) = refresh_chat_pointer(store, chat_id) {
        tracing::error!(chat = %chat_id, %error, "the chat pointer could not be refreshed");
    }
    Ok(row)
}

/// The mime of a media blob by its first bytes; unknown falls back to a
/// generic stream so the upload still happens.
pub(crate) fn sniff_mime(bytes: &[u8]) -> String {
    match bytes.first_chunk::<12>() {
        Some([0x89, b'P', b'N', b'G', ..]) => "image/png",
        Some([0xFF, 0xD8, 0xFF, ..]) => "image/jpeg",
        Some([b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P']) => "image/webp",
        Some([b'G', b'I', b'F', b'8', ..]) => "image/gif",
        _ => "application/octet-stream",
    }
    .to_owned()
}

/// The mime an outgoing image must have: the Go refused anything outside the
/// four formats WhatsApp renders (`supportedImageMIME`) and refused blobs over
/// 32 MiB before uploading them.
fn outgoing_image_mime(bytes: &[u8]) -> Result<String, (String, String, bool)> {
    if bytes.is_empty() || bytes.len() > MAX_OUTGOING_IMAGE_BYTES {
        return Err((
            "invalid_argument".into(),
            format!(
                "image must be a regular file between 1 byte and {} MiB",
                MAX_OUTGOING_IMAGE_BYTES / (1024 * 1024)
            ),
            false,
        ));
    }
    let mime = sniff_mime(bytes);
    match mime.as_str() {
        "image/jpeg" | "image/png" | "image/gif" | "image/webp" => Ok(mime),
        _ => Err((
            "invalid_argument".into(),
            format!("unsupported image type {mime}"),
            false,
        )),
    }
}

/// The decode bounds `safeImageConfig` enforced: an edge over 8192 or more
/// than 16 megapixels is refused before the upload.
fn check_image_bounds(width: u32, height: u32) -> Result<(), (String, String, bool)> {
    if width == 0
        || height == 0
        || width > MAX_IMAGE_EDGE
        || height > MAX_IMAGE_EDGE
        || u64::from(width) * u64::from(height) > MAX_IMAGE_PIXELS
    {
        return Err((
            "invalid_argument".into(),
            format!("image dimensions {width}x{height} exceed the safe limit"),
            false,
        ));
    }
    Ok(())
}

/// Whether a blob is an Ogg container that carries an Opus stream, the only
/// payload WhatsApp accepts as a voice note (`openAttachmentSource`).
fn is_ogg_opus(bytes: &[u8]) -> bool {
    bytes.starts_with(b"OggS")
        && bytes
            .windows(8)
            .take(64)
            .any(|window| window == b"OpusHead")
}

/// What an outgoing file says about itself, read from its own bytes.
///
/// port: `outgoingMediaMetadata` of the oracle. Every value comes from
/// parsing the file: none of them is guessed.
#[derive(Clone, Copy, Debug, Default)]
struct OutgoingMediaMetadata {
    duration_seconds: u32,
    width: u32,
    height: u32,
    animated: bool,
}

/// The mime of an outgoing attachment, by its first bytes.
///
/// port: `detectAttachmentMIME`. A `.wav` is `audio/wav`, not `audio/ogg`,
/// because the receiving side decrypts and decodes the blob with the mime the
/// message declares. A generic stream falls back to the extension, and a
/// document keeps its real type so WhatsApp shows the right icon.
fn detect_attachment_mime(header: &[u8], path: &str, kind: &str) -> String {
    let mut detected = match header.first_chunk::<12>() {
        Some([0x89, b'P', b'N', b'G', ..]) => "image/png",
        Some([0xFF, 0xD8, 0xFF, ..]) => "image/jpeg",
        Some([b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P']) => "image/webp",
        Some([b'G', b'I', b'F', b'8', ..]) => "image/gif",
        _ => "",
    }
    .to_owned();
    if detected.is_empty() {
        if header.starts_with(b"OggS") {
            detected = "audio/ogg".into();
        } else if header.len() >= 12 && &header[4..8] == b"ftyp" {
            detected = if kind == "audio" {
                "audio/mp4"
            } else {
                "video/mp4"
            }
            .into();
        } else if header.len() >= 4 && header[..4] == [0x1a, 0x45, 0xdf, 0xa3] {
            detected = if kind == "audio" {
                "audio/webm"
            } else {
                "video/webm"
            }
            .into();
        } else if header.len() >= 12 && &header[..4] == b"RIFF" && &header[8..12] == b"WAVE" {
            detected = "audio/wav".into();
        } else if header.len() >= 12 && &header[..4] == b"RIFF" && &header[8..12] == b"AVI " {
            detected = "video/x-msvideo".into();
        } else if header.starts_with(b"ID3") || header.starts_with(&[0xFF, 0xFB]) {
            detected = "audio/mpeg".into();
        } else if header.starts_with(b"%PDF") {
            detected = "application/pdf".into();
        } else if header.starts_with(b"PK\x03\x04") {
            detected = "application/zip".into();
        }
    }
    if detected.is_empty() || detected == "application/octet-stream" {
        detected = mime_by_extension(path);
    }
    if detected.is_empty() {
        "application/octet-stream".into()
    } else {
        detected
    }
}

/// The mime a common extension names, used when the bytes are generic.
///
/// port: the `mime.TypeByExtension` fallback of `detectAttachmentMIME`.
fn mime_by_extension(path: &str) -> String {
    let extension = std::path::Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase())
        .unwrap_or_default();
    let mime = match extension.as_str() {
        "pdf" => "application/pdf",
        "txt" | "log" => "text/plain",
        "html" | "htm" => "text/html",
        "csv" => "text/csv",
        "json" => "application/json",
        "xml" => "application/xml",
        "zip" => "application/zip",
        "rar" => "application/vnd.rar",
        "7z" => "application/x-7z-compressed",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ppt" => "application/vnd.ms-powerpoint",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "apk" => "application/vnd.android.package-archive",
        "wav" => "audio/wav",
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "aac" => "audio/aac",
        "ogg" | "oga" | "opus" => "audio/ogg",
        "flac" => "audio/flac",
        "mp4" => "video/mp4",
        "mkv" => "video/x-matroska",
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        "avi" => "video/x-msvideo",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        _ => "",
    };
    mime.to_owned()
}

/// Reads what an outgoing audio or video says about itself.
///
/// port: `probeOutgoingMedia`. The duration of a voice note and the size of a
/// video are what the receiving client uses to draw the bubble; a message
/// without them is accepted by the server and never lands well on the phone.
fn probe_outgoing_media(path: &str) -> OutgoingMediaMetadata {
    let mut metadata = OutgoingMediaMetadata::default();
    let Ok(mut file) = std::fs::File::open(path) else {
        return metadata;
    };
    let mut header = vec![0_u8; 64 * 1024];
    let read = std::io::Read::read(&mut file, &mut header).unwrap_or(0);
    header.truncate(read);
    let header = header.as_slice();
    if header.len() >= 12 && &header[..4] == b"RIFF" && &header[8..12] == b"WAVE" {
        metadata.duration_seconds = probe_wav(header);
    } else if header.starts_with(b"OggS") {
        metadata.duration_seconds = probe_ogg(path);
    } else if header.len() >= 8 && &header[4..8] == b"ftyp" {
        metadata = probe_iso_bmff(path);
    } else if header.starts_with(b"ID3") || header.starts_with(&[0xFF, 0xFB]) {
        metadata.duration_seconds = probe_mp3(path);
    } else if header.len() >= 10 && (&header[..6] == b"GIF87a" || &header[..6] == b"GIF89a") {
        let width = u16::from_le_bytes([header[6], header[7]]) as u32;
        let height = u16::from_le_bytes([header[8], header[9]]) as u32;
        metadata.width = width;
        metadata.height = height;
        metadata.animated = std::fs::metadata(path)
            .map(|meta| meta.len() <= 64 * 1024 * 1024)
            .unwrap_or(false)
            && std::fs::read(path)
                .map(|bytes| gif_frame_count(&bytes) > 1)
                .unwrap_or(false);
    } else if header.len() >= 12 && &header[..4] == b"RIFF" && &header[8..12] == b"WEBP" {
        let (width, height) = webp_dimensions(header);
        metadata.width = width;
        metadata.height = height;
        metadata.animated = header
            .windows(4)
            .any(|window| window == b"ANIM" || window == b"ANMF");
    }
    if metadata.duration_seconds == 0 && metadata.width == 0 && metadata.height == 0 {
        let (width, height, seconds) = probe_with_ffprobe(path);
        metadata.width = width;
        metadata.height = height;
        metadata.duration_seconds = seconds;
    }
    metadata
}

/// The duration of a RIFF WAVE file, from its `fmt ` byte rate and its
/// `data` size.
///
/// port: `probeWAV`.
fn probe_wav(header: &[u8]) -> u32 {
    if header.len() < 12 {
        return 0;
    }
    let mut byte_rate = 0_u32;
    let mut data_size = 0_u32;
    let mut offset = 12_usize;
    while offset + 8 <= header.len() {
        let kind = &header[offset..offset + 4];
        let size = u32::from_le_bytes([
            header[offset + 4],
            header[offset + 5],
            header[offset + 6],
            header[offset + 7],
        ]) as usize;
        let payload = offset + 8;
        match kind {
            b"fmt " if payload + 12 <= header.len() => {
                byte_rate = u32::from_le_bytes([
                    header[payload + 8],
                    header[payload + 9],
                    header[payload + 10],
                    header[payload + 11],
                ]);
            }
            b"data" => data_size = size as u32,
            _ => {}
        }
        if size > header.len().saturating_sub(payload) {
            break;
        }
        offset = payload + size + size % 2;
    }
    data_size.checked_div(byte_rate).unwrap_or(0)
}

/// The duration of an Ogg stream, from the granule position of its last page.
///
/// port: `probeOgg`. Opus granules always count samples at 48 kHz.
fn probe_ogg(path: &str) -> u32 {
    use std::io::Read as _;
    let Ok(file) = std::fs::File::open(path) else {
        return 0;
    };
    let mut reader = std::io::BufReader::new(file);
    let mut sample_rate = 0_u32;
    let mut last_granule = -1_i64;
    loop {
        let mut page = [0_u8; 27];
        if reader.read_exact(&mut page).is_err() {
            break;
        }
        if &page[..4] != b"OggS" {
            break;
        }
        let granule = i64::from_le_bytes([
            page[6], page[7], page[8], page[9], page[10], page[11], page[12], page[13],
        ]);
        let segments = page[26] as usize;
        let mut table = vec![0_u8; segments];
        if reader.read_exact(&mut table).is_err() {
            break;
        }
        let payload_size: usize = table.iter().map(|size| *size as usize).sum();
        if sample_rate == 0 {
            let mut payload = vec![0_u8; payload_size];
            if reader.read_exact(&mut payload).is_err() {
                break;
            }
            sample_rate = ogg_sample_rate(&payload);
        } else if std::io::copy(&mut (&mut reader).take(payload_size as u64), &mut std::io::sink())
            .is_err()
        {
            break;
        }
        if granule >= 0 {
            last_granule = granule;
        }
    }
    if sample_rate == 0 || last_granule <= 0 {
        return 0;
    }
    ((last_granule as u64 + sample_rate as u64 / 2) / sample_rate as u64) as u32
}

/// The sample rate of the codec identification header of an Ogg stream.
///
/// port: `oggSampleRate`.
fn ogg_sample_rate(payload: &[u8]) -> u32 {
    if payload.windows(8).any(|window| window == b"OpusHead") {
        return 48_000;
    }
    if let Some(index) = payload.windows(6).position(|window| window == b"vorbis")
        && payload.len() >= index + 16
    {
        return u32::from_le_bytes([
            payload[index + 12],
            payload[index + 13],
            payload[index + 14],
            payload[index + 15],
        ]);
    }
    0
}

/// The duration and the size of an ISO base media file (`mp4`, `m4a`, `mov`),
/// from its `mvhd` and its first video track.
///
/// port: `probeISOBMPF` and `readISOMovie`.
fn probe_iso_bmff(path: &str) -> OutgoingMediaMetadata {
    // Un video de WhatsApp no pasa de unos pocos megabytes. La lectura del
    // archivo entero se corta para no cargar un adjunto de 2 GiB en memoria:
    // el que supera el tope queda con la metadata de `ffprobe`.
    if std::fs::metadata(path)
        .map(|meta| meta.len() > 256 * 1024 * 1024)
        .unwrap_or(true)
    {
        return OutgoingMediaMetadata::default();
    }
    let Ok(bytes) = std::fs::read(path) else {
        return OutgoingMediaMetadata::default();
    };
    let mut metadata = OutgoingMediaMetadata::default();
    walk_iso_boxes(&bytes, &mut metadata);
    metadata
}

fn walk_iso_boxes(bytes: &[u8], metadata: &mut OutgoingMediaMetadata) {
    let mut offset = 0_usize;
    while offset + 8 <= bytes.len() {
        let size = u32::from_be_bytes([
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
        ]) as usize;
        let kind = &bytes[offset + 4..offset + 8];
        let header = if size == 1 {
            16
        } else if size == 0 {
            bytes.len() - offset
        } else {
            8
        };
        if size < header || offset + header > bytes.len() {
            break;
        }
        let body_start = offset + header;
        let body_end = if size == 0 {
            bytes.len()
        } else {
            (offset + size).min(bytes.len())
        };
        let body = &bytes[body_start..body_end];
        match kind {
            b"moov" => walk_iso_boxes(body, metadata),
            b"mvhd" => {
                if let Some(duration) = iso_mvhd_duration(body)
                    && duration > 0
                {
                    metadata.duration_seconds = duration;
                }
            }
            b"trak" => {
                let (width, height) = iso_track_size(body);
                if width > 0 {
                    metadata.width = width;
                }
                if height > 0 {
                    metadata.height = height;
                }
                walk_iso_boxes(body, metadata);
            }
            _ => {}
        }
        if size == 0 {
            break;
        }
        offset += size;
    }
}

fn iso_mvhd_duration(payload: &[u8]) -> Option<u32> {
    let (timescale, duration) = if payload.first() == Some(&1) {
        if payload.len() < 32 {
            return None;
        }
        let timescale = u32::from_be_bytes([
            payload[20], payload[21], payload[22], payload[23],
        ]);
        let duration = u64::from_be_bytes([
            payload[24], payload[25], payload[26], payload[27], payload[28], payload[29],
            payload[30], payload[31],
        ]);
        (timescale, duration)
    } else {
        if payload.len() < 20 {
            return None;
        }
        let timescale = u32::from_be_bytes([
            payload[12], payload[13], payload[14], payload[15],
        ]);
        let duration = u32::from_be_bytes([
            payload[16], payload[17], payload[18], payload[19],
        ]) as u64;
        (timescale, duration)
    };
    if timescale == 0 {
        return None;
    }
    Some(((duration + timescale as u64 / 2) / timescale as u64) as u32)
}

fn iso_track_size(trak: &[u8]) -> (u32, u32) {
    let mut width = 0_u32;
    let mut height = 0_u32;
    let mut offset = 0_usize;
    while offset + 8 <= trak.len() {
        let size = u32::from_be_bytes([
            trak[offset],
            trak[offset + 1],
            trak[offset + 2],
            trak[offset + 3],
        ]) as usize;
        let kind = &trak[offset + 4..offset + 8];
        if size < 8 || offset + size > trak.len() {
            break;
        }
        let body = &trak[offset + 8..offset + size];
        if kind == b"tkhd" && body.len() >= 84 {
            // The track header carries the display size in its last eight
            // bytes as 16.16 fixed point numbers.
            let width_fixed = u32::from_be_bytes([
                body[body.len() - 8],
                body[body.len() - 7],
                body[body.len() - 6],
                body[body.len() - 5],
            ]);
            let height_fixed = u32::from_be_bytes([
                body[body.len() - 4],
                body[body.len() - 3],
                body[body.len() - 2],
                body[body.len() - 1],
            ]);
            width = width_fixed >> 16;
            height = height_fixed >> 16;
        }
        offset += size;
    }
    (width, height)
}

/// The duration of an MPEG audio file, from its frame headers.
///
/// port: the `audio/mpeg` branch of the oracle: a constant bit rate file is
/// walked frame by frame, and the size of the first frame gives the bit rate.
fn probe_mp3(path: &str) -> u32 {
    let Ok(bytes) = std::fs::read(path) else {
        return 0;
    };
    let mut offset = 0_usize;
    if bytes.starts_with(b"ID3") && bytes.len() >= 10 {
        let size = ((bytes[6] as usize & 0x7f) << 21)
            | ((bytes[7] as usize & 0x7f) << 14)
            | ((bytes[8] as usize & 0x7f) << 7)
            | (bytes[9] as usize & 0x7f);
        offset = 10 + size;
    }
    while offset + 4 <= bytes.len() {
        if bytes[offset] == 0xFF && bytes[offset + 1] & 0xE0 == 0xE0 {
            break;
        }
        offset += 1;
    }
    if offset + 4 > bytes.len() {
        return 0;
    }
    let header = u32::from_be_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ]);
    let bitrate_index = ((header >> 12) & 0xF) as usize;
    let sample_rate_index = ((header >> 10) & 0x3) as usize;
    let padding = ((header >> 9) & 0x1) as u64;
    // MPEG 1 layer III bit rates and sample rates.
    let bitrates = [
        0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 0,
    ];
    let sample_rates = [44_100_u64, 48_000, 32_000, 0];
    let bitrate = bitrates.get(bitrate_index).copied().unwrap_or(0) as u64;
    let sample_rate = sample_rates.get(sample_rate_index).copied().unwrap_or(0);
    if bitrate == 0 || sample_rate == 0 {
        return 0;
    }
    let frame_size = 144 * bitrate * 1000 / sample_rate + padding;
    if frame_size == 0 {
        return 0;
    }
    let audio_bytes = bytes.len().saturating_sub(offset) as u64;
    let frames = audio_bytes / frame_size;
    let samples = frames * 1152;
    (samples / sample_rate) as u32
}

/// Counts the image descriptors of a GIF without decoding a frame.
///
/// port: `gifFrameCount`.
fn gif_frame_count(bytes: &[u8]) -> usize {
    let mut frames = 0_usize;
    let mut index = 13_usize;
    if let Some(flags) = bytes.get(10)
        && flags & 0x80 != 0
    {
        index += 3 * (1 << ((flags & 0x07) + 1));
    }
    while index < bytes.len() {
        match bytes[index] {
            0x2C => {
                frames += 1;
                if index + 10 > bytes.len() {
                    break;
                }
                let flags = bytes[index + 9];
                index += 10;
                if flags & 0x80 != 0 {
                    index += 3 * (1 << ((flags & 0x07) + 1));
                }
                index += 1;
                while index < bytes.len() {
                    let size = bytes[index] as usize;
                    index += 1;
                    if size == 0 {
                        break;
                    }
                    index += size;
                }
            }
            0x21 => {
                index += 2;
                while index < bytes.len() {
                    let size = bytes[index] as usize;
                    index += 1;
                    if size == 0 {
                        break;
                    }
                    index += size;
                }
            }
            0x3B => break,
            _ => index += 1,
        }
    }
    frames
}

/// Whether the upload answered every piece of metadata the message needs.
///
/// port: `validUploadResponse`. A short answer would carry a wrong length or
/// an empty key, and the receiving client would never decrypt the blob.
fn valid_upload_response(
    response: &whatsapp_rust::upload::UploadResponse,
    expected_size: u64,
) -> bool {
    response.file_length == expected_size
        && !response.url.is_empty()
        && !response.direct_path.is_empty()
        && response.media_key.len() == 32
        && response.file_sha256.len() == 32
        && response.file_enc_sha256.len() == 32
}

/// The pixel size of a PNG, JPEG, GIF or WebP; unknown or truncated formats
/// answer zero and the caller refuses the upload.
pub(crate) fn sniff_dimensions(bytes: &[u8], mime: &str) -> (u32, u32) {
    match mime {
        "image/png" => {
            // IHDR: 8-byte signature, 4 length, 4 "IHDR", then width/height.
            if let Some(ihdr) = bytes.get(16..24) {
                let width = u32::from_be_bytes([ihdr[0], ihdr[1], ihdr[2], ihdr[3]]);
                let height = u32::from_be_bytes([ihdr[4], ihdr[5], ihdr[6], ihdr[7]]);
                return (width, height);
            }
            (0, 0)
        }
        "image/gif" => {
            // The logical screen descriptor: "GIF8" + 2 version + 2 size.
            if bytes.len() >= 10 {
                let width = u16::from_le_bytes([bytes[6], bytes[7]]) as u32;
                let height = u16::from_le_bytes([bytes[8], bytes[9]]) as u32;
                return (width, height);
            }
            (0, 0)
        }
        "image/webp" => webp_dimensions(bytes),
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

/// The canvas size of a WebP file, read from whichever bitstream chunk it
/// carries: VP8X stores the canvas, VP8 the lossy frame header and VP8L the
/// packed lossless size.
fn webp_dimensions(bytes: &[u8]) -> (u32, u32) {
    let Some(header) = bytes.get(..16) else {
        return (0, 0);
    };
    if &header[8..12] != b"WEBP" {
        return (0, 0);
    }
    match &header[12..16] {
        b"VP8X" => {
            let Some(header) = bytes.get(..30) else {
                return (0, 0);
            };
            // The canvas is stored minus one in three bytes each; the +1 has
            // to apply after the three bytes are joined.
            let width = 1
                + (u32::from(header[24])
                    | (u32::from(header[25]) << 8)
                    | (u32::from(header[26]) << 16));
            let height = 1
                + (u32::from(header[27])
                    | (u32::from(header[28]) << 8)
                    | (u32::from(header[29]) << 16));
            (width, height)
        }
        b"VP8 " => {
            let Some(header) = bytes.get(..30) else {
                return (0, 0);
            };
            if header[23] != 0x9d || header[24] != 0x01 || header[25] != 0x2a {
                return (0, 0);
            }
            let width = u16::from_le_bytes([header[26], header[27]]) & 0x3fff;
            let height = u16::from_le_bytes([header[28], header[29]]) & 0x3fff;
            (u32::from(width), u32::from(height))
        }
        b"VP8L" => {
            let Some(header) = bytes.get(..25) else {
                return (0, 0);
            };
            if header[20] != 0x2f {
                return (0, 0);
            }
            let bits = u32::from_le_bytes([header[21], header[22], header[23], header[24]]);
            (1 + (bits & 0x3fff), 1 + ((bits >> 14) & 0x3fff))
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
    if caption.len() > 4096 {
        return Err((
            "invalid_argument".into(),
            "caption must be valid UTF-8 up to 4096 bytes".into(),
            false,
        ));
    }
    if kind == whatfast_protocol::AttachmentKind::Audio && !caption.is_empty() {
        return Err((
            "invalid_argument".into(),
            "audio messages do not support captions".into(),
            false,
        ));
    }
    if voice_note && kind != whatfast_protocol::AttachmentKind::Audio {
        return Err((
            "invalid_argument".into(),
            "voice notes must be audio attachments".into(),
            false,
        ));
    }
    // port: `openAttachmentSource`. The bridge of the oracle refuses a
    // relative path because the sidecar may run from another directory, and
    // a name the receiving phone cannot render.
    if !std::path::Path::new(file_path).is_absolute() {
        return Err((
            "invalid_argument".into(),
            "attachment path must be absolute".into(),
            false,
        ));
    }
    let path = std::path::Path::new(file_path);
    let Some(file_name) = path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty() && name.len() <= 255)
    else {
        return Err((
            "invalid_argument".into(),
            "attachment file name is invalid".into(),
            false,
        ));
    };
    let file_name = file_name.to_owned();
    let size = upload_size(file_path).await?;
    if size == 0 {
        return Err((
            "invalid_argument".into(),
            "attachment must be a regular file between 1 byte and 2 GiB".into(),
            false,
        ));
    }
    if size > MAX_MEDIA_BYTES {
        return Err((
            "failed_precondition".into(),
            "attachment exceeds the 2 GiB upload limit".into(),
            false,
        ));
    }
    let bytes = read_upload(file_path, "attachment").await?;
    let (media_type, wire_kind) = match kind {
        whatfast_protocol::AttachmentKind::Video => (MediaType::Video, "video"),
        whatfast_protocol::AttachmentKind::Audio => (MediaType::Audio, "audio"),
        _ => (MediaType::Document, "document"),
    };
    let mut mime = detect_attachment_mime(&bytes[..bytes.len().min(512)], file_path, wire_kind);
    if wire_kind == "video" && !mime.starts_with("video/") {
        return Err((
            "invalid_argument".into(),
            format!("video attachment has content type {mime}"),
            false,
        ));
    }
    if wire_kind == "audio" {
        if !mime.starts_with("audio/") {
            return Err((
                "invalid_argument".into(),
                format!("audio attachment has content type {mime}"),
                false,
            ));
        }
        if voice_note {
            if !is_ogg_opus(&bytes) {
                return Err((
                    "invalid_argument".into(),
                    "voice notes must contain Ogg Opus data".into(),
                    false,
                ));
            }
            // port: the oracle pins the codec in the mime of a voice note;
            // without it the phone leaves the note unplayed.
            mime = "audio/ogg; codecs=opus".into();
        }
    }
    // The duration and the display size come from the file itself: a voice
    // note without `seconds` and a video without its size land on the phone
    // as an empty bubble.
    let metadata = probe_outgoing_media(file_path);
    let fingerprint = format!(
        "{wire_kind}\u{0}{caption}\u{0}{file_name}\u{0}{}",
        hash_hex(&bytes)
    );
    if let Some(row) =
        crate::send::stored_outgoing(store, client_message_id, chat_id, &fingerprint)?
    {
        return Ok(row);
    }
    let context = reply_context(store, chat_id, reply_to_chat_id, reply_to_message_id)?;
    let response = session
        .client
        .upload(bytes, media_type, UploadOptions::new())
        .await
        .map_err(|error| ("upload_failed".into(), format!("{error}"), true))?;
    if !valid_upload_response(&response, size) {
        return Err((
            "upload_failed".into(),
            "WhatsApp returned incomplete attachment upload metadata".into(),
            true,
        ));
    }

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
            video.width = Some(metadata.width);
            video.height = Some(metadata.height);
            video.seconds = Some(metadata.duration_seconds);
            video.gif_playback = Some(metadata.animated);
            if let Some(context) = context.clone() {
                video.context_info = whatsapp_rust::waproto::buffa::MessageField::some(context);
            }
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
            // El campo va presente aunque valga cero: el telefono dibuja la
            // nota con el, y ausente la deja sin reproducir.
            audio.seconds = Some(metadata.duration_seconds);
            audio.ptt = Some(voice_note);
            if let Some(context) = context.clone() {
                audio.context_info = whatsapp_rust::waproto::buffa::MessageField::some(context);
            }
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
            if let Some(context) = context {
                document.context_info = whatsapp_rust::waproto::buffa::MessageField::some(context);
            }
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
            mime: mime.clone(),
            file_name,
            local_path: file_path.to_owned(),
            direct_path: response.direct_path.clone(),
            media_key: response.media_key.to_vec(),
            file_sha256: response.file_sha256.to_vec(),
            file_enc_sha256: response.file_enc_sha256.to_vec(),
            size: response.file_length,
            width: metadata.width,
            height: metadata.height,
            duration: metadata.duration_seconds,
            voice: voice_note && wire_kind == "audio",
            animated: metadata.animated,
        },
        reply_to_message_id,
        reply_to_chat_id,
        &fingerprint,
    )
    .await
    .inspect(|row| {
        tracing::info!(
            "attachment {} sent as {}: mime={mime} size={size} seconds={} {}x{}",
            row.id,
            wire_kind,
            metadata.duration_seconds,
            metadata.width,
            metadata.height
        );
    })
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
    let fingerprint = format!("sticker\u{0}{}", hash_hex(webp_data));
    if let Some(row) =
        crate::send::stored_outgoing(store, client_message_id, chat_id, &fingerprint)?
    {
        return Ok(row);
    }
    let context = reply_context(store, chat_id, reply_to_chat_id, reply_to_message_id)?;
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
    if let Some(context) = context {
        sticker.context_info = whatsapp_rust::waproto::buffa::MessageField::some(context);
    }
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
        &fingerprint,
    )
    .await
}

/// The WebP dimensions and animation flag of a sticker. The VP8X chunk also
/// carries the ALPH/ANIM flag; a plain VP8/VP8L file is a still image.
fn sniff_webp(bytes: &[u8]) -> (u32, u32, bool) {
    let (width, height) = webp_dimensions(bytes);
    let animated = bytes
        .first_chunk::<21>()
        .is_some_and(|header| &header[12..16] == b"VP8X" && header[20] & 0x02 != 0);
    (width, height, animated)
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

/// Reads the duration and the size of a media file with `ffprobe`, when the
/// system has it.
///
/// The parsers of [`probe_outgoing_media`] cover the formats WhatsApp sends
/// without external tools. This is the fallback for a container they do not
/// read (a Matroska video, for instance): an absent `ffprobe` answers nothing
/// and the message goes out with the metadata the file did carry.
fn probe_with_ffprobe(file_path: &str) -> (u32, u32, u32) {
    let output = std::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=width,height,duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
            file_path,
        ])
        .output();
    let Ok(output) = output else {
        return (0, 0, 0);
    };
    if !output.status.success() {
        return (0, 0, 0);
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    let width = lines.first().and_then(|w| w.parse::<u32>().ok()).unwrap_or(0);
    let height = lines.get(1).and_then(|h| h.parse::<u32>().ok()).unwrap_or(0);
    let seconds = lines
        .get(2)
        .and_then(|d| d.parse::<f64>().ok())
        .map(|s| s.round() as u32)
        .unwrap_or(0);
    (width, height, seconds)
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
            // This port writes an attachment's descriptor to the media_*
            // columns; a row that carries it in image_* instead (the Go
            // schema stored every media kind there) still downloads.
            let descriptor_in_image_columns = row.media_direct_path.is_empty()
                && row.media_media_key.is_empty()
                && !row.image_direct_path.is_empty();
            if descriptor_in_image_columns {
                Self {
                    mime: if row.image_mime.is_empty() {
                        row.media_mime.clone()
                    } else {
                        row.image_mime.clone()
                    },
                    file_name: row.media_file_name.clone(),
                    local_path: if row.media_local_path.is_empty() {
                        row.image_local_path.clone()
                    } else {
                        row.media_local_path.clone()
                    },
                    direct_path: row.image_direct_path.clone(),
                    media_key: row.image_media_key.clone(),
                    file_sha256: row.image_file_sha256.clone(),
                    file_enc_sha256: row.image_file_enc_sha256.clone(),
                    file_size: row.image_size.max(0) as u64,
                    media_type,
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

/// A file writer that records when it last received bytes.
///
/// The download thread writes through it, so the wait in
/// [`fetch_message_media`] can tell a slow transfer from a dead one.
struct WatchedFile {
    file: std::fs::File,
    last_write_ms: Arc<AtomicI64>,
}

impl std::io::Write for WatchedFile {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        let written = self.file.write(buffer)?;
        if written > 0 {
            self.last_write_ms.store(db::now_ms(), Ordering::Relaxed);
        }
        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}

impl std::io::Seek for WatchedFile {
    fn seek(&mut self, position: std::io::SeekFrom) -> std::io::Result<u64> {
        self.file.seek(position)
    }
}

impl DownloadWriter for WatchedFile {
    fn truncate(&mut self, len: u64) -> std::io::Result<()> {
        self.file.set_len(len)
    }
}

/// The media descriptor of one row.
///
/// An older build saved a photo, a video or an audio with its kind and an
/// empty descriptor: the envelope with the URL and the key stayed inside
/// `payload`, so the download could never start. The row is decoded once
/// here and written back, so the file downloads and the bubble keeps the
/// size and the embedded preview.
fn media_source(store: &Arc<Store>, row: &MessageRow) -> Option<MediaSource> {
    let source = MediaSource::of(row);
    if source
        .as_ref()
        .is_some_and(|source| !source.direct_path.is_empty())
    {
        return source;
    }
    let mut hydrated = row.clone();
    if !crate::ingest::hydrate_from_payload(&mut hydrated) {
        return source;
    }
    let _ = store.insert_message(&hydrated);
    MediaSource::of(&hydrated).or(source)
}

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
    let source = media_source(store, &row).ok_or_else(|| {
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
    let last_write = Arc::new(std::sync::atomic::AtomicI64::new(db::now_ms()));
    let mut download = Box::pin(async {
        let file = std::fs::File::create(&temporary).map_err(|error| {
            (
                "internal".into(),
                format!("creating the temp file: {error}"),
                false,
            )
        })?;
        // The writer travels by value and comes back inside the result. It
        // also says when the last byte arrived, so a silent connection stops
        // the wait instead of holding the loop.
        let writer = WatchedFile {
            file,
            last_write_ms: last_write.clone(),
        };
        let writer = session
            .client
            .download_to_writer(&params, writer, DownloadLimits::default())
            .await
            .map_err(|error| {
                (
                    "unavailable".into(),
                    format!("media download: {error}"),
                    true,
                )
            })?;
        let file = writer.file;
        // Flushing a large file waits on the disk: it runs off the async
        // thread, or the bridge stops answering until the write lands.
        tokio::task::spawn_blocking(move || file.sync_all())
            .await
            .map_err(|error| {
                (
                    "internal".to_string(),
                    format!("flushing the download was interrupted: {error}"),
                    true,
                )
            })?
            .map_err(|error| {
                (
                    "internal".into(),
                    format!("flushing the download: {error}"),
                    false,
                )
            })?;
        Ok(())
    });
    // A transfer that stops receiving bytes used to hold the request loop
    // until the HTTPS stack gave up: the log kept a `GetMessageAttachment`
    // of ten minutes, and every other answer waited behind it. The poll
    // ends the wait at the first long silence, and the next open retries.
    let finished = loop {
        tokio::select! {
            done = &mut download => break Some(done),
            _ = tokio::time::sleep(DOWNLOAD_PROGRESS_POLL) => {
                let idle = db::now_ms().saturating_sub(last_write.load(Ordering::Relaxed));
                if idle >= DOWNLOAD_STALL_MS {
                    break None;
                }
            }
        }
    };
    let Some(finished) = finished else {
        let _ = std::fs::remove_file(&temporary);
        return Err((
            "unavailable".into(),
            format!(
                "the download received no data for {} seconds",
                DOWNLOAD_STALL_MS / 1000
            ),
            true,
        ));
    };
    if let Err(error) = finished {
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

struct FavoriteDownloadable<'a> {
    direct_path: &'a str,
    media_key: &'a [u8],
    file_enc_sha256: &'a [u8],
    file_length: u64,
    media_type: MediaType,
}

impl<'a> Downloadable for FavoriteDownloadable<'a> {
    fn direct_path(&self) -> Option<&str> {
        Some(self.direct_path)
    }
    fn media_key(&self) -> Option<&[u8]> {
        Some(self.media_key)
    }
    fn file_enc_sha256(&self) -> Option<&[u8]> {
        Some(self.file_enc_sha256)
    }
    fn file_sha256(&self) -> Option<&[u8]> {
        None
    }
    fn file_length(&self) -> Option<u64> {
        if self.file_length > 0 {
            Some(self.file_length)
        } else {
            None
        }
    }
    fn app_info(&self) -> MediaType {
        self.media_type
    }
}

/// Downloads one favourite sticker into the cache and returns its file path.
pub(crate) async fn fetch_favorite_sticker(
    session: &Session,
    store: &Store,
    media_dir: &Path,
    id: &str,
) -> Result<String, (String, String, bool)> {
    let fav = store
        .favorite_sticker(id)
        .map_err(internal)?
        .ok_or_else(|| ("not_found".into(), "favourite sticker is not stored".into(), false))?;

    let target = media_dir.join(format!("fav-{}.webp", fav.id));
    if let Ok(size) = std::fs::metadata(&target).map(|m| m.len())
        && size > 0
    {
        let path = target.to_string_lossy().into_owned();
        if fav.local_path != path {
            let _ = store.set_favorite_sticker_local_path(id, &path);
        }
        return Ok(path);
    }

    // An older download may have left the file under another name; the
    // library shows it from that path, so sending has to accept it too.
    if !fav.local_path.is_empty()
        && std::fs::metadata(&fav.local_path).is_ok_and(|metadata| metadata.len() > 0)
    {
        return Ok(fav.local_path.clone());
    }

    if fav.direct_path.is_empty() || fav.media_key.len() != 32 || fav.file_enc_sha256.len() != 32 {
        return Err((
            "not_found".into(),
            "favourite sticker descriptor is incomplete".into(),
            false,
        ));
    }

    std::fs::create_dir_all(media_dir).map_err(|error| {
        (
            "internal".into(),
            format!("creating the media cache: {error}"),
            false,
        )
    })?;

    let temporary = media_dir.join(format!(".fav-{}.tmp", fav.id));
    let file_length = fav.file_size.max(0) as u64;

    let download = async {
        for media_type in [MediaType::Sticker, MediaType::Image] {
            let file = std::fs::File::create(&temporary).map_err(|error| {
                (
                    "internal".into(),
                    format!("creating the temp file: {error}"),
                    false,
                )
            })?;
            let downloadable = FavoriteDownloadable {
                direct_path: &fav.direct_path,
                media_key: &fav.media_key,
                file_enc_sha256: &fav.file_enc_sha256,
                file_length,
                media_type,
            };
            if let Ok(file) = session
                .client
                .download_to_writer(&downloadable, file, DownloadLimits::default())
                .await
            {
                let _ = file.sync_all();
                return Ok(());
            }
        }
        Err((
            "unavailable".into(),
            "favourite sticker download failed".into(),
            true,
        ))
    };

    if let Err(error) = download.await {
        let _ = std::fs::remove_file(&temporary);
        return Err(error);
    }

    let _ = std::fs::remove_file(&target);
    std::fs::rename(&temporary, &target).map_err(|error| {
        (
            "internal".into(),
            format!("moving the favourite sticker into cache: {error}"),
            false,
        )
    })?;

    let path = target.to_string_lossy().into_owned();
    let _ = store.set_favorite_sticker_local_path(id, &path);
    Ok(path)
}

