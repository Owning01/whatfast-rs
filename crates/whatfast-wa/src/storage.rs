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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{clear, scan, MediaKind};

    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "whatfast-storage-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_cache_is_grouped_by_kind_and_cleared() {
        let dir = temp_dir("grouped");
        std::fs::write(dir.join("aaaa.jpg"), vec![0u8; 100]).unwrap();
        std::fs::write(dir.join("bbbb.mp4"), vec![0u8; 300]).unwrap();
        std::fs::write(dir.join("cccc.ogg"), vec![0u8; 50]).unwrap();
        std::fs::write(dir.join("dddd.pdf"), vec![0u8; 20]).unwrap();
        std::fs::write(dir.join("avatar-0123-normal.jpg"), vec![0u8; 10]).unwrap();
        std::fs::write(dir.join("eeee.bin"), vec![0u8; 5]).unwrap();

        let usage = scan(&dir);
        assert_eq!(usage.total_bytes, 485);
        assert_eq!(usage.total_files, 6);
        let bytes = |kind: MediaKind| {
            usage
                .buckets
                .iter()
                .find(|bucket| bucket.kind == kind)
                .unwrap()
                .bytes
        };
        assert_eq!(bytes(MediaKind::Photo), 100);
        assert_eq!(bytes(MediaKind::Video), 300);
        assert_eq!(bytes(MediaKind::Audio), 50);
        assert_eq!(bytes(MediaKind::Document), 20);
        assert_eq!(bytes(MediaKind::Avatar), 10, "the avatar wins over the JPEG");
        assert_eq!(bytes(MediaKind::Other), 5);

        let (freed, files) = clear(&dir);
        assert_eq!((freed, files), (485, 6));
        assert_eq!(scan(&dir).total_bytes, 0);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_missing_folder_is_an_empty_cache() {
        let missing = std::env::temp_dir().join("whatfast-storage-does-not-exist");
        let usage = scan(&missing);
        assert_eq!(usage.total_bytes, 0);
        assert_eq!(usage.total_files, 0);
        assert_eq!(clear(&missing), (0, 0));
    }

    #[test]
    fn the_kind_comes_from_the_name_the_sidecar_gave_the_file() {
        assert_eq!(MediaKind::of("avatar-abc-normal.jpg"), MediaKind::Avatar);
        assert_eq!(MediaKind::of("3f2a.JPG"), MediaKind::Photo);
        assert_eq!(MediaKind::of("3f2a.webp"), MediaKind::Photo);
        assert_eq!(MediaKind::of("nota.opus"), MediaKind::Audio);
        assert_eq!(
            MediaKind::of("nota.oga"),
            MediaKind::Audio,
            "the sidecar caches an Ogg audio with .oga"
        );
        assert_eq!(MediaKind::of("clip.MP4"), MediaKind::Video);
        assert_eq!(
            MediaKind::of("clip.3gpp"),
            MediaKind::Video,
            "the sidecar caches a 3GPP video with .3gpp"
        );
        assert_eq!(MediaKind::of("contrato.pdf"), MediaKind::Document);
        assert_eq!(MediaKind::of("sin-extension"), MediaKind::Other);
    }
}
