//! Provider-neutral Compose presentation state for M8 assistant resources.

use std::collections::BTreeSet;

use diary_control_contract::{
    AssistantBackendDto, DraftResultDto, JobDto, MemoryReviewDto, ParagraphSuggestionDto,
    ParagraphSuggestionSummaryDto, WorkflowDto,
};

use crate::resources::{ResourceRow, ResourcesSidebarModel};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AssistantResource {
    Draft,
    MemoryReview,
    MemoryCandidate(usize),
    ConfirmedSource(usize),
    Suggestion(usize),
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AssistantViewModel {
    pub backends: Vec<AssistantBackendDto>,
    pub memory_review: Option<MemoryReviewDto>,
    pub included_memory_ids: BTreeSet<String>,
    pub reviewed_memory_draft: String,
    pub draft_result: Option<DraftResultDto>,
    pub suggestions: Vec<ParagraphSuggestionSummaryDto>,
    pub active_suggestion: Option<ParagraphSuggestionDto>,
    pub active_job: Option<JobDto>,
}

impl AssistantViewModel {
    pub fn load_memory_review(&mut self, review: MemoryReviewDto) {
        self.included_memory_ids = review
            .candidates
            .iter()
            .map(|candidate| candidate.id.clone())
            .collect();
        self.reviewed_memory_draft = if review.reviewed_memory.is_empty() {
            review
                .candidates
                .iter()
                .map(|candidate| candidate.text.as_str())
                .collect::<Vec<_>>()
                .join("\n")
        } else {
            review.reviewed_memory.clone()
        };
        self.memory_review = Some(review);
    }

    pub fn toggle_memory(&mut self, id: &str) {
        if !self.included_memory_ids.remove(id) {
            self.included_memory_ids.insert(id.to_owned());
        }
        if let Some(review) = &self.memory_review {
            self.reviewed_memory_draft = review
                .candidates
                .iter()
                .filter(|candidate| self.included_memory_ids.contains(&candidate.id))
                .map(|candidate| candidate.text.as_str())
                .collect::<Vec<_>>()
                .join("\n");
        }
    }

    #[must_use]
    pub fn resources(
        &self,
        workflow: Option<&WorkflowDto>,
        draft_dirty: bool,
    ) -> (ResourcesSidebarModel, Vec<AssistantResource>) {
        let mut rows = vec![ResourceRow {
            label: "Draft".to_owned(),
            state: if draft_dirty { "unsaved" } else { "saved" }.to_owned(),
            indent: 0,
        }];
        let mut resources = vec![AssistantResource::Draft];
        rows.push(ResourceRow {
            label: "Memory review".to_owned(),
            state: self
                .memory_review
                .as_ref()
                .map_or("not extracted", |review| {
                    if review.stale {
                        "stale"
                    } else if review.reviewed_memory.is_empty() {
                        "needs confirmation"
                    } else {
                        "confirmed"
                    }
                })
                .to_owned(),
            indent: 0,
        });
        resources.push(AssistantResource::MemoryReview);
        if let Some(review) = &self.memory_review {
            for (index, candidate) in review.candidates.iter().enumerate() {
                rows.push(ResourceRow {
                    label: candidate.text.clone(),
                    state: if candidate.stale {
                        "stale"
                    } else if self.included_memory_ids.contains(&candidate.id) {
                        "included"
                    } else {
                        "excluded"
                    }
                    .to_owned(),
                    indent: 1,
                });
                resources.push(AssistantResource::MemoryCandidate(index));
            }
        }
        if let Some(workflow) = workflow {
            for (index, source) in workflow
                .candidates
                .iter()
                .filter(|candidate| candidate.selected)
                .enumerate()
            {
                rows.push(ResourceRow {
                    label: source.source_note_name.clone(),
                    state: "confirmed source".to_owned(),
                    indent: 0,
                });
                resources.push(AssistantResource::ConfirmedSource(index));
            }
        }
        for (index, suggestion) in self.suggestions.iter().enumerate() {
            rows.push(ResourceRow {
                label: format!("Paragraph suggestion {}", index + 1),
                state: suggestion
                    .resolution
                    .as_deref()
                    .unwrap_or(if suggestion.stale { "stale" } else { "ready" })
                    .to_owned(),
                indent: 0,
            });
            resources.push(AssistantResource::Suggestion(index));
        }
        (ResourcesSidebarModel { rows }, resources)
    }

    /// The selected backend as `<label> · <path>`, or `manual` when none is
    /// selected or the selected ID is not listed. Shown in Compose, Work
    /// Status, and the Status Line; the Assistant Widget does not replace it.
    #[must_use]
    pub fn selected_label(&self, workflow: Option<&WorkflowDto>) -> String {
        workflow
            .and_then(|value| value.assistant_backend_id.as_deref())
            .and_then(|id| self.backends.iter().find(|backend| backend.id == id))
            .map_or_else(
                || "manual".to_owned(),
                |backend| format!("{} · {}", backend.display_label, backend.composition_path),
            )
    }

    #[must_use]
    pub fn job_label(&self) -> Option<String> {
        self.active_job
            .as_ref()
            .map(|job| format!("{}: {} ({}%)", job.kind, job.status, job.progress_percent))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resources_list_no_backend_rows_because_the_assistant_widget_owns_selection() {
        let view = AssistantViewModel {
            backends: vec![AssistantBackendDto {
                id: "one".to_owned(),
                display_label: "첫 번째".to_owned(),
                composition_path: "hosted".to_owned(),
                available: true,
                reason_code: None,
                supported_languages: vec!["en-US".to_owned()],
                supported_actions: vec!["draft-entry".to_owned()],
            }],
            ..AssistantViewModel::default()
        };
        let (resources, kinds) = view.resources(None, false);
        assert_eq!(kinds.len(), resources.rows.len());
        assert!(
            !resources
                .rows
                .iter()
                .any(|row| row.label.contains("첫 번째"))
        );
    }

    #[test]
    fn selected_label_names_backend_and_path_or_falls_back_to_manual() {
        let view = AssistantViewModel {
            backends: vec![AssistantBackendDto {
                id: "one".to_owned(),
                display_label: "Codex".to_owned(),
                composition_path: "hosted".to_owned(),
                available: true,
                reason_code: None,
                supported_languages: vec!["en-US".to_owned()],
                supported_actions: vec!["draft-entry".to_owned()],
            }],
            ..AssistantViewModel::default()
        };
        let mut workflow: WorkflowDto = serde_json::from_value(serde_json::json!({
            "id": "w", "diary_day": "2026-10-30", "state": "composing",
            "revision": 1, "buffer_version": 1, "language": "en-US",
            "style": "plain", "assistant_backend_id": "one",
            "composition_path": "hosted", "selection_digest": null,
            "explicit_empty_selection": false, "invalidation_code": null,
            "entry_text": "", "entry_confirmed": false, "local_status": null,
            "notion_status": null, "amendment": null,
            "candidates": []
        }))
        .unwrap();
        assert_eq!(view.selected_label(Some(&workflow)), "Codex · hosted");
        workflow.assistant_backend_id = None;
        workflow.composition_path = None;
        assert_eq!(view.selected_label(Some(&workflow)), "manual");
        assert_eq!(view.selected_label(None), "manual");
    }

    #[test]
    fn two_pass_memory_review_can_exclude_a_candidate_before_confirmation() {
        let mut view = AssistantViewModel::default();
        view.load_memory_review(MemoryReviewDto {
            candidate_set_id: Some("set".to_owned()),
            candidates: vec![
                diary_control_contract::MemoryCandidateDto {
                    id: "a".to_owned(),
                    text: "아침 산책".to_owned(),
                    source_snapshot_digests: vec!["digest-a".to_owned()],
                    stale: false,
                },
                diary_control_contract::MemoryCandidateDto {
                    id: "b".to_owned(),
                    text: "coffee".to_owned(),
                    source_snapshot_digests: vec!["digest-b".to_owned()],
                    stale: false,
                },
            ],
            reviewed_memory: String::new(),
            reviewed_memory_revision: 0,
            stale: false,
        });
        view.toggle_memory("b");
        assert_eq!(view.reviewed_memory_draft, "아침 산책");
    }
}
