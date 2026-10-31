//! Inward-facing ports implemented by outer adapters.

use std::fmt;
use std::sync::atomic::AtomicBool;

use crate::{
    AssistantBackendDescriptor, AssistantRequest, AssistantResponse, CatalogRequest, DiaryDay,
    Digest, DraftAssistantError, FreshnessCheckpoint, FreshnessOutcome, InspectionOutcome,
    OperationIntent, Preferences, Receipt, ReconciliationGate, SourceAccessError, SourceDiscovery,
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LocalOutputState {
    Absent,
    Identical,
    Different(String),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocalOutputError {
    UnsafePath,
    Unavailable,
}

/// Result of a digest-guarded replacement of an app-owned file.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocalReplaceOutcome {
    Replaced,
    /// The file already holds the replacement bytes (a replayed attempt).
    AlreadyReplaced,
    /// The file is absent or no longer matches the expected digest; nothing
    /// was written.
    Changed,
}

pub trait LocalOutputPort: Send + Sync {
    /// Atomically replaces the day's file, but only while its current bytes
    /// hash to `expected`. The default refuses so a port without replacement
    /// support can never overwrite.
    fn replace(
        &self,
        _day: &DiaryDay,
        _expected: &Digest,
        _payload: &str,
    ) -> Result<LocalReplaceOutcome, LocalOutputError> {
        Err(LocalOutputError::Unavailable)
    }
    /// Copies the day's file beside it, but only while its current bytes hash
    /// to `expected`, and reads the copy back. `Ok(false)` means the file is
    /// absent or changed and nothing was copied. The default refuses, so a
    /// port without backup support can never be asked to overwrite.
    fn backup(&self, _day: &DiaryDay, _expected: &Digest) -> Result<bool, LocalOutputError> {
        Err(LocalOutputError::Unavailable)
    }
    fn target(&self, day: &DiaryDay) -> String;
    fn inspect(&self, day: &DiaryDay, payload: &str) -> Result<LocalOutputState, LocalOutputError>;
    fn write(&self, day: &DiaryDay, payload: &str) -> Result<LocalOutputState, LocalOutputError>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NotionBlockPreview {
    pub kind: String,
    pub text: String,
    pub depth: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NotionPreflightStatus {
    Ready,
    Blocked,
    Conflict,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NotionPreflight {
    /// Stable server-only identity. It must never enter a DTO or diagnostic.
    pub target_identity: String,
    pub root_label: String,
    pub year_title: String,
    pub month_title: String,
    pub year_exists: bool,
    pub month_exists: bool,
    /// Where the new daily section goes on the month page (`end`,
    /// `after YYYY-MM-DD`, or `before YYYY-MM-DD`); content-free.
    pub placement: String,
    pub renderer_version: u32,
    pub api_version: String,
    pub payload_digest: Digest,
    pub observed_state_digest: Digest,
    pub blocks: Vec<NotionBlockPreview>,
    pub status: NotionPreflightStatus,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NotionRequest {
    pub day: DiaryDay,
    pub markdown: String,
    pub expected_target_identity: String,
    pub expected_payload_digest: Digest,
    pub expected_state_digest: Digest,
}

/// Replaces only the section the app itself receipted. `owned_*` come from
/// the latest Notion receipt; nothing here is user-supplied.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NotionReplaceRequest {
    pub day: DiaryDay,
    pub markdown: String,
    pub expected_target_identity: String,
    pub expected_payload_digest: Digest,
    /// Month page the receipt recorded.
    pub owned_month_id: String,
    /// Receipted heading block ID first, then the owned body block IDs.
    pub owned_block_ids: Vec<String>,
    /// Payload digest the receipt recorded for the owned section.
    pub owned_payload_digest: Digest,
}

/// The canonical same-day section as it stands on the month page, when the
/// app did not write it. Appending to it or replacing it needs a request built
/// from this observation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NotionExistingSection {
    pub month_id: String,
    /// Heading block ID first, then the body block IDs.
    pub block_ids: Vec<String>,
    /// Digest of the heading and body blocks as read.
    pub section_digest: Digest,
    pub blocks: Vec<NotionBlockPreview>,
}

/// Adds a dated separator and the new text to an existing section, after its
/// last block. `owned_*` come from [`NotionExistingSection`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NotionExtendRequest {
    pub day: DiaryDay,
    pub markdown: String,
    /// Mac-local `YYYY-MM-DD HH:MM`, fixed when the plan was built.
    pub appended_at: String,
    pub expected_target_identity: String,
    /// Digest of the whole section once the addition lands; `None` while
    /// previewing, when the adapter computes it.
    pub expected_result_digest: Option<Digest>,
    pub owned_month_id: String,
    pub owned_block_ids: Vec<String>,
    pub owned_section_digest: Digest,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NotionReplacePreflight {
    /// `observed_state_digest` is the digest of the verified owned section.
    /// A mismatch reports `Conflict` with the reason.
    pub preflight: NotionPreflight,
    pub current_blocks: Vec<NotionBlockPreview>,
}

/// Secret-bearing remote destination. Implementations own credentials and
/// remote IDs; neither is exposed through preview DTOs or diagnostics.
pub trait NotionOutputPort: Send + Sync {
    fn preflight(&self, day: &DiaryDay, markdown: &str) -> Result<NotionPreflight, String>;
    fn write(&self, request: &NotionRequest, cancelled: &AtomicBool) -> WriteOutcome;
    fn inspect(&self, request: &NotionRequest) -> InspectionOutcome;

    /// Read-only: the web URL of the day's Notion month page, for a person to
    /// open and fix a page this app cannot safely change. `None` when the
    /// page cannot be resolved. Never logged and never placed in a diagnostic.
    fn month_page_url(&self, _day: &DiaryDay) -> Option<String> {
        None
    }

    /// Read-only: verifies the owned section against its receipt.
    fn preflight_replace(
        &self,
        _request: &NotionReplaceRequest,
    ) -> Result<NotionReplacePreflight, String> {
        Err("notion-amend-unsupported".to_owned())
    }

    /// Resumable replacement: inserts the new body after the heading, reads it
    /// back, then trashes the old body blocks. It inspects before every step,
    /// so calling it again after an interruption never duplicates a write.
    fn replace_section(&self, _request: &NotionReplaceRequest, _: &AtomicBool) -> WriteOutcome {
        WriteOutcome::Unavailable
    }

    /// Finishes an interrupted replacement by trashing whichever set is stale,
    /// never inserting, and reports the resulting section.
    fn reconcile_replace(&self, _request: &NotionReplaceRequest) -> InspectionOutcome {
        InspectionOutcome::Unavailable
    }

    /// Read-only: the canonical same-day section, `None` when the day has no
    /// section, or the content-free reason when it cannot be appended to or
    /// replaced (duplicate or non-canonical headings, unreadable page).
    fn observe_existing(&self, _day: &DiaryDay) -> Result<Option<NotionExistingSection>, String> {
        Err("notion-existing-unsupported".to_owned())
    }

    /// Read-only: previews the addition to a section against the content the
    /// person saw. The preflight's `payload_digest` is the digest of the whole
    /// section after the addition.
    fn preflight_extend(
        &self,
        _request: &NotionExtendRequest,
    ) -> Result<NotionReplacePreflight, String> {
        Err("notion-existing-unsupported".to_owned())
    }

    /// Resumable addition: inspects the section before every chunk, so calling
    /// it again after an interruption never duplicates a write.
    fn extend_section(&self, _request: &NotionExtendRequest, _: &AtomicBool) -> WriteOutcome {
        WriteOutcome::Unavailable
    }
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
