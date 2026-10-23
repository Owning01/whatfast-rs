//! The starred messages of the profile: one file, one message per line.
//!
//! WhatsApp syncs the star through the app-state of the account, and the
//! linked device does not expose that action: there is no RPC for it in
//! `bridge.proto`. The star therefore belongs to this profile, the same way
//! the favorites of the conversations do, and the file keeps the pair
//! `chat_id` + tab + `message_id` so a message id that repeats in another
//! conversation cannot be confused with it.
//!
//! The replacement is atomic: a note that dies in the middle of the write
//! leaves the previous file, never half of the new one.

use std::{
    collections::{HashMap, HashSet},
    fs,
    io::Write as _,
    path::{Path, PathBuf},
};

fn starred_path() -> PathBuf {
    crate::paths::config_dir().join("starred-messages")
}

fn parse(value: &str) -> HashMap<String, HashSet<String>> {
    let mut starred: HashMap<String, HashSet<String>> = HashMap::new();
    for line in value.lines() {
        let Some((chat_id, message_id)) = line.split_once('\t') else {
            continue;
        };
        let (chat_id, message_id) = (chat_id.trim(), message_id.trim());
        if chat_id.is_empty() || message_id.is_empty() {
            continue;
        }
        starred
            .entry(chat_id.to_owned())
            .or_default()
            .insert(message_id.to_owned());
    }
    starred
}

fn load_from(path: &Path) -> HashMap<String, HashSet<String>> {
    fs::read_to_string(path)
        .ok()
        .map(|value| parse(&value))
        .unwrap_or_default()
}

fn save_to(path: &Path, starred: &HashMap<String, HashSet<String>>) -> std::io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "the path of the starred messages has no parent",
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
    let mut entries: Vec<(&str, &str)> = starred
        .iter()
        .flat_map(|(chat_id, messages)| {
            messages
                .iter()
                .map(move |message_id| (chat_id.as_str(), message_id.as_str()))
        })
        .collect();
    // The file is a set: the order does not matter, but a stable one keeps
    // the file from changing when nothing did.
    entries.sort_unstable();
    for (chat_id, message_id) in entries {
        temporary.write_all(chat_id.as_bytes())?;
        temporary.write_all(b"\t")?;
        temporary.write_all(message_id.as_bytes())?;
        temporary.write_all(b"\n")?;
    }
    temporary.as_file_mut().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}

/// Reads the messages the profile starred.
pub fn load_starred() -> HashMap<String, HashSet<String>> {
    load_from(&starred_path())
}

/// Stores the messages the profile starred, replacing the file atomically.
pub fn save_starred(starred: &HashMap<String, HashSet<String>>) -> std::io::Result<()> {
    save_to(&starred_path(), starred)
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use super::{load_from, parse, save_to};

    fn chat(messages: &[&str]) -> HashSet<String> {
        messages
            .iter()
            .map(|message| (*message).to_owned())
            .collect()
    }

    #[test]
    fn the_starred_messages_round_trip_through_atomic_replacement() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("starred-messages");
        let mut starred = HashMap::from([
            ("chat-1".to_owned(), chat(&["message-1"])),
            ("chat-2".to_owned(), chat(&["message-1"])),
        ]);

        save_to(&path, &starred).unwrap();
        assert_eq!(load_from(&path), starred);

        starred.remove("chat-1");
        save_to(&path, &starred).unwrap();
        assert_eq!(load_from(&path), starred);
    }

    #[test]
    fn a_missing_file_leaves_no_star() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("starred-messages");
        assert!(load_from(&path).is_empty());
    }

    #[test]
    fn the_same_message_id_in_two_chats_is_two_stars() {
        let parsed = parse("chat-a\tm1\nchat-b\tm1\n");
        assert!(parsed.get("chat-a").is_some_and(|ids| ids.contains("m1")));
        assert!(parsed.get("chat-b").is_some_and(|ids| ids.contains("m1")));
        assert_eq!(parsed.len(), 2);
    }

    #[test]
    fn a_damaged_line_is_left_out() {
        let parsed = parse("chat\tm1\nno-tab-here\n\t\nchat\tm2\n");
        assert_eq!(parsed.get("chat").map(HashSet::len), Some(2));
    }
}
