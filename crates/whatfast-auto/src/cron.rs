//! The five fields of a schedule, and the window that decides when a task that
//! came due actually runs.
//!
//! The engine fires in a window, not on the exact second: a task that comes due
//! at 03:00 while the user's window is `08:00-23:00` waits for the opening
//! instead of sending a message at three in the morning. The arithmetic is done
//! in "local" time by adding a fixed offset to the instant, which is what the
//! profile stores; a change of daylight saving time moves a task by an hour,
//! never by a day.

use crate::grant::Window;

/// The most days a schedule can wait for its next turn: a year plus the leap.
const HORIZON_DAYS: i64 = 366 * 4;

/// A parsed cron of five fields: minute, hour, day of month, month, weekday.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cron {
    minutes: Vec<u8>,
    hours: Vec<u8>,
    days: Vec<u8>,
    months: Vec<u8>,
    weekdays: Vec<u8>,
    /// Whether the day of month field was `*`, which changes the union rule.
    any_day: bool,
    /// Whether the weekday field was `*`.
    any_weekday: bool,
}

impl Cron {
    /// Reads `"0 20 * * *"`. The names of the months and the weekdays are not
    /// accepted: the interface writes numbers.
    pub fn parse(expression: &str) -> Option<Self> {
        let fields: Vec<&str> = expression.split_whitespace().collect();
        if fields.len() != 5 {
            return None;
        }
        Some(Self {
            minutes: field(fields[0], 0, 59)?,
            hours: field(fields[1], 0, 23)?,
            days: field(fields[2], 1, 31)?,
            months: field(fields[3], 1, 12)?,
            weekdays: weekday_field(fields[4])?,
            any_day: fields[2].trim() == "*",
            any_weekday: fields[4].trim() == "*",
        })
    }

    /// The first minute the expression allows after the instant.
    ///
    /// `offset_minutes` is the local time of the profile, in minutes east of
    /// UTC, the same number the desktop applies to the timestamps.
    pub fn next(&self, after_ms: i64, offset_minutes: i32) -> Option<i64> {
        let offset_ms = i64::from(offset_minutes) * 60_000;
        let local = after_ms.saturating_add(offset_ms);
        let day = local.div_euclid(86_400_000);
        let minute_of_day = local.rem_euclid(86_400_000) / 60_000;
        for ahead in 0..=HORIZON_DAYS {
            let this_day = day + ahead;
            let (_, month, day_of_month) = civil_from_days(this_day);
            let weekday = weekday_of(this_day);
            if !self.matches_day(month, day_of_month, weekday) {
                continue;
            }
            for hour in &self.hours {
                for minute in &self.minutes {
                    let candidate = u64::from(*hour) * 60 + u64::from(*minute);
                    if ahead == 0 && candidate <= minute_of_day as u64 {
                        continue;
                    }
                    let local_ms = this_day * 86_400_000 + candidate as i64 * 60_000;
                    return Some(local_ms - offset_ms);
                }
            }
        }
        None
    }

    /// Whether the date is one the expression allows. When both the day of the
    /// month and the weekday are restricted, either one matching is enough,
    /// which is the rule every cron implementation follows.
    fn matches_day(&self, month: i64, day_of_month: i64, weekday: i64) -> bool {
        if !self.months.contains(&(month as u8)) {
            return false;
        }
        let day_ok = self.days.contains(&(day_of_month as u8));
        let weekday_ok = self.weekdays.contains(&(weekday as u8));
        match (self.any_day, self.any_weekday) {
            (true, true) => true,
            (true, false) => weekday_ok,
            (false, true) => day_ok,
            (false, false) => day_ok || weekday_ok,
        }
    }
}

/// Moves a due instant into the working window: it stays where it is when the
/// window is open, and it waits for the opening when it is not.
pub fn defer_to_window(due_ms: i64, window: Option<Window>, offset_minutes: i32) -> i64 {
    let Some(window) = window else {
        return due_ms;
    };
    let offset_ms = i64::from(offset_minutes) * 60_000;
    let local = due_ms.saturating_add(offset_ms);
    let minute_of_day = (local.rem_euclid(86_400_000) / 60_000) as u16;
    if window.contains(minute_of_day) {
        return due_ms;
    }
    // The window opens later the same local day, or the next one.
    let wait_minutes = i64::from(window.opens_in(minute_of_day));
    due_ms + wait_minutes * 60_000
}

/// The next turn of an expression, or `None` when it never comes again.
pub fn next_at(expression: &str, after_ms: i64, offset_minutes: i32) -> Option<i64> {
    Cron::parse(expression)?.next(after_ms, offset_minutes)
}

/// One field, with `*`, `a`, `a-b`, `*/n`, `a-b/n` and comma separated lists.
fn field(text: &str, low: u8, high: u8) -> Option<Vec<u8>> {
    let mut values = Vec::new();
    for part in text.split(',') {
        let part = part.trim();
        let (range, step) = match part.split_once('/') {
            Some((range, step)) => (range.trim(), step.trim().parse::<u8>().ok()?),
            None => (part, 1),
        };
        if step == 0 {
            return None;
        }
        let (start, end) = if range == "*" {
            (low, high)
        } else if let Some((start, end)) = range.split_once('-') {
            (start.trim().parse().ok()?, end.trim().parse().ok()?)
        } else {
            let value: u8 = range.parse().ok()?;
            // `a/n` means every n from a to the top, like `*` with a floor.
            let end = if part.contains('/') { high } else { value };
            (value, end)
        };
        if start < low || end > high || start > end {
            return None;
        }
        let mut value = start;
        while value <= end {
            if !values.contains(&value) {
                values.push(value);
            }
            value = value.checked_add(step)?;
        }
    }
    if values.is_empty() {
        return None;
    }
    values.sort_unstable();
    Some(values)
}

/// The weekday field: `0` and `7` are both Sunday, and the count starts at
/// Sunday like every other cron.
fn weekday_field(text: &str) -> Option<Vec<u8>> {
    let values = field(text, 0, 7)?;
    let mut normalized: Vec<u8> = values
        .into_iter()
        .map(|value| if value == 7 { 0 } else { value })
        .collect();
    normalized.sort_unstable();
    normalized.dedup();
    Some(normalized)
}

/// The year, month and day of a count of days since 1970-01-01, in the
/// proleptic Gregorian calendar.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted + 2) / 5 + 1;
    let month = if shifted < 10 {
        shifted + 3
    } else {
        shifted - 9
    };
    let year = if month <= 2 { year + 1 } else { year };
    (year, month, day)
}

/// The weekday of a count of days since 1970-01-01, Sunday as `0`.
fn weekday_of(days: i64) -> i64 {
    (days + 4).rem_euclid(7)
}

