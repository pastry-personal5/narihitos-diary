//! The tip-style next-step message shown in Work Status (P1-M16): one short
//! sentence telling an end user what to do to reach the next stage. Pure, so
//! it is tested for every stage and state without a terminal. The message
//! only describes controls that already exist; it never decides anything.

use diary_control_contract::WorkflowDto;

use crate::stage::Stage;

/// What the tip depends on, gathered by the caller each frame.
#[derive(Clone, Copy, Debug)]
pub struct TipInput<'a> {
    pub stage: Stage,
    pub workflow: Option<&'a WorkflowDto>,
    /// Notes currently included in the Sources stage (hidden ones excluded).
    pub included_count: usize,
    pub candidate_count: usize,
    /// A source discovery, preparation, or confirmation is running.
    pub source_work_running: bool,
    /// The Draft has text (saved or not).
    pub entry_has_text: bool,
}

#[must_use]
pub fn next_step_tip(input: &TipInput<'_>) -> String {
    let tip = match (input.stage, input.workflow) {
        (Stage::Start, None) => {
            "Choose a day, then select Start/resume chosen day to open its workflow.".to_owned()
        }
        (Stage::Start, Some(_)) => "A workflow is open. Press Ctrl-N to go to Sources.".to_owned(),
        (_, None) => "No workflow is open. Press Ctrl-P until Start, then pick a day.".to_owned(),
        (Stage::Sources, Some(workflow)) => sources_tip(input, workflow),
        (Stage::Compose, Some(workflow)) => match workflow.state.as_str() {
            "sources-confirmed" => {
                "Select Draft in the list and write the entry (or run an assistant action), then press Ctrl-N for Review.".to_owned()
            }
            "composing" if !input.entry_has_text => {
                "Select Draft in the list and write the entry, then press Ctrl-N for Review."
                    .to_owned()
            }
            "composing" => "Keep writing, or press Ctrl-N to review the entry.".to_owned(),
            _ => "Press Ctrl-N to continue to Review.".to_owned(),
        },
        (Stage::Review, Some(workflow)) => {
            if workflow.entry_confirmed {
                "The entry is confirmed. Press Ctrl-N to go to Preview.".to_owned()
            } else {
                "Read the entry, then select Confirm complete entry to unlock Preview.".to_owned()
            }
        }
        (Stage::Preview, Some(workflow)) => match workflow.state.as_str() {
            "entry-confirmed" => {
                "Select Generate/refresh preview to see exactly what will be written.".to_owned()
            }
            "output-planned" => {
                "Check the preview, then select Approve exact plan to unlock Publish.".to_owned()
            }
            _ => "The plan is approved. Press Ctrl-N to go to Publish.".to_owned(),
        },
        (Stage::Publish, Some(workflow)) => match workflow.state.as_str() {
            "completed" => "Published. Press Ctrl-P to revisit a stage, or amend from here.".to_owned(),
            "partial" => "Part of the output is still pending. Select Publish to finish it.".to_owned(),
            _ => "Select Publish approved destination to write the entry.".to_owned(),
        },
    };
    format!("Tip: {tip}")
}

fn sources_tip(input: &TipInput<'_>, workflow: &WorkflowDto) -> String {
    if input.source_work_running {
        return "Working on your notes; the next step appears when it finishes.".to_owned();
    }
    if workflow.state == "sources-confirmed" {
        return "Your source list is confirmed. Press Ctrl-N to go to Compose.".to_owned();
    }
    if input.included_count > 0 {
        "Select Confirm Source List to continue to Compose.".to_owned()
    } else if input.candidate_count > 0 {
        "Include notes (Include this note, or i), or select Continue without notes.".to_owned()
    } else {
        "No notes found. Refresh notes or Add a date, or select Continue without notes.".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workflow(state: &str, entry_confirmed: bool) -> WorkflowDto {
        serde_json::from_value(serde_json::json!({
            "id": "wf_20261030",
            "diary_day": "2026-10-30",
            "state": state,
            "revision": 1,
            "buffer_version": 0,
            "language": null,
            "style": "concise",
            "assistant_backend_id": null,
            "composition_path": null,
            "selection_digest": null,
            "explicit_empty_selection": false,
            "invalidation_code": null,
            "entry_text": "",
            "entry_confirmed": entry_confirmed,
            "local_status": null,
            "notion_status": null,
            "icloud_status": null,
            "amendment": null,
            "candidates": []
        }))
        .expect("synthetic workflow")
    }

    fn tip(
        stage: Stage,
        workflow: Option<&WorkflowDto>,
        included: usize,
        candidates: usize,
        running: bool,
        text: bool,
    ) -> String {
        next_step_tip(&TipInput {
            stage,
            workflow,
            included_count: included,
            candidate_count: candidates,
            source_work_running: running,
            entry_has_text: text,
        })
    }

    #[test]
    fn every_tip_is_a_single_tip_prefixed_line() {
        let states = [
            "started",
            "discovered",
            "sources-confirmed",
            "composing",
            "entry-confirmed",
            "output-planned",
            "approved",
            "partial",
            "completed",
        ];
        for stage in crate::stage::ALL_STAGES {
            for state in states {
                let value = workflow(state, state == "entry-confirmed");
                let text = tip(stage, Some(&value), 1, 2, false, true);
                assert!(text.starts_with("Tip: "), "{stage:?}/{state}: {text}");
                assert!(!text.contains('\n'), "{stage:?}/{state}");
            }
            assert!(tip(stage, None, 0, 0, false, false).starts_with("Tip: "));
        }
    }

    #[test]
    fn start_without_a_workflow_says_how_to_open_one() {
        assert!(tip(Stage::Start, None, 0, 0, false, false).contains("Start/resume"));
    }

    #[test]
    fn sources_tip_follows_inclusion_and_confirmation() {
        let open = workflow("discovered", false);
        assert!(
            tip(Stage::Sources, Some(&open), 2, 3, false, false).contains("Confirm Source List")
        );
        assert!(tip(Stage::Sources, Some(&open), 0, 3, false, false).contains("Include notes"));
        let empty = tip(Stage::Sources, Some(&open), 0, 0, false, false);
        assert!(empty.contains("Continue without notes"));
        assert!(tip(Stage::Sources, Some(&open), 1, 3, true, false).contains("Working"));
        let confirmed = workflow("sources-confirmed", false);
        assert!(tip(Stage::Sources, Some(&confirmed), 1, 3, false, false).contains("Ctrl-N"));
    }

    #[test]
    fn later_stages_name_the_control_that_unlocks_the_next_one() {
        let composing = workflow("composing", false);
        assert!(tip(Stage::Compose, Some(&composing), 0, 0, false, false).contains("write"));
        assert!(tip(Stage::Compose, Some(&composing), 0, 0, false, true).contains("review"));
        let review = workflow("composing", false);
        assert!(
            tip(Stage::Review, Some(&review), 0, 0, false, true).contains("Confirm complete entry")
        );
        let planned = workflow("output-planned", true);
        assert!(
            tip(Stage::Preview, Some(&planned), 0, 0, false, true).contains("Approve exact plan")
        );
        let approved = workflow("approved", true);
        assert!(
            tip(Stage::Publish, Some(&approved), 0, 0, false, true).contains("Publish approved")
        );
    }
}
