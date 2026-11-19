//! The space the cache of media takes on this device, and its cleanup.
//!
//! The cache is `<data-dir>/media`: the pictures, videos, voice notes,
//! documents and avatars this computer downloaded. The files are only a copy;
//! WhatsApp keeps the originals, and the app fetches one again when the user
//! opens the message. Clearing the cache never touches the account, the
//! phone or the database of the conversation.

use std::path::Path;

/// One kind of file of the cache.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub enum MediaKind {
    Photo,
    Video,
    Audio,
    Document,
    Avatar,
    Other,
}

impl MediaKind {
    /// The id the bridge sends: the panel groups the rows by it.
    pub fn id(self) -> &'static str {
        match self {
            Self::Photo => "photo",
            Self::Video => "video",
            Self::Audio => "audio",
            Self::Document => "document",
            Self::Avatar => "avatar",
            Self::Other => "other",
        }
    }

    /// The kind of one cached file, by the name the sidecar gave it.
    ///
    /// The avatars are the only files with a prefix that wins over the
    /// extension: an avatar is a JPEG too, and the panel shows it apart.
    fn of(file_name: &str) -> Self {
        if file_name.starts_with("avatar-") {
            return Self::Avatar;
        }
        let extension = Path::new(file_name)
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        match extension.as_str() {
            "jpg" | "jpeg" | "png" | "webp" | "gif" | "heic" | "bmp" | "tif" | "tiff" => Self::Photo,
            "mp4" | "mov" | "m4v" | "webm" | "mkv" | "3gp" | "3gpp" | "avi" => Self::Video,
            "ogg" | "oga" | "opus" | "mp3" | "m4a" | "aac" | "wav" | "flac" | "amr" => Self::Audio,
            "pdf" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "txt" | "csv" | "zip"
            | "rar" | "7z" | "apk" => Self::Document,
            _ => Self::Other,
        }
    }
}

/// What one kind of file takes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Bucket {
    pub kind: MediaKind,
    pub bytes: u64,
    pub files: u32,
}

/// What the cache takes, grouped by kind.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CacheUsage {
    pub total_bytes: u64,
    pub total_files: u32,
    /// One entry per kind, in the order the panel paints them.
    pub buckets: Vec<Bucket>,
}

impl CacheUsage {
    /// The wire answer of the panel of the settings, with the folder the
    /// cache lives in.
    pub fn to_response(&self, media_dir: &Path) -> whatfast_protocol::StorageUsageResponse {
        whatfast_protocol::StorageUsageResponse {
            total_bytes: self.total_bytes,
            total_files: self.total_files,
            buckets: self
                .buckets
                .iter()
                .map(|bucket| whatfast_protocol::StorageBucket {
                    kind: bucket.kind.id().to_owned(),
                    bytes: bucket.bytes,
                    files: bucket.files,
                })
                .collect(),
            folder: media_dir.to_string_lossy().into_owned(),
        }
    }
}

/// Measures the cache. A missing folder is an empty cache, not an error: a
/// profile that never opened a photo does not have one.
pub fn scan(media_dir: &Path) -> CacheUsage {
    let mut bytes = [0u64; 6];
    let mut files = [0u32; 6];
    walk(media_dir, &mut |path, size| {
        let index = kind_index(path);
        bytes[index] += size;
        files[index] += 1;
    });
    let total_bytes = bytes.iter().sum();
    let total_files = files.iter().sum();
    let buckets = [
        MediaKind::Photo,
        MediaKind::Video,
        MediaKind::Audio,
        MediaKind::Document,
        MediaKind::Avatar,
        MediaKind::Other,
    ]
    .into_iter()
    .map(|kind| {
        let index = kind_slot(kind);
        Bucket {
            kind,
            bytes: bytes[index],
            files: files[index],
        }
    })
    .collect();
    CacheUsage {
        total_bytes,
        total_files,
        buckets,
    }
}

/// Deletes every file of the cache and answers the bytes and the files it
/// could free. A file the system does not let go is left alone and not
/// counted.
pub fn clear(media_dir: &Path) -> (u64, u32) {
    let mut bytes = 0u64;
    let mut files = 0u32;
    walk(media_dir, &mut |path, size| {
        if std::fs::remove_file(path).is_ok() {
            bytes += size;
            files += 1;
        }
    });
    (bytes, files)
}

/// Deletes specific cached files referenced by expired records (such as
/// statuses older than two days). Only deletes files located inside
/// `media_dir` so arbitrary paths on disk are never touched.
pub fn purge_paths(media_dir: &Path, paths: &[String]) -> u32 {
    let Ok(canonical_dir) = std::fs::canonicalize(media_dir) else {
        return 0;
    };
    let mut removed = 0u32;
    for raw in paths {
        if raw.is_empty() {
            continue;
        }
        let path = Path::new(raw);
        let Ok(canonical_file) = std::fs::canonicalize(path) else {
            continue;
        };
        if canonical_file.starts_with(&canonical_dir)
            && canonical_file.is_file()
            && std::fs::remove_file(&canonical_file).is_ok()
        {
            removed += 1;
        }
    }
    removed
}

/// Removes non-avatar, non-favorite-sticker cached media files whose last
/// modification time is older than `max_age`, freeing disk space after a
/// generous retention period without touching recent videos or photos.
pub fn prune_expired_media(media_dir: &Path, max_age: std::time::Duration) -> (u64, u32) {
    let Ok(entries) = std::fs::read_dir(media_dir) else {
        return (0, 0);
    };
    let now = std::time::SystemTime::now();
    let mut bytes = 0u64;
    let mut files = 0u32;
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if metadata.is_dir() {
            let (b, f) = prune_expired_media(&path, max_age);
            bytes += b;
            files += f;
            continue;
        }
        if !metadata.is_file() {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        // Avatars have their own byte-budgeted cache (`prune_avatar_cache`),
        // and favorite stickers (`fav-`) are user-pinned assets.
        if name.starts_with("avatar-") || name.starts_with("fav-") || name.starts_with('.') {
            continue;
        }
        let Ok(modified) = metadata.modified() else {
            continue;
        };
        let Ok(age) = now.duration_since(modified) else {
            continue;
        };
        if age > max_age && std::fs::remove_file(&path).is_ok() {
            bytes += metadata.len();
            files += 1;
        }
    }
    (bytes, files)
}

/// Visits every file under the folder, the nested ones included.
fn walk(root: &Path, visit: &mut impl FnMut(&Path, u64)) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        let path = entry.path();
        if metadata.is_dir() {
            walk(&path, visit);
        } else if metadata.is_file() {
            visit(&path, metadata.len());
        }
    }
}

fn kind_index(path: &Path) -> usize {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    kind_slot(MediaKind::of(name))
}

/// The position of a kind in the arrays of [`scan`].
fn kind_slot(kind: MediaKind) -> usize {
    match kind {
        MediaKind::Photo => 0,
        MediaKind::Video => 1,
        MediaKind::Audio => 2,
        MediaKind::Document => 3,
        MediaKind::Avatar => 4,
        MediaKind::Other => 5,
    }
}

