//! Port of `tauri/src/state/unread.ts`.
//!
//! The original is two pure decisions about the optimistic unread badge.

/// Returns the only safe optimistic badge value for a visible read boundary.
///
/// A bounded window with newer messages cannot prove how many unread rows lie
/// beyond its last incoming message. Keep the backend-provided count until the
/// mark-read operation emits an authoritative chat upsert. A window at the
/// newest boundary can safely clear the badge immediately.
///
/// port: `current` is a count, so it is a `u32`; `hasNewer` is a plain `bool`.
pub fn optimistic_unread_count(current: u32, has_newer: bool) -> u32 {
    if has_newer { current } else { 0 }
}

/// Roll back an optimistic badge only while both the value and its backend
/// revision are unchanged. The revision distinguishes an authoritative upsert
/// that confirms the same value from the absence of any backend update.
///
/// port: `current` is `number | undefined` in the original, so it becomes an
/// `Option<u32>`. `undefined === optimistic` is always false, which is what
/// `None` gives here.
pub fn should_restore_optimistic_unread(
    current: Option<u32>,
    optimistic: u32,
    revision_before: u32,
    revision_now: u32,
) -> bool {
    current == Some(optimistic) && revision_now == revision_before
}
