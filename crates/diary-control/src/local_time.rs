//! Mac-local rendering of Ayumi's UTC source note times.
//!
//! The offset comes from `/bin/date -r <epoch> +%z`, the same mechanism the
//! system clock uses for the local diary day, so the offset reflects the
//! daylight-saving rules in force at that instant.

use std::collections::HashMap;
use std::process::Command;
use std::sync::{Mutex, OnceLock};

use diary_core::DiaryDay;
use diary_core::source_time::{format_local, local_day, utc_epoch_from_note_name};

/// Offsets only change on half-hour boundaries, so one lookup serves a bucket.
const BUCKET_SECONDS: i64 = 1_800;

fn utc_offset_seconds(epoch: i64) -> Option<i32> {
    static CACHE: OnceLock<Mutex<HashMap<i64, i32>>> = OnceLock::new();
    let bucket = epoch.div_euclid(BUCKET_SECONDS);
    let cache = CACHE.get_or_init(Mutex::default);
    if let Some(offset) = cache.lock().ok()?.get(&bucket) {
        return Some(*offset);
    }
    let output = Command::new("/bin/date")
        .args(["-r", &epoch.to_string(), "+%z"])
        .output()
        .ok()
        .filter(|output| output.status.success())?;
    let offset = parse_offset(String::from_utf8(output.stdout).ok()?.trim())?;
    cache.lock().ok()?.insert(bucket, offset);
    Some(offset)
}

fn parse_offset(value: &str) -> Option<i32> {
    let bytes = value.as_bytes();
    if bytes.len() != 5 || !bytes[1..].iter().all(u8::is_ascii_digit) {
        return None;
    }
    let sign = match bytes[0] {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let hours: i32 = value[1..3].parse().ok()?;
    let minutes: i32 = value[3..5].parse().ok()?;
    Some(sign * (hours * 3_600 + minutes * 60))
}

/// `YYYY-MM-DD HH:MM:SS` in the Mac's local zone, when the name carries a
/// UTC timestamp and the offset is known.
pub(crate) fn local_time(note_name: &str) -> Option<String> {
    let epoch = utc_epoch_from_note_name(note_name)?;
    Some(format_local(epoch, utc_offset_seconds(epoch)?))
}

/// `YYYY-MM-DD HH:MM` in the Mac's local zone for a Unix instant, marked UTC
/// when the offset cannot be read.
pub(crate) fn local_minute(epoch: i64) -> String {
    match utc_offset_seconds(epoch) {
        Some(offset) => format_local(epoch, offset)[..16].to_owned(),
        None => format!("{} UTC", &format_local(epoch, 0)[..16]),
    }
}

/// The Mac-local calendar day of the note, when it can be determined.
pub(crate) fn local_note_day(note_name: &str) -> Option<DiaryDay> {
    let epoch = utc_epoch_from_note_name(note_name)?;
    local_day(epoch, utc_offset_seconds(epoch)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_date_offsets() {
        assert_eq!(parse_offset("+0900"), Some(32_400));
        assert_eq!(parse_offset("-0330"), Some(-12_600));
        assert_eq!(parse_offset("0900"), None);
        assert_eq!(parse_offset("+09:00"), None);
    }

    #[test]
    fn local_minute_has_a_fixed_shape() {
        let stamp = local_minute(1_793_400_000);
        let bare = stamp.strip_suffix(" UTC").unwrap_or(&stamp);
        assert_eq!(bare.len(), 16, "{stamp}");
        assert_eq!(&bare[4..5], "-");
        assert_eq!(&bare[10..11], " ");
        assert_eq!(&bare[13..14], ":");
    }

    #[test]
    fn unparseable_names_have_no_local_time() {
        assert_eq!(local_time("note.md"), None);
        assert_eq!(local_note_day("note.md"), None);
    }
}
