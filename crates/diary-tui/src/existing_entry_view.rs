//! When Preview offers Append and Overwrite for a local file that already
//! exists. Remotes offer the same choice in their own menu (`remote_view`).

use diary_control_contract::LocalPreviewDto;

/// Control indices after the two standing Preview controls.
pub const APPEND_CONTROL: usize = 2;
pub const OVERWRITE_CONTROL: usize = 3;
/// Controls the commands add to Preview when they are offered.
pub const EXTRA_CONTROLS: usize = 2;

/// Append and Overwrite appear only while the preview shows a local file that
/// already exists. The preview already defaults to Append, which keeps what is
/// there, so this is a choice to switch, never a block.
#[must_use]
pub fn offered(preview: Option<&LocalPreviewDto>) -> bool {
    preview
        .is_some_and(|preview| preview.existing_text.is_some() || preview.existing_mode.is_some())
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
