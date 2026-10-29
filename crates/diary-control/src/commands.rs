use std::sync::Arc;

use diary_core::{
    CatalogRequest, Clock, CompositionPath, DiaryDay, FreshnessCheckpoint, FreshnessOutcome,
    OperationLog, Preferences, ReconciliationGate, SelectedSource, SourceAccessError,
    SourceCatalog, SourceReadOutcome, SourceReader, SourceRef, StartOutcome, StoreError, Timestamp,
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
        path: CompositionPath,
    ) -> Result<Workflow, CommandError> {
        self.ensure_clear(day)?;
        let mut workflow = self.load_workflow(day)?;
        ensure_revision(&workflow, expected_revision)?;
        workflow
            .set_writing_settings(expected_revision, language, style, path, self.clock.now())
            .map_err(CommandError::Workflow)?;
        let preferences = Preferences {
            last_language: Some(language),
            last_composition_path: path,
        };
        self.store
            .compare_and_swap_with_preferences(expected_revision, &workflow, &preferences)
            .map_err(CommandError::Store)?;
        Ok(workflow)
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

fn ensure_revision(workflow: &Workflow, expected: u64) -> Result<(), CommandError> {
    if workflow.revision() == expected {
        Ok(())
    } else {
        Err(CommandError::PreconditionFailed)
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
    UnavailablePath,
    Store(StoreError),
    Workflow(WorkflowError),
    Source(SourceAccessError),
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
            Self::UnavailablePath => "composition-path-unavailable",
            Self::Workflow(WorkflowError::WrongState) => "wrong-state",
            Self::Workflow(WorkflowError::BlankEntry) => "blank-entry",
            Self::Workflow(WorkflowError::LanguageRequired) => "language-required",
            Self::Workflow(WorkflowError::UnresolvedInferenceMarker) => {
                "unresolved-inference-marker"
            }
            Self::Workflow(_) => "workflow-conflict",
            Self::Source(SourceAccessError::Busy) => "source-busy",
            Self::Source(SourceAccessError::TimedOut) => "source-timeout",
            Self::Source(_) => "source-unavailable",
            Self::Store(_) => "state-unavailable",
        }
    }
}
