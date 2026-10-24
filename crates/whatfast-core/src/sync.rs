//! Port of `tauri/src/state/sync.ts` (SolidJS frontend, frozen oracle).
//!
//! The module decides whether a sync event is fresh enough to be applied,
//! what the sidebar shows while the account syncs and what an empty
//! conversation explains.
//!
//! port: three things the original takes from the runtime cannot be ported
//! as they are, and the port injects them instead:
//!
//! - the number of the label is rendered with `toLocaleString` and the clock
//!   with `toLocaleTimeString`. The crate has no ICU, so the port takes the
//!   locale as a parameter and implements the two locales the app ships.
//! - the clock is the local time of the reader. The crate has no time zone
//!   database, so the port takes the offset of the local time zone.
//! - `SyncStatusView` is a local interface of the original with the six
//!   fields the module reads, and the port keeps those six fields.
//!
//! `SyncPhase` is the enum of `tauri/src/lib/types.ts`. The shared DTOs live
//! in [`crate::types`], but that module is not part of this batch and the
//! phase is only read by this one, so the enum is declared here.

/// The phase of the sync of the account.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(i32)]
pub enum SyncPhase {
    #[default]
    Unspecified = 0,
    NotStarted = 1,
    Connecting = 2,
    InitialHistory = 3,
    AppState = 4,
    CatchingUp = 5,
    Complete = 6,
    Partial = 7,
    Failed = 8,
    Offline = 9,
}

impl SyncPhase {
    /// The raw wire value of the phase: the number the bridge sends.
    pub const fn as_wire(self) -> i32 {
        self as i32
    }

    /// The phase of a raw wire value.
    ///
    /// port: the bridge crosses the enum as its numeric value, so a value the
    /// crate does not know is `Unspecified`.
    pub const fn from_wire(value: i32) -> Self {
        match value {
            1 => Self::NotStarted,
            2 => Self::Connecting,
            3 => Self::InitialHistory,
            4 => Self::AppState,
            5 => Self::CatchingUp,
            6 => Self::Complete,
            7 => Self::Partial,
            8 => Self::Failed,
            9 => Self::Offline,
            _ => Self::Unspecified,
        }
    }
}

/// The fields of the sync status the module reads.
///
/// port: the original declares this interface locally, with the six fields
/// below, and takes the whole `SyncStatus` of the bridge by structural
/// typing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SyncStatusView {
    pub phase: SyncPhase,
    pub revision: u32,
    pub messages_processed: u32,
    pub whats_app_progress: u32,
    pub completed_at_ms: i64,
    pub detail: String,
}

/// The locale the labels of the sync render with.
///
/// port: the original leaves the locale to the runtime of `Intl`. The crate
/// has no ICU, so the port implements the two locales the app ships.
///
/// - `EnUs` groups the thousands with a comma and renders the twelve hour
///   clock with `AM` and `PM` (`04:05 PM`).
/// - `EsAr` groups the thousands with a dot and renders the twelve hour
///   clock with `a. m.` and `p. m.` (`04:05 p. m.`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SyncLocale {
    #[default]
    EnUs,
    EsAr,
}

/// Whether the account is transferring data right now.
pub fn sync_status_active(status: &SyncStatusView) -> bool {
    status.phase == SyncPhase::Connecting
        || status.phase == SyncPhase::InitialHistory
        || status.phase == SyncPhase::AppState
        || status.phase == SyncPhase::CatchingUp
}

/// Whether a sync event is fresh enough to replace the current one.
pub fn should_apply_sync_status(current: &SyncStatusView, next: &SyncStatusView) -> bool {
    next.revision >= current.revision
}

/// The label the sidebar shows for the sync status.
///
/// port: the original takes an optional BCP-47 locale and renders the clock
/// of the local time zone; the port takes the locale and the offset of the
/// local time zone.
pub fn sync_status_label(
    status: &SyncStatusView,
    locale: SyncLocale,
    utc_offset_minutes: i32,
) -> String {
    match status.phase {
        SyncPhase::Connecting => "Checking WhatsApp sync".to_string(),
        SyncPhase::InitialHistory => format!(
            "Syncing {} messages ({}%)",
            to_locale_string(status.messages_processed, locale),
            status.whats_app_progress
        ),
        SyncPhase::AppState => "Finishing local setup".to_string(),
        SyncPhase::CatchingUp => "Catching up with recent messages".to_string(),
        SyncPhase::Complete => {
            if status.completed_at_ms > 0 {
                format!(
                    "Up to date \u{b7} {}",
                    format_clock(status.completed_at_ms, utc_offset_minutes, locale)
                )
            } else {
                "Up to date".to_string()
            }
        }
        SyncPhase::Partial => {
            if status.messages_processed == 0 && status.whats_app_progress == 0 {
                "Sync incomplete \u{b7} Initial chat history was not received; relink this device to request it again".to_string()
            } else if !status.detail.is_empty() {
                format!("Sync incomplete \u{b7} {}", status.detail)
            } else {
                "Sync incomplete".to_string()
            }
        }
        SyncPhase::Failed => {
            // port: `status.detail || "Sync failed"`, the falsy value of the
            // original is the empty string.
            if status.detail.is_empty() {
                "Sync failed".to_string()
            } else {
                status.detail.clone()
            }
        }
        SyncPhase::Offline => "Offline \u{b7} sync will resume when connected".to_string(),
        _ => {
            if status.detail.is_empty() {
                "Waiting for initial sync".to_string()
            } else {
                status.detail.clone()
            }
        }
    }
}

/// What an empty conversation explains while the account syncs.
pub fn empty_conversation_sync_message(status: &SyncStatusView) -> Option<String> {
    match status.phase {
        SyncPhase::NotStarted
        | SyncPhase::Connecting
        | SyncPhase::InitialHistory
        | SyncPhase::AppState
        | SyncPhase::CatchingUp => Some(
            "This chat's history is still syncing. Messages will appear when WhatsApp sends them."
                .to_string(),
        ),
        SyncPhase::Partial => {
            if status.messages_processed == 0 && status.whats_app_progress == 0 {
                Some(
                    "This chat's history was not received from WhatsApp. Relink this device to request initial history again."
                        .to_string(),
                )
            } else if !status.detail.is_empty() {
                Some(format!(
                    "This chat's history has not been received. {}.",
                    status.detail
                ))
            } else {
                Some("This chat's history has only partially synced.".to_string())
            }
        }
        SyncPhase::Failed => {
            if !status.detail.is_empty() {
                Some(format!(
                    "This chat's history could not be synced. {}.",
                    status.detail
                ))
            } else {
                Some("This chat's history could not be synced.".to_string())
            }
        }
        SyncPhase::Offline => Some(
            "This chat has no messages stored locally. Sync will resume when WhatsApp reconnects."
                .to_string(),
        ),
        _ => None,
    }
}

/// port: `Number.prototype.toLocaleString`.
///
/// port: the crate has no ICU, so the port groups the thousands with the
/// separator of the locale.
fn to_locale_string(value: u32, locale: SyncLocale) -> String {
    let digits = value.to_string();
    let separator = match locale {
        SyncLocale::EnUs => ",",
        SyncLocale::EsAr => ".",
    };
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    let bytes = digits.as_bytes();
    for (index, digit) in bytes.iter().enumerate() {
        // port: the separator goes before the digit that starts a group of
        // three, counted from the right.
        if index > 0 && (bytes.len() - index).is_multiple_of(3) {
            grouped.push_str(separator);
        }
        grouped.push(*digit as char);
    }
    grouped
}

/// port: `new Date(epochMs).toLocaleTimeString(locale, { hour: "2-digit",
/// minute: "2-digit" })`.
fn format_clock(epoch_ms: i64, utc_offset_minutes: i32, locale: SyncLocale) -> String {
    let local_ms = epoch_ms + i64::from(utc_offset_minutes) * 60_000;
    let millis_of_day = local_ms.rem_euclid(86_400_000);
    let hour = (millis_of_day / 3_600_000) as u32;
    let minute = (millis_of_day / 60_000 % 60) as u32;
    match locale {
        SyncLocale::EnUs => {
            let period = if hour < 12 { "AM" } else { "PM" };
            let hour = match hour % 12 {
                0 => 12,
                other => other,
            };
            format!("{hour:02}:{minute:02} {period}")
        }
        SyncLocale::EsAr => {
            // port: the locale of the runtime of the oracle renders the
            // twelve hour clock with a period: "04:05 p. m.".
            let period = if hour < 12 { "a. m." } else { "p. m." };
            let hour = match hour % 12 {
                0 => 12,
                other => other,
            };
            format!("{hour:02}:{minute:02} {period}")
        }
    }
}
