//! The offset of the local time zone, asked to the only one that knows it.
//!
//! `std` has no time zone database. The hours of a grant and the turn of a
//! schedule are read in the local time of the user, so the offset is asked to
//! the operating system, for the instant being read: a zone with daylight
//! saving time has two offsets a year, and a schedule of July must keep its
//! hour in December.
//!
//! - Windows: `GetTimeZoneInformationForYear` gives the rules of the year and
//!   `SystemTimeToTzSpecificLocalTime` applies them to the instant.
//! - unix: `localtime_r` fills a `tm` whose `tm_gmtoff` is the offset.
//!
//! The answer is kept per hour of UTC: a schedule is evaluated every minute and
//! would ask the system for the same answer over and over.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// The milliseconds of an hour of UTC: the bucket of the cache.
const HOUR_MS: i64 = 3_600_000;

/// The most hours kept before the cache starts over.
const MAX_BUCKETS: usize = 4_096;

/// The offset of the local zone at an instant, in minutes east of UTC.
///
/// `-180` is Buenos Aires, `0` is London in the winter, `120` is Berlin in the
/// summer.
pub fn offset_minutes(epoch_ms: i64) -> i32 {
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

/// The offset now, which is the one of the schedules running today.
pub fn offset_now() -> i32 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or_default();
    offset_minutes(now)
}

/// The cache of the answers, by hour of UTC.
fn offsets() -> &'static Mutex<HashMap<i64, i32>> {
    static OFFSETS: OnceLock<Mutex<HashMap<i64, i32>>> = OnceLock::new();
    OFFSETS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The offset of the system at an instant, in minutes east of UTC.
///
/// When the system does not answer the instant is read as UTC: an hour in UTC
/// is better than a window that never opens.
#[cfg(windows)]
fn offset_from_system(epoch_ms: i64) -> i32 {
    // The milliseconds between the first of January of 1601, the epoch of
    // `FILETIME`, and the first of January of 1970.
    const EPOCH_DIFFERENCE_MS: i64 = 11_644_473_600_000;
    // The ticks of 100 nanoseconds of a minute.
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
    // SAFETY: every structure is a local one the call writes, the pointers are
    // handed out only while the value is alive, and the codes of return are
    // checked before the fields a call fills are read.
    unsafe {
        let mut utc = std::mem::zeroed::<SYSTEMTIME>();
        if FileTimeToSystemTime(&utc_file_time, &mut utc) == 0 {
            return 0;
        }
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
    // SAFETY: `localtime_r` writes the structure it is given and reads only the
    // instant; the pointer is checked before the value is assumed.
    let filled = unsafe { libc::localtime_r(&seconds, local.as_mut_ptr()) };
    if filled.is_null() {
        return 0;
    }
    // SAFETY: the call returned the pointer it was given, so the fields are
    // written.
    let local = unsafe { local.assume_init() };
    i32::try_from(local.tm_gmtoff / 60).unwrap_or_default()
}

/// The offset of the system at an instant, in minutes east of UTC.
#[cfg(not(any(windows, unix)))]
fn offset_from_system(_epoch_ms: i64) -> i32 {
    0
}

#[cfg(test)]
mod tests {
    use super::{offset_minutes, offset_now};

    /// The most an offset can be, in minutes: UTC+14 is the farthest east.
    const LIMIT: i32 = 14 * 60;

    #[test]
    fn the_offset_of_an_instant_is_inside_the_offsets_of_the_world() {
        let offset = offset_minutes(1_700_000_000_000);
        assert!(
            (-LIMIT..=LIMIT).contains(&offset),
            "the system answered {offset} minutes, outside the world"
        );
    }

    #[test]
    fn the_offset_is_whole_minutes_and_kept() {
        let instant = 1_700_000_000_000;
        let first = offset_minutes(instant);
        let second = offset_minutes(instant + 60_000);
        assert_eq!(first, second, "the same hour answers the same offset");
        assert_eq!(offset_minutes(instant) % 15, 0, "zones move by quarters");
    }

    #[test]
    fn the_offset_now_is_inside_the_world_too() {
        let offset = offset_now();
        assert!((-LIMIT..=LIMIT).contains(&offset));
    }
}
