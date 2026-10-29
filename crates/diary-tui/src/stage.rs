//! Pure mapping from server workflow state to the six canonical user-facing
//! stages. See `docs/phase-1/milestone-07-tui-manual-workflow-architecture.md`
//! ("Canonical stages in M7") for the table this implements.

use diary_control_contract::WorkflowDto;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Stage {
    Start,
    Sources,
    Compose,
    Review,
    Preview,
    Publish,
}

pub const ALL_STAGES: [Stage; 6] = [
    Stage::Start,
    Stage::Sources,
    Stage::Compose,
    Stage::Review,
    Stage::Preview,
    Stage::Publish,
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Progress {
    Locked,
    Available,
    Stale,
    Done,
}

/// A fully computed, presentation-only stage model: progress for every
/// canonical stage plus which one is `current`. Navigation can change
/// `current`; it never derives from or mutates server state itself.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StageModel {
    pub progress: [Progress; 6],
    pub default_current: Stage,
}

impl StageModel {
    #[must_use]
    pub fn progress_of(&self, stage: Stage) -> Progress {
        self.progress[stage_index(stage)]
    }
}

const fn stage_index(stage: Stage) -> usize {
    match stage {
        Stage::Start => 0,
        Stage::Sources => 1,
        Stage::Compose => 2,
        Stage::Review => 3,
        Stage::Preview => 4,
        Stage::Publish => 5,
    }
}

/// Invalidation reasons the core can report. `Other` is a fail-closed bucket
/// for any value not in the known set; it must not be treated as "no
/// invalidation".
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InvalidationReason {
    None,
    SelectionChanged,
    SourceChanged,
    FreshnessUnverifiable,
    EntryEdited,
    CompositionChoiceChanged,
    Other,
}

fn invalidation_reason(code: Option<&str>) -> InvalidationReason {
    match code {
        None => InvalidationReason::None,
        Some("selection-changed") => InvalidationReason::SelectionChanged,
        Some("source-changed") => InvalidationReason::SourceChanged,
        Some("freshness-unverifiable") => InvalidationReason::FreshnessUnverifiable,
        Some("entry-edited") => InvalidationReason::EntryEdited,
        Some("composition-choice-changed") => InvalidationReason::CompositionChoiceChanged,
        Some(_) => InvalidationReason::Other,
    }
}

/// Maps a server workflow representation to the canonical stage model. No
/// attached workflow maps to `None` (the Start-only state); the caller
/// renders every later stage locked in that case.
#[must_use]
pub fn map_workflow(
    workflow: Option<&WorkflowDto>,
    has_recovered_entry_buffer: bool,
) -> StageModel {
    use Progress::{Available, Done, Locked, Stale};

    let Some(workflow) = workflow else {
        return StageModel {
            progress: [Available, Locked, Locked, Locked, Locked, Locked],
            default_current: Stage::Start,
        };
    };

    let reason = invalidation_reason(workflow.invalidation_code.as_deref());

    match workflow.state.as_str() {
        "started" | "discovered" if matches!(reason, InvalidationReason::None) => StageModel {
            progress: [Done, Available, Locked, Locked, Locked, Locked],
            default_current: Stage::Sources,
        },
        "discovered"
            if matches!(
                reason,
                InvalidationReason::SelectionChanged
                    | InvalidationReason::SourceChanged
                    | InvalidationReason::FreshnessUnverifiable
            ) =>
        {
            let compose = if has_recovered_entry_buffer {
                Stale
            } else {
                Locked
            };
            StageModel {
                progress: [Done, Stale, compose, Locked, Locked, Locked],
                default_current: Stage::Sources,
            }
        }
        "sources-confirmed" => StageModel {
            progress: [Done, Done, Available, Locked, Locked, Locked],
            default_current: Stage::Compose,
        },
        "composing"
            if matches!(
                reason,
                InvalidationReason::EntryEdited | InvalidationReason::CompositionChoiceChanged
            ) =>
        {
            StageModel {
                progress: [Done, Done, Available, Stale, Locked, Locked],
                default_current: Stage::Compose,
            }
        }
        "composing" => StageModel {
            progress: [Done, Done, Available, Available, Locked, Locked],
            default_current: Stage::Compose,
        },
        "entry-confirmed" | "output-planned" | "approved" | "publishing" | "partial"
        | "completed" => StageModel {
            progress: [Done, Done, Done, Done, Locked, Locked],
            default_current: Stage::Review,
        },
        _ => StageModel {
            // Fail closed: an unknown state is never treated as Start.
            progress: [Locked, Locked, Locked, Locked, Locked, Locked],
            default_current: Stage::Start,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workflow(state: &str, invalidation_code: Option<&str>) -> WorkflowDto {
        serde_json::from_value(serde_json::json!({
            "id": "wf_20261029",
            "diary_day": "2026-10-29",
            "state": state,
            "revision": 1,
            "buffer_version": 0,
            "language": null,
            "style": "concise",
            "assistant_backend_id": null,
            "composition_path": null,
            "selection_digest": null,
            "explicit_empty_selection": false,
            "invalidation_code": invalidation_code,
            "entry_text": "",
            "entry_confirmed": false,
            "candidates": []
        }))
        .unwrap()
    }

    #[test]
    fn no_attached_workflow_is_start_only() {
        let model = map_workflow(None, false);
        assert_eq!(model.default_current, Stage::Start);
        assert_eq!(model.progress_of(Stage::Start), Progress::Available);
        assert_eq!(model.progress_of(Stage::Sources), Progress::Locked);
    }

    #[test]
    fn every_core_invalidation_reason_drops_discovered_to_stale_sources() {
        for reason in [
            "selection-changed",
            "source-changed",
            "freshness-unverifiable",
        ] {
            let model = map_workflow(Some(&workflow("discovered", Some(reason))), false);
            assert_eq!(
                model.progress_of(Stage::Sources),
                Progress::Stale,
                "reason {reason} must mark Sources stale"
            );
            assert_eq!(model.progress_of(Stage::Compose), Progress::Locked);
        }
    }

    #[test]
    fn a_recovered_entry_buffer_keeps_compose_stale_rather_than_locked() {
        let model = map_workflow(Some(&workflow("discovered", Some("source-changed"))), true);
        assert_eq!(model.progress_of(Stage::Compose), Progress::Stale);
    }

    #[test]
    fn entry_edited_and_composition_choice_changed_are_treated_identically() {
        let edited = map_workflow(Some(&workflow("composing", Some("entry-edited"))), false);
        let choice = map_workflow(
            Some(&workflow("composing", Some("composition-choice-changed"))),
            false,
        );
        assert_eq!(edited.progress_of(Stage::Review), Progress::Stale);
        assert_eq!(choice.progress_of(Stage::Review), Progress::Stale);
        assert_eq!(edited.default_current, Stage::Compose);
        assert_eq!(choice.default_current, Stage::Compose);
    }

    #[test]
    fn later_states_render_read_only_progress_with_preview_and_publish_locked() {
        for state in [
            "entry-confirmed",
            "output-planned",
            "approved",
            "publishing",
            "partial",
            "completed",
        ] {
            let model = map_workflow(Some(&workflow(state, None)), false);
            assert_eq!(model.progress_of(Stage::Review), Progress::Done, "{state}");
            assert_eq!(
                model.progress_of(Stage::Preview),
                Progress::Locked,
                "{state}"
            );
            assert_eq!(
                model.progress_of(Stage::Publish),
                Progress::Locked,
                "{state}"
            );
        }
    }

    #[test]
    fn an_unknown_state_fails_closed_rather_than_defaulting_to_start_progress() {
        let model = map_workflow(Some(&workflow("mystery-future-state", None)), false);
        assert!(model.progress.iter().all(|p| *p == Progress::Locked));
    }

    #[test]
    fn sources_confirmed_unlocks_compose_only() {
        let model = map_workflow(Some(&workflow("sources-confirmed", None)), false);
        assert_eq!(model.progress_of(Stage::Compose), Progress::Available);
        assert_eq!(model.progress_of(Stage::Review), Progress::Locked);
    }
}
