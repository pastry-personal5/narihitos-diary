//! Amendment view model: the offer on a completed day, the changed-since-
//! publication banner, the replacement preview, the Conflict choices, and the
//! partial and completion states. Everything here is derived from server DTOs;
//! no state is kept and no text is stored, so it renders from rendered state.

use diary_control_contract::{AmendmentPreviewDto, NotionBlockDto, WorkflowDto};

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
    /// The local file holds the new text; Notion is `pending` or `blocked`.
    LocalReplacedNotionOutstanding,
    /// A destination changed since the seed or preview.
    Conflict,
}

/// The only two ways out of an amendment conflict.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConflictChoice {
    Reread,
    AbandonAmendment,
}

impl ConflictChoice {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Reread => "Reread the current file",
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
    let notion = workflow.notion_status.as_deref().unwrap_or("none");
    if local == "conflict" {
        return Some(AmendmentState::Conflict);
    }
    Some(match workflow.state.as_str() {
        "composing" => AmendmentState::Editing,
        "entry-confirmed" => AmendmentState::Confirmed,
        "output-planned" => AmendmentState::Previewed,
        "approved" => AmendmentState::Approved,
        "publishing" => AmendmentState::Replacing,
        "partial" if notion == "conflict" => AmendmentState::Conflict,
        "partial" => AmendmentState::LocalReplacedNotionOutstanding,
        _ => return None,
    })
}

/// Exactly reread and abandon, never a force-overwrite or a new day. For a
/// Notion conflict after the local file was replaced, reread previews the
/// section again against its receipt; the local file keeps the new text either
/// way.
#[must_use]
pub fn conflict_choices(workflow: &WorkflowDto) -> Option<&'static [ConflictChoice]> {
    match amendment_state(workflow)? {
        AmendmentState::Conflict => {
            Some(&[ConflictChoice::Reread, ConflictChoice::AbandonAmendment])
        }
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
        AmendmentState::LocalReplacedNotionOutstanding => {
            "Local file replaced. Notion is still pending; retry Notion when ready."
        }
        AmendmentState::Conflict => "A destination changed. Reread it or abandon the amendment.",
    })
}

/// What a finished day says, including that it can be amended again.
#[must_use]
pub fn completion_text(workflow: &WorkflowDto) -> Option<&'static str> {
    if workflow.state != "completed" || workflow.amendment.is_some() {
        return None;
    }
    Some(match workflow.notion_status.as_deref() {
        Some("verified") => {
            "Completed: the local file and the Notion section are up to date. You can amend it again."
        }
        _ => "Completed: the local file is up to date. You can amend it again.",
    })
}

fn block_lines(heading: &str, blocks: &[NotionBlockDto], lines: &mut Vec<String>) {
    lines.push(heading.to_owned());
    if blocks.is_empty() {
        lines.push("  (nothing)".to_owned());
    }
    for block in blocks {
        let indent = "  ".repeat(usize::from(block.depth) + 1);
        lines.push(format!("{indent}{}", sanitize_for_display(&block.text)));
    }
}

/// Current and replacement content per destination, safe to render.
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
    match &preview.notion {
        Some(notion) => {
            lines.push(format!("Notion section ({})", notion.status));
            block_lines("Current:", &notion.current_blocks, &mut lines);
            block_lines("Replacement:", &notion.replacement_blocks, &mut lines);
            if let Some(reason) = &notion.reason {
                lines.push(format!("Notion: {}", sanitize_for_display(reason)));
            }
        }
        None => lines.push("Notion: not part of this amendment.".to_owned()),
    }
    lines
}

#[cfg(test)]
mod tests {
    use diary_control_contract::{AmendmentDto, AmendmentNotionDto};

    use super::*;

    fn workflow(
        state: &str,
        local: Option<&str>,
        notion: Option<&str>,
        hand: Option<bool>,
    ) -> WorkflowDto {
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
            "notion_status": notion,
            "amendment": hand.map(|hand_edited| AmendmentDto { hand_edited }),
            "candidates": [],
        }))
        .unwrap()
    }

    #[test]
    fn a_completed_day_offers_amendment_and_an_amended_one_does_not() {
        assert!(offers_amendment(&workflow("completed", None, None, None)));
        assert!(!offers_amendment(&workflow(
            "composing",
            None,
            None,
            Some(false)
        )));
        assert!(!offers_amendment(&workflow("partial", None, None, None)));
    }

    #[test]
    fn the_banner_shows_for_a_hand_edit_and_is_not_a_conflict() {
        let edited = workflow("composing", None, None, Some(true));
        assert_eq!(banner(&edited), Some(BANNER));
        assert_eq!(amendment_state(&edited), Some(AmendmentState::Editing));
        assert!(conflict_choices(&edited).is_none());
        assert_eq!(
            banner(&workflow("composing", None, None, Some(false))),
            None
        );
        assert_eq!(banner(&workflow("completed", None, None, None)), None);
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
            let flow = workflow(state, Some("pending"), Some("pending"), Some(false));
            assert_eq!(amendment_state(&flow), Some(expected), "{state}");
            assert!(status_text(&flow).is_some(), "{state}");
        }
        let partial = workflow("partial", Some("verified"), Some("pending"), Some(false));
        assert_eq!(
            amendment_state(&partial),
            Some(AmendmentState::LocalReplacedNotionOutstanding)
        );
        assert!(
            status_text(&partial)
                .unwrap()
                .contains("Local file replaced")
        );
        let blocked = workflow("partial", Some("verified"), Some("blocked"), Some(false));
        assert_eq!(
            amendment_state(&blocked),
            Some(AmendmentState::LocalReplacedNotionOutstanding)
        );
        // Not amending: no amendment state at all.
        assert_eq!(
            amendment_state(&workflow("completed", None, None, None)),
            None
        );
    }

    #[test]
    fn a_completed_day_reports_completion_and_offers_another_amendment() {
        let done = workflow("completed", Some("verified"), Some("verified"), None);
        assert!(completion_text(&done).unwrap().contains("Notion section"));
        assert!(offers_amendment(&done));
        let local_only = workflow("completed", Some("verified"), None, None);
        assert!(!completion_text(&local_only).unwrap().contains("Notion"));
        // Mid-amendment days and unfinished days are not "completed".
        assert!(completion_text(&workflow("completed", None, None, Some(false))).is_none());
        assert!(completion_text(&workflow("partial", None, None, None)).is_none());
    }

    #[test]
    fn a_conflict_offers_exactly_reread_and_abandon() {
        let local = workflow(
            "output-planned",
            Some("conflict"),
            Some("pending"),
            Some(false),
        );
        assert_eq!(amendment_state(&local), Some(AmendmentState::Conflict));
        assert_eq!(
            conflict_choices(&local),
            Some(&[ConflictChoice::Reread, ConflictChoice::AbandonAmendment][..])
        );
        let labels: Vec<_> = conflict_choices(&local)
            .unwrap()
            .iter()
            .map(|choice| choice.label())
            .collect();
        assert_eq!(labels, ["Reread the current file", "Abandon the amendment"]);
        // After the local file was replaced, a Notion conflict offers the same two.
        let notion = workflow("partial", Some("verified"), Some("conflict"), Some(false));
        assert_eq!(
            conflict_choices(&notion),
            Some(&[ConflictChoice::Reread, ConflictChoice::AbandonAmendment][..])
        );
    }

    #[test]
    fn the_preview_shows_current_and_replacement_per_destination_and_sanitizes() {
        let preview = AmendmentPreviewDto {
            workflow: workflow(
                "output-planned",
                Some("pending"),
                Some("pending"),
                Some(false),
            ),
            plan_digest: "d".into(),
            local_target: "/entries/2026/10/2026-10-30.md".into(),
            local_current: "old line\x1b[31m\n".into(),
            local_payload: "new line\n".into(),
            local_status: "pending".into(),
            notion: Some(AmendmentNotionDto {
                current_blocks: vec![NotionBlockDto {
                    kind: "paragraph".into(),
                    text: "old block".into(),
                    depth: 0,
                }],
                replacement_blocks: vec![NotionBlockDto {
                    kind: "paragraph".into(),
                    text: "new block".into(),
                    depth: 1,
                }],
                status: "pending".into(),
                reason: None,
            }),
        };
        let lines = preview_lines(&preview);
        let text = lines.join("\n");
        assert!(!text.contains('\x1b'), "controls never reach the terminal");
        for expected in [
            "Local file: /entries/2026/10/2026-10-30.md",
            "  old line\u{FFFD}[31m",
            "  new line",
            "Notion section (pending)",
            "  old block",
            "    new block",
        ] {
            assert!(
                lines.iter().any(|line| line == expected),
                "{expected}\n{text}"
            );
        }
        let mut local_only = preview;
        local_only.notion = None;
        assert!(
            preview_lines(&local_only)
                .last()
                .unwrap()
                .contains("not part of this amendment")
        );
    }
}
