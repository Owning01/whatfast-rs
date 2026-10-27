//! The grant of one agent: what it may do, in which chats, and at which hours.
//!
//! The three scopes are cumulative: `send` implies `draft`, and `draft` implies
//! `read`. A grant that names no chat covers none — the user picks the chats in
//! the interface, and an empty list is a closed door, not an open one.

use std::fmt;

use serde_json::{Value, json};

/// What an agent may do with the chats its grant covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Scope {
    /// Read the chats and their messages.
    Read,
    /// Read, and leave drafts for the user to approve.
    Draft,
    /// Read, draft, and send without asking.
    Send,
}

impl Scope {
    /// The scope of one stored name, `read`, `draft` or `send`.
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "read" => Some(Self::Read),
            "draft" => Some(Self::Draft),
            "send" => Some(Self::Send),
            _ => None,
        }
    }

    /// The name the database stores.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Draft => "draft",
            Self::Send => "send",
        }
    }

    /// Whether this scope reaches the one asked for.
    pub fn covers(self, needed: Self) -> bool {
        self >= needed
    }
}

impl fmt::Display for Scope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Which chats a grant covers.
///
/// `allow` is the list the user ticked; `None` means every chat. `groups` says
/// whether groups are part of it: a broadcast group is a lot of recipients, so
/// they stay out unless the user asked for them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatFilter {
    /// The chats the grant covers, or `None` for all of them.
    pub allow: Option<Vec<String>>,
    /// Whether groups are covered.
    pub groups: bool,
}

impl Default for ChatFilter {
    fn default() -> Self {
        // Out of the box an agent is trusted with the conversations the user
        // opened for it, which is no group until one is picked.
        Self {
            allow: Some(Vec::new()),
            groups: false,
        }
    }
}

impl ChatFilter {
    /// Every chat, groups included.
    pub fn all() -> Self {
        Self {
            allow: None,
            groups: true,
        }
    }

    /// Only these chats, groups included when they are in the list.
    pub fn only(chats: impl IntoIterator<Item = String>) -> Self {
        Self {
            allow: Some(chats.into_iter().collect()),
            groups: true,
        }
    }

    /// Whether the chat is inside the grant.
    pub fn allows(&self, chat_id: &str, is_group: bool) -> bool {
        if is_group && !self.groups {
            return false;
        }
        match &self.allow {
            None => true,
            Some(chats) => chats.iter().any(|allowed| allowed == chat_id),
        }
    }

    /// The filter as the column of `agents` keeps it.
    pub fn to_json(&self) -> String {
        let allow = match &self.allow {
            None => Value::Null,
            Some(chats) => json!(chats),
        };
        json!({ "allow": allow, "groups": self.groups }).to_string()
    }

    /// The filter back from its column. A column that cannot be read covers
    /// nothing: the failure has to close the door, never open it.
    pub fn from_json(text: &str) -> Self {
        let Ok(value) = serde_json::from_str::<Value>(text) else {
            return Self::default();
        };
        let allow = match value.get("allow") {
            Some(Value::Null) => None,
            Some(Value::Array(items)) => Some(
                items
                    .iter()
                    .filter_map(|item| item.as_str().map(str::to_string))
                    .collect(),
            ),
            // No list and no explicit `null` is not "everything".
            _ => return Self::default(),
        };
        Self {
            allow,
            groups: value
                .get("groups")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        }
    }
}

/// The hours an agent works in, in minutes of the local day.
///
/// `start` is inclusive and `end` exclusive. An end before the start crosses
/// midnight (`22:00-06:00`), and the same value on both sides means the whole
/// day.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Window {
    /// Minutes after midnight when the window opens.
    pub start: u16,
    /// Minutes after midnight when the window closes.
    pub end: u16,
}

impl Window {
    /// The window of the prudent default: 8:00 to 23:00.
    pub const PRUDENT: Self = Self {
        start: 8 * 60,
        end: 23 * 60,
    };

    /// Reads `"08:00-23:00"`.
    pub fn parse(text: &str) -> Option<Self> {
        let (open, close) = text.split_once('-')?;
        Some(Self {
            start: minute_of_day(open)?,
            end: minute_of_day(close)?,
        })
    }

    /// Whether the minute of the local day is inside the window.
    pub fn contains(&self, minute_of_day: u16) -> bool {
        let minute = minute_of_day % (24 * 60);
        if self.start == self.end {
            return true;
        }
        if self.start < self.end {
            return minute >= self.start && minute < self.end;
        }
        // A window that crosses midnight is two stretches of the same day.
        minute >= self.start || minute < self.end
    }

    /// The minutes from the start of the day that the window opens at.
    pub fn opens_in(&self, minute_of_day: u16) -> u32 {
        let minute = u32::from(minute_of_day % (24 * 60));
        let start = u32::from(self.start);
        if minute <= start {
            start - minute
        } else {
            24 * 60 - minute + start
        }
    }
}

impl fmt::Display for Window {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{:02}:{:02}-{:02}:{:02}",
            self.start / 60,
            self.start % 60,
            self.end / 60,
            self.end % 60
        )
    }
}

/// `"08:30"` as minutes after midnight.
fn minute_of_day(text: &str) -> Option<u16> {
    let (hours, minutes) = text.trim().split_once(':')?;
    let hours: u16 = hours.trim().parse().ok()?;
    let minutes: u16 = minutes.trim().parse().ok()?;
    if hours > 24 || minutes > 59 || (hours == 24 && minutes > 0) {
        return None;
    }
    Some(hours * 60 + minutes)
}

/// Everything one agent may do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grant {
    /// The highest thing the agent may do.
    pub scope: Scope,
    /// The chats the scope applies to.
    pub chats: ChatFilter,
    /// The ceiling of messages per hour, `0` for no ceiling.
    pub per_hour: u16,
    /// The hours it works in, or `None` for every hour.
    pub window: Option<Window>,
    /// Whether its messages go out at the pace of a person instead of the
    /// instant the model finished.
    pub humanize: bool,
}

impl Default for Grant {
    /// Reads by default: connecting an agent never hands it the pen.
    fn default() -> Self {
        Self {
            scope: Scope::Read,
            chats: ChatFilter::default(),
            per_hour: 20,
            window: Some(Window::PRUDENT),
            humanize: true,
        }
    }
}

impl Grant {
    /// The grant of an agent that may read everything and nothing else.
    pub fn read_all() -> Self {
        Self {
            scope: Scope::Read,
            chats: ChatFilter::all(),
            ..Self::default()
        }
    }

    /// Whether the chat is covered and the scope reaches what is needed.
    pub fn allows(&self, chat_id: &str, is_group: bool, needed: Scope) -> bool {
        self.scope.covers(needed) && self.chats.allows(chat_id, is_group)
    }

    /// Whether the minute of the local day is inside the working hours.
    pub fn open_at(&self, minute_of_day: u16) -> bool {
        match self.window {
            None => true,
            Some(window) => window.contains(minute_of_day),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scopes_are_cumulative() {
        assert!(Scope::Send.covers(Scope::Read));
        assert!(Scope::Send.covers(Scope::Draft));
        assert!(Scope::Draft.covers(Scope::Read));
        assert!(!Scope::Draft.covers(Scope::Send));
        assert!(!Scope::Read.covers(Scope::Draft));
    }

    #[test]
    fn a_scope_round_trips_through_its_name() {
        for scope in [Scope::Read, Scope::Draft, Scope::Send] {
            assert_eq!(Scope::parse(scope.as_str()), Some(scope));
        }
        assert_eq!(Scope::parse("SEND"), Some(Scope::Send));
        assert_eq!(Scope::parse("admin"), None);
    }

    #[test]
    fn an_empty_list_covers_no_chat() {
        let filter = ChatFilter::default();
        assert!(!filter.allows("5491100000000@s.whatsapp.net", false));
        assert!(!filter.allows("123@g.us", true));
    }

    #[test]
    fn the_list_covers_only_its_chats() {
        let filter = ChatFilter::only(vec!["5491100000000@s.whatsapp.net".to_string()]);
        assert!(filter.allows("5491100000000@s.whatsapp.net", false));
        assert!(!filter.allows("5491100000001@s.whatsapp.net", false));
    }

    #[test]
    fn groups_stay_out_unless_the_user_asked() {
        let filter = ChatFilter::only(vec!["123@g.us".to_string()]);
        assert!(filter.allows("123@g.us", true));
        let without = ChatFilter {
            groups: false,
            ..filter
        };
        assert!(!without.allows("123@g.us", true));
        assert!(ChatFilter::all().allows("123@g.us", true));
    }

    #[test]
    fn the_filter_round_trips_through_json() {
        let filter = ChatFilter {
            allow: Some(vec!["a@s.whatsapp.net".into()]),
            groups: false,
        };
        assert_eq!(ChatFilter::from_json(&filter.to_json()), filter);
        assert_eq!(
            ChatFilter::from_json(&ChatFilter::all().to_json()),
            ChatFilter::all()
        );
        // A broken or empty column covers nothing rather than everything.
        assert_eq!(ChatFilter::from_json("{}"), ChatFilter::default());
        assert_eq!(ChatFilter::from_json("not json"), ChatFilter::default());
        assert!(!ChatFilter::from_json("{").allows("a@s.whatsapp.net", false));
    }

    #[test]
    fn a_window_inside_the_day() {
        let window = Window::parse("08:00-23:00").expect("window");
        assert!(!window.contains(7 * 60 + 59));
        assert!(window.contains(8 * 60));
        assert!(window.contains(22 * 60 + 59));
        assert!(!window.contains(23 * 60));
        assert_eq!(window.to_string(), "08:00-23:00");
    }

    #[test]
    fn a_window_that_crosses_midnight() {
        let window = Window::parse("22:00-06:00").expect("window");
        assert!(window.contains(23 * 60));
        assert!(window.contains(2 * 60));
        assert!(!window.contains(6 * 60));
        assert!(!window.contains(12 * 60));
    }

    #[test]
    fn the_same_minute_twice_is_the_whole_day() {
        let open = Window::parse("00:00-00:00").expect("window");
        assert!(open.contains(3 * 60));
        assert!(open.contains(23 * 60 + 59));
        assert!(!Window::PRUDENT.contains(7 * 60));
    }

    #[test]
    fn a_window_parses_only_its_form() {
        assert!(Window::parse("8-23").is_none());
        assert!(Window::parse("25:00-23:00").is_none());
        assert!(Window::parse("08:70-23:00").is_none());
        assert!(Window::parse("08:00").is_none());
        assert_eq!(Window::parse(" 08:00 - 23:30 ").map(|w| w.start), Some(480));
    }

    #[test]
    fn the_window_says_how_long_until_it_opens() {
        let window = Window::parse("08:00-23:00").expect("window");
        assert_eq!(window.opens_in(7 * 60), 60);
        assert_eq!(window.opens_in(8 * 60), 0);
        assert_eq!(window.opens_in(23 * 60 + 30), 8 * 60 + 30);
    }

    #[test]
    fn the_default_grant_only_reads() {
        let grant = Grant::default();
        assert!(!grant.allows("a@s.whatsapp.net", false, Scope::Draft));
        assert!(!grant.allows("a@s.whatsapp.net", false, Scope::Send));
        assert!(!grant.allows("a@s.whatsapp.net", false, Scope::Read));
        assert!(grant.open_at(12 * 60));
        assert!(!grant.open_at(3 * 60));
    }

    #[test]
    fn a_grant_with_open_hours_works_at_night() {
        let grant = Grant {
            window: None,
            ..Grant::read_all()
        };
        assert!(grant.open_at(3 * 60));
        assert!(grant.allows("a@s.whatsapp.net", false, Scope::Read));
        assert!(!grant.allows("a@s.whatsapp.net", false, Scope::Send));
    }
}
