//! The emojis that the user used lately.
//!
//! The reference draws a section "Recent" at the top of the panel of the
//! emojis, with the last emojis that were used. The web client keeps that
//! list in the local storage of the browser; the native app keeps it in the
//! memory of the process: it is a state of the interface and not of the
//! domain, so it does not live in `app.rs`.
//!
//! The list belongs to the thread that draws the interface: GPUI paints
//! everything in the thread of the window, so the thread local is the state
//! of the panel.

use std::cell::RefCell;

/// How many emojis the section "Recent" holds.
pub(crate) const MAX_RECENT: usize = 16;

thread_local! {
    /// The emojis used lately, the most recent first.
    static RECENT: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// The emojis that the user used lately: the most recent first and without
/// repeats.
pub(crate) fn recent_emojis() -> Vec<String> {
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
    RECENT.with(|recent| {
        let mut recent = recent.borrow_mut();
        recent.retain(|item| item != emoji);
        recent.insert(0, emoji.to_string());
        recent.truncate(MAX_RECENT);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_newest_emoji_goes_to_the_head_of_the_list() {
        remember_emoji("😀");
        remember_emoji("🚀");
        let recent = recent_emojis();
        assert_eq!(recent.first().map(String::as_str), Some("🚀"));
    }

    #[test]
    fn an_emoji_that_repeats_is_not_in_the_list_twice() {
        remember_emoji("😀");
        remember_emoji("🚀");
        remember_emoji("😀");
        let recent = recent_emojis();
        assert_eq!(recent.iter().filter(|item| *item == "😀").count(), 1);
        assert_eq!(recent.first().map(String::as_str), Some("😀"));
    }

    #[test]
    fn the_list_holds_at_most_sixteen_emojis() {
        for index in 0..MAX_RECENT + 4 {
            remember_emoji(&format!("{index}"));
        }
        assert_eq!(recent_emojis().len(), MAX_RECENT);
    }
}
