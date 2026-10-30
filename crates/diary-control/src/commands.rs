use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use diary_core::{
    ArtifactProvenance, AssistantRequest, AssistantResponse, AssistantSelection, CatalogRequest,
    Clock, Destination, DestinationKind, DestinationPlan, DestinationProgress, DestinationStatus,
    DiaryDay, Digest, DraftAssistantError, DraftAssistantRegistry, FreshnessCheckpoint,
    FreshnessOutcome, InspectionOutcome, LocalOutputError, LocalOutputPort, LocalOutputState,
    NotionBlockPreview, NotionOutputPort, NotionPreflightStatus, NotionRequest, OperationIntent,
    OperationLog, OutputPlan, Preferences, PublicationCoordinator, PublicationResult,
    ReconciliationGate, ReflectionRange, RegistryInvokeError, SelectedSource, SourceAccessError,
    SourceCatalog, SourceReadOutcome, SourceReader, SourceRef, SourceWriteError, SourceWriter,
    StartOutcome, StoreError, Timestamp, Workflow, WorkflowError, WorkflowStore, WorkflowSummary,
    WriteOutcome, WritingLanguage, WritingStyle, render_local_markdown,
    revalidate_workflow_sources,
};

use crate::lifecycle::random_id;

mod amendment;
pub use amendment::{AmendmentNotionPreview, AmendmentPreview, AmendmentSeeded, local_body};

#[derive(Clone, Debug)]
pub struct LocalPreview {
    pub workflow: Workflow,
    pub target: String,
    pub payload: String,
    pub payload_digest: Digest,
    pub existing_text: Option<String>,
    pub intended_changes: Vec<String>,
    pub mirror_target: Option<String>,
    pub mirror_status: Option<DestinationStatus>,
}

#[derive(Clone, Debug)]
pub struct NotionPreview {
    pub workflow: Workflow,
    pub root_label: String,
    pub year_title: String,
    pub month_title: String,
    pub year_exists: bool,
    pub month_exists: bool,
    pub renderer_version: u32,
    pub api_version: String,
    pub payload_digest: Digest,
    pub plan_digest: Option<Digest>,
    pub blocks: Vec<NotionBlockPreview>,
    pub status: DestinationStatus,
    pub reason: Option<String>,
}

pub trait ControlStore: WorkflowStore + OperationLog {}
impl<T: WorkflowStore + OperationLog> ControlStore for T {}

#[derive(Clone)]
pub struct CommandService {
    store: Arc<dyn ControlStore + Send + Sync>,
    catalog: Arc<dyn SourceCatalog + Send + Sync>,
    reader: Arc<dyn SourceReader + Send + Sync>,
    writer: Arc<dyn SourceWriter + Send + Sync>,
    clock: Arc<dyn Clock + Send + Sync>,
    local_output: Option<Arc<dyn LocalOutputPort>>,
    mirror_output: Option<Arc<dyn LocalOutputPort>>,
    notion_output: Option<Arc<dyn NotionOutputPort>>,
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
            local_output: None,
            mirror_output: None,
            notion_output: None,
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
            local_output: None,
            mirror_output: None,
            notion_output: None,
        }
    }

    pub fn with_local_output(mut self, local_output: Arc<dyn LocalOutputPort>) -> Self {
        self.local_output = Some(local_output);
        self
    }

    /// Optional iCloud mirror of the local file, destination name `icloud`.
    pub fn with_mirror_output(mut self, mirror_output: Arc<dyn LocalOutputPort>) -> Self {
        self.mirror_output = Some(mirror_output);
        self
    }

    pub fn with_notion_output(mut self, notion_output: Arc<dyn NotionOutputPort>) -> Self {
        self.notion_output = Some(notion_output);
        self
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

    pub fn preview_local(
        &self,
        day: &DiaryDay,
        expected_revision: u64,
    ) -> Result<LocalPreview, CommandError> {
        self.ensure_clear(day)?;
        let output = self
            .local_output
            .as_ref()
            .ok_or(CommandError::LocalOutput(LocalOutputError::Unavailable))?;
        let mut workflow = self.load_workflow(day)?;
        ensure_revision(&workflow, expected_revision)?;
        let prior_revision = workflow.revision();
        let freshness = revalidate_workflow_sources(
            &mut workflow,
            self.reader.as_ref(),
            FreshnessCheckpoint::BeforePreview,
            self.clock.now(),
        );
        if freshness.is_err() || workflow.revision() != prior_revision {
            if workflow.revision() != prior_revision {
                self.store
                    .compare_and_swap(prior_revision, None, &workflow)
                    .map_err(CommandError::Store)?;
            }
            return Err(CommandError::SourcesInvalidated);
        }
        let language = workflow
            .language()
            .ok_or(CommandError::Workflow(WorkflowError::LanguageRequired))?;
        let payload = render_local_markdown(day, language, &workflow.entry().text);
        let payload_digest = Digest::of_bytes(payload.as_bytes());
        let state = output.inspect(day, &payload);
        let (observed, status, existing_text) = match state {
            Ok(LocalOutputState::Absent) => (
                Digest::of_bytes(b"absent"),
                DestinationStatus::Pending,
                None,
            ),
            Ok(LocalOutputState::Identical) => {
                (payload_digest.clone(), DestinationStatus::Pending, None)
            }
            Ok(LocalOutputState::Different(text)) => (
                Digest::of_bytes(text.as_bytes()),
                DestinationStatus::Conflict,
                Some(text),
            ),
            Err(_) => (
                Digest::of_bytes(b"unverifiable"),
                DestinationStatus::Blocked,
                None,
            ),
        };
        let target = output.target(day);
        let intended_changes = if status == DestinationStatus::Pending {
            if existing_text.is_none() && observed == Digest::of_bytes(b"absent") {
                vec![
                    format!("Create parent directories as needed for {target}"),
                    format!("Create {target} without replacing an existing file"),
                ]
            } else {
                vec![format!("Verify identical existing file at {target}")]
            }
        } else {
            Vec::new()
        };
        let local = DestinationPlan::new(
            DestinationKind::Local,
            "local",
            target.clone(),
            payload_digest.clone(),
            payload_digest.clone(),
            observed,
        )
        .map_err(|_| CommandError::LocalOutput(LocalOutputError::Unavailable))?;
        let mut targets = vec![local];
        let mut mirror_target = None;
        let mut mirror_status = None;
        if let Some(mirror) = &self.mirror_output {
            let (observed, status) = match mirror.inspect(day, &payload) {
                Ok(LocalOutputState::Absent) => {
                    (Digest::of_bytes(b"absent"), DestinationStatus::Pending)
                }
                Ok(LocalOutputState::Identical) => {
                    (payload_digest.clone(), DestinationStatus::Pending)
                }
                Ok(LocalOutputState::Different(text)) => (
                    Digest::of_bytes(text.as_bytes()),
                    DestinationStatus::Conflict,
                ),
                Err(_) => (
                    Digest::of_bytes(b"unverifiable"),
                    DestinationStatus::Blocked,
                ),
            };
            let identity = mirror.target(day);
            targets.push(
                DestinationPlan::new(
                    DestinationKind::Mirror,
                    MIRROR_DESTINATION,
                    identity.clone(),
                    payload_digest.clone(),
                    payload_digest.clone(),
                    observed,
                )
                .map_err(|_| CommandError::LocalOutput(LocalOutputError::Unavailable))?,
            );
            mirror_target = Some(identity);
            mirror_status = Some(status);
        }
        let blocked = DestinationPlan::new(
            DestinationKind::Remote,
            "notion",
            "unavailable",
            Digest::of_bytes(b"blocked"),
            Digest::of_bytes(b"blocked"),
            Digest::of_bytes(b"uninspected"),
        )
        .map_err(|_| CommandError::LocalOutput(LocalOutputError::Unavailable))?;
        let plan = OutputPlan::new(
            random_id("plan_")
                .map_err(|_| CommandError::LocalOutput(LocalOutputError::Unavailable))?,
            workflow
                .entry()
                .confirmed
                .as_ref()
                .ok_or(CommandError::Workflow(WorkflowError::EntryNotConfirmed))?
                .entry_digest
                .clone(),
            workflow
                .selection_digest()
                .ok_or(CommandError::Workflow(WorkflowError::PlanBindingMismatch))?
                .clone(),
            {
                targets.push(blocked);
                targets
            },
            self.clock.now(),
        )
        .map_err(|_| CommandError::LocalOutput(LocalOutputError::Unavailable))?;
        let mut progress = vec![DestinationProgress {
            destination: "local".into(),
            status,
        }];
        if let Some(status) = mirror_status {
            progress.push(DestinationProgress {
                destination: MIRROR_DESTINATION.into(),
                status,
            });
        }
        progress.push(DestinationProgress {
            destination: "notion".into(),
            status: DestinationStatus::Blocked,
        });
        workflow
            .preview_output_with_progress(expected_revision, plan, progress, self.clock.now())
            .map_err(CommandError::Workflow)?;
        self.store
            .compare_and_swap(expected_revision, None, &workflow)
            .map_err(CommandError::Store)?;
        Ok(LocalPreview {
            workflow,
            target,
            payload,
            payload_digest,
            existing_text,
            intended_changes,
            mirror_target,
            mirror_status,
        })
    }

    pub fn approve_local(
        &self,
        day: &DiaryDay,
        expected_revision: u64,
        digest: &Digest,
    ) -> Result<Workflow, CommandError> {
        self.ensure_clear(day)?;
        let mut workflow = self.load_workflow(day)?;
        ensure_revision(&workflow, expected_revision)?;
        let prior_revision = workflow.revision();
        let freshness = revalidate_workflow_sources(
            &mut workflow,
            self.reader.as_ref(),
            FreshnessCheckpoint::BeforePreview,
            self.clock.now(),
        );
        if freshness.is_err() || workflow.revision() != prior_revision {
            if workflow.revision() != prior_revision {
                self.store
                    .compare_and_swap(prior_revision, None, &workflow)
                    .map_err(CommandError::Store)?;
            }
            return Err(CommandError::SourcesInvalidated);
        }
        let plan = workflow
            .output_plan()
            .ok_or(CommandError::Workflow(WorkflowError::MissingPlan))?;
        let target = plan
            .targets
            .iter()
            .find(|target| target.kind == DestinationKind::Local)
            .ok_or(CommandError::Workflow(WorkflowError::MissingPlan))?;
        let output = self
            .local_output
            .as_ref()
            .ok_or(CommandError::LocalOutput(LocalOutputError::Unavailable))?;
        let payload = render_local_markdown(
            day,
            workflow
                .language()
                .ok_or(CommandError::Workflow(WorkflowError::LanguageRequired))?,
            &workflow.entry().text,
        );
        if Digest::of_bytes(payload.as_bytes()) != target.payload_digest
            || output.target(day) != target.target_identity
            || observed_digest(
                output
                    .inspect(day, &payload)
                    .map_err(CommandError::LocalOutput)?,
                &payload,
            ) != target.inspected_state_digest
        {
            return Err(CommandError::PreconditionFailed);
        }
        self.ensure_mirror_binding(plan, day, &target.payload_digest)?;
        workflow
            .approve(expected_revision, digest, self.clock.now())
            .map_err(CommandError::Workflow)?;
        self.store
            .compare_and_swap(expected_revision, None, &workflow)
            .map_err(CommandError::Store)?;
        Ok(workflow)
    }

    pub fn publish_local(
        &self,
        day: &DiaryDay,
        expected_revision: u64,
    ) -> Result<Workflow, CommandError> {
        self.publish_local_guarded(day, expected_revision, || true)
    }

    pub fn publish_local_guarded(
        &self,
        day: &DiaryDay,
        expected_revision: u64,
        commit: impl FnOnce() -> bool,
    ) -> Result<Workflow, CommandError> {
        self.ensure_clear(day)?;
        let output = self
            .local_output
            .as_ref()
            .ok_or(CommandError::LocalOutput(LocalOutputError::Unavailable))?;
        let mut workflow = self.load_workflow(day)?;
        ensure_revision(&workflow, expected_revision)?;
        let plan = workflow
            .output_plan()
            .ok_or(CommandError::Workflow(WorkflowError::MissingPlan))?
            .clone();
        let target = plan
            .targets
            .iter()
            .find(|target| target.kind == DestinationKind::Local)
            .ok_or(CommandError::Workflow(WorkflowError::MissingPlan))?;
        let payload = render_local_markdown(
            day,
            workflow
                .language()
                .ok_or(CommandError::Workflow(WorkflowError::LanguageRequired))?,
            &workflow.entry().text,
        );
        if Digest::of_bytes(payload.as_bytes()) != target.payload_digest
            || output.target(day) != target.target_identity
            || observed_digest(
                output
                    .inspect(day, &payload)
                    .map_err(CommandError::LocalOutput)?,
                &payload,
            ) != target.inspected_state_digest
        {
            return Err(CommandError::PreconditionFailed);
        }
        self.ensure_mirror_binding(&plan, day, &target.payload_digest)?;
        let prior = workflow.revision();
        let fresh = revalidate_workflow_sources(
            &mut workflow,
            self.reader.as_ref(),
            FreshnessCheckpoint::BeforeFirstWrite,
            self.clock.now(),
        );
        if fresh.is_err() || workflow.revision() != prior {
            if workflow.revision() != prior {
                self.store
                    .compare_and_swap(prior, None, &workflow)
                    .map_err(CommandError::Store)?;
            }
            return Err(CommandError::SourcesInvalidated);
        }
        if !commit() {
            return Err(CommandError::Cancelled);
        }
        if workflow.state() == diary_core::WorkflowState::Partial {
            workflow
                .begin_local_retry(expected_revision, self.clock.now())
                .map_err(CommandError::Workflow)?;
        } else {
            workflow
                .begin_publication(expected_revision, self.clock.now())
                .map_err(CommandError::Workflow)?;
        }
        self.store
            .compare_and_swap(expected_revision, None, &workflow)
            .map_err(CommandError::Store)?;
        let intent = OperationIntent::new(
            random_id("attempt_")
                .map_err(|_| CommandError::LocalOutput(LocalOutputError::Unavailable))?,
            plan.plan_id.clone(),
            day.clone(),
            target,
            plan.entry_digest.clone(),
            plan.selection_digest.clone(),
            self.clock.now(),
        )
        .map_err(|_| CommandError::LocalOutput(LocalOutputError::Unavailable))?;
        let mut destination = LocalDestination {
            output: output.as_ref(),
            day,
            payload: &payload,
            receipt_id: "local".into(),
        };
        let result = PublicationCoordinator::execute(
            self.store.as_ref(),
            &mut destination,
            &intent,
            1,
            self.clock.now(),
        )
        .map_err(|_| CommandError::LocalOutput(LocalOutputError::Unavailable))?;
        let status = match result {
            PublicationResult::Verified(_) => DestinationStatus::Verified,
            PublicationResult::Conflict => DestinationStatus::Conflict,
            _ => DestinationStatus::Reconciling,
        };
        if status == DestinationStatus::Reconciling {
            return Ok(workflow);
        }
        let mut overrides = vec![("local", status), ("notion", DestinationStatus::Blocked)];
        if status == DestinationStatus::Verified && self.mirror_output.is_some() {
            overrides.push((
                MIRROR_DESTINATION,
                self.run_mirror(&workflow, &plan, day, &payload),
            ));
        }
        let progress = progress_with(&workflow, &overrides);
        workflow
            .finish_publication(workflow.revision(), progress, self.clock.now())
            .map_err(CommandError::Workflow)?;
        self.store
            .compare_and_swap(workflow.revision() - 1, None, &workflow)
            .map_err(CommandError::Store)?;
        Ok(workflow)
    }

    /// Re-preflights the blocked remote after the local receipt is verified.
    pub fn preview_notion(
        &self,
        day: &DiaryDay,
        expected_revision: u64,
    ) -> Result<NotionPreview, CommandError> {
        self.ensure_clear(day)?;
        let notion = self
            .notion_output
            .as_ref()
            .ok_or(CommandError::NotionUnavailable)?;
        let mut workflow = self.load_workflow(day)?;
        ensure_revision(&workflow, expected_revision)?;
        self.verify_local_receipt(&workflow)?;
        let preflight = match notion.preflight(day, &workflow.entry().text) {
            Ok(preflight) => preflight,
            Err(reason) => {
                return Ok(NotionPreview {
                    workflow,
                    root_label: "Notion root".into(),
                    year_title: day.as_str()[..4].to_owned(),
                    month_title: day.as_str()[..7].to_owned(),
                    year_exists: false,
                    month_exists: false,
                    renderer_version: 0,
                    api_version: String::new(),
                    payload_digest: Digest::of_bytes(b"notion-unavailable"),
                    plan_digest: None,
                    blocks: Vec::new(),
                    status: DestinationStatus::Blocked,
                    reason: Some(reason),
                });
            }
        };
        let status = match preflight.status {
            NotionPreflightStatus::Ready => DestinationStatus::Pending,
            NotionPreflightStatus::Blocked => DestinationStatus::Blocked,
            NotionPreflightStatus::Conflict => DestinationStatus::Conflict,
        };
        if status != DestinationStatus::Pending {
            return Ok(NotionPreview {
                workflow,
                root_label: preflight.root_label,
                year_title: preflight.year_title,
                month_title: preflight.month_title,
                year_exists: preflight.year_exists,
                month_exists: preflight.month_exists,
                renderer_version: preflight.renderer_version,
                api_version: preflight.api_version,
                payload_digest: preflight.payload_digest,
                plan_digest: None,
                blocks: preflight.blocks,
                status,
                reason: preflight.reason,
            });
        }
        let replacement = DestinationPlan::new(
            DestinationKind::Remote,
            "notion",
            preflight.target_identity,
            preflight.payload_digest.clone(),
            preflight.payload_digest.clone(),
            preflight.observed_state_digest,
        )
        .map_err(|_| CommandError::NotionUnavailable)?;
        let plan = workflow
            .preview_blocked_retry(
                expected_revision,
                "notion",
                replacement,
                random_id("plan_").map_err(|_| CommandError::NotionUnavailable)?,
                self.clock.now(),
            )
            .map_err(CommandError::Workflow)?;
        self.store
            .compare_and_swap(expected_revision, None, &workflow)
            .map_err(CommandError::Store)?;
        Ok(NotionPreview {
            workflow,
            root_label: preflight.root_label,
            year_title: preflight.year_title,
            month_title: preflight.month_title,
            year_exists: preflight.year_exists,
            month_exists: preflight.month_exists,
            renderer_version: preflight.renderer_version,
            api_version: preflight.api_version,
            payload_digest: preflight.payload_digest,
            plan_digest: Some(plan.plan_digest),
            blocks: preflight.blocks,
            status,
            reason: None,
        })
    }

    pub fn approve_notion(
        &self,
        day: &DiaryDay,
        expected_revision: u64,
        digest: &Digest,
    ) -> Result<Workflow, CommandError> {
        self.ensure_clear(day)?;
        let notion = self
            .notion_output
            .as_ref()
            .ok_or(CommandError::NotionUnavailable)?;
        let mut workflow = self.load_workflow(day)?;
        ensure_revision(&workflow, expected_revision)?;
        self.verify_local_receipt(&workflow)?;
        let plan = workflow
            .output_plan()
            .ok_or(CommandError::Workflow(WorkflowError::MissingPlan))?;
        if &plan.plan_digest != digest {
            return Err(CommandError::PreconditionFailed);
        }
        let target = plan
            .targets
            .iter()
            .find(|target| target.destination == "notion")
            .ok_or(CommandError::Workflow(WorkflowError::MissingPlan))?;
        let preflight = notion
            .preflight(day, &workflow.entry().text)
            .map_err(|_| CommandError::NotionUnavailable)?;
        if preflight.status != NotionPreflightStatus::Ready
            || preflight.target_identity != target.target_identity
            || preflight.payload_digest != target.payload_digest
            || preflight.observed_state_digest != target.inspected_state_digest
        {
            return Err(CommandError::PreconditionFailed);
        }
        workflow
            .approve_blocked_retry(expected_revision, "notion", digest, self.clock.now())
            .map_err(CommandError::Workflow)?;
        self.store
            .compare_and_swap(expected_revision, None, &workflow)
            .map_err(CommandError::Store)?;
        Ok(workflow)
    }

    pub fn publish_notion(
        &self,
        day: &DiaryDay,
        expected_revision: u64,
    ) -> Result<Workflow, CommandError> {
        self.publish_notion_guarded(day, expected_revision, &AtomicBool::new(false))
    }

    pub fn publish_notion_guarded(
        &self,
        day: &DiaryDay,
        expected_revision: u64,
        cancelled: &AtomicBool,
    ) -> Result<Workflow, CommandError> {
        self.ensure_clear(day)?;
        let notion = self
            .notion_output
            .as_ref()
            .ok_or(CommandError::NotionUnavailable)?;
        let mut workflow = self.load_workflow(day)?;
        ensure_revision(&workflow, expected_revision)?;
        self.verify_local_receipt(&workflow)?;
        let plan = workflow
            .output_plan()
            .ok_or(CommandError::Workflow(WorkflowError::MissingPlan))?
            .clone();
        let target = plan
            .targets
            .iter()
            .find(|target| target.destination == "notion")
            .ok_or(CommandError::Workflow(WorkflowError::MissingPlan))?;
        let request = NotionRequest {
            day: day.clone(),
            markdown: workflow.entry().text.clone(),
            expected_target_identity: target.target_identity.clone(),
            expected_payload_digest: target.payload_digest.clone(),
            expected_state_digest: target.inspected_state_digest.clone(),
        };
        // Revalidate the approved binding immediately before the first effect.
        let preflight = notion
            .preflight(day, &workflow.entry().text)
            .map_err(|_| CommandError::NotionUnavailable)?;
        if preflight.status != NotionPreflightStatus::Ready
            || preflight.target_identity != target.target_identity
            || preflight.payload_digest != target.payload_digest
            || preflight.observed_state_digest != target.inspected_state_digest
        {
            return Err(CommandError::PreconditionFailed);
        }
        if workflow.state() == diary_core::WorkflowState::Partial {
            workflow
                .begin_pending_retry(
                    expected_revision,
                    "notion",
                    &plan.plan_digest,
                    self.clock.now(),
                )
                .map_err(CommandError::Workflow)?;
        } else {
            workflow
                .begin_publication(expected_revision, self.clock.now())
                .map_err(CommandError::Workflow)?;
        }
        self.store
            .compare_and_swap(expected_revision, None, &workflow)
            .map_err(CommandError::Store)?;
        let intent = OperationIntent::new(
            random_id("attempt_").map_err(|_| CommandError::NotionUnavailable)?,
            plan.plan_id,
            day.clone(),
            target,
            plan.entry_digest,
            plan.selection_digest,
            self.clock.now(),
        )
        .map_err(|_| CommandError::NotionUnavailable)?;
        let mut destination = NotionDestination {
            output: notion.as_ref(),
            request: &request,
            cancelled,
        };
        let result = PublicationCoordinator::execute(
            self.store.as_ref(),
            &mut destination,
            &intent,
            2,
            self.clock.now(),
        )
        .map_err(|_| CommandError::NotionUnavailable)?;
        let status = match result {
            PublicationResult::Verified(_) => DestinationStatus::Verified,
            PublicationResult::Conflict => DestinationStatus::Conflict,
            PublicationResult::DefiniteNoEffect | PublicationResult::SafeToRetry => {
                DestinationStatus::Pending
            }
            _ => DestinationStatus::Reconciling,
        };
        if status == DestinationStatus::Reconciling {
            return Ok(workflow);
        }
        let revision = workflow.revision();
        workflow
            .finish_publication(
                revision,
                progress_with(
                    &workflow,
                    &[("local", DestinationStatus::Verified), ("notion", status)],
                ),
                self.clock.now(),
            )
            .map_err(CommandError::Workflow)?;
        self.store
            .compare_and_swap(revision, None, &workflow)
            .map_err(CommandError::Store)?;
        Ok(workflow)
    }

    /// A plan carries an `icloud` target exactly when a mirror is configured,
    /// for the same target and bytes; otherwise a new preview is required.
    fn ensure_mirror_binding(
        &self,
        plan: &OutputPlan,
        day: &DiaryDay,
        payload_digest: &Digest,
    ) -> Result<(), CommandError> {
        let planned = plan
            .targets
            .iter()
            .find(|target| target.kind == DestinationKind::Mirror);
        match (&self.mirror_output, planned) {
            (None, None) => Ok(()),
            (Some(mirror), Some(target))
                if target.destination == MIRROR_DESTINATION
                    && target.target_identity == mirror.target(day)
                    && &target.payload_digest == payload_digest =>
            {
                Ok(())
            }
            _ => Err(CommandError::PreconditionFailed),
        }
    }

    /// Runs the mirror write after a verified local write. Every failure is
    /// contained: it yields a non-verified status and never an error.
    fn run_mirror(
        &self,
        workflow: &Workflow,
        plan: &OutputPlan,
        day: &DiaryDay,
        payload: &str,
    ) -> DestinationStatus {
        let planned = workflow
            .destination_progress()
            .iter()
            .find(|item| item.destination == MIRROR_DESTINATION)
            .map_or(DestinationStatus::Blocked, |item| item.status);
        if planned != DestinationStatus::Pending {
            return planned;
        }
        self.execute_mirror(plan, day, payload)
    }

    fn execute_mirror(
        &self,
        plan: &OutputPlan,
        day: &DiaryDay,
        payload: &str,
    ) -> DestinationStatus {
        let (Some(mirror), Some(target)) = (
            &self.mirror_output,
            plan.targets
                .iter()
                .find(|target| target.kind == DestinationKind::Mirror),
        ) else {
            return DestinationStatus::Blocked;
        };
        // Read-only preflight: never record an intent for a mirror that
        // cannot be inspected, so a missing folder cannot gate the day.
        match mirror.inspect(day, payload) {
            Err(_) => return DestinationStatus::Blocked,
            Ok(LocalOutputState::Different(_)) => return DestinationStatus::Conflict,
            Ok(_) => {}
        }
        let Ok(attempt) = random_id("attempt_") else {
            return DestinationStatus::Blocked;
        };
        let receipt_id = format!("icloud-{attempt}");
        let Ok(intent) = OperationIntent::new(
            attempt,
            plan.plan_id.clone(),
            day.clone(),
            target,
            plan.entry_digest.clone(),
            plan.selection_digest.clone(),
            self.clock.now(),
        ) else {
            return DestinationStatus::Blocked;
        };
        let mut destination = LocalDestination {
            output: mirror.as_ref(),
            day,
            payload,
            receipt_id,
        };
        match PublicationCoordinator::execute(
            self.store.as_ref(),
            &mut destination,
            &intent,
            1,
            self.clock.now(),
        ) {
            Ok(PublicationResult::Verified(_)) => DestinationStatus::Verified,
            Ok(PublicationResult::Conflict) => DestinationStatus::Conflict,
            // An uncertain write keeps its intent; startup reconciliation
            // resolves it before any retry.
            _ => DestinationStatus::Pending,
        }
    }

    /// Retries only the iCloud mirror from the verified local file.
    pub fn publish_mirror_guarded(
        &self,
        day: &DiaryDay,
        expected_revision: u64,
        commit: impl FnOnce() -> bool,
    ) -> Result<Workflow, CommandError> {
        self.ensure_clear(day)?;
        if self.mirror_output.is_none() {
            return Err(CommandError::LocalOutput(LocalOutputError::Unavailable));
        }
        let mut workflow = self.load_workflow(day)?;
        ensure_revision(&workflow, expected_revision)?;
        if workflow.state() != diary_core::WorkflowState::Partial {
            return Err(CommandError::Workflow(WorkflowError::WrongState));
        }
        self.verify_local_receipt(&workflow)?;
        let plan = workflow
            .output_plan()
            .ok_or(CommandError::Workflow(WorkflowError::MissingPlan))?
            .clone();
        let payload = render_local_markdown(
            day,
            workflow
                .language()
                .ok_or(CommandError::Workflow(WorkflowError::LanguageRequired))?,
            &workflow.entry().text,
        );
        self.ensure_mirror_binding(&plan, day, &Digest::of_bytes(payload.as_bytes()))?;
        if !commit() {
            return Err(CommandError::Cancelled);
        }
        let status = self.execute_mirror(&plan, day, &payload);
        workflow
            .finish_mirror_retry(expected_revision, status, self.clock.now())
            .map_err(CommandError::Workflow)?;
        self.store
            .compare_and_swap(expected_revision, None, &workflow)
            .map_err(CommandError::Store)?;
        Ok(workflow)
    }

    /// Recovers uncertain mirror effects. An interrupted publication job
    /// finishes through `finish_publication`; a mirror-only retry through
    /// `finish_mirror_retry`.
    pub fn reconcile_mirror_intents(&self) -> Result<(), CommandError> {
        let Some(mirror) = &self.mirror_output else {
            return Ok(());
        };
        for intent in self
            .store
            .unresolved_intents(None)
            .map_err(CommandError::Store)?
        {
            if intent.destination != MIRROR_DESTINATION {
                continue;
            }
            let mut workflow = self.load_workflow(&intent.day)?;
            if workflow.amendment().is_some() {
                continue;
            }
            let Some(language) = workflow.language() else {
                continue;
            };
            let payload = render_local_markdown(&intent.day, language, &workflow.entry().text);
            if Digest::of_bytes(payload.as_bytes()) != intent.payload_digest
                || mirror.target(&intent.day) != intent.target_identity
            {
                continue;
            }
            let mut destination = LocalDestination {
                output: mirror.as_ref(),
                day: &intent.day,
                payload: &payload,
                receipt_id: format!("icloud-{}", intent.attempt_id),
            };
            let result = PublicationCoordinator::reconcile(
                self.store.as_ref(),
                &mut destination,
                &intent,
                1,
                self.clock.now(),
            )
            .map_err(|_| CommandError::LocalOutput(LocalOutputError::Unavailable))?;
            let status = match result {
                PublicationResult::Verified(_) => DestinationStatus::Verified,
                PublicationResult::Conflict => DestinationStatus::Conflict,
                PublicationResult::SafeToRetry => DestinationStatus::Pending,
                _ => continue,
            };
            let revision = workflow.revision();
            match workflow.state() {
                diary_core::WorkflowState::Publishing => {
                    workflow
                        .finish_publication(
                            revision,
                            progress_with(
                                &workflow,
                                &[
                                    ("local", DestinationStatus::Verified),
                                    (MIRROR_DESTINATION, status),
                                ],
                            ),
                            self.clock.now(),
                        )
                        .map_err(CommandError::Workflow)?;
                }
                diary_core::WorkflowState::Partial => {
                    workflow
                        .finish_mirror_retry(revision, status, self.clock.now())
                        .map_err(CommandError::Workflow)?;
                }
                _ => continue,
            }
            self.store
                .compare_and_swap(revision, None, &workflow)
                .map_err(CommandError::Store)?;
        }
        Ok(())
    }

    fn verify_local_receipt(&self, workflow: &Workflow) -> Result<(), CommandError> {
        let output = self
            .local_output
            .as_ref()
            .ok_or(CommandError::LocalOutput(LocalOutputError::Unavailable))?;
        let language = workflow
            .language()
            .ok_or(CommandError::Workflow(WorkflowError::LanguageRequired))?;
        let payload = render_local_markdown(workflow.day(), language, &workflow.entry().text);
        let payload_digest = Digest::of_bytes(payload.as_bytes());
        if output
            .inspect(workflow.day(), &payload)
            .map_err(CommandError::LocalOutput)?
            != LocalOutputState::Identical
        {
            return Err(CommandError::Workflow(WorkflowError::LocalMustBeVerified));
        }
        let confirmation = workflow
            .entry()
            .confirmed
            .as_ref()
            .ok_or(CommandError::Workflow(WorkflowError::EntryNotConfirmed))?;
        let selection = workflow
            .selection_digest()
            .ok_or(CommandError::Workflow(WorkflowError::PlanBindingMismatch))?;
        let target = output.target(workflow.day());
        let receipted = self
            .store
            .receipts(workflow.day())
            .map_err(CommandError::Store)?
            .iter()
            .any(|receipt| {
                receipt.destination == "local"
                    && receipt.target_identity == target
                    && receipt.payload_digest == payload_digest
                    && receipt.entry_digest == confirmation.entry_digest
                    && &receipt.selection_digest == selection
            });
        if !receipted {
            return Err(CommandError::Workflow(WorkflowError::LocalMustBeVerified));
        }
        Ok(())
    }

    /// Recover uncertain local effects before accepting commands for their day.
    pub fn reconcile_local_intents(&self) -> Result<(), CommandError> {
        let Some(output) = &self.local_output else {
            return Ok(());
        };
        for intent in self
            .store
            .unresolved_intents(None)
            .map_err(CommandError::Store)?
        {
            if intent.destination != "local" {
                continue;
            }
            let mut workflow = self.load_workflow(&intent.day)?;
            let Some(language) = workflow.language() else {
                continue;
            };
            let payload = render_local_markdown(&intent.day, language, &workflow.entry().text);
            if Digest::of_bytes(payload.as_bytes()) != intent.payload_digest
                || output.target(&intent.day) != intent.target_identity
            {
                continue;
            }
            let mut destination = LocalDestination {
                output: output.as_ref(),
                day: &intent.day,
                payload: &payload,
                receipt_id: "local".into(),
            };
            let result = PublicationCoordinator::reconcile(
                self.store.as_ref(),
                &mut destination,
                &intent,
                1,
                self.clock.now(),
            )
            .map_err(|_| CommandError::LocalOutput(LocalOutputError::Unavailable))?;
            let status = match result {
                PublicationResult::Verified(_) => DestinationStatus::Verified,
                PublicationResult::Conflict => DestinationStatus::Conflict,
                PublicationResult::SafeToRetry => DestinationStatus::Pending,
                _ => continue,
            };
            if workflow.state() == diary_core::WorkflowState::Publishing {
                let revision = workflow.revision();
                workflow
                    .finish_publication(
                        revision,
                        progress_with(
                            &workflow,
                            &[("local", status), ("notion", DestinationStatus::Blocked)],
                        ),
                        self.clock.now(),
                    )
                    .map_err(CommandError::Workflow)?;
                self.store
                    .compare_and_swap(revision, None, &workflow)
                    .map_err(CommandError::Store)?;
            }
        }
        // A crash may land between persisting `Publishing` and recording its
        // intent, or between committing a receipt and advancing the workflow.
        for summary in self
            .store
            .list_summaries(diary_control_contract::MAX_WORKFLOWS)
            .map_err(CommandError::Store)?
        {
            if summary.state != diary_core::WorkflowState::Publishing {
                continue;
            }
            let day = summary.day;
            if !self
                .store
                .unresolved_intents(Some(&day))
                .map_err(CommandError::Store)?
                .is_empty()
            {
                continue;
            }
            let mut workflow = self.load_workflow(&day)?;
            if workflow.amendment().is_some() {
                continue;
            }
            let Some(plan) = workflow.output_plan() else {
                continue;
            };
            let Some(target) = plan
                .targets
                .iter()
                .find(|target| target.destination == "local")
            else {
                continue;
            };
            let Some(language) = workflow.language() else {
                continue;
            };
            let payload = render_local_markdown(&day, language, &workflow.entry().text);
            if Digest::of_bytes(payload.as_bytes()) != target.payload_digest {
                continue;
            }
            let receipted = self
                .store
                .receipts(&day)
                .map_err(CommandError::Store)?
                .iter()
                .any(|receipt| {
                    receipt.destination == "local"
                        && receipt.target_identity == target.target_identity
                        && receipt.payload_digest == target.payload_digest
                        && receipt.entry_digest == plan.entry_digest
                        && receipt.selection_digest == plan.selection_digest
                });
            let status = match output.inspect(&day, &payload) {
                Ok(LocalOutputState::Identical) if receipted => DestinationStatus::Verified,
                Ok(LocalOutputState::Absent) if !receipted => DestinationStatus::Pending,
                Ok(_) => DestinationStatus::Conflict,
                Err(_) => continue,
            };
            let revision = workflow.revision();
            workflow
                .finish_publication(
                    revision,
                    progress_with(
                        &workflow,
                        &[("local", status), ("notion", DestinationStatus::Blocked)],
                    ),
                    self.clock.now(),
                )
                .map_err(CommandError::Workflow)?;
            self.store
                .compare_and_swap(revision, None, &workflow)
                .map_err(CommandError::Store)?;
        }
        self.reconcile_amendment_intents()
    }

    /// Reconciles every uncertain Notion effect before that diary day accepts
    /// another command. Exact active intents may adopt a unique matching
    /// section; unrelated same-day content remains a conflict.
    pub fn reconcile_notion_intents(&self) -> Result<(), CommandError> {
        let Some(output) = &self.notion_output else {
            return Ok(());
        };
        for intent in self
            .store
            .unresolved_intents(None)
            .map_err(CommandError::Store)?
        {
            if intent.destination != "notion" {
                continue;
            }
            let mut workflow = self.load_workflow(&intent.day)?;
            if workflow.amendment().is_some() {
                continue;
            }
            let Some(plan) = workflow.output_plan() else {
                continue;
            };
            let Some(target) = plan
                .targets
                .iter()
                .find(|target| target.destination == "notion")
            else {
                continue;
            };
            if target.target_identity != intent.target_identity
                || target.payload_digest != intent.payload_digest
            {
                continue;
            }
            let request = NotionRequest {
                day: intent.day.clone(),
                markdown: workflow.entry().text.clone(),
                expected_target_identity: target.target_identity.clone(),
                expected_payload_digest: target.payload_digest.clone(),
                expected_state_digest: target.inspected_state_digest.clone(),
            };
            let not_cancelled = AtomicBool::new(false);
            let mut destination = NotionDestination {
                output: output.as_ref(),
                request: &request,
                cancelled: &not_cancelled,
            };
            let result = PublicationCoordinator::reconcile(
                self.store.as_ref(),
                &mut destination,
                &intent,
                2,
                self.clock.now(),
            )
            .map_err(|_| CommandError::NotionUnavailable)?;
            let status = match result {
                PublicationResult::Verified(_) => DestinationStatus::Verified,
                PublicationResult::Conflict => DestinationStatus::Conflict,
                PublicationResult::SafeToRetry => DestinationStatus::Pending,
                _ => continue,
            };
            if workflow.state() == diary_core::WorkflowState::Publishing {
                let revision = workflow.revision();
                workflow
                    .finish_publication(
                        revision,
                        vec![
                            DestinationProgress {
                                destination: "local".into(),
                                status: DestinationStatus::Verified,
                            },
                            DestinationProgress {
                                destination: "notion".into(),
                                status,
                            },
                        ],
                        self.clock.now(),
                    )
                    .map_err(CommandError::Workflow)?;
                self.store
                    .compare_and_swap(revision, None, &workflow)
                    .map_err(CommandError::Store)?;
            }
        }
        Ok(())
    }

    pub fn save_local_conflict_under_day(
        &self,
        source_day: &DiaryDay,
        expected_revision: u64,
        new_day: DiaryDay,
    ) -> Result<Workflow, CommandError> {
        self.ensure_clear(source_day)?;
        self.ensure_clear(&new_day)?;
        let source = self.load_workflow(source_day)?;
        ensure_revision(&source, expected_revision)?;
        if source_day == &new_day {
            return Err(CommandError::PreconditionFailed);
        }
        let preferences = self.store.load_preferences().map_err(CommandError::Store)?;
        let carried = source
            .carry_conflict_to_day(new_day.clone(), self.clock.now(), &preferences)
            .map_err(CommandError::Workflow)?;
        match self
            .store
            .start_or_resume(new_day.clone(), self.clock.now(), &preferences)
            .map_err(CommandError::Store)?
        {
            StartOutcome::Created(created) => {
                let output = self
                    .local_output
                    .as_ref()
                    .ok_or(CommandError::LocalOutput(LocalOutputError::Unavailable))?;
                let candidate_payload = render_local_markdown(
                    &new_day,
                    source.language().unwrap_or(WritingLanguage::EnUs),
                    &source.entry().text,
                );
                if output
                    .inspect(&new_day, &candidate_payload)
                    .map_err(CommandError::LocalOutput)?
                    != LocalOutputState::Absent
                {
                    return Ok(created);
                }
                self.store
                    .compare_and_swap(0, None, &carried)
                    .map_err(CommandError::Store)?;
                Ok(carried)
            }
            StartOutcome::Resumed(existing) | StartOutcome::Completed(existing) => Ok(existing),
        }
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
const MIRROR_DESTINATION: &str = "icloud";

/// The workflow's current progress with the named destinations overridden.
/// Destinations absent from the plan are ignored, so callers need not know
/// whether a mirror is configured.
fn progress_with(
    workflow: &Workflow,
    overrides: &[(&str, DestinationStatus)],
) -> Vec<DestinationProgress> {
    workflow
        .destination_progress()
        .iter()
        .map(|item| DestinationProgress {
            destination: item.destination.clone(),
            status: overrides
                .iter()
                .find(|(destination, _)| *destination == item.destination)
                .map_or(item.status, |(_, status)| *status),
        })
        .collect()
}

fn observed_digest(state: LocalOutputState, payload: &str) -> Digest {
    match state {
        LocalOutputState::Absent => Digest::of_bytes(b"absent"),
        LocalOutputState::Identical => Digest::of_bytes(payload.as_bytes()),
        LocalOutputState::Different(text) => Digest::of_bytes(text.as_bytes()),
    }
}

struct LocalDestination<'a> {
    output: &'a dyn LocalOutputPort,
    day: &'a DiaryDay,
    payload: &'a str,
    receipt_id: String,
}

struct NotionDestination<'a> {
    output: &'a dyn NotionOutputPort,
    request: &'a NotionRequest,
    cancelled: &'a AtomicBool,
}

impl Destination for NotionDestination<'_> {
    fn write(&mut self, _: &OperationIntent) -> WriteOutcome {
        self.output.write(self.request, self.cancelled)
    }

    fn inspect(&mut self, _: &OperationIntent) -> InspectionOutcome {
        self.output.inspect(self.request)
    }
}

impl Destination for LocalDestination<'_> {
    fn write(&mut self, _: &OperationIntent) -> WriteOutcome {
        match self.output.write(self.day, self.payload) {
            Ok(LocalOutputState::Identical) => WriteOutcome::Verified {
                receipt_id: self.receipt_id.clone(),
                returned_ids: Vec::new(),
            },
            Ok(LocalOutputState::Different(_)) => WriteOutcome::Conflict,
            Ok(LocalOutputState::Absent) => WriteOutcome::AmbiguousWrite,
            Err(_) => WriteOutcome::Unavailable,
        }
    }

    fn inspect(&mut self, _: &OperationIntent) -> InspectionOutcome {
        match self.output.inspect(self.day, self.payload) {
            Ok(LocalOutputState::Identical) => InspectionOutcome::Matching {
                receipt_id: self.receipt_id.clone(),
                returned_ids: Vec::new(),
            },
            Ok(LocalOutputState::Absent) => InspectionOutcome::Absent,
            Ok(LocalOutputState::Different(_)) => InspectionOutcome::Conflict,
            Err(_) => InspectionOutcome::Unavailable,
        }
    }
}

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

pub(crate) const fn workflow_diagnostic_code(error: &WorkflowError) -> &'static str {
    match error {
        WorkflowError::StaleRevision { .. } => "workflow-stale-revision",
        WorkflowError::StaleBuffer { .. } => "workflow-stale-buffer",
        WorkflowError::WrongState => "wrong-state",
        WorkflowError::EmptySelectionNeedsConfirmation => "empty-selection-needs-confirmation",
        WorkflowError::UnknownSource => "workflow-unknown-source",
        WorkflowError::DuplicateSource => "workflow-duplicate-source",
        WorkflowError::InvalidSourceNoteName => "workflow-invalid-source-note-name",
        WorkflowError::NeedsDownload => "workflow-needs-download",
        WorkflowError::PermissionDeniedSource => "workflow-source-permission-denied",
        WorkflowError::UnreadableSource => "workflow-unreadable-source",
        WorkflowError::AssistantActionsDisabled => "assistant-action-disabled-empty-selection",
        WorkflowError::AssistantRequestInvalid => "assistant-request-invalid",
        WorkflowError::AssistantDraftRequestInvalid => "assistant-draft-request-invalid",
        WorkflowError::AssistantDraftResultInvalid => "assistant-draft-result-invalid",
        WorkflowError::AssistantParagraphRangeInvalid => "assistant-paragraph-range-invalid",
        WorkflowError::AssistantParagraphRequestInvalid => "assistant-paragraph-request-invalid",
        WorkflowError::AssistantParagraphResultInvalid => "assistant-paragraph-result-invalid",
        WorkflowError::AssistantBackendRequired => "assistant-backend-required",
        WorkflowError::ReviewedMemoryRequired => "reviewed-memory-required",
        WorkflowError::AssistantArtifactNotFound => "assistant-artifact-not-found",
        WorkflowError::AssistantBindingStale => "assistant-binding-stale",
        WorkflowError::AssistantSuggestionAlreadyResolved => {
            "assistant-suggestion-already-resolved"
        }
        WorkflowError::LanguageRequired => "language-required",
        WorkflowError::BlankEntry => "blank-entry",
        WorkflowError::UnresolvedInferenceMarker => "assistant-unresolved-reflection-marker",
        WorkflowError::EntryNotConfirmed => "entry-not-confirmed",
        WorkflowError::MissingPlan => "workflow-missing-plan",
        WorkflowError::PlanBindingMismatch => "workflow-plan-binding-mismatch",
        WorkflowError::UnknownDestination => "workflow-unknown-destination",
        WorkflowError::LocalMustBeVerified => "workflow-local-must-be-verified",
        WorkflowError::RetryNeedsPreview => "workflow-retry-needs-preview",
        WorkflowError::MissingApproval => "workflow-missing-approval",
        WorkflowError::VerifiedDestinationRegressed => "workflow-verified-destination-regressed",
        WorkflowError::DestinationOutsideExecutionScope => {
            "workflow-destination-outside-execution-scope"
        }
        WorkflowError::InvalidPersistedState => "workflow-invalid-persisted-state",
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
    LocalOutput(LocalOutputError),
    NotionUnavailable,
}

impl CommandError {
    /// The log-only counterpart of [`Self::code`]. The wire code folds many
    /// workflow errors into `workflow-conflict`; the diagnostic log needs the
    /// exact failed check, and the matches below stay wildcard-free so a new
    /// `WorkflowError` cannot ship without a token.
    #[must_use]
    pub const fn diagnostic_code(&self) -> &'static str {
        match self {
            Self::Workflow(error) => workflow_diagnostic_code(error),
            other => other.code(),
        }
    }

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
            Self::LocalOutput(LocalOutputError::UnsafePath) => "local-output-unsafe-path",
            Self::LocalOutput(LocalOutputError::Unavailable) => "local-output-unavailable",
            Self::NotionUnavailable => "notion-unavailable",
            Self::Store(_) => "state-unavailable",
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn every_workflow_error_has_its_own_diagnostic_token() {
        let errors = [
            WorkflowError::StaleRevision {
                expected: 1,
                actual: 2,
            },
            WorkflowError::StaleBuffer {
                expected: 1,
                actual: 2,
            },
            WorkflowError::WrongState,
            WorkflowError::EmptySelectionNeedsConfirmation,
            WorkflowError::UnknownSource,
            WorkflowError::DuplicateSource,
            WorkflowError::InvalidSourceNoteName,
            WorkflowError::NeedsDownload,
            WorkflowError::PermissionDeniedSource,
            WorkflowError::UnreadableSource,
            WorkflowError::AssistantActionsDisabled,
            WorkflowError::AssistantRequestInvalid,
            WorkflowError::AssistantDraftRequestInvalid,
            WorkflowError::AssistantDraftResultInvalid,
            WorkflowError::AssistantParagraphRangeInvalid,
            WorkflowError::AssistantParagraphRequestInvalid,
            WorkflowError::AssistantParagraphResultInvalid,
            WorkflowError::AssistantBackendRequired,
            WorkflowError::ReviewedMemoryRequired,
            WorkflowError::AssistantArtifactNotFound,
            WorkflowError::AssistantBindingStale,
            WorkflowError::AssistantSuggestionAlreadyResolved,
            WorkflowError::LanguageRequired,
            WorkflowError::BlankEntry,
            WorkflowError::UnresolvedInferenceMarker,
            WorkflowError::EntryNotConfirmed,
            WorkflowError::MissingPlan,
            WorkflowError::PlanBindingMismatch,
            WorkflowError::UnknownDestination,
            WorkflowError::LocalMustBeVerified,
            WorkflowError::RetryNeedsPreview,
            WorkflowError::MissingApproval,
            WorkflowError::VerifiedDestinationRegressed,
            WorkflowError::DestinationOutsideExecutionScope,
            WorkflowError::InvalidPersistedState,
        ];
        let tokens: BTreeSet<_> = errors.iter().map(workflow_diagnostic_code).collect();
        assert_eq!(tokens.len(), errors.len());
        assert!(!tokens.contains("workflow-conflict"));
        assert!(tokens.iter().all(|token| {
            token.len() <= 64
                && token
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte == b'-')
        }));
    }

    #[test]
    fn wire_code_is_unchanged_while_the_log_code_names_the_check() {
        let error = CommandError::Workflow(WorkflowError::ReviewedMemoryRequired);
        assert_eq!(error.code(), "workflow-conflict");
        assert_eq!(error.diagnostic_code(), "reviewed-memory-required");
        assert_eq!(
            CommandError::NotFound.diagnostic_code(),
            CommandError::NotFound.code()
        );
    }
}
