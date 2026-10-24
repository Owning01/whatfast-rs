//! Metadata that answers over the network: group subjects backfilled once
//! per chat, and profile pictures cached as files.
//!
//! The rules mirror the Go sidecar:
//! - A group chat's name fills lazily (`BackfillGroupName`): the history
//!   sync and the live traffic carry no reliable subject, so the first
//!   presentation with an empty name asks the server once and updates.
//! - Avatars land in `<data-dir>/media/` under a deterministic name per jid,
//!   so the desktop's `GetChatAvatar` is a file-exists check after the first
//!   fetch. Privacy refusals (`NotAuthorized`) and "not set" cache an empty
//!   marker file, so a chat whose picture is private does not re-query on
//!   every list render.

use std::sync::Arc;

use wacore::net::HttpRequest;
use whatsapp_rust::Jid;
use whatsapp_rust::client::Client;
use whatsapp_rust::http::UreqHttpClient;

use crate::db::Store;
use crate::ingest;

/// How many unnamed groups one backfill pass fetches. The next `Connected`
/// event (every reconnect) picks up more.
const BACKFILL_BATCH: u32 = 40;

/// Fills the names of group chats that still show none, and announces each
/// change. One pass per connection; cheap groups skip the query once named.
pub fn spawn_group_backfill(
    client: Arc<Client>,
    store: Arc<Store>,
    events: async_channel::Sender<whatfast_protocol::BackendEvent>,
) {
    tokio::spawn(async move {
        let unnamed = store.unnamed_groups(BACKFILL_BATCH).unwrap_or_default();
        for jid in unnamed {
            let Ok(parsed) = jid.parse::<Jid>() else {
                continue;
            };
            match client.groups().fetch_metadata(&parsed).await {
                Ok(metadata) => {
                    let subject = metadata.subject.unwrap_or_default();
                    if subject.is_empty() {
                        continue;
                    }
                    if store.set_chat_name(&jid, &subject).is_ok()
                        && let Some(chat) = store.chat(&jid).unwrap_or(None)
                    {
                        let _ = events.send(ingest::chat_upserted(chat)).await;
                    }
                }
                Err(error) => {
                    tracing::debug!("group backfill for one chat failed: {error}");
                }
            }
            // A polite pace: one metadata query per tick keeps a fresh
            // account's first minutes calm.
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
    });
}

/// The deterministic avatar file path for one jid. `not-set` is the empty
/// marker: a known privacy refusal, never re-queried.
fn avatar_path(media_dir: &std::path::Path, jid: &str, variant: &str) -> std::path::PathBuf {
    media_dir.join(format!("avatar-{}-{variant}.jpg", hash_hex(jid.as_bytes())))
}

pub(crate) fn hash_hex(bytes: &[u8]) -> String {
    // FNV-1a: a cache key, not a security boundary.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// Answers the avatar path for one jid, fetching it once and caching the
/// answer (the file, or the empty marker). `group` picks the lookup route.
pub async fn ensure_avatar(
    client: &Client,
    media_dir: &std::path::Path,
    jid: &str,
    group: bool,
) -> Result<String, (String, String, bool)> {
    let _ = media_dir;
    let Ok(parsed) = jid.parse::<Jid>() else {
        return Err((
            "invalid_argument".into(),
            format!("not a WhatsApp JID: {jid}"),
            false,
        ));
    };
    let media_dir_owned = media_dir.to_path_buf();
    let found = client
        .pictures()
        .lookup(whatsapp_rust::ProfilePictureRequest::new(
            if group {
                whatsapp_rust::ProfilePictureTarget::Group(&parsed)
            } else {
                whatsapp_rust::ProfilePictureTarget::Contact(&parsed)
            },
            whatsapp_rust::ProfilePictureType::Preview,
        ))
        .await;
    let picture = match found {
        Ok(outcome) => outcome.into_found(),
        Err(error) => {
            // A refusal is an answer: cache the marker so the UI stops
            // asking, and answer with an empty path.
            let marker = avatar_path(&media_dir_owned, jid, "not-set");
            let _ = std::fs::write(&marker, b"");
            tracing::debug!("no picture for one jid: {error}");
            return Ok(String::new());
        }
    };
    let Some(picture) = picture else {
        let marker = avatar_path(&media_dir_owned, jid, "not-set");
        let _ = std::fs::write(&marker, b"");
        return Ok(String::new());
    };
    let target = avatar_path(&media_dir_owned, jid, &picture.id.clone());
    if target.exists() {
        return Ok(target.to_string_lossy().into_owned());
    }
    use wacore::net::HttpClient as _;
    let response = UreqHttpClient::new()
        .execute(HttpRequest::get(picture.url.clone()))
        .await
        .map_err(|error| ("fetch_failed".into(), format!("{error}"), true))?;
    if !(200..300).contains(&response.status_code) {
        return Err((
            "fetch_failed".into(),
            format!("the picture server answered {}", response.status_code),
            true,
        ));
    }
    std::fs::create_dir_all(&media_dir_owned)
        .map_err(|error| ("internal".into(), format!("{error}"), false))?;
    std::fs::write(&target, &response.body)
        .map_err(|error| ("internal".into(), format!("{error}"), false))?;
    Ok(target.to_string_lossy().into_owned())
}

/// Whether the avatar question was already answered for one jid (either a
/// file or the not-set marker). The dispatch uses it to skip network work
/// on every list render.
pub fn avatar_known(media_dir: &std::path::Path, jid: &str) -> bool {
    let prefix = format!("avatar-{}-", hash_hex(jid.as_bytes()));
    media_dir
        .read_dir()
        .map(|entries| {
            entries
                .filter_map(|entry| entry.ok())
                .any(|entry| entry.file_name().to_string_lossy().starts_with(&prefix))
        })
        .unwrap_or(false)
}

/// The cached avatar path when one exists, without touching the network:
/// the newest file for the jid wins, and the not-set marker answers empty.
pub fn cached_avatar_path(media_dir: &std::path::Path, jid: &str) -> String {
    let prefix = format!("avatar-{}-", hash_hex(jid.as_bytes()));
    let mut best: Option<std::path::PathBuf> = None;
    let mut best_time = std::time::SystemTime::UNIX_EPOCH;
    if let Ok(entries) = media_dir.read_dir() {
        for entry in entries.filter_map(|entry| entry.ok()) {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.starts_with(&prefix) {
                continue;
            }
            if name.ends_with("not-set.jpg") {
                return String::new();
            }
            let modified = entry
                .metadata()
                .and_then(|meta| meta.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            if modified >= best_time {
                best_time = modified;
                best = Some(entry.path());
            }
        }
    }
    best.map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_default()
}
