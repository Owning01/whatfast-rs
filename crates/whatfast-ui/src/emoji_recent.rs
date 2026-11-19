//! The emojis that the user used lately.
//!
//! The reference draws a section "Recent" at the top of the panel of the
//! emojis, with the last emojis that were used. The web client keeps that
//! list in the local storage of the browser; the native app keeps it in
//! `<data>/recent-emojis.json`, so the section survives a restart.
//!
//! The list belongs to the thread that draws the interface: GPUI paints
//! everything in the thread of the window, so the thread local is the state
//! of the panel and the file is its memory.

use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
};

/// How many emojis the section "Recent" holds.
pub(crate) const MAX_RECENT: usize = 16;

thread_local! {
    /// The emojis used lately, the most recent first.
    static RECENT: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    /// Whether the file was already read into `RECENT`.
    static LOADED: Cell<bool> = const { Cell::new(false) };
}

/// The file of the recent emojis in the folder of the profile.
fn path() -> PathBuf {
    whatfast_model::paths::data_dir().join("recent-emojis.json")
}

/// The emojis that the user used lately: the most recent first and without
/// repeats.
pub(crate) fn recent_emojis() -> Vec<String> {
    load_once();
    RECENT.with(|recent| recent.borrow().clone())
}

/// Puts an emoji at the head of the list of the recent ones.
///
/// The emoji that repeats leaves the list and the one that is new goes to
/// the head: the section shows the last used emojis without a duplicate and
/// with the newest one first.
pub(crate) fn remember_emoji(emoji: &str) {
    if emoji.trim().is_empty() {
        return;
    }
    load_once();
    RECENT.with(|recent| {
        let mut recent = recent.borrow_mut();
        recent.retain(|item| item != emoji);
        recent.insert(0, emoji.to_string());
        recent.truncate(MAX_RECENT);
        save(&recent);
    });
}

/// Reads the file the first time the panel or the history asks for the list.
///
/// A missing or broken file leaves the list empty: the first run of a profile
/// has no recent emojis, which is what the reference shows too.
fn load_once() {
    LOADED.with(|loaded| {
        if loaded.replace(true) {
            return;
        }
        let list = read();
        if list.is_empty() {
            return;
        }
        RECENT.with(|recent| {
            *recent.borrow_mut() = list.into_iter().take(MAX_RECENT).collect();
        });
    });
}

fn read() -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(path()) else {
        return Vec::new();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

fn save(list: &[String]) {
    let Ok(text) = serde_json::to_string(list) else {
        return;
    };
    let path = path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, text);
}

// The tests do not read or write the profile of the user who runs them: each
// one points the module at a throwaway file.

