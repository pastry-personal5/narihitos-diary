//! When Preview offers Append and Overwrite for an entry that already exists.

use diary_control_contract::LocalPreviewDto;

/// Control indices after the three standing Preview controls.
pub const APPEND_CONTROL: usize = 3;
pub const OVERWRITE_CONTROL: usize = 4;
/// Controls the commands add to Preview when they are offered.
pub const EXTRA_CONTROLS: usize = 2;

/// Append and Overwrite appear only while the local preview shows an entry
/// that already exists (a local or iCloud conflict) or after one was chosen,
/// so the person can switch. They end once the local file is verified.
#[must_use]
pub fn offered(local_verified: bool, preview: Option<&LocalPreviewDto>) -> bool {
    !local_verified
        && preview.is_some_and(|preview| {
            preview.local_status == "conflict"
                || preview.icloud_status.as_deref() == Some("conflict")
                || preview.existing_mode.is_some()
        })
}

/// The mode word the Control API takes.
#[must_use]
pub const fn mode_for(control: usize) -> Option<&'static str> {
    match control {
        APPEND_CONTROL => Some("append"),
        OVERWRITE_CONTROL => Some("overwrite"),
        _ => None,
    }
}

/// Plain-language line for a chosen mode.
#[must_use]
pub fn mode_line(mode: &str) -> &'static str {
    match mode {
        "append" => {
            "Mode: Append — the existing text is kept and the new text is added under a dated separator"
        }
        "overwrite" => "Mode: Overwrite — the existing file is backed up, then replaced",
        _ => "Mode: unknown",
    }
}
