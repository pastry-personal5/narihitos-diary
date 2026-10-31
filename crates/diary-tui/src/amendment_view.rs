//! Amendment view model: the offer on a completed day, the changed-since-
//! publication banner, the replacement preview, and the completion states.
//! Everything here is derived from server DTOs; no state is kept and no text
//! is stored, so it renders from rendered state. An amendment replaces the
//! local file only; a remote is pushed again afterward.

use diary_control_contract::{AmendmentPreviewDto, WorkflowDto};

use crate::sanitize::sanitize_for_display;

pub const BANNER: &str =
    "This entry changed since it was published. The editor shows the file's current text.";

/// Where an amendment stands, in the order a person moves through it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AmendmentState {
    Editing,
    Confirmed,
    Previewed,
    Approved,
    Replacing,
    /// The file changed after the replacement was approved. Previewing again
    /// shows it as it is now.
    FileChanged,
}

/// The one way out of an amendment that is not "keep going".
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AmendmentChoice {
    AbandonAmendment,
}

impl AmendmentChoice {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::AbandonAmendment => "Abandon the amendment",
        }
    }
}

/// A `Completed` day offers an amendment instead of the existing-entry check.
#[must_use]
pub fn offers_amendment(workflow: &WorkflowDto) -> bool {
    workflow.state == "completed" && workflow.amendment.is_none()
}

/// Present when the seeded file no longer matched its receipt (body or front
/// matter). It is information, not a conflict.
#[must_use]
pub fn banner(workflow: &WorkflowDto) -> Option<&'static str> {
    workflow
        .amendment
        .as_ref()
        .filter(|amendment| amendment.hand_edited)
        .map(|_| BANNER)
}

#[must_use]
pub fn amendment_state(workflow: &WorkflowDto) -> Option<AmendmentState> {
    workflow.amendment.as_ref()?;
    let local = workflow.local_status.as_deref().unwrap_or("none");
    if local == "changed" {
        return Some(AmendmentState::FileChanged);
    }
    Some(match workflow.state.as_str() {
        "composing" => AmendmentState::Editing,
        "entry-confirmed" => AmendmentState::Confirmed,
        "output-planned" => AmendmentState::Previewed,
        "approved" => AmendmentState::Approved,
        "publishing" => AmendmentState::Replacing,
        _ => return None,
    })
}

/// Abandoning is always available once the file has changed under an approved
/// plan; previewing again is the main control and needs no choice.
#[must_use]
pub fn amendment_choices(workflow: &WorkflowDto) -> Option<&'static [AmendmentChoice]> {
    match amendment_state(workflow)? {
        AmendmentState::FileChanged => Some(&[AmendmentChoice::AbandonAmendment]),
        _ => None,
    }
}

/// One-line, plain-language status for the status area.
#[must_use]
pub fn status_text(workflow: &WorkflowDto) -> Option<&'static str> {
    Some(match amendment_state(workflow)? {
        AmendmentState::Editing => "Amending: edit the saved text, then confirm it.",
        AmendmentState::Confirmed => "Amending: preview the replacement.",
        AmendmentState::Previewed => {
            "Amending: review current and replacement content, then approve."
        }
        AmendmentState::Approved => "Amending: approved. Replace when ready.",
        AmendmentState::Replacing => "Replacing... the day is locked until this is settled.",
        AmendmentState::FileChanged => {
            "The file changed after you approved. Preview the replacement again."
        }
    })
}

/// What a finished day says, including that it can be amended again.
#[must_use]
pub fn completion_text(workflow: &WorkflowDto) -> Option<&'static str> {
    if workflow.state != "completed" || workflow.amendment.is_some() {
        return None;
    }
    Some("Completed: the local file is up to date. You can amend it again.")
}

#[must_use]
pub fn preview_lines(preview: &AmendmentPreviewDto) -> Vec<String> {
    let mut lines = vec![format!("Local file: {}", preview.local_target)];
    lines.push("Current:".to_owned());
    lines.extend(
        sanitize_for_display(&preview.local_current)
            .lines()
            .map(|line| format!("  {line}")),
    );
    lines.push("Replacement:".to_owned());
    lines.extend(
        sanitize_for_display(&preview.local_payload)
            .lines()
            .map(|line| format!("  {line}")),
    );
    lines
}

#[cfg(test)]
mod tests {
    use diary_control_contract::AmendmentDto;

    use super::*;

    fn workflow(state: &str, local: Option<&str>, hand: Option<bool>) -> WorkflowDto {
        serde_json::from_value(serde_json::json!({
            "id": "2026-10-30",
            "diary_day": "2026-10-30",
            "state": state,
            "revision": 3,
            "buffer_version": 1,
            "language": "en-US",
            "style": "concise",
            "assistant_backend_id": null,
            "composition_path": null,
            "selection_digest": null,
            "explicit_empty_selection": true,
            "invalidation_code": null,
            "entry_text": "Invented text.",
            "entry_confirmed": false,
            "local_status": local,
            "notion_status": null,
            "amendment": hand.map(|hand_edited| AmendmentDto { hand_edited }),
            "candidates": [],
        }))
        .unwrap()
    }

    #[test]
    fn a_completed_day_offers_amendment_and_an_amended_one_does_not() {
        assert!(offers_amendment(&workflow("completed", None, None)));
        assert!(!offers_amendment(&workflow("composing", None, Some(false))));
        assert!(!offers_amendment(&workflow("partial", None, None)));
    }

    #[test]
    fn the_banner_shows_for_a_hand_edit_and_is_not_a_conflict() {
        let edited = workflow("composing", None, Some(true));
        assert_eq!(banner(&edited), Some(BANNER));
        assert_eq!(amendment_state(&edited), Some(AmendmentState::Editing));
        assert!(amendment_choices(&edited).is_none());
        assert_eq!(banner(&workflow("composing", None, Some(false))), None);
        assert_eq!(banner(&workflow("completed", None, None)), None);
    }

    #[test]
    fn states_follow_the_workflow_and_every_state_has_status_text() {
        let cases = [
            ("composing", AmendmentState::Editing),
            ("entry-confirmed", AmendmentState::Confirmed),
            ("output-planned", AmendmentState::Previewed),
            ("approved", AmendmentState::Approved),
            ("publishing", AmendmentState::Replacing),
        ];
        for (state, expected) in cases {
            let flow = workflow(state, Some("pending"), Some(false));
            assert_eq!(amendment_state(&flow), Some(expected), "{state}");
            assert!(status_text(&flow).is_some(), "{state}");
        }
        // Not amending: no amendment state at all.
        assert_eq!(amendment_state(&workflow("completed", None, None)), None);
    }

    #[test]
    fn a_completed_day_reports_completion_and_offers_another_amendment() {
        let done = workflow("completed", Some("verified"), None);
        assert!(completion_text(&done).unwrap().contains("local file"));
        assert!(offers_amendment(&done));
        // Mid-amendment days and unfinished days are not "completed".
        assert!(completion_text(&workflow("completed", None, Some(false))).is_none());
        assert!(completion_text(&workflow("partial", None, None)).is_none());
    }

    #[test]
    fn a_changed_file_says_to_preview_again_and_only_offers_abandon() {
        let changed = workflow("output-planned", Some("changed"), Some(false));
        assert_eq!(amendment_state(&changed), Some(AmendmentState::FileChanged));
        assert!(
            status_text(&changed)
                .unwrap()
                .contains("Preview the replacement again")
        );
        assert_eq!(
            amendment_choices(&changed),
            Some(&[AmendmentChoice::AbandonAmendment][..])
        );
        assert_eq!(
            AmendmentChoice::AbandonAmendment.label(),
            "Abandon the amendment"
        );
    }

    #[test]
    fn the_preview_shows_the_current_and_replacement_file_and_sanitizes() {
        let preview = AmendmentPreviewDto {
            workflow: workflow("output-planned", Some("pending"), Some(false)),
            plan_digest: "d".into(),
            local_target: "/entries/2026/10/2026-10-30.md".into(),
            local_current: "old line\x1b[31m\n".into(),
            local_payload: "new line\n".into(),
            local_status: "pending".into(),
        };
        let lines = preview_lines(&preview);
        let text = lines.join("\n");
        assert!(!text.contains('\x1b'), "controls never reach the terminal");
        for expected in [
            "Local file: /entries/2026/10/2026-10-30.md",
            "  old line\u{FFFD}[31m",
            "  new line",
        ] {
            assert!(
                lines.iter().any(|line| line == expected),
                "{expected}\n{text}"
            );
        }
        assert!(!text.to_lowercase().contains("notion"));
        assert!(!text.to_lowercase().contains("icloud"));
    }
}
