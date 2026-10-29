use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use diary_core::{
    ArtifactProvenance, AssistantRequest, AssistantResponse, AssistantSelection, CatalogRequest,
    Clock, DiaryDay, DraftAssistantError, DraftAssistantRegistry, FreshnessCheckpoint,
    FreshnessOutcome, OperationLog, Preferences, ReconciliationGate, ReflectionRange,
    RegistryInvokeError, SelectedSource, SourceAccessError, SourceCatalog, SourceReadOutcome,
    SourceReader, SourceRef, SourceWriteError, SourceWriter, StartOutcome, StoreError, Timestamp,
    Workflow, WorkflowError, WorkflowStore, WorkflowSummary, WritingLanguage, WritingStyle,
    revalidate_workflow_sources,
};

pub trait ControlStore: WorkflowStore + OperationLog {}
impl<T: WorkflowStore + OperationLog> ControlStore for T {}

#[derive(Clone)]
pub struct CommandService {
    store: Arc<dyn ControlStore + Send + Sync>,
    catalog: Arc<dyn SourceCatalog + Send + Sync>,
    reader: Arc<dyn SourceReader + Send + Sync>,
    writer: Arc<dyn SourceWriter + Send + Sync>,
    clock: Arc<dyn Clock + Send + Sync>,
}

impl CommandService {
    pub fn new(
        store: Arc<dyn ControlStore + Send + Sync>,
        catalog: Arc<dyn SourceCatalog + Send + Sync>,
        reader: Arc<dyn SourceReader + Send + Sync>,
        clock: Arc<dyn Clock + Send + Sync>,
    ) -> Self {
        Self {
            store,
            catalog,
            reader,
            writer: Arc::new(UnavailableSourceWriter),
            clock,
        }
    }

    pub fn new_with_writer(
        store: Arc<dyn ControlStore + Send + Sync>,
        catalog: Arc<dyn SourceCatalog + Send + Sync>,
        reader: Arc<dyn SourceReader + Send + Sync>,
        writer: Arc<dyn SourceWriter + Send + Sync>,
        clock: Arc<dyn Clock + Send + Sync>,
    ) -> Self {
        Self {
            store,
            catalog,
            reader,
            writer,
            clock,
        }
    }

    pub fn list_workflows(&self, limit: usize) -> Result<Vec<WorkflowSummary>, CommandError> {
        self.store
            .list_summaries(limit)
            .map_err(CommandError::Store)
    }

    pub fn start_workflow(&self, day: Option<DiaryDay>) -> Result<StartOutcome, CommandError> {
        let day = day.unwrap_or_else(|| self.clock.local_day());
        self.ensure_clear(&day)?;
        let preferences = self.store.load_preferences().map_err(CommandError::Store)?;
        self.store
            .start_or_resume(day, self.clock.now(), &preferences)
            .map_err(CommandError::Store)
    }

    pub fn load_workflow(&self, day: &DiaryDay) -> Result<Workflow, CommandError> {
        self.store
            .load(day)
            .map_err(CommandError::Store)?
            .ok_or(CommandError::NotFound)
    }

    pub fn preferences(&self) -> Result<Preferences, CommandError> {
        self.store.load_preferences().map_err(CommandError::Store)
    }

    pub fn save_preferences(&self, preferences: &Preferences) -> Result<(), CommandError> {
        self.store
            .save_preferences(preferences)
            .map_err(CommandError::Store)
    }

    pub fn discard(&self, day: &DiaryDay, expected_revision: u64) -> Result<(), CommandError> {
        self.ensure_clear(day)?;
        let workflow = self.load_workflow(day)?;
        ensure_revision(&workflow, expected_revision)?;
        self.store
            .discard(day, expected_revision, self.clock.now())
            .map_err(CommandError::Store)
    }

    pub fn discover(
        &self,
        day: &DiaryDay,
        expected_revision: u64,
        explicit_date: Option<DiaryDay>,
    ) -> Result<Workflow, CommandError> {
        self.discover_guarded(day, expected_revision, explicit_date, || true)
    }

    pub fn discover_guarded(
        &self,
        day: &DiaryDay,
        expected_revision: u64,
        explicit_date: Option<DiaryDay>,
        commit: impl FnOnce() -> bool,
    ) -> Result<Workflow, CommandError> {
        self.ensure_clear(day)?;
        let mut workflow = self.load_workflow(day)?;
        ensure_revision(&workflow, expected_revision)?;
        let request = explicit_date
            .map(CatalogRequest::ExplicitDate)
            .unwrap_or_else(|| CatalogRequest::Automatic(day.clone()));
        let discovery = self
            .catalog
            .discover(request)
            .map_err(CommandError::Source)?;
        if discovery.candidates.len() > diary_control_contract::MAX_CANDIDATES {
            return Err(CommandError::LimitExceeded);
        }
        workflow
            .discover(expected_revision, discovery.candidates, self.clock.now())
            .map_err(CommandError::Workflow)?;
        if !commit() {
            return Err(CommandError::Cancelled);
        }
        self.store
            .compare_and_swap(expected_revision, None, &workflow)
            .map_err(CommandError::Store)?;
        Ok(workflow)
    }

    pub fn prepare_sources(
        &self,
        day: &DiaryDay,
        expected_revision: u64,
        sources: &[SourceRef],
    ) -> Result<Vec<SelectedSource>, CommandError> {
        self.ensure_clear(day)?;
        let workflow = self.load_workflow(day)?;
        ensure_revision(&workflow, expected_revision)?;
        if sources.is_empty() {
            return Err(CommandError::EmptySelection);
        }
        if sources.len() > diary_control_contract::MAX_SELECTED_SOURCES {
            return Err(CommandError::LimitExceeded);
        }
        for source in sources {
            if !workflow
                .candidates()
                .iter()
                .any(|candidate| &candidate.source == source)
            {
                return Err(CommandError::Workflow(WorkflowError::UnknownSource));
            }
        }
        let mut selected = Vec::with_capacity(sources.len());
        let mut total_bytes = 0_usize;
        for source in sources {
            let mut outcome = self
                .reader
                .read_selected(source)
                .map_err(CommandError::Source)?;
            if matches!(outcome, SourceReadOutcome::NeedsDownload { .. }) {
                self.reader
                    .request_download(source)
                    .map_err(CommandError::Source)?;
                outcome = self
                    .reader
                    .read_selected(source)
                    .map_err(CommandError::Source)?;
            }
            let item = match outcome {
                SourceReadOutcome::Read(item) => item,
                SourceReadOutcome::NeedsDownload { .. } => {
                    return Err(CommandError::Source(SourceAccessError::NeedsDownload));
                }
                SourceReadOutcome::PermissionDenied { .. } => {
                    return Err(CommandError::Source(SourceAccessError::PermissionDenied));
                }
                SourceReadOutcome::Unreadable { .. } => {
                    return Err(CommandError::Workflow(WorkflowError::UnreadableSource));
                }
                SourceReadOutcome::Unverifiable { .. } => {
                    return Err(CommandError::SourcesInvalidated);
                }
            };
            total_bytes = total_bytes.saturating_add(item.snapshot.body.len());
            if total_bytes > diary_control_contract::MAX_PREPARED_BYTES {
                return Err(CommandError::LimitExceeded);
            }
            match self
                .reader
                .revalidate(&item.snapshot)
                .map_err(CommandError::Source)?
            {
                FreshnessOutcome::Fresh => selected.push(item),
                _ => return Err(CommandError::SourcesInvalidated),
            }
        }
        let current = self.load_workflow(day)?;
        ensure_revision(&current, expected_revision)?;
        Ok(selected)
    }

    pub fn confirm_selection(
        &self,
        day: &DiaryDay,
        expected_revision: u64,
        selected: Vec<SelectedSource>,
    ) -> Result<Workflow, CommandError> {
        self.ensure_clear(day)?;
        if selected.is_empty() {
            return Err(CommandError::EmptySelection);
        }
        let mut workflow = self.load_workflow(day)?;
        ensure_revision(&workflow, expected_revision)?;
        for item in &selected {
            match self
                .reader
                .revalidate(&item.snapshot)
                .map_err(CommandError::Source)?
            {
                FreshnessOutcome::Fresh => {}
                _ => return Err(CommandError::SourcesInvalidated),
            }
        }
        workflow
            .confirm_selected_sources(expected_revision, selected, false, self.clock.now())
            .map_err(CommandError::Workflow)?;
        self.store
            .compare_and_swap(expected_revision, None, &workflow)
            .map_err(CommandError::Store)?;
        Ok(workflow)
    }

    pub fn confirm_empty_selection(
        &self,
        day: &DiaryDay,
        expected_revision: u64,
    ) -> Result<Workflow, CommandError> {
        self.ensure_clear(day)?;
        let mut workflow = self.load_workflow(day)?;
        ensure_revision(&workflow, expected_revision)?;
        workflow
            .confirm_selected_sources(expected_revision, Vec::new(), true, self.clock.now())
            .map_err(CommandError::Workflow)?;
        self.store
            .compare_and_swap(expected_revision, None, &workflow)
            .map_err(CommandError::Store)?;
        Ok(workflow)
    }

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

    fn ensure_assistant_sources_fresh(
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

    pub fn begin_manual_composition(
        &self,
        day: &DiaryDay,
        expected_revision: u64,
    ) -> Result<Workflow, CommandError> {
        self.ensure_clear(day)?;
        let mut workflow = self.load_workflow(day)?;
        ensure_revision(&workflow, expected_revision)?;
        workflow
            .begin_manual_composition(expected_revision, self.clock.now())
            .map_err(CommandError::Workflow)?;
        self.store
            .compare_and_swap(expected_revision, None, &workflow)
            .map_err(CommandError::Store)?;
        Ok(workflow)
    }

    pub fn save_entry(
        &self,
        day: &DiaryDay,
        expected_buffer: u64,
        text: String,
    ) -> Result<Workflow, CommandError> {
        self.ensure_clear(day)?;
        if text.len() > diary_control_contract::MAX_ENTRY_BYTES {
            return Err(CommandError::LimitExceeded);
        }
        for attempt in 0..2 {
            let mut workflow = self.load_workflow(day)?;
            if workflow.entry().version != expected_buffer {
                return Err(CommandError::PreconditionFailed);
            }
            let revision = workflow.revision();
            workflow
                .save_entry(expected_buffer, text.clone(), self.clock.now())
                .map_err(CommandError::Workflow)?;
            match self
                .store
                .compare_and_swap(revision, Some(expected_buffer), &workflow)
            {
                Ok(()) => return Ok(workflow),
                Err(StoreError::StaleRevision) if attempt == 0 => continue,
                Err(StoreError::StaleBuffer | StoreError::StaleRevision) => {
                    return Err(CommandError::PreconditionFailed);
                }
                Err(error) => return Err(CommandError::Store(error)),
            }
        }
        Err(CommandError::PreconditionFailed)
    }

    pub fn confirm_entry(
        &self,
        day: &DiaryDay,
        expected_revision: u64,
        expected_buffer: u64,
    ) -> Result<Workflow, CommandError> {
        self.ensure_clear(day)?;
        let mut workflow = self.load_workflow(day)?;
        ensure_revision(&workflow, expected_revision)?;
        if workflow.entry().version != expected_buffer {
            return Err(CommandError::PreconditionFailed);
        }
        let before_revision = workflow.revision();
        let freshness = revalidate_workflow_sources(
            &mut workflow,
            self.reader.as_ref(),
            FreshnessCheckpoint::BeforeEntryConfirmation,
            self.clock.now(),
        );
        if freshness.is_err() || workflow.revision() != before_revision {
            if workflow.revision() != before_revision {
                self.store
                    .compare_and_swap(before_revision, Some(expected_buffer), &workflow)
                    .map_err(CommandError::Store)?;
            }
            return Err(CommandError::SourcesInvalidated);
        }
        workflow
            .confirm_entry(expected_revision, expected_buffer, self.clock.now())
            .map_err(CommandError::Workflow)?;
        self.store
            .compare_and_swap(expected_revision, Some(expected_buffer), &workflow)
            .map_err(CommandError::Store)?;
        Ok(workflow)
    }

    pub fn now(&self) -> Timestamp {
        self.clock.now()
    }

    fn ensure_clear(&self, day: &DiaryDay) -> Result<(), CommandError> {
        match self
            .store
            .reconciliation_gate(day)
            .map_err(CommandError::Store)?
        {
            ReconciliationGate::Clear => Ok(()),
            ReconciliationGate::Reconciling => Err(CommandError::Reconciling),
        }
    }
}

struct UnavailableSourceWriter;
impl SourceWriter for UnavailableSourceWriter {
    fn overwrite_body(
        &self,
        _: &SourceRef,
        _: &str,
        _: &diary_core::Digest,
    ) -> Result<diary_core::FreshnessStamp, SourceWriteError> {
        Err(SourceWriteError::Unavailable)
    }
    fn delete_source(&self, _: &SourceRef) -> Result<(), SourceWriteError> {
        Err(SourceWriteError::Unavailable)
    }
}

fn ensure_revision(workflow: &Workflow, expected: u64) -> Result<(), CommandError> {
    if workflow.revision() == expected {
        Ok(())
    } else {
        Err(CommandError::PreconditionFailed)
    }
}

fn map_registry_error(error: RegistryInvokeError) -> CommandError {
    match error {
        RegistryInvokeError::UnknownBackend => CommandError::UnavailableAssistantBackend,
        RegistryInvokeError::Backend(error) => CommandError::Assistant(error),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CommandError {
    NotFound,
    PreconditionRequired,
    PreconditionFailed,
    EmptySelection,
    LimitExceeded,
    Cancelled,
    SourcesInvalidated,
    Reconciling,
    UnavailableAssistantBackend,
    Assistant(DraftAssistantError),
    Store(StoreError),
    Workflow(WorkflowError),
    Source(SourceAccessError),
    SourceWrite(SourceWriteError),
    DeleteNotConfirmed,
}

impl CommandError {
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::NotFound => "not-found",
            Self::PreconditionRequired => "precondition-required",
            Self::PreconditionFailed
            | Self::Store(StoreError::StaleRevision | StoreError::StaleBuffer) => {
                "precondition-failed"
            }
            Self::EmptySelection => "empty-selection-needs-explicit-confirmation",
            Self::LimitExceeded => "limit-exceeded",
            Self::Cancelled => "operation-cancelled",
            Self::SourcesInvalidated => "sources-invalidated",
            Self::Reconciling => "reconciling",
            Self::UnavailableAssistantBackend => "assistant-backend-unavailable",
            Self::Assistant(DraftAssistantError::Unavailable) => "assistant-backend-unavailable",
            Self::Assistant(DraftAssistantError::UnsupportedLanguage) => {
                "assistant-backend-unsupported-language"
            }
            Self::Assistant(DraftAssistantError::UnsupportedAction) => {
                "assistant-backend-unsupported-action"
            }
            Self::Assistant(DraftAssistantError::Refused) => "assistant-refused",
            Self::Assistant(DraftAssistantError::ContextOverflow) => "assistant-context-overflow",
            Self::Assistant(DraftAssistantError::InvalidOutput) => "assistant-invalid-output",
            Self::Assistant(DraftAssistantError::TimedOut) => "assistant-timeout",
            Self::Assistant(DraftAssistantError::Cancelled) => "operation-cancelled",
            Self::Workflow(WorkflowError::WrongState) => "wrong-state",
            Self::Workflow(WorkflowError::BlankEntry) => "blank-entry",
            Self::Workflow(WorkflowError::LanguageRequired) => "language-required",
            Self::Workflow(WorkflowError::UnresolvedInferenceMarker) => {
                "assistant-unresolved-reflection-marker"
            }
            Self::Workflow(WorkflowError::AssistantBindingStale) => "assistant-binding-stale",
            Self::Workflow(WorkflowError::AssistantSuggestionAlreadyResolved) => {
                "assistant-suggestion-already-resolved"
            }
            Self::Workflow(WorkflowError::AssistantActionsDisabled) => {
                "assistant-action-disabled-empty-selection"
            }
            Self::Workflow(_) => "workflow-conflict",
            Self::Source(SourceAccessError::Busy) => "source-busy",
            Self::Source(SourceAccessError::TimedOut) => "source-timeout",
            Self::Source(_) => "source-unavailable",
            Self::SourceWrite(SourceWriteError::Stale) => "source-binding-stale",
            Self::SourceWrite(SourceWriteError::UnsafePath | SourceWriteError::PartialFailure) => {
                "source-delete-partial-failure"
            }
            Self::SourceWrite(SourceWriteError::Unavailable) => "source-write-unavailable",
            Self::DeleteNotConfirmed => "source-delete-not-confirmed",
            Self::Store(_) => "state-unavailable",
        }
    }
}
