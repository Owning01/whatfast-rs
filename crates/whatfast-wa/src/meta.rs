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
//! - A transient failure (offline, timeout, server error) caches nothing: the
//!   next render retries, the way `fetchAvatar` only remembered the two
//!   privacy answers.
//! - The cache keeps itself under 128 MiB by dropping the oldest pictures
//!   (`pruneAvatarCache` did the same with a 128 MiB budget).

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

/// How long a permanent backfill refusal sticks: a group the account left
/// (or that no longer exists) answers the same error every time, so the
/// jid rests in the meta table instead of burning a query per reconnect.
const BACKFILL_SKIP_MS: i64 = 30 * 24 * 60 * 60 * 1000;

/// Fills the names of group chats that still show none, and announces each
/// change. One pass per connection; named groups skip the query, and dead
/// ones rest after one refusal instead of warning every launch.
pub fn spawn_group_backfill(
    client: Arc<Client>,
    store: Arc<Store>,
    events: async_channel::Sender<whatfast_protocol::BackendEvent>,
) {
    tokio::spawn(async move {
        let unnamed = store.unnamed_groups(BACKFILL_BATCH).unwrap_or_default();
        for jid in unnamed {
            if backfill_rests(&store, &jid) {
                continue;
            }
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
                    let message = format!("{error}");
                    // Leaving (or a deleted group) is permanent: rest the
                    // jid, transient failures keep retrying next pass.
                    if message.contains("not participating") || message.contains("does not exist") {
                        rest_backfill(&store, &jid);
                    }
                    tracing::debug!("group backfill for one chat failed: {error}");
                }
            }
            // A polite pace: one metadata query per tick keeps a fresh
            // account's first minutes calm.
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
    });
}

/// Whether this jid rests after a permanent backfill refusal.
fn backfill_rests(store: &Store, jid: &str) -> bool {
    let key = format!("group-backfill-skip:{jid}");
    store
        .meta_get(&key)
        .ok()
        .flatten()
        .and_then(|value| value.parse::<i64>().ok())
        .is_some_and(|at| crate::db::now_ms() - at < BACKFILL_SKIP_MS)
}

/// Rests one jid after a permanent backfill refusal.
fn rest_backfill(store: &Store, jid: &str) {
    let key = format!("group-backfill-skip:{jid}");
    let _ = store.meta_set(&key, &crate::db::now_ms().to_string());
}

/// What one avatar may weigh and how large its canvas may be, the bounds
/// `fetchAvatar` enforced before caching the bytes.
const MAX_AVATAR_BYTES: usize = 2 * 1024 * 1024;
const MAX_AVATAR_EDGE: u32 = 2048;
const MAX_AVATAR_PIXELS: u64 = 4_000_000;

/// The avatar cache budget (`pruneAvatarCache(128 * 1024 * 1024)`).
const AVATAR_CACHE_BYTES: u64 = 128 * 1024 * 1024;

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
    let Ok(parsed) = jid.parse::<Jid>() else {
        return Err((
            "invalid_argument".into(),
            format!("not a WhatsApp JID: {jid}"),
            false,
        ));
    };
    std::fs::create_dir_all(media_dir).map_err(|error| {
        (
            "internal".into(),
            format!("creating the avatar cache: {error}"),
            false,
        )
    })?;
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
        Ok(outcome) => {
            if !outcome.is_found() {
                if outcome.is_unchanged() {
                    // The server says the current picture is still current but
                    // names no URL; answer what the cache has, if anything.
                    return Ok(cached_avatar_path(media_dir, jid));
                }
                // Privacy refusal or no picture: cache the marker so the UI
                // stops asking, and answer with an empty path.
                write_marker(media_dir, jid);
                return Ok(String::new());
            }
            outcome
        }
        Err(error) => {
            // Offline, a timeout or a server error says nothing about the
            // picture: leave the cache alone so the next render retries.
            return Err((
                "fetch_failed".into(),
                format!("profile picture lookup: {error}"),
                true,
            ));
        }
    };
    let Some(picture) = picture.into_found() else {
        write_marker(media_dir, jid);
        return Ok(String::new());
    };
    let target = avatar_path(media_dir, jid, &picture.id.clone());
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
    if let Err(message) = check_avatar_bytes(&response.body) {
        return Err(("fetch_failed".into(), message, true));
    }
    // Publish through a temporary file so a crash never leaves a truncated
    // picture behind the final name, the way `fetchAvatar` renamed it in.
    let temporary = media_dir.join(format!(".avatar-{}.tmp", hash_hex(jid.as_bytes())));
    std::fs::write(&temporary, &response.body).map_err(|error| {
        let _ = std::fs::remove_file(&temporary);
        (
            "internal".into(),
            format!("caching the profile picture: {error}"),
            false,
        )
    })?;
    if let Err(error) = std::fs::rename(&temporary, &target) {
        let _ = std::fs::remove_file(&temporary);
        return Err((
            "internal".into(),
            format!("publishing the profile picture: {error}"),
            false,
        ));
    }
    prune_avatar_cache(media_dir, AVATAR_CACHE_BYTES);
    Ok(target.to_string_lossy().into_owned())
}

/// Writes the empty marker that records a privacy refusal or a missing
/// picture, so the next render answers without a query.
fn write_marker(media_dir: &std::path::Path, jid: &str) {
    let marker = avatar_path(media_dir, jid, "not-set");
    let _ = std::fs::write(&marker, b"");
}

/// Refuses a downloaded picture outside the bounds `fetchAvatar` enforced:
/// 2 MiB, a JPEG or PNG body, and a canvas of at most 2048 px and 4 MP.
fn check_avatar_bytes(body: &[u8]) -> Result<(), String> {
    if body.is_empty() || body.len() > MAX_AVATAR_BYTES {
        return Err("the profile picture has an invalid size".into());
    }
    let mime = crate::media::sniff_mime(body);
    if mime != "image/jpeg" && mime != "image/png" {
        return Err(format!(
            "the profile picture has unsupported content type {mime}"
        ));
    }
    let (width, height) = crate::media::sniff_dimensions(body, &mime);
    if width == 0
        || height == 0
        || width > MAX_AVATAR_EDGE
        || height > MAX_AVATAR_EDGE
        || u64::from(width) * u64::from(height) > MAX_AVATAR_PIXELS
    {
        return Err("the profile picture has invalid dimensions".into());
    }
    Ok(())
}

/// Drops the oldest cached pictures until the folder fits `max_bytes`, the
/// same oldest-first sweep `pruneAvatarCache` ran after every fetch.
fn prune_avatar_cache(media_dir: &std::path::Path, max_bytes: u64) {
    let Ok(entries) = std::fs::read_dir(media_dir) else {
        return;
    };
    let mut files: Vec<(std::path::PathBuf, u64, std::time::SystemTime)> = Vec::new();
    let mut total: u64 = 0;
    for entry in entries.filter_map(|entry| entry.ok()) {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with("avatar-") {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        let modified = metadata
            .modified()
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        total += metadata.len();
        files.push((entry.path(), metadata.len(), modified));
    }
    if total <= max_bytes {
        return;
    }
    files.sort_by_key(|(_, _, modified)| *modified);
    for (path, size, _) in files {
        if total <= max_bytes {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            total = total.saturating_sub(size);
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A throwaway folder under the system temp dir; the caller removes it.
    fn temporary_dir(tag: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "whatfast-meta-{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("the test folder is created");
        path
    }

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = vec![
            0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13, b'I', b'H', b'D', b'R',
        ];
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes
    }

    #[test]
    fn the_marker_answers_empty_and_counts_as_known() {
        let dir = temporary_dir("marker");
        let jid = "5491100000001@s.whatsapp.net";
        assert!(!avatar_known(&dir, jid));
        assert_eq!(cached_avatar_path(&dir, jid), "");
        write_marker(&dir, jid);
        assert!(avatar_known(&dir, jid));
        assert_eq!(cached_avatar_path(&dir, jid), "");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_avatar_path_is_keyed_by_the_jid_and_the_variant() {
        let dir = temporary_dir("paths");
        let first = avatar_path(&dir, "5491100000002@s.whatsapp.net", "abc");
        let same = avatar_path(&dir, "5491100000002@s.whatsapp.net", "abc");
        let other_jid = avatar_path(&dir, "5491100000003@s.whatsapp.net", "abc");
        let other_variant = avatar_path(&dir, "5491100000002@s.whatsapp.net", "def");
        assert_eq!(first, same);
        assert_ne!(first, other_jid);
        assert_ne!(first, other_variant);
        assert_eq!(first.extension().and_then(|ext| ext.to_str()), Some("jpg"));
        assert!(!first.to_string_lossy().contains("5491100000002"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_newest_picture_wins_over_an_older_one() {
        let dir = temporary_dir("newest");
        let jid = "5491100000004@s.whatsapp.net";
        let older = avatar_path(&dir, jid, "old");
        std::fs::write(&older, b"old").expect("the older file lands");
        // A file system with one-second timestamps needs a distinct mtime.
        std::thread::sleep(std::time::Duration::from_millis(20));
        let newer = avatar_path(&dir, jid, "new");
        std::fs::write(&newer, b"new").expect("the newer file lands");
        let answer = cached_avatar_path(&dir, jid);
        assert_eq!(answer, newer.to_string_lossy());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn pruning_drops_the_oldest_pictures_first() {
        let dir = temporary_dir("prune");
        let jid = "5491100000005@s.whatsapp.net";
        let oldest = avatar_path(&dir, jid, "a");
        std::fs::write(&oldest, vec![0u8; 100]).expect("the oldest file lands");
        std::thread::sleep(std::time::Duration::from_millis(20));
        let newest = avatar_path(&dir, jid, "b");
        std::fs::write(&newest, vec![0u8; 100]).expect("the newest file lands");

        prune_avatar_cache(&dir, 150);
        assert!(!oldest.exists(), "the oldest picture is dropped");
        assert!(newest.exists(), "the newest picture survives");
        prune_avatar_cache(&dir, 1000);
        assert!(newest.exists(), "a folder inside the budget is left alone");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn avatar_bytes_must_be_a_bounded_jpeg_or_png() {
        assert!(check_avatar_bytes(&png(640, 480)).is_ok());
        assert!(check_avatar_bytes(&png(2049, 10)).is_err());
        assert!(check_avatar_bytes(&png(2001, 2000)).is_err());
        assert!(check_avatar_bytes(&png(100, 100).repeat(110_000)).is_err());
        assert!(check_avatar_bytes(b"GIF89a").is_err());
        assert!(check_avatar_bytes(b"").is_err());
    }
}
