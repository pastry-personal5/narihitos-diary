//! The server-owned diary workflow state machine.

use std::collections::BTreeSet;
use std::fmt;

use crate::digest::push_framed;
use crate::{
    AmendmentSeed, Approval, ArtifactProvenance, AssistantSelection, CompositionPath,
    DestinationKind, DestinationProgress, DestinationStatus, DiaryDay, Digest, DraftEntryRequest,
    DraftEntryResult, DraftProposal, DraftProposalBinding, EntryBuffer, EntryConfirmation,
    FreshnessCheckpoint, FreshnessOutcome, InvalidationReason, JobStatus, MemoryCandidateArtifact,
    MemoryCandidateBinding, MemoryCandidateSet, MemoryExtractionRequest, MemoryExtractionResult,
    OutputPlan, ParagraphReviewRequest, ParagraphReviewResult, ParagraphSuggestion,
    ParagraphSuggestionBinding, Preferences, ReflectionRange, ReviewedMemoryArtifact,
    ReviewedMemoryBinding, SelectedSource, SourceCandidate, SourceRefineSuggestion, SourceSnapshot,
    SuggestionResolution, Timestamp, WorkflowState, WritingLanguage, WritingStyle,
    source_note_name_is_valid,
};

/// Reserved markers used around assistant-inferred reflection text.
pub const INFERRED_REFLECTION_OPEN: char = '⟪';
pub const INFERRED_REFLECTION_CLOSE: char = '⟫';

/// Direct forward state edges. Invalidation and retry edges are enforced by
/// their owning commands rather than treated as free-form transitions.
pub const LEGAL_TRANSITIONS: &[(WorkflowState, WorkflowState)] = &[
    (WorkflowState::Started, WorkflowState::Discovered),
    (WorkflowState::Discovered, WorkflowState::SourcesConfirmed),
    (WorkflowState::SourcesConfirmed, WorkflowState::Composing),
    (WorkflowState::Composing, WorkflowState::EntryConfirmed),
    (WorkflowState::EntryConfirmed, WorkflowState::OutputPlanned),
    (WorkflowState::OutputPlanned, WorkflowState::Approved),
    (WorkflowState::Approved, WorkflowState::Publishing),
    (WorkflowState::Publishing, WorkflowState::Completed),
    (WorkflowState::Publishing, WorkflowState::Partial),
    (WorkflowState::Partial, WorkflowState::Publishing),
    (WorkflowState::Partial, WorkflowState::OutputPlanned),
    (WorkflowState::Completed, WorkflowState::Composing),
    (WorkflowState::Completed, WorkflowState::OutputPlanned),
    (WorkflowState::Completed, WorkflowState::Publishing),
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Workflow {
    day: DiaryDay,
    state: WorkflowState,
    revision: u64,
    created_at: Timestamp,
    updated_at: Timestamp,
    candidates: Vec<SourceCandidate>,
    snapshots: Vec<SourceSnapshot>,
    selection_digest: Option<Digest>,
    explicit_empty_selection: bool,
    language: Option<WritingLanguage>,
    style: WritingStyle,
    assistant_selection: Option<AssistantSelection>,
    reviewed_memories: Vec<String>,
    assistant_draft: Option<String>,
    assistant_material_stale: bool,
    memory_candidate_sets: Vec<MemoryCandidateSet>,
    reviewed_memory_artifact: Option<ReviewedMemoryArtifact>,
    draft_proposals: Vec<DraftProposal>,
    paragraph_suggestions: Vec<ParagraphSuggestion>,
    source_refine_suggestions: Vec<SourceRefineSuggestion>,
    entry: EntryBuffer,
    output_plan: Option<OutputPlan>,
    approval: Option<Approval>,
    destination_progress: Vec<DestinationProgress>,
    invalidation_reason: Option<InvalidationReason>,
    job_status: Option<JobStatus>,
    all_verified_at: Option<Timestamp>,
    discarded_at: Option<Timestamp>,
    amendment: Option<AmendmentSeed>,
}

/// Adapter-neutral durable representation. Adapters must round-trip through
/// [`Workflow::restore`] so invalid persisted records fail closed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PersistedWorkflow {
    pub day: DiaryDay,
    pub state: WorkflowState,
    pub revision: u64,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub candidates: Vec<SourceCandidate>,
    pub snapshots: Vec<SourceSnapshot>,
    pub selection_digest: Option<Digest>,
    pub explicit_empty_selection: bool,
    pub language: Option<WritingLanguage>,
    pub style: WritingStyle,
    pub assistant_selection: Option<AssistantSelection>,
    pub reviewed_memories: Vec<String>,
    pub assistant_draft: Option<String>,
    pub assistant_material_stale: bool,
    pub memory_candidate_sets: Vec<MemoryCandidateSet>,
    pub reviewed_memory_artifact: Option<ReviewedMemoryArtifact>,
    pub draft_proposals: Vec<DraftProposal>,
    pub paragraph_suggestions: Vec<ParagraphSuggestion>,
    pub source_refine_suggestions: Vec<SourceRefineSuggestion>,
    pub entry: EntryBuffer,
    pub output_plan: Option<OutputPlan>,
    pub approval: Option<Approval>,
    pub destination_progress: Vec<DestinationProgress>,
    pub invalidation_reason: Option<InvalidationReason>,
    pub job_status: Option<JobStatus>,
    pub all_verified_at: Option<Timestamp>,
    pub discarded_at: Option<Timestamp>,
    pub amendment: Option<AmendmentSeed>,
}

impl Workflow {
    /// Seeds a new diary day from a conflicted local plan. The carried text
    /// and selection require fresh confirmation before any output.
    pub fn carry_conflict_to_day(
        &self,
        new_day: DiaryDay,
        now: Timestamp,
        preferences: &crate::Preferences,
    ) -> Result<Self, WorkflowError> {
        if !matches!(
            self.state,
            WorkflowState::Partial | WorkflowState::OutputPlanned
        ) || !self
            .destination_progress
            .iter()
            .any(|item| item.destination == "local" && item.status == DestinationStatus::Conflict)
        {
            return Err(WorkflowError::WrongState);
        }
        let mut next = Self::new(new_day, now, preferences);
        next.candidates = self.candidates.clone();
        next.snapshots = self.snapshots.clone();
        next.language = self.language;
        next.style = self.style;
        next.entry.text = self.entry.text.clone();
        next.entry.version = 1;
        next.entry.confirmed = None;
        next.state = WorkflowState::Discovered;
        next.revision = 1;
        Ok(next)
    }
    #[must_use]
    pub fn new(day: DiaryDay, now: Timestamp, preferences: &Preferences) -> Self {
        Self {
            day,
            state: WorkflowState::Started,
            revision: 0,
            created_at: now,
            updated_at: now,
            candidates: Vec::new(),
            snapshots: Vec::new(),
            selection_digest: None,
            explicit_empty_selection: false,
            language: preferences.last_language,
            style: WritingStyle::Concise,
            assistant_selection: None,
            reviewed_memories: Vec::new(),
            assistant_draft: None,
            assistant_material_stale: false,
            memory_candidate_sets: Vec::new(),
            reviewed_memory_artifact: None,
            draft_proposals: Vec::new(),
            paragraph_suggestions: Vec::new(),
            source_refine_suggestions: Vec::new(),
            entry: EntryBuffer::default(),
            output_plan: None,
            approval: None,
            destination_progress: Vec::new(),
            invalidation_reason: None,
            job_status: None,
            all_verified_at: None,
            discarded_at: None,
            amendment: None,
        }
    }

    pub fn restore(record: PersistedWorkflow) -> Result<Self, WorkflowError> {
        let mut record = record;
        // P1-M22 migration: before remotes stopped gating completion, a day
        // whose local file was verified but whose iCloud or Notion row was not
        // stayed `Partial`. Such a day is now `Completed`; its remote rows keep
        // their statuses and are not deleted.
        if record.state == WorkflowState::Partial
            && record.amendment.is_none()
            && record.all_verified_at.is_none()
            && record.output_plan.as_ref().is_some_and(|plan| {
                plan.targets
                    .iter()
                    .any(|target| target.kind == DestinationKind::Local)
                    && ensure_every_local_verified(plan, &record.destination_progress).is_ok()
            })
        {
            record.state = WorkflowState::Completed;
            record.all_verified_at = Some(record.updated_at);
        }
        let workflow = Self {
            day: record.day,
            state: record.state,
            revision: record.revision,
            created_at: record.created_at,
            updated_at: record.updated_at,
            candidates: record.candidates,
            snapshots: record.snapshots,
            selection_digest: record.selection_digest,
            explicit_empty_selection: record.explicit_empty_selection,
            language: record.language,
            style: record.style,
            assistant_selection: record.assistant_selection,
            reviewed_memories: record.reviewed_memories,
            assistant_draft: record.assistant_draft,
            assistant_material_stale: record.assistant_material_stale,
            memory_candidate_sets: record.memory_candidate_sets,
            reviewed_memory_artifact: record.reviewed_memory_artifact,
            draft_proposals: record.draft_proposals,
            paragraph_suggestions: record.paragraph_suggestions,
            source_refine_suggestions: record.source_refine_suggestions,
            entry: record.entry,
            output_plan: record.output_plan,
            approval: record.approval,
            destination_progress: record.destination_progress,
            invalidation_reason: record.invalidation_reason,
            job_status: record.job_status,
            all_verified_at: record.all_verified_at,
            discarded_at: record.discarded_at,
            amendment: record.amendment,
        };
        workflow.validate()?;
        Ok(workflow)
    }

    #[must_use]
    pub fn persisted(&self) -> PersistedWorkflow {
        PersistedWorkflow {
            day: self.day.clone(),
            state: self.state,
            revision: self.revision,
            created_at: self.created_at,
            updated_at: self.updated_at,
            candidates: self.candidates.clone(),
            snapshots: self.snapshots.clone(),
            selection_digest: self.selection_digest.clone(),
            explicit_empty_selection: self.explicit_empty_selection,
            language: self.language,
            style: self.style,
            assistant_selection: self.assistant_selection.clone(),
            reviewed_memories: self.reviewed_memories.clone(),
            assistant_draft: self.assistant_draft.clone(),
            assistant_material_stale: self.assistant_material_stale,
            memory_candidate_sets: self.memory_candidate_sets.clone(),
            reviewed_memory_artifact: self.reviewed_memory_artifact.clone(),
            draft_proposals: self.draft_proposals.clone(),
            paragraph_suggestions: self.paragraph_suggestions.clone(),
            source_refine_suggestions: self.source_refine_suggestions.clone(),
            entry: self.entry.clone(),
            output_plan: self.output_plan.clone(),
            approval: self.approval.clone(),
            destination_progress: self.destination_progress.clone(),
            invalidation_reason: self.invalidation_reason.clone(),
            job_status: self.job_status,
            all_verified_at: self.all_verified_at,
            discarded_at: self.discarded_at,
            amendment: self.amendment.clone(),
        }
    }

    #[must_use]
    pub fn day(&self) -> &DiaryDay {
        &self.day
    }

    #[must_use]
    pub const fn state(&self) -> WorkflowState {
        self.state
    }

    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    #[must_use]
    pub const fn updated_at(&self) -> Timestamp {
        self.updated_at
    }

    #[must_use]
    pub fn amendment(&self) -> Option<&AmendmentSeed> {
        self.amendment.as_ref()
    }

    pub fn entry(&self) -> &EntryBuffer {
        &self.entry
    }

    #[must_use]
    pub fn snapshots(&self) -> &[SourceSnapshot] {
        &self.snapshots
    }

    #[must_use]
    pub fn selection_digest(&self) -> Option<&Digest> {
        self.selection_digest.as_ref()
    }

    #[must_use]
    pub const fn explicit_empty_selection(&self) -> bool {
        self.explicit_empty_selection
    }

    #[must_use]
    pub const fn language(&self) -> Option<WritingLanguage> {
        self.language
    }

    #[must_use]
    pub const fn style(&self) -> WritingStyle {
        self.style
    }

    #[must_use]
    pub fn assistant_selection(&self) -> Option<&AssistantSelection> {
        self.assistant_selection.as_ref()
    }

    #[must_use]
    pub fn composition_path(&self) -> Option<CompositionPath> {
        self.assistant_selection
            .as_ref()
            .map(|selection| selection.composition_path)
    }

    #[must_use]
    pub fn output_plan(&self) -> Option<&OutputPlan> {
        self.output_plan.as_ref()
    }

    #[must_use]
    pub fn approval(&self) -> Option<&Approval> {
        self.approval.as_ref()
    }

    #[must_use]
    pub fn destination_progress(&self) -> &[DestinationProgress] {
        &self.destination_progress
    }

    #[must_use]
    pub fn invalidation_reason(&self) -> Option<&InvalidationReason> {
        self.invalidation_reason.as_ref()
    }

    #[must_use]
    pub const fn job_status(&self) -> Option<JobStatus> {
        self.job_status
    }

    #[must_use]
    pub const fn all_verified_at(&self) -> Option<Timestamp> {
        self.all_verified_at
    }

    #[must_use]
    pub const fn discarded_at(&self) -> Option<Timestamp> {
        self.discarded_at
    }

    #[must_use]
    pub fn candidates(&self) -> &[SourceCandidate] {
        &self.candidates
    }
}

mod amendments;
mod assistants;
mod composition;
mod invariants;
mod publication_flow;
mod sources;

fn validate_progress(
    plan: &OutputPlan,
    progress: &[DestinationProgress],
    preflight: bool,
) -> Result<(), WorkflowError> {
    if progress.len() != plan.targets.len() {
        return Err(WorkflowError::UnknownDestination);
    }
    let expected: BTreeSet<_> = plan
        .targets
        .iter()
        .map(|target| target.destination.as_str())
        .collect();
    let actual: BTreeSet<_> = progress
        .iter()
        .map(|item| item.destination.as_str())
        .collect();
    if expected.len() != progress.len() || expected != actual {
        return Err(WorkflowError::UnknownDestination);
    }
    if preflight
        && progress.iter().any(|item| {
            matches!(
                item.status,
                DestinationStatus::Verified | DestinationStatus::Reconciling
            )
        })
    {
        return Err(WorkflowError::PlanBindingMismatch);
    }
    Ok(())
}

fn ensure_every_local_verified(
    plan: &OutputPlan,
    progress: &[DestinationProgress],
) -> Result<(), WorkflowError> {
    if plan
        .targets
        .iter()
        .filter(|target| target.kind == DestinationKind::Local)
        .all(|target| {
            progress.iter().any(|item| {
                item.destination == target.destination && item.status == DestinationStatus::Verified
            })
        })
    {
        Ok(())
    } else {
        Err(WorkflowError::LocalMustBeVerified)
    }
}

fn ensure_unique_candidates(candidates: &[SourceCandidate]) -> Result<(), WorkflowError> {
    let mut keys = BTreeSet::new();
    for candidate in candidates {
        if !source_note_name_is_valid(&candidate.source_note_name) {
            return Err(WorkflowError::InvalidSourceNoteName);
        }
        if !keys.insert(candidate.source.clone()) {
            return Err(WorkflowError::DuplicateSource);
        }
    }
    Ok(())
}

fn selection_digest(snapshots: &[SourceSnapshot]) -> Digest {
    let mut binding = Vec::new();
    for snapshot in snapshots {
        push_framed(&mut binding, snapshot.source.connector.as_bytes());
        push_framed(&mut binding, snapshot.source.source_id.as_bytes());
        push_framed(
            &mut binding,
            snapshot.stamp.content_digest.as_str().as_bytes(),
        );
    }
    Digest::of_bytes(&binding)
}

fn validate_artifact_id(value: &str) -> Result<(), WorkflowError> {
    if value.is_empty()
        || value.len() > 128
        || !value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
        })
    {
        return Err(WorkflowError::AssistantRequestInvalid);
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkflowError {
    StaleRevision { expected: u64, actual: u64 },
    StaleBuffer { expected: u64, actual: u64 },
    WrongState,
    EmptySelectionNeedsConfirmation,
    UnknownSource,
    DuplicateSource,
    InvalidSourceNoteName,
    NeedsDownload,
    PermissionDeniedSource,
    UnreadableSource,
    AssistantActionsDisabled,
    AssistantRequestInvalid,
    AssistantDraftRequestInvalid,
    AssistantDraftResultInvalid,
    AssistantParagraphRangeInvalid,
    AssistantParagraphRequestInvalid,
    AssistantParagraphResultInvalid,
    AssistantBackendRequired,
    ReviewedMemoryRequired,
    AssistantArtifactNotFound,
    AssistantBindingStale,
    AssistantSuggestionAlreadyResolved,
    LanguageRequired,
    BlankEntry,
    UnresolvedInferenceMarker,
    EntryNotConfirmed,
    MissingPlan,
    PlanBindingMismatch,
    UnknownDestination,
    LocalMustBeVerified,
    RetryNeedsPreview,
    MissingApproval,
    VerifiedDestinationRegressed,
    DestinationOutsideExecutionScope,
    InvalidPersistedState,
}

impl fmt::Display for WorkflowError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StaleRevision { .. } => formatter.write_str("stale workflow revision"),
            Self::StaleBuffer { .. } => formatter.write_str("stale entry buffer version"),
            Self::WrongState => formatter.write_str("command is not valid in this workflow state"),
            Self::EmptySelectionNeedsConfirmation => {
                formatter.write_str("empty selection needs explicit confirmation")
            }
            Self::UnknownSource => formatter.write_str("selected source was not discovered"),
            Self::DuplicateSource => formatter.write_str("source identity is duplicated"),
            Self::InvalidSourceNoteName => formatter.write_str("source-note name is unsafe"),
            Self::NeedsDownload => formatter.write_str("source needs download"),
            Self::PermissionDeniedSource => formatter.write_str("source permission was denied"),
            Self::UnreadableSource => formatter.write_str("source is unreadable"),
            Self::AssistantActionsDisabled => formatter.write_str("assistant actions are disabled"),
            Self::AssistantRequestInvalid => formatter.write_str("assistant request is invalid"),
            Self::AssistantDraftRequestInvalid => {
                formatter.write_str("assistant draft request is invalid")
            }
            Self::AssistantDraftResultInvalid => {
                formatter.write_str("assistant draft result is invalid")
            }
            Self::AssistantParagraphRangeInvalid => {
                formatter.write_str("paragraph review range is invalid")
            }
            Self::AssistantParagraphRequestInvalid => {
                formatter.write_str("paragraph review request is invalid")
            }
            Self::AssistantParagraphResultInvalid => {
                formatter.write_str("paragraph review result is invalid")
            }
            Self::AssistantBackendRequired => formatter.write_str("assistant backend is required"),
            Self::ReviewedMemoryRequired => formatter.write_str("reviewed memory is required"),
            Self::AssistantArtifactNotFound => {
                formatter.write_str("assistant artifact was not found")
            }
            Self::AssistantBindingStale => {
                formatter.write_str("assistant artifact binding is stale")
            }
            Self::AssistantSuggestionAlreadyResolved => {
                formatter.write_str("assistant suggestion is already resolved")
            }
            Self::LanguageRequired => formatter.write_str("writing language is required"),
            Self::BlankEntry => formatter.write_str("blank entry cannot be confirmed"),
            Self::UnresolvedInferenceMarker => {
                formatter.write_str("entry contains an unresolved inferred-reflection marker")
            }
            Self::EntryNotConfirmed => formatter.write_str("entry is not confirmed"),
            Self::MissingPlan => formatter.write_str("output plan is missing"),
            Self::PlanBindingMismatch => formatter.write_str("output plan binding changed"),
            Self::UnknownDestination => formatter.write_str("destination is not in the plan"),
            Self::LocalMustBeVerified => {
                formatter.write_str("local destination must verify before a remote write")
            }
            Self::RetryNeedsPreview => {
                formatter.write_str("retry needs a new preview and approval")
            }
            Self::MissingApproval => formatter.write_str("publication approval is missing"),
            Self::VerifiedDestinationRegressed => {
                formatter.write_str("a verified destination cannot regress")
            }
            Self::DestinationOutsideExecutionScope => {
                formatter.write_str("destination changed outside this publication execution")
            }
            Self::InvalidPersistedState => formatter.write_str("persisted workflow is invalid"),
        }
    }
}

impl std::error::Error for WorkflowError {}

#[cfg(test)]
mod tests;
