//! Inward-facing ports implemented by outer adapters.

use std::fmt;
use std::sync::atomic::AtomicBool;

use crate::{
    AssistantBackendDescriptor, AssistantRequest, AssistantResponse, CatalogRequest, DiaryDay,
    DraftAssistantError, FreshnessCheckpoint, FreshnessOutcome, InspectionOutcome, OperationIntent,
    Preferences, Receipt, ReconciliationGate, SourceAccessError, SourceDiscovery,
    SourceReadOutcome, SourceSnapshot, Timestamp, Workflow, WorkflowError, WorkflowSummary,
    WriteOutcome,
};

pub trait Clock {
    fn now(&self) -> Timestamp;
    /// The current diary day in the Mac's local time zone.
    fn local_day(&self) -> DiaryDay;
}

pub fn start_default_workflow(
    store: &impl WorkflowStore,
    clock: &impl Clock,
) -> Result<StartOutcome, StoreError> {
    let preferences = store.load_preferences()?;
    store.start_or_resume(clock.local_day(), clock.now(), &preferences)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StartOutcome {
    Created(Workflow),
    Resumed(Workflow),
    Completed(Workflow),
}

pub trait WorkflowStore {
    fn start_or_resume(
        &self,
        day: DiaryDay,
        now: Timestamp,
        preferences: &Preferences,
    ) -> Result<StartOutcome, StoreError>;

    fn load(&self, day: &DiaryDay) -> Result<Option<Workflow>, StoreError>;

    /// Returns at most `limit` summaries, unfinished and most-recently updated
    /// first, followed by completed workflows in the same ordering.
    fn list_summaries(&self, limit: usize) -> Result<Vec<WorkflowSummary>, StoreError>;

    /// Atomically replaces a workflow when both independent versions still
    /// match. The buffer condition is optional for non-editor commands.
    fn compare_and_swap(
        &self,
        expected_revision: u64,
        expected_buffer_version: Option<u64>,
        workflow: &Workflow,
    ) -> Result<(), StoreError>;

    fn load_preferences(&self) -> Result<Preferences, StoreError>;
    fn save_preferences(&self, preferences: &Preferences) -> Result<(), StoreError>;

    /// Atomically commits per-entry writing settings and remembered choices.
    fn compare_and_swap_with_preferences(
        &self,
        expected_revision: u64,
        workflow: &Workflow,
        preferences: &Preferences,
    ) -> Result<(), StoreError>;

    /// Clears recoverable source/assistant/entry text but preserves digests,
    /// receipts, and unresolved intents.
    fn discard(
        &self,
        day: &DiaryDay,
        expected_revision: u64,
        now: Timestamp,
    ) -> Result<(), StoreError>;

    /// Applies the seven-day verified-output and 30-day abandoned-draft rules.
    fn purge_expired(&self, now: Timestamp) -> Result<Vec<DiaryDay>, StoreError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IntentTerminal {
    DefiniteNoEffect,
    Conflict,
}

impl IntentTerminal {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DefiniteNoEffect => "definite-no-effect",
            Self::Conflict => "conflict",
        }
    }
}

pub trait OperationLog {
    /// Returns `false` for an exact duplicate attempt ID.
    fn record_intent(&self, intent: &OperationIntent) -> Result<bool, StoreError>;
    fn mark_terminal(&self, attempt_id: &str, terminal: IntentTerminal) -> Result<(), StoreError>;
    /// Stores the receipt and resolves its intent in one transaction.
    fn record_receipt(&self, receipt: &Receipt) -> Result<(), StoreError>;
    fn unresolved_intents(
        &self,
        day: Option<&DiaryDay>,
    ) -> Result<Vec<OperationIntent>, StoreError>;
    fn request_cancellation(&self, attempt_id: &str) -> Result<(), StoreError>;
    fn reconciliation_gate(&self, day: &DiaryDay) -> Result<ReconciliationGate, StoreError>;
    fn receipts(&self, day: &DiaryDay) -> Result<Vec<Receipt>, StoreError>;
}

pub trait Destination {
    fn write(&mut self, intent: &OperationIntent) -> WriteOutcome;
    fn inspect(&mut self, intent: &OperationIntent) -> InspectionOutcome;
}

pub trait SourceCatalog {
    fn discover(&self, request: CatalogRequest) -> Result<SourceDiscovery, SourceAccessError>;
}

pub trait SourceReader {
    fn read_selected(
        &self,
        source: &crate::SourceRef,
    ) -> Result<SourceReadOutcome, SourceAccessError>;

    fn revalidate(&self, snapshot: &SourceSnapshot) -> Result<FreshnessOutcome, SourceAccessError>;

    fn request_download(&self, source: &crate::SourceRef) -> Result<(), SourceAccessError>;
}

pub trait SourceWriter {
    fn overwrite_body(
        &self,
        source: &crate::SourceRef,
        new_body: &str,
        expected_digest: &crate::Digest,
    ) -> Result<crate::FreshnessStamp, SourceWriteError>;

    fn delete_source(&self, source: &crate::SourceRef) -> Result<(), SourceWriteError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceWriteError {
    Stale,
    UnsafePath,
    PartialFailure,
    Unavailable,
}

/// One assistant backend. The port exposes no filesystem, connector, workflow,
/// confirmation, approval, publication, or shell capability.
pub trait DraftAssistant: Send + Sync {
    fn descriptor(&self) -> AssistantBackendDescriptor;

    fn invoke(
        &self,
        request: &AssistantRequest,
        cancelled: &AtomicBool,
    ) -> Result<AssistantResponse, DraftAssistantError>;
}

pub fn revalidate_workflow_sources(
    workflow: &mut Workflow,
    reader: &(impl SourceReader + ?Sized),
    checkpoint: FreshnessCheckpoint,
    now: Timestamp,
) -> Result<(), SourceCheckpointError> {
    let snapshots = workflow.snapshots().to_vec();
    for snapshot in snapshots {
        let outcome = match reader.revalidate(&snapshot) {
            Ok(outcome) => outcome,
            Err(error) => {
                workflow
                    .apply_freshness(
                        workflow.revision(),
                        checkpoint,
                        FreshnessOutcome::Unverifiable {
                            source: snapshot.source,
                        },
                        now,
                    )
                    .map_err(SourceCheckpointError::Workflow)?;
                return Err(SourceCheckpointError::Access(error));
            }
        };
        workflow
            .apply_freshness(workflow.revision(), checkpoint, outcome.clone(), now)
            .map_err(SourceCheckpointError::Workflow)?;
        if outcome != FreshnessOutcome::Fresh {
            break;
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SourceCheckpointError {
    Access(SourceAccessError),
    Workflow(WorkflowError),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StoreError {
    StaleRevision,
    StaleBuffer,
    DuplicateDay,
    UnsupportedSchema(i64),
    Corrupt(String),
    InvalidRecord(String),
    Unavailable(String),
}

impl fmt::Display for StoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StaleRevision => formatter.write_str("stale workflow revision"),
            Self::StaleBuffer => formatter.write_str("stale entry buffer version"),
            Self::DuplicateDay => formatter.write_str("workflow day already exists"),
            Self::UnsupportedSchema(version) => {
                write!(formatter, "unsupported database schema version {version}")
            }
            Self::Corrupt(_) => formatter.write_str("private state database is corrupt"),
            Self::InvalidRecord(_) => formatter.write_str("private state record is invalid"),
            Self::Unavailable(_) => formatter.write_str("private state database is unavailable"),
        }
    }
}

impl std::error::Error for StoreError {}
