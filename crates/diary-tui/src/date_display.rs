//! Diary-day date rendering: `YYYY-MM-DD`, with the abbreviated weekday
//! appended as `YYYY-MM-DD (Mon)` only where the available display width
//! permits the longer form (see AGENTS.md "TUI date display"). Pure and
//! presentation-only: it never recomputes or validates the date as a
//! workflow fact, only renders one the server already returned.

use ratatui::text::Line;

const WEEKDAY_NAMES: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

/// Renders `date` (expected `YYYY-MM-DD`) as `YYYY-MM-DD (Www)` if that
/// fits within `max_width` display columns, otherwise as the bare
/// `YYYY-MM-DD`. A date that does not parse as a valid calendar date is
/// returned unchanged — this is a rendering function, not a validator;
/// malformed input is a defect upstream, not something to hide or panic
/// on here.
#[must_use]
pub fn format_diary_day(date: &str, max_width: u16) -> String {
    match weekday_abbrev(date) {
        Some(weekday) => {
            let with_weekday = format!("{date} ({weekday})");
            if Line::from(with_weekday.as_str()).width() <= usize::from(max_width) {
                with_weekday
            } else {
                date.to_owned()
            }
        }
        None => date.to_owned(),
    }
}

/// Parses a strict `YYYY-MM-DD` date and returns its abbreviated weekday
/// name, or `None` if `date` is not exactly that shape.
fn weekday_abbrev(date: &str) -> Option<&'static str> {
    let (year, month, day) = parse_iso_date(date)?;
    let days_since_epoch = days_from_civil(year, month, day);
    // 1970-01-01 (z = 0) was a Thursday (index 4 in a Sunday-first table).
    let index = (days_since_epoch + 4).rem_euclid(7) as usize;
    Some(WEEKDAY_NAMES[index])
}

fn parse_iso_date(date: &str) -> Option<(i64, u32, u32)> {
    let bytes = date.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return None;
    }
    if !date
        .bytes()
        .enumerate()
        .all(|(index, byte)| index == 4 || index == 7 || byte.is_ascii_digit())
    {
        return None;
    }
    let year: i64 = date[0..4].parse().ok()?;
    let month: u32 = date[5..7].parse().ok()?;
    let day: u32 = date[8..10].parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    Some((year, month, day))
}

/// Howard Hinnant's `days_from_civil`: the number of days since the Unix
/// epoch (1970-01-01) for a proleptic-Gregorian civil date. Pure integer
/// arithmetic, correct for any year this client will ever see.
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // [0, 399]
    let mp = (i64::from(month) + 9) % 12; // [0, 11]
    let doy = (153 * mp + 2) / 5 + i64::from(day) - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_known_date_renders_with_its_correct_weekday_when_space_permits() {
        // Cross-checked against a real calendar: 2026-10-29 is a Thursday.
        assert_eq!(format_diary_day("2026-10-29", 80), "2026-10-29 (Thu)");
    }

    #[test]
    fn a_narrow_width_falls_back_to_the_bare_date() {
        assert_eq!(format_diary_day("2026-10-29", 10), "2026-10-29");
    }

    #[test]
    fn exactly_the_required_width_still_includes_the_weekday() {
        let with_weekday = "2026-10-29 (Thu)";
        assert_eq!(
            format_diary_day("2026-10-29", with_weekday.len() as u16),
            with_weekday
        );
    }

    #[test]
    fn a_malformed_date_passes_through_unchanged_rather_than_panicking() {
        assert_eq!(format_diary_day("not-a-date", 80), "not-a-date");
        assert_eq!(format_diary_day("2026-13-40", 80), "2026-13-40");
    }

    #[test]
    fn every_day_of_one_known_week_maps_to_the_expected_weekday() {
        for (date, expected) in [
            ("2026-10-25", "Sun"),
            ("2026-10-26", "Mon"),
            ("2026-10-27", "Tue"),
            ("2026-10-28", "Wed"),
            ("2026-10-29", "Thu"),
            ("2026-10-30", "Fri"),
            ("2026-10-31", "Sat"),
        ] {
            assert_eq!(format_diary_day(date, 80), format!("{date} ({expected})"));
        }
    }

    #[test]
    fn the_unix_epoch_itself_is_a_thursday() {
        assert_eq!(format_diary_day("1970-01-01", 80), "1970-01-01 (Thu)");
    }
}
