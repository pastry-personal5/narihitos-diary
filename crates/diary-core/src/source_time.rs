//! Pure time arithmetic for Ayumi source note names.
//!
//! Ayumi encodes the UTC creation instant in both the date directory and the
//! file name (`YYYY-MM-DD_HH-MM-SS-mmm.md`). These helpers turn that instant
//! into a local calendar date and clock time once the caller supplies the
//! local UTC offset; they never read the system time zone themselves.

use crate::DiaryDay;

/// Parses the UTC instant (Unix seconds) from the final path segment of a
/// source note name such as `2026/10/29/2026-10-29_15-30-00-123.md`.
#[must_use]
pub fn utc_epoch_from_note_name(note_name: &str) -> Option<i64> {
    let file = note_name.rsplit('/').next()?;
    let stem = file.strip_suffix(".md")?;
    let bytes = stem.as_bytes();
    if bytes.len() < 19 || bytes[4] != b'-' || bytes[7] != b'-' || bytes[10] != b'_' {
        return None;
    }
    if bytes[13] != b'-' || bytes[16] != b'-' {
        return None;
    }
    if bytes.len() > 19 && (bytes[19] != b'-' || !bytes[20..].iter().all(u8::is_ascii_digit)) {
        return None;
    }
    let number = |range: std::ops::Range<usize>| -> Option<i64> {
        let part = &stem[range];
        part.bytes()
            .all(|byte| byte.is_ascii_digit())
            .then(|| part.parse().ok())?
    };
    let (year, month, day) = (number(0..4)?, number(5..7)?, number(8..10)?);
    let (hour, minute, second) = (number(11..13)?, number(14..16)?, number(17..19)?);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    if hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    Some(days_from_civil(year, month, day) * 86_400 + hour * 3_600 + minute * 60 + second)
}

/// Formats `epoch` shifted by `offset_seconds` as `YYYY-MM-DD HH:MM:SS`.
#[must_use]
pub fn format_local(epoch: i64, offset_seconds: i32) -> String {
    let (year, month, day, seconds) = local_parts(epoch, offset_seconds);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}",
        seconds / 3_600,
        seconds % 3_600 / 60,
        seconds % 60
    )
}

/// The local calendar day of `epoch` shifted by `offset_seconds`.
#[must_use]
pub fn local_day(epoch: i64, offset_seconds: i32) -> Option<DiaryDay> {
    let (year, month, day, _) = local_parts(epoch, offset_seconds);
    DiaryDay::parse(format!("{year:04}-{month:02}-{day:02}")).ok()
}

fn local_parts(epoch: i64, offset_seconds: i32) -> (i64, i64, i64, i64) {
    let shifted = epoch + i64::from(offset_seconds);
    let days = shifted.div_euclid(86_400);
    let seconds = shifted.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    (year, month, day, seconds)
}

// Howard Hinnant's civil-calendar algorithms.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year.rem_euclid(400);
    let shifted_month = (month + 9) % 12;
    let day_of_year = (153 * shifted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let days = days + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_utc_instant_from_the_final_segment() {
        let epoch = utc_epoch_from_note_name("2026/10/29/2026-10-29_15-30-00-123.md").unwrap();
        assert_eq!(format_local(epoch, 0), "2026-10-29 15:30:00");
        assert_eq!(
            utc_epoch_from_note_name("2026-10-29_15-30-00.md"),
            Some(epoch)
        );
    }

    #[test]
    fn rejects_malformed_names() {
        for name in [
            "n.md",
            "2026-10-29_15-30-00-123.txt",
            "2026-10-29 15-30-00.md",
            "2026-13-29_15-30-00.md",
            "2026-10-29_24-30-00.md",
            "2026-10-29_15-30-00-12x.md",
        ] {
            assert_eq!(utc_epoch_from_note_name(name), None, "{name}");
        }
    }

    #[test]
    fn local_conversion_crosses_the_date_boundary_both_ways() {
        let epoch = utc_epoch_from_note_name("2026-10-29_15-30-00-000.md").unwrap();
        assert_eq!(format_local(epoch, 9 * 3_600), "2026-10-30 00:30:00");
        assert_eq!(local_day(epoch, 9 * 3_600).unwrap().as_str(), "2026-10-30");
        assert_eq!(format_local(epoch, -8 * 3_600), "2026-10-29 07:30:00");
        let early = utc_epoch_from_note_name("2026-03-01_00-10-00.md").unwrap();
        assert_eq!(format_local(early, -3_600), "2026-02-28 23:10:00");
        let leap = utc_epoch_from_note_name("2028-02-29_23-59-59.md").unwrap();
        assert_eq!(format_local(leap, 1), "2028-03-01 00:00:00");
    }
}
