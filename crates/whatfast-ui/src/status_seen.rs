//! The statuses that the user already opened.
//!
//! WhatsApp marks a status as seen when it is opened, and the ring of the
//! photo stops being dashed. The sidecar does not keep that mark for the
//! statuses — `MarkRead` works per chat and the status chat is never opened —,
//! so the client writes it here: for every person, the time of the newest
//! status that was opened. A status newer than that time is new again.
//!
//! The file is `<data>/statuses-seen.json`. It is small and it is only the
//! list of what this profile saw: the ring of the chat list survives a restart
//! with it.

use std::collections::HashMap;
use std::path::PathBuf;

/// The time of the newest status that was opened, by sender.
pub(crate) type Seen = HashMap<String, i64>;

/// The file of the seen statuses in the folder of the profile.
fn path() -> PathBuf {
    whatfast_model::paths::data_dir().join("statuses-seen.json")
}

// The tests do not read or write the profile of the user who runs them: each
// one points the module at a throwaway file.

/// The statuses that were seen, as they were left the last time.
///
/// A file that is missing or broken leaves an empty list: the first ring of
/// the session is dashed, which is the same thing that a new profile sees.
pub(crate) fn load() -> Seen {
    let Ok(text) = std::fs::read_to_string(path()) else {
        return Seen::new();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

/// Writes the statuses that were seen.
///
/// The ring is not worth an error in the face of the user: a profile folder
/// that does not take the file keeps the marks of the session in memory.
pub(crate) fn save(seen: &Seen) {
    let Ok(text) = serde_json::to_string(seen) else {
        return;
    };
    let path = path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, text);
}

