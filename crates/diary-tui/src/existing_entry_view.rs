//! When Preview offers Append and Overwrite for an entry that already exists.

use diary_control_contract::{LocalPreviewDto, NotionPreviewDto};

/// Control indices after the three standing Preview controls.
pub const APPEND_CONTROL: usize = 3;
pub const OVERWRITE_CONTROL: usize = 4;
/// Controls the commands add to Preview when they are offered.
pub const EXTRA_CONTROLS: usize = 2;

/// Append and Overwrite appear only while the preview of the current phase
/// shows an entry that already exists, or after one was chosen so the person
/// can switch. Before the local file is verified that is the local preview (a
/// local or iCloud conflict); afterwards it is the Notion preview (a
/// same-day section conflict).
#[must_use]
pub fn offered(
    local_verified: bool,
    preview: Option<&LocalPreviewDto>,
    notion: Option<&NotionPreviewDto>,
) -> bool {
    if local_verified {
        notion.is_some_and(|notion| {
            (notion.status == "conflict"
                && notion.reason.as_deref() == Some("notion-same-day-conflict"))
                || notion.existing_mode.is_some()
        })
    } else {
        preview.is_some_and(|preview| {
            preview.local_status == "conflict"
                || preview.icloud_status.as_deref() == Some("conflict")
                || preview.existing_mode.is_some()
        })
    }
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
