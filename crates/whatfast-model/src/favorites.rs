//! The favorites of the profile: one file, the ids of the conversations.
//!
//! port: the origin keeps them in the local storage of the browser
//! (`tauri/src/state/app.ts`, the key `whatfast-favorites`): the linked
//! device has no RPC for them, so they belong to the profile and not to the
//! account. The file holds one id per line, and the replacement is atomic,
//! the same way the settings file is written.

use std::{
    collections::HashSet,
    fs,
    io::Write as _,
    path::{Path, PathBuf},
};

fn favorites_path() -> PathBuf {
    crate::paths::config_dir().join("favorites")
}

fn load_from(path: &Path) -> HashSet<String> {
    fs::read_to_string(path)
        .ok()
        .map(|value| {
            value
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn save_to(path: &Path, favorites: &HashSet<String>) -> std::io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "favorites path has no parent",
        )
    })?;
    fs::create_dir_all(parent)?;
    let existing_permissions = fs::metadata(path)
        .ok()
        .map(|metadata| metadata.permissions());
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    if let Some(permissions) = existing_permissions {
        temporary.as_file().set_permissions(permissions)?;
    }
    let mut ids: Vec<&str> = favorites.iter().map(String::as_str).collect();
    // The file is a set: the order does not matter, but a stable one keeps
    // the file from changing when nothing did.
    ids.sort_unstable();
    for id in ids {
        temporary.write_all(id.as_bytes())?;
        temporary.write_all(b"\n")?;
    }
    temporary.as_file_mut().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}

/// Reads the conversations the profile marked as favorites.
pub fn load_favorites() -> HashSet<String> {
    load_from(&favorites_path())
}

/// Stores the conversations the profile marked as favorites, replacing the
/// file atomically.
pub fn save_favorites(favorites: &HashSet<String>) -> std::io::Result<()> {
    save_to(&favorites_path(), favorites)
}

