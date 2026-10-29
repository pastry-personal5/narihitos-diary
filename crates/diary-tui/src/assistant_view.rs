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
    Backend(usize),
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
        for (index, backend) in self.backends.iter().enumerate() {
            let selected = workflow.and_then(|value| value.assistant_backend_id.as_deref())
                == Some(backend.id.as_str());
            let state = if selected && backend.available {
                "selected"
            } else if selected {
                "selected; unavailable"
            } else if !backend.available {
                "unavailable"
            } else if workflow
                .and_then(|value| value.language.as_deref())
                .is_some_and(|language| !backend.supported_languages.iter().any(|v| v == language))
            {
                "unsupported language"
            } else {
                "available"
            };
            rows.push(ResourceRow {
                label: format!("{} · {}", backend.display_label, backend.composition_path),
                state: state.to_owned(),
                indent: 1,
            });
            resources.push(AssistantResource::Backend(index));
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

    fn backend(id: &str, label: &str, available: bool) -> AssistantBackendDto {
        AssistantBackendDto {
            id: id.to_owned(),
            display_label: label.to_owned(),
            composition_path: "hosted".to_owned(),
            available,
            reason_code: (!available).then(|| "offline".to_owned()),
            supported_languages: vec!["en-US".to_owned(), "ko-KR".to_owned()],
            supported_actions: vec!["extract-memories".to_owned()],
        }
    }

    #[test]
    fn backend_collection_is_provider_neutral_scrollable_and_explicit() {
        let view = AssistantViewModel {
            backends: vec![
                backend("one", "첫 번째", true),
                backend("two", "Second", false),
                backend("three", "Third", true),
            ],
            ..AssistantViewModel::default()
        };
        let (resources, kinds) = view.resources(None, false);
        assert_eq!(kinds.len(), resources.rows.len());
        assert!(
            resources
                .rows
                .iter()
                .any(|row| row.label.contains("첫 번째"))
        );
        assert!(resources.rows.iter().any(|row| row.state == "unavailable"));
        assert!(matches!(kinds[4], AssistantResource::Backend(2)));
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
