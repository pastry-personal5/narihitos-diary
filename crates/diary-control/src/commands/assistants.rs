//! Assistants use cases.

use super::*;

impl CommandService {
    pub fn set_writing_settings(
        &self,
        day: &DiaryDay,
        expected_revision: u64,
        language: WritingLanguage,
        style: WritingStyle,
        assistant_selection: Option<AssistantSelection>,
    ) -> Result<Workflow, CommandError> {
        self.ensure_clear(day)?;
        let mut workflow = self.load_workflow(day)?;
        ensure_revision(&workflow, expected_revision)?;
        workflow
            .set_writing_settings(
                expected_revision,
                language,
                style,
                assistant_selection.clone(),
                self.clock.now(),
            )
            .map_err(CommandError::Workflow)?;
        let preferences = Preferences {
            last_language: Some(language),
            last_assistant_backend_id: assistant_selection.map(|selection| selection.backend_id),
        };
        self.store
            .compare_and_swap_with_preferences(expected_revision, &workflow, &preferences)
            .map_err(CommandError::Store)?;
        Ok(workflow)
    }

    pub fn set_assistant_selection(
        &self,
        day: &DiaryDay,
        expected_revision: u64,
        assistant_selection: Option<AssistantSelection>,
    ) -> Result<Workflow, CommandError> {
        self.ensure_clear(day)?;
        let mut workflow = self.load_workflow(day)?;
        ensure_revision(&workflow, expected_revision)?;
        workflow
            .set_assistant_selection(
                expected_revision,
                assistant_selection.clone(),
                self.clock.now(),
            )
            .map_err(CommandError::Workflow)?;
        let mut preferences = self.store.load_preferences().map_err(CommandError::Store)?;
        preferences.last_assistant_backend_id =
            assistant_selection.map(|selection| selection.backend_id);
        self.store
            .compare_and_swap_with_preferences(expected_revision, &workflow, &preferences)
            .map_err(CommandError::Store)?;
        Ok(workflow)
    }

    pub fn extract_memories(
        &self,
        day: &DiaryDay,
        expected_revision: u64,
        artifact_id: String,
        registry: &DraftAssistantRegistry,
        cancelled: &AtomicBool,
        commit: impl FnOnce() -> bool,
    ) -> Result<Workflow, CommandError> {
        self.ensure_clear(day)?;
        let mut workflow = self.load_workflow(day)?;
        ensure_revision(&workflow, expected_revision)?;
        self.ensure_assistant_sources_fresh(&mut workflow, FreshnessCheckpoint::BeforeExtraction)?;
        let selection = workflow
            .assistant_selection()
            .cloned()
            .ok_or(CommandError::Workflow(
                WorkflowError::AssistantBackendRequired,
            ))?;
        let source_digest = workflow
            .selection_digest()
            .cloned()
            .ok_or(CommandError::Workflow(
                WorkflowError::AssistantActionsDisabled,
            ))?;
        let request = workflow
            .memory_extraction_request()
            .map_err(CommandError::Workflow)?;
        let response = registry
            .invoke(
                &selection.backend_id,
                WritingLanguage::EnUs,
                &AssistantRequest::ExtractMemories(request),
                cancelled,
            )
            .map_err(map_registry_error)?;
        let AssistantResponse::Memories(result) = response else {
            return Err(CommandError::Assistant(DraftAssistantError::InvalidOutput));
        };
        let mut current = self.load_workflow(day)?;
        self.ensure_assistant_sources_fresh(&mut current, FreshnessCheckpoint::BeforeExtraction)?;
        if !commit() || cancelled.load(std::sync::atomic::Ordering::Acquire) {
            return Err(CommandError::Cancelled);
        }
        let revision = current.revision();
        current
            .commit_memory_extraction(
                artifact_id,
                ArtifactProvenance {
                    backend_id: selection.backend_id,
                    composition_path: selection.composition_path,
                },
                &source_digest,
                &result,
                self.clock.now(),
            )
            .map_err(CommandError::Workflow)?;
        self.store
            .compare_and_swap(revision, None, &current)
            .map_err(CommandError::Store)?;
        Ok(current)
    }

    pub fn replace_reviewed_memory(
        &self,
        day: &DiaryDay,
        expected_revision: u64,
        artifact_id: String,
        candidate_set_id: &str,
        text: String,
    ) -> Result<Workflow, CommandError> {
        self.ensure_clear(day)?;
        let mut workflow = self.load_workflow(day)?;
        workflow
            .replace_reviewed_memory(
                expected_revision,
                artifact_id,
                candidate_set_id,
                text,
                self.clock.now(),
            )
            .map_err(CommandError::Workflow)?;
        self.store
            .compare_and_swap(expected_revision, None, &workflow)
            .map_err(CommandError::Store)?;
        Ok(workflow)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draft_entry(
        &self,
        day: &DiaryDay,
        expected_revision: u64,
        artifact_id: String,
        include_prior_draft: bool,
        registry: &DraftAssistantRegistry,
        cancelled: &AtomicBool,
        commit: impl FnOnce() -> bool,
    ) -> Result<Workflow, CommandError> {
        self.ensure_clear(day)?;
        let mut workflow = self.load_workflow(day)?;
        ensure_revision(&workflow, expected_revision)?;
        self.ensure_assistant_sources_fresh(&mut workflow, FreshnessCheckpoint::BeforeDrafting)?;
        let selection = workflow
            .assistant_selection()
            .cloned()
            .ok_or(CommandError::Workflow(
                WorkflowError::AssistantBackendRequired,
            ))?;
        let (request, binding) = workflow
            .draft_entry_request(include_prior_draft)
            .map_err(CommandError::Workflow)?;
        let language = request.language();
        let response = registry
            .invoke(
                &selection.backend_id,
                language,
                &AssistantRequest::DraftEntry(request),
                cancelled,
            )
            .map_err(map_registry_error)?;
        let AssistantResponse::EntryDraft(result) = response else {
            return Err(CommandError::Assistant(DraftAssistantError::InvalidOutput));
        };
        let mut current = self.load_workflow(day)?;
        self.ensure_assistant_sources_fresh(&mut current, FreshnessCheckpoint::BeforeDrafting)?;
        if !commit() || cancelled.load(std::sync::atomic::Ordering::Acquire) {
            return Err(CommandError::Cancelled);
        }
        let revision = current.revision();
        let buffer = current.entry().version;
        current
            .commit_draft_proposal(
                artifact_id,
                ArtifactProvenance {
                    backend_id: selection.backend_id,
                    composition_path: selection.composition_path,
                },
                binding,
                &result,
                self.clock.now(),
            )
            .map_err(CommandError::Workflow)?;
        self.store
            .compare_and_swap(revision, Some(buffer), &current)
            .map_err(CommandError::Store)?;
        Ok(current)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn review_paragraph(
        &self,
        day: &DiaryDay,
        expected_revision: u64,
        suggestion_id: String,
        range: ReflectionRange,
        registry: &DraftAssistantRegistry,
        cancelled: &AtomicBool,
        commit: impl FnOnce() -> bool,
    ) -> Result<Workflow, CommandError> {
        self.ensure_clear(day)?;
        let mut workflow = self.load_workflow(day)?;
        ensure_revision(&workflow, expected_revision)?;
        self.ensure_assistant_sources_fresh(&mut workflow, FreshnessCheckpoint::BeforeDrafting)?;
        let selection = workflow
            .assistant_selection()
            .cloned()
            .ok_or(CommandError::Workflow(
                WorkflowError::AssistantBackendRequired,
            ))?;
        let (request, binding) = workflow
            .paragraph_review_request(range)
            .map_err(CommandError::Workflow)?;
        let language = request.language();
        let response = registry
            .invoke(
                &selection.backend_id,
                language,
                &AssistantRequest::ReviewParagraph(request),
                cancelled,
            )
            .map_err(map_registry_error)?;
        let AssistantResponse::ParagraphReview(result) = response else {
            return Err(CommandError::Assistant(DraftAssistantError::InvalidOutput));
        };
        let mut current = self.load_workflow(day)?;
        self.ensure_assistant_sources_fresh(&mut current, FreshnessCheckpoint::BeforeDrafting)?;
        if !commit() || cancelled.load(std::sync::atomic::Ordering::Acquire) {
            return Err(CommandError::Cancelled);
        }
        let revision = current.revision();
        current
            .commit_paragraph_suggestion(
                suggestion_id,
                ArtifactProvenance {
                    backend_id: selection.backend_id,
                    composition_path: selection.composition_path,
                },
                binding,
                &result,
                self.clock.now(),
            )
            .map_err(CommandError::Workflow)?;
        self.store
            .compare_and_swap(revision, None, &current)
            .map_err(CommandError::Store)?;
        Ok(current)
    }

    pub fn resolve_paragraph_suggestion(
        &self,
        day: &DiaryDay,
        expected_revision: u64,
        suggestion_id: &str,
        resolution: diary_core::SuggestionResolution,
    ) -> Result<Workflow, CommandError> {
        self.ensure_clear(day)?;
        let mut workflow = self.load_workflow(day)?;
        let buffer = workflow.entry().version;
        workflow
            .resolve_paragraph_suggestion(
                expected_revision,
                suggestion_id,
                resolution,
                self.clock.now(),
            )
            .map_err(CommandError::Workflow)?;
        self.store
            .compare_and_swap(expected_revision, Some(buffer), &workflow)
            .map_err(CommandError::Store)?;
        Ok(workflow)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn refine_source(
        &self,
        day: &DiaryDay,
        expected_revision: u64,
        source: &SourceRef,
        expected_digest: &diary_core::Digest,
        registry: &DraftAssistantRegistry,
        cancelled: &AtomicBool,
        commit: impl FnOnce() -> bool,
    ) -> Result<Workflow, CommandError> {
        self.ensure_clear(day)?;
        let workflow = self.load_workflow(day)?;
        ensure_revision(&workflow, expected_revision)?;
        if !workflow
            .candidates()
            .iter()
            .any(|value| &value.source == source)
        {
            return Err(CommandError::NotFound);
        }
        let selection = workflow
            .assistant_selection()
            .cloned()
            .ok_or(CommandError::Workflow(
                WorkflowError::AssistantBackendRequired,
            ))?;
        let selected = match self
            .reader
            .read_selected(source)
            .map_err(CommandError::Source)?
        {
            SourceReadOutcome::Read(value) => value,
            _ => return Err(CommandError::Source(SourceAccessError::Unavailable)),
        };
        if &selected.snapshot.stamp.content_digest != expected_digest {
            return Err(CommandError::SourceWrite(SourceWriteError::Stale));
        }
        let original = selected.snapshot.body;
        let request = diary_core::SourceRefinementRequest::new(original.clone())
            .map_err(|_| CommandError::Workflow(WorkflowError::AssistantRequestInvalid))?;
        let response = registry
            .invoke(
                &selection.backend_id,
                WritingLanguage::EnUs,
                &AssistantRequest::RefineSource(request),
                cancelled,
            )
            .map_err(map_registry_error)?;
        let AssistantResponse::SourceRefinement(result) = response else {
            return Err(CommandError::Assistant(DraftAssistantError::InvalidOutput));
        };
        if !commit() || cancelled.load(std::sync::atomic::Ordering::Acquire) {
            return Err(CommandError::Cancelled);
        }
        let mut current = self.load_workflow(day)?;
        ensure_revision(&current, expected_revision)?;
        let reread = match self
            .reader
            .read_selected(source)
            .map_err(CommandError::Source)?
        {
            SourceReadOutcome::Read(value) => value,
            _ => return Err(CommandError::Source(SourceAccessError::Unavailable)),
        };
        if reread.snapshot.stamp.content_digest != *expected_digest {
            return Err(CommandError::SourceWrite(SourceWriteError::Stale));
        }
        current
            .record_source_refine_suggestion(
                source,
                original,
                result.body().to_owned(),
                expected_digest.clone(),
                ArtifactProvenance {
                    backend_id: selection.backend_id,
                    composition_path: selection.composition_path,
                },
                self.clock.now(),
            )
            .map_err(CommandError::Workflow)?;
        self.store
            .compare_and_swap(expected_revision, None, &current)
            .map_err(CommandError::Store)?;
        Ok(current)
    }

    pub fn resolve_source_refinement(
        &self,
        day: &DiaryDay,
        expected_revision: u64,
        source: &SourceRef,
        accept: bool,
        binding: &diary_core::Digest,
    ) -> Result<Workflow, CommandError> {
        self.ensure_clear(day)?;
        let mut workflow = self.load_workflow(day)?;
        ensure_revision(&workflow, expected_revision)?;
        let suggestion = workflow
            .source_refine_suggestion(source)
            .ok_or(CommandError::Workflow(
                WorkflowError::AssistantArtifactNotFound,
            ))?
            .clone();
        if &suggestion.binding_digest != binding {
            return Err(CommandError::Workflow(WorkflowError::AssistantBindingStale));
        }
        if accept {
            self.writer
                .overwrite_body(source, &suggestion.suggested_body, binding)
                .map_err(CommandError::SourceWrite)?;
        }
        let _ = workflow
            .take_source_refine_suggestion(
                expected_revision,
                source,
                binding,
                accept,
                self.clock.now(),
            )
            .map_err(CommandError::Workflow)?;
        if accept {
            let revision = workflow.revision();
            workflow
                .source_body_changed(revision, source, self.clock.now())
                .map_err(CommandError::Workflow)?;
        }
        self.store
            .compare_and_swap(expected_revision, None, &workflow)
            .map_err(CommandError::Store)?;
        Ok(workflow)
    }

    pub fn edit_source_body(
        &self,
        day: &DiaryDay,
        expected_revision: u64,
        source: &SourceRef,
        text: &str,
        binding: &diary_core::Digest,
    ) -> Result<Workflow, CommandError> {
        self.ensure_clear(day)?;
        let mut workflow = self.load_workflow(day)?;
        ensure_revision(&workflow, expected_revision)?;
        self.writer
            .overwrite_body(source, text, binding)
            .map_err(CommandError::SourceWrite)?;
        workflow
            .source_body_changed(expected_revision, source, self.clock.now())
            .map_err(CommandError::Workflow)?;
        self.store
            .compare_and_swap(expected_revision, None, &workflow)
            .map_err(CommandError::Store)?;
        Ok(workflow)
    }

    pub fn delete_source(
        &self,
        day: &DiaryDay,
        expected_revision: u64,
        source: &SourceRef,
        confirmed: bool,
    ) -> Result<Workflow, CommandError> {
        if !confirmed {
            return Err(CommandError::DeleteNotConfirmed);
        }
        self.ensure_clear(day)?;
        let mut workflow = self.load_workflow(day)?;
        ensure_revision(&workflow, expected_revision)?;
        self.writer
            .delete_source(source)
            .map_err(CommandError::SourceWrite)?;
        workflow
            .source_deleted(expected_revision, source, self.clock.now())
            .map_err(CommandError::Workflow)?;
        self.store
            .compare_and_swap(expected_revision, None, &workflow)
            .map_err(CommandError::Store)?;
        Ok(workflow)
    }

    pub(super) fn ensure_assistant_sources_fresh(
        &self,
        workflow: &mut Workflow,
        checkpoint: FreshnessCheckpoint,
    ) -> Result<(), CommandError> {
        let revision = workflow.revision();
        let freshness = revalidate_workflow_sources(
            workflow,
            self.reader.as_ref(),
            checkpoint,
            self.clock.now(),
        );
        if freshness.is_err() || workflow.revision() != revision {
            if workflow.revision() != revision {
                self.store
                    .compare_and_swap(revision, None, workflow)
                    .map_err(CommandError::Store)?;
            }
            return Err(CommandError::SourcesInvalidated);
        }
        Ok(())
    }
}
