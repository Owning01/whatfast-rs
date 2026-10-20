//! Port of `tauri/src/lib/chat-merge.ts`.
//!
//! The original has a single function.

use std::collections::BTreeMap;

/// A chat merge invalidates requests sent against its old transport identity.
/// Drop those pending flags so their now-ignored completions cannot leave the
/// canonical poll disabled forever. Canonical requests remain untouched.
///
/// port: the original returns a `Record<string, boolean>`, so the port uses a
/// `BTreeMap<String, bool>`. The set of entries is identical to the original;
/// the original keeps the insertion order of the object and the port keeps the
/// sorted order of the map.
pub fn clear_merged_poll_votes(
    pending: &BTreeMap<String, bool>,
    old_chat_id: &str,
) -> BTreeMap<String, bool> {
    let old_prefix = format!("{old_chat_id}:");
    let mut remapped: BTreeMap<String, bool> = BTreeMap::new();
    for (key, value) in pending {
        if !key.starts_with(&old_prefix) {
            remapped.insert(key.clone(), *value);
        }
    }
    remapped
}
