//! The offset of the local time zone.
//!
//! `std` has no time zone database and the crates of the app do not carry
//! one, so the question goes to the only one that knows the answer: the
//! operating system. The offset is not a constant of the machine: a zone with
//! daylight saving time has two of them a year, and the one that counts is
//! the one of the instant that is being written. That is why every function
//! here takes the instant and not only "the zone".
//!
//! - Windows: `GetTimeZoneInformationForYear` gives the rules of the zone for
//!   the year of the instant, and `SystemTimeToTzSpecificLocalTime` applies
//!   them to the exact instant.
//! - unix: `localtime_r` fills a `tm` whose `tm_gmtoff` is the offset of the
//!   instant.
//!
//! The answer is kept per hour of UTC. A list of messages asks for the offset
//! of many instants, and a window that scrolls would ask again for the same
//! ones on every frame: the call to the system is a syscall, the cache is a
//! lookup. The hour is the bucket because a change of offset happens on the
//! hour or on the half hour, so two instants of the same hour of UTC always
//! have the same offset.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// The milliseconds of an hour of UTC: the bucket of the cache.
const HOUR_MS: i64 = 3_600_000;

/// The most days an answer is kept for before the cache starts over.
///
/// A conversation of a few years of scattered days does not get near it, and
/// a list that somehow did would grow without a limit.
const MAX_BUCKETS: usize = 4096;

/// The offset of the local time zone at an instant, in minutes east of UTC.
///
/// `-180` is Buenos Aires, `0` is London in the winter, `120` is Berlin in
/// the summer.
pub(crate) fn local_offset_minutes(epoch_ms: i64) -> i32 {
    let bucket = epoch_ms.div_euclid(HOUR_MS);
    let mut cache = offsets()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(offset) = cache.get(&bucket) {
        return *offset;
    }
    let offset = offset_from_system(epoch_ms);
    if cache.len() >= MAX_BUCKETS {
        cache.clear();
    }
    cache.insert(bucket, offset);
    offset
}

/// The cache of the answers, by hour of UTC.
fn offsets() -> &'static Mutex<HashMap<i64, i32>> {
    static OFFSETS: OnceLock<Mutex<HashMap<i64, i32>>> = OnceLock::new();
    OFFSETS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The offset of the system at an instant, in minutes east of UTC.
///
/// Windows reads the rules of the zone of the year of the instant, so a
/// message of July keeps its hour when the winter comes. When the system does
/// not answer, the instant is written in UTC: an hour in UTC is better than
/// no hour at all.
#[cfg(windows)]
fn offset_from_system(epoch_ms: i64) -> i32 {
    // The milliseconds between the first of January of 1601, which is the
    // epoch of `FILETIME`, and the first of January of 1970.
    const EPOCH_DIFFERENCE_MS: i64 = 11_644_473_600_000;
    // The ticks of 100 nanoseconds of a minute, which is the unit of a
    // `FILETIME`.
    const TICKS_PER_MINUTE: i64 = 600_000_000;

    use windows_sys::Win32::Foundation::{FILETIME, SYSTEMTIME};
    use windows_sys::Win32::System::Time::{
        DYNAMIC_TIME_ZONE_INFORMATION, FileTimeToSystemTime, GetDynamicTimeZoneInformation,
        GetTimeZoneInformationForYear, SystemTimeToFileTime, SystemTimeToTzSpecificLocalTime,
        TIME_ZONE_INFORMATION,
    };

    let ticks = u64::try_from(epoch_ms.saturating_add(EPOCH_DIFFERENCE_MS))
        .unwrap_or_default()
        .saturating_mul(10_000);
    let utc_file_time = FILETIME {
        dwLowDateTime: ticks as u32,
        dwHighDateTime: (ticks >> 32) as u32,
    };
    // SAFETY: every structure is a local one that the call writes, and the
    // pointers are only handed to the call while the value is alive. The
    // codes of return are checked before the fields that a call fills are
    // read.
    unsafe {
        let mut utc = std::mem::zeroed::<SYSTEMTIME>();
        if FileTimeToSystemTime(&utc_file_time, &mut utc) == 0 {
            return 0;
        }
        // The rules of the zone, which change with the year: a zone with
        // daylight saving time does not have the same offset in every month.
        let mut dynamic = std::mem::zeroed::<DYNAMIC_TIME_ZONE_INFORMATION>();
        let mut zone = std::mem::zeroed::<TIME_ZONE_INFORMATION>();
        let rules = if GetDynamicTimeZoneInformation(&mut dynamic) != u32::MAX
            && GetTimeZoneInformationForYear(utc.wYear, &dynamic, &mut zone) != 0
        {
            &raw const zone
        } else {
            // No rules for that year: the zone of the system as it is now.
            std::ptr::null()
        };
        let mut local = std::mem::zeroed::<SYSTEMTIME>();
        if SystemTimeToTzSpecificLocalTime(rules, &utc, &mut local) == 0 {
            return 0;
        }
        let mut local_file_time = std::mem::zeroed::<FILETIME>();
        if SystemTimeToFileTime(&local, &mut local_file_time) == 0 {
            return 0;
        }
        let local_ticks = (u64::from(local_file_time.dwHighDateTime) << 32)
            | u64::from(local_file_time.dwLowDateTime);
        let difference = local_ticks as i64 - ticks as i64;
        i32::try_from(difference / TICKS_PER_MINUTE).unwrap_or_default()
    }
}

/// The offset of the system at an instant, in minutes east of UTC.
#[cfg(unix)]
fn offset_from_system(epoch_ms: i64) -> i32 {
    let seconds = epoch_ms.div_euclid(1000) as libc::time_t;
    let mut local = std::mem::MaybeUninit::<libc::tm>::zeroed();
    // SAFETY: `localtime_r` writes the structure it is given and reads only
    // the instant; the pointer is checked before the value is assumed.
    let filled = unsafe { libc::localtime_r(&seconds, local.as_mut_ptr()) };
    if filled.is_null() {
        return 0;
    }
    // SAFETY: the call returned a pointer to the structure it was given, so
    // all of its fields are written.
    let local = unsafe { local.assume_init() };
    // `tm_gmtoff` is the offset of a `tm` that `localtime_r` filled, in
    // seconds east of UTC, and it already has the daylight saving time of
    // the instant applied.
    i32::try_from(local.tm_gmtoff / 60).unwrap_or_default()
}

/// The offset of the system at an instant, in minutes east of UTC.
#[cfg(not(any(windows, unix)))]
fn offset_from_system(_epoch_ms: i64) -> i32 {
    0
}

#[cfg(test)]
mod tests {
    use super::local_offset_minutes;

    /// The most an offset can be, in minutes: UTC+14 is the farthest east.
    const LIMIT: i32 = 14 * 60;

    #[test]
    fn the_offset_of_an_instant_is_inside_the_offsets_of_the_world() {
        let offset = local_offset_minutes(1_700_000_000_000);
        assert!(
            (-LIMIT..=LIMIT).contains(&offset),
            "the offset of the system is {offset} minutes, out of the world"
        );
    }

    #[test]
    fn two_instants_of_the_same_hour_have_the_same_offset() {
        let base = 1_700_000_000_000;
        assert_eq!(
            local_offset_minutes(base),
            local_offset_minutes(base + 3_599_999)
        );
    }

    #[test]
    fn the_offset_of_an_instant_before_the_epoch_does_not_panic() {
        let offset = local_offset_minutes(-2_208_988_800_000);
        assert!((-LIMIT..=LIMIT).contains(&offset));
    }

    #[test]
    fn the_offset_of_an_instant_far_in_the_future_does_not_panic() {
        let offset = local_offset_minutes(4_102_444_800_000);
        assert!((-LIMIT..=LIMIT).contains(&offset));
    }
}
