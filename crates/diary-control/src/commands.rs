use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use diary_core::{
    ArtifactProvenance, AssistantBackendDescriptor, AssistantRequest, AssistantResponse,
    AssistantSelection, CatalogRequest, Clock, Destination, DestinationKind, DestinationPlan,
    DestinationProgress, DestinationStatus, DiaryDay, Digest, DraftAssistantError,
    DraftAssistantRegistry, FreshnessCheckpoint, FreshnessOutcome, InspectionOutcome,
    LocalOutputError, LocalOutputPort, LocalOutputState, NotionBlockPreview, NotionOutputPort,
    NotionPreflightStatus, NotionRequest, OperationIntent, OperationLog, OutputPlan, Preferences,
    PublicationCoordinator, PublicationError, PublicationResult, ReconciliationGate,
    ReflectionRange, RegistryInvokeError, SelectedSource, SourceAccessError, SourceCatalog,
    SourceReadOutcome, SourceReader, SourceRef, SourceWriteError, SourceWriter, StartOutcome,
    StoreError, Timestamp, Workflow, WorkflowError, WorkflowStore, WorkflowSummary, WriteOutcome,
    WritingLanguage, WritingStyle, render_local_markdown, revalidate_workflow_sources,
};

use crate::lifecycle::random_id;

mod amendment;
pub use amendment::{AmendmentNotionPreview, AmendmentPreview, AmendmentSeeded, local_body};

/// An explicitly requested day is the Mac-local calendar day, while Ayumi
/// files are dated in UTC. A note whose name carries no readable UTC time
/// falls back to its directory date.
fn on_local_day(candidate: &diary_core::SourceCandidate, requested: &DiaryDay) -> bool {
    match crate::local_time::local_note_day(&candidate.source_note_name) {
        Some(day) => &day == requested,
        None => candidate.date_evidence.directory_day.as_ref() == Some(requested),
    }
}

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

    /// Starts a new day with the current writing choices. Resuming an existing
    /// day leaves its own settings untouched.
    pub fn start_workflow_with_settings(
        &self,
        day: Option<DiaryDay>,
        language: Option<WritingLanguage>,
        style: Option<WritingStyle>,
        descriptors: &[AssistantBackendDescriptor],
    ) -> Result<StartOutcome, CommandError> {
        let outcome = self.start_workflow(day)?;
        let StartOutcome::Created(workflow) = outcome else {
            return Ok(outcome);
        };
        let preferences = self.preferences()?;
        let preferred = preferences
            .last_assistant_backend_id
            .and_then(|backend_id| {
                descriptors
                    .iter()
                    .find(|descriptor| descriptor.id == backend_id && descriptor.available)
                    .map(|descriptor| {
                        AssistantSelection::new(backend_id, descriptor.composition_path)
                    })
            });
        if preferred.is_none() && language.is_none() && style.is_none() {
            return Ok(StartOutcome::Created(workflow));
        }
        let effective_language = language
            .or(workflow.language())
            .unwrap_or(WritingLanguage::EnUs);
        let effective_style = style.unwrap_or(workflow.style());
        let workflow = self.set_writing_settings(
            workflow.day(),
            workflow.revision(),
            effective_language,
            effective_style,
            preferred,
        )?;
        Ok(StartOutcome::Created(workflow))
    }

    /// Receipt and unresolved-intent counts for a day, for the content-free
    /// publication diagnostics.
    pub fn publication_counts(&self, day: &DiaryDay) -> Result<(u64, u64), CommandError> {
        let receipts = self.store.receipts(day).map_err(CommandError::Store)?.len();
        let unresolved = self
            .store
            .unresolved_intents(Some(day))
            .map_err(CommandError::Store)?
            .len();
        Ok((receipts as u64, unresolved as u64))
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
}

mod assistants;
mod composition;
mod local_publication;
mod mirror_publication;
mod notion_publication;
mod reconciliation;
mod sources;

impl CommandService {
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
    /// The Notion preflight no longer matches the approved plan. The wire code
    /// stays `precondition-failed`; the token names the check for the log.
    NotionBindingChanged(&'static str),
    /// The Notion preflight could not run. The wire code stays
    /// `notion-unavailable`; the token is the closed `notion-*` reason.
    NotionUnavailableAt(&'static str),
}

/// Maps a connector preflight failure onto a closed token so the diagnostic
/// log can name it; anything unlisted collapses to `notion-unavailable`.
fn notion_preflight_error(reason: &str) -> CommandError {
    let token = crate::diagnostics::NOTION_PREVIEW_REASONS
        .iter()
        .copied()
        .find(|known| *known == reason)
        .unwrap_or("notion-unavailable");
    CommandError::NotionUnavailableAt(token)
}

/// Names the first way a fresh Notion preflight departs from the approved
/// target, or `None` while it still matches. `check_target` is false for the
/// replace preflight, which has no target identity in the plan to compare.
fn notion_binding_refusal(
    preflight: &diary_core::NotionPreflight,
    target: &DestinationPlan,
    check_target: bool,
) -> Option<&'static str> {
    match preflight.status {
        NotionPreflightStatus::Ready => {}
        NotionPreflightStatus::Blocked => return Some("notion-preflight-blocked"),
        NotionPreflightStatus::Conflict => return Some("notion-preflight-conflict"),
    }
    if check_target && preflight.target_identity != target.target_identity {
        Some("notion-target-changed")
    } else if preflight.payload_digest != target.payload_digest {
        Some("notion-payload-changed")
    } else if preflight.observed_state_digest != target.inspected_state_digest {
        Some("notion-state-changed")
    } else {
        None
    }
}

/// Buckets a SQLite failure message into a closed token so the diagnostic log
/// can tell a lock from a missing column without carrying the message itself.
fn unavailable_diagnostic_code(detail: &str) -> &'static str {
    let detail = detail.to_ascii_lowercase();
    [
        ("locked", "state-locked"),
        ("busy", "state-locked"),
        ("readonly", "state-readonly"),
        ("read-only", "state-readonly"),
        ("disk is full", "state-disk-full"),
        ("disk i/o", "state-io"),
        ("unable to open", "state-open-failed"),
        ("no such table", "state-schema-mismatch"),
        ("no such column", "state-schema-mismatch"),
        ("constraint", "state-constraint"),
        ("trigger", "state-constraint"),
        ("query returned no rows", "state-row-missing"),
        ("out of range", "state-row-invalid"),
        ("invalid column", "state-row-invalid"),
        ("invalid type", "state-row-invalid"),
    ]
    .into_iter()
    .find(|(needle, _)| detail.contains(needle))
    .map_or("state-unavailable", |(_, token)| token)
}

impl CommandError {
    /// The log-only counterpart of [`Self::code`]. The wire code folds many
    /// workflow errors into `workflow-conflict`; the diagnostic log needs the
    /// exact failed check, and the matches below stay wildcard-free so a new
    /// `WorkflowError` cannot ship without a token.
    #[must_use]
    pub fn diagnostic_code(&self) -> &'static str {
        match self {
            Self::Workflow(error) => workflow_diagnostic_code(error),
            Self::Assistant(DraftAssistantError::InvalidOutputAt(reason)) => reason,
            Self::Store(StoreError::DuplicateDay) => "state-duplicate-day",
            Self::Store(StoreError::UnsupportedSchema(_)) => "state-unsupported-schema",
            Self::Store(StoreError::Corrupt(_)) => "state-corrupt",
            Self::Store(StoreError::InvalidRecord(_)) => "state-invalid-record",
            Self::Store(StoreError::Unavailable(detail)) => unavailable_diagnostic_code(detail),
            Self::NotionBindingChanged(reason) | Self::NotionUnavailableAt(reason) => reason,
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
            Self::Assistant(
                DraftAssistantError::InvalidOutput | DraftAssistantError::InvalidOutputAt(_),
            ) => "assistant-invalid-output",
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
            Self::NotionUnavailable | Self::NotionUnavailableAt(_) => "notion-unavailable",
            Self::NotionBindingChanged(_) => "precondition-failed",
            Self::Store(_) => "state-unavailable",
        }
    }
}

#[cfg(test)]
mod tests;
