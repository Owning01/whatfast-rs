//! The timestamps the tools print.
//!
//! A model reads `2026-10-26T14:05:00Z` better than an epoch, and the tools
//! print both: the epoch stays for comparisons. The conversion is here, with
//! no date crate: a civil calendar from days is a dozen lines and a test.

/// Milliseconds since the Unix epoch as `2026-10-26T14:05:00Z`.
pub fn iso(ms: i64) -> String {
    let seconds = ms.div_euclid(1000);
    let days = seconds.div_euclid(86_400);
    let time = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        time / 3600,
        (time % 3600) / 60,
        time % 60
    )
}

/// The date of a day count since 1970-01-01, after Howard Hinnant's
/// `civil_from_days`.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    } / 146_097;
    let day_of_era = (shifted - era * 146_097) as u64; // [0, 146096]
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era as i64 + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_position = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_position + 2) / 5 + 1) as u32;
    let month = if month_position < 10 {
        month_position + 3
    } else {
        month_position - 9
    } as u32;
    let year = if month <= 2 { year + 1 } else { year };
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_epoch_is_named() {
        assert_eq!(iso(0), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn a_known_instant_keeps_its_name() {
        assert_eq!(iso(1_000_000_000_000), "2001-09-09T01:46:40Z");
    }

    #[test]
    fn a_leap_day_survives() {
        // 2024-02-29T12:00:00Z
        assert_eq!(iso(1_709_208_000_000), "2024-02-29T12:00:00Z");
    }

    #[test]
    fn an_instant_before_the_epoch_keeps_its_sign() {
        assert_eq!(iso(-86_400_000), "1969-12-31T00:00:00Z");
    }
}
