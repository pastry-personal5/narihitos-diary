//! The server-owned diary workflow state machine.

use std::collections::BTreeSet;
use std::fmt;

use crate::digest::push_framed;
use crate::{
    Approval, CompositionPath, DestinationKind, DestinationProgress, DestinationStatus, DiaryDay,
    Digest, DraftAssistantRequest, EntryBuffer, EntryConfirmation, FreshnessCheckpoint,
    FreshnessOutcome, InvalidationReason, JobStatus, OutputPlan, Preferences, SelectedSource,
    SourceCandidate, SourceSnapshot, Timestamp, WorkflowState, WritingLanguage, WritingStyle,
};

/// Reserved markers used around model-inferred reflection text.
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
    (WorkflowState::Completed, WorkflowState::Composing),
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
    composition_path: CompositionPath,
    reviewed_memories: Vec<String>,
    model_draft: Option<String>,
    model_material_stale: bool,
    entry: EntryBuffer,
    output_plan: Option<OutputPlan>,
    approval: Option<Approval>,
    destination_progress: Vec<DestinationProgress>,
    invalidation_reason: Option<InvalidationReason>,
    job_status: Option<JobStatus>,
    all_verified_at: Option<Timestamp>,
    discarded_at: Option<Timestamp>,
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
    pub composition_path: CompositionPath,
    pub reviewed_memories: Vec<String>,
    pub model_draft: Option<String>,
    pub model_material_stale: bool,
    pub entry: EntryBuffer,
    pub output_plan: Option<OutputPlan>,
    pub approval: Option<Approval>,
    pub destination_progress: Vec<DestinationProgress>,
    pub invalidation_reason: Option<InvalidationReason>,
    pub job_status: Option<JobStatus>,
    pub all_verified_at: Option<Timestamp>,
    pub discarded_at: Option<Timestamp>,
}

impl Workflow {
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
            composition_path: preferences.last_composition_path,
            reviewed_memories: Vec::new(),
            model_draft: None,
            model_material_stale: false,
            entry: EntryBuffer::default(),
            output_plan: None,
            approval: None,
            destination_progress: Vec::new(),
            invalidation_reason: None,
            job_status: None,
            all_verified_at: None,
            discarded_at: None,
        }
    }

    pub fn restore(record: PersistedWorkflow) -> Result<Self, WorkflowError> {
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
            composition_path: record.composition_path,
            reviewed_memories: record.reviewed_memories,
            model_draft: record.model_draft,
            model_material_stale: record.model_material_stale,
            entry: record.entry,
            output_plan: record.output_plan,
            approval: record.approval,
            destination_progress: record.destination_progress,
            invalidation_reason: record.invalidation_reason,
            job_status: record.job_status,
            all_verified_at: record.all_verified_at,
            discarded_at: record.discarded_at,
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
            composition_path: self.composition_path,
            reviewed_memories: self.reviewed_memories.clone(),
            model_draft: self.model_draft.clone(),
            model_material_stale: self.model_material_stale,
            entry: self.entry.clone(),
            output_plan: self.output_plan.clone(),
            approval: self.approval.clone(),
            destination_progress: self.destination_progress.clone(),
            invalidation_reason: self.invalidation_reason.clone(),
            job_status: self.job_status,
            all_verified_at: self.all_verified_at,
            discarded_at: self.discarded_at,
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
    pub const fn composition_path(&self) -> CompositionPath {
        self.composition_path
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

    pub fn discover(
        &mut self,
        expected_revision: u64,
        candidates: Vec<SourceCandidate>,
        now: Timestamp,
    ) -> Result<(), WorkflowError> {
        self.expect_revision(expected_revision)?;
        if !matches!(
            self.state,
            WorkflowState::Started
                | WorkflowState::Discovered
                | WorkflowState::SourcesConfirmed
                | WorkflowState::Composing
                | WorkflowState::EntryConfirmed
                | WorkflowState::OutputPlanned
                | WorkflowState::Approved
        ) {
            return Err(WorkflowError::WrongState);
        }
        let replaces_confirmation = !matches!(
            self.state,
            WorkflowState::Started | WorkflowState::Discovered
        );
        ensure_unique_candidates(&candidates)?;
        self.candidates = candidates;
        self.snapshots.clear();
        self.selection_digest = None;
        self.explicit_empty_selection = false;
        self.clear_after_selection();
        if replaces_confirmation {
            self.invalidation_reason = Some(InvalidationReason::SelectionChanged);
        }
        self.advance(WorkflowState::Discovered, now);
        Ok(())
    }

    pub fn confirm_sources(
        &mut self,
        expected_revision: u64,
        snapshots: Vec<SourceSnapshot>,
        explicitly_continue_empty: bool,
        now: Timestamp,
    ) -> Result<(), WorkflowError> {
        self.expect_revision(expected_revision)?;
        if !matches!(
            self.state,
            WorkflowState::Discovered
                | WorkflowState::SourcesConfirmed
                | WorkflowState::Composing
                | WorkflowState::EntryConfirmed
                | WorkflowState::OutputPlanned
                | WorkflowState::Approved
        ) {
            return Err(WorkflowError::WrongState);
        }
        let replaces_confirmation = self.state != WorkflowState::Discovered;
        if snapshots.is_empty() && !explicitly_continue_empty {
            return Err(WorkflowError::EmptySelectionNeedsConfirmation);
        }
        let candidate_keys: BTreeSet<_> = self
            .candidates
            .iter()
            .map(|candidate| candidate.source.clone())
            .collect();
        let mut selected_keys = BTreeSet::new();
        for snapshot in &snapshots {
            let key = snapshot.source.clone();
            if !candidate_keys.contains(&key) {
                return Err(WorkflowError::UnknownSource);
            }
            if !selected_keys.insert(key) {
                return Err(WorkflowError::DuplicateSource);
            }
        }
        self.snapshots = snapshots;
        self.snapshots
            .sort_by_key(|snapshot| snapshot.source.clone());
        self.selection_digest = Some(selection_digest(&self.snapshots));
        self.explicit_empty_selection = self.snapshots.is_empty();
        self.clear_after_selection();
        if replaces_confirmation {
            self.invalidation_reason = Some(InvalidationReason::SelectionChanged);
        }
        self.advance(WorkflowState::SourcesConfirmed, now);
        Ok(())
    }

    /// Confirms server-prepared sources and incorporates date evidence observed
    /// by the trusted reader in the same workflow revision.
    pub fn confirm_selected_sources(
        &mut self,
        expected_revision: u64,
        selected: Vec<SelectedSource>,
        explicitly_continue_empty: bool,
        now: Timestamp,
    ) -> Result<(), WorkflowError> {
        self.expect_revision(expected_revision)?;
        let mut seen = BTreeSet::new();
        for source in &selected {
            if !seen.insert(source.snapshot.source.clone()) {
                return Err(WorkflowError::DuplicateSource);
            }
            let candidate = self
                .candidates
                .iter_mut()
                .find(|candidate| candidate.source == source.snapshot.source)
                .ok_or(WorkflowError::UnknownSource)?;
            candidate.date_evidence = source.date_evidence.clone();
        }
        self.confirm_sources(
            expected_revision,
            selected.into_iter().map(|source| source.snapshot).collect(),
            explicitly_continue_empty,
            now,
        )
    }

    pub fn apply_freshness(
        &mut self,
        expected_revision: u64,
        _checkpoint: FreshnessCheckpoint,
        outcome: FreshnessOutcome,
        now: Timestamp,
    ) -> Result<(), WorkflowError> {
        self.expect_revision(expected_revision)?;
        if !matches!(
            self.state,
            WorkflowState::SourcesConfirmed
                | WorkflowState::Composing
                | WorkflowState::EntryConfirmed
                | WorkflowState::OutputPlanned
                | WorkflowState::Approved
        ) {
            return Err(WorkflowError::WrongState);
        }
        match outcome {
            FreshnessOutcome::Fresh => Ok(()),
            FreshnessOutcome::Changed { source } => {
                self.invalidate_sources(InvalidationReason::SourceChanged(source), now);
                Ok(())
            }
            FreshnessOutcome::Unverifiable { source } => {
                self.invalidate_sources(InvalidationReason::FreshnessUnverifiable(source), now);
                Ok(())
            }
            FreshnessOutcome::NeedsDownload { .. } => Err(WorkflowError::NeedsDownload),
            FreshnessOutcome::PermissionDenied { .. } => Err(WorkflowError::PermissionDeniedSource),
            FreshnessOutcome::Unreadable { .. } => Err(WorkflowError::UnreadableSource),
        }
    }

    pub fn begin_manual_composition(
        &mut self,
        expected_revision: u64,
        now: Timestamp,
    ) -> Result<(), WorkflowError> {
        self.expect_revision(expected_revision)?;
        if !matches!(
            self.state,
            WorkflowState::SourcesConfirmed | WorkflowState::Composing
        ) {
            return Err(WorkflowError::WrongState);
        }
        self.advance(WorkflowState::Composing, now);
        Ok(())
    }

    #[must_use]
    pub fn model_steps_available(&self) -> bool {
        !self.snapshots.is_empty()
            && matches!(
                self.state,
                WorkflowState::SourcesConfirmed | WorkflowState::Composing
            )
    }

    pub fn assistant_request(&self) -> Result<DraftAssistantRequest, WorkflowError> {
        if !self.model_steps_available() {
            return Err(WorkflowError::ModelStepsDisabled);
        }
        let language = self.language.ok_or(WorkflowError::LanguageRequired)?;
        Ok(DraftAssistantRequest {
            readable_note_bodies: self
                .snapshots
                .iter()
                .map(|snapshot| snapshot.body.clone())
                .collect(),
            reviewed_memories: self.reviewed_memories.clone(),
            prior_draft: self.model_draft.clone(),
            edited_entry: (!self.entry.text.is_empty()).then(|| self.entry.text.clone()),
            language,
            style: self.style,
        })
    }

    pub fn set_language(
        &mut self,
        expected_revision: u64,
        language: WritingLanguage,
        now: Timestamp,
    ) -> Result<(), WorkflowError> {
        self.expect_revision(expected_revision)?;
        self.ensure_entry_editable()?;
        if self.language == Some(language) {
            return Ok(());
        }
        self.language = Some(language);
        self.invalidate_composition(now);
        Ok(())
    }

    pub fn set_style(
        &mut self,
        expected_revision: u64,
        style: WritingStyle,
        now: Timestamp,
    ) -> Result<(), WorkflowError> {
        self.expect_revision(expected_revision)?;
        self.ensure_entry_editable()?;
        if self.style == style {
            return Ok(());
        }
        self.style = style;
        self.invalidate_composition(now);
        Ok(())
    }

    pub fn set_composition_path(
        &mut self,
        expected_revision: u64,
        path: CompositionPath,
        now: Timestamp,
    ) -> Result<(), WorkflowError> {
        self.expect_revision(expected_revision)?;
        self.ensure_entry_editable()?;
        if self.composition_path == path {
            return Ok(());
        }
        self.composition_path = path;
        self.invalidate_composition(now);
        Ok(())
    }

    pub fn set_writing_settings(
        &mut self,
        expected_revision: u64,
        language: WritingLanguage,
        style: WritingStyle,
        path: CompositionPath,
        now: Timestamp,
    ) -> Result<(), WorkflowError> {
        self.expect_revision(expected_revision)?;
        self.ensure_entry_editable()?;
        if self.language == Some(language) && self.style == style && self.composition_path == path {
            return Ok(());
        }
        self.language = Some(language);
        self.style = style;
        self.composition_path = path;
        self.invalidate_composition(now);
        Ok(())
    }

    pub fn save_entry(
        &mut self,
        expected_buffer_version: u64,
        text: String,
        now: Timestamp,
    ) -> Result<(), WorkflowError> {
        self.ensure_entry_editable()?;
        if self.entry.version != expected_buffer_version {
            return Err(WorkflowError::StaleBuffer {
                expected: expected_buffer_version,
                actual: self.entry.version,
            });
        }
        if self.entry.text == text {
            return Ok(());
        }
        let invalidates_confirmation = self.entry.confirmed.is_some();
        self.entry.text = text;
        self.entry.version += 1;
        self.updated_at = now;
        if invalidates_confirmation {
            self.entry.confirmed = None;
            self.clear_plan_and_approval();
            self.state = WorkflowState::Composing;
            self.invalidation_reason = Some(InvalidationReason::EntryEdited);
            self.revision += 1;
        }
        Ok(())
    }

    pub fn confirm_entry(
        &mut self,
        expected_revision: u64,
        expected_buffer_version: u64,
        now: Timestamp,
    ) -> Result<(), WorkflowError> {
        self.expect_revision(expected_revision)?;
        if self.entry.version != expected_buffer_version {
            return Err(WorkflowError::StaleBuffer {
                expected: expected_buffer_version,
                actual: self.entry.version,
            });
        }
        if !matches!(
            self.state,
            WorkflowState::SourcesConfirmed | WorkflowState::Composing
        ) {
            return Err(WorkflowError::WrongState);
        }
        if self.language.is_none() {
            return Err(WorkflowError::LanguageRequired);
        }
        if self.entry.text.trim().is_empty() {
            return Err(WorkflowError::BlankEntry);
        }
        if self.entry.text.contains(INFERRED_REFLECTION_OPEN)
            || self.entry.text.contains(INFERRED_REFLECTION_CLOSE)
        {
            return Err(WorkflowError::UnresolvedInferenceMarker);
        }
        self.entry.confirmed = Some(EntryConfirmation {
            buffer_version: self.entry.version,
            entry_digest: Digest::of_bytes(self.entry.text.as_bytes()),
            confirmed_at: now,
        });
        self.clear_plan_and_approval();
        self.advance(WorkflowState::EntryConfirmed, now);
        Ok(())
    }

    pub fn preview_output(
        &mut self,
        expected_revision: u64,
        plan: OutputPlan,
        now: Timestamp,
    ) -> Result<(), WorkflowError> {
        let progress = plan
            .targets
            .iter()
            .map(|target| DestinationProgress {
                destination: target.destination.clone(),
                status: DestinationStatus::Pending,
            })
            .collect();
        self.preview_output_with_progress(expected_revision, plan, progress, now)
    }

    /// Records the exact preflight result. A blocked remote remains visible in
    /// the approved plan while local output can proceed.
    pub fn preview_output_with_progress(
        &mut self,
        expected_revision: u64,
        plan: OutputPlan,
        progress: Vec<DestinationProgress>,
        now: Timestamp,
    ) -> Result<(), WorkflowError> {
        self.expect_revision(expected_revision)?;
        if !matches!(
            self.state,
            WorkflowState::EntryConfirmed | WorkflowState::OutputPlanned | WorkflowState::Approved
        ) {
            return Err(WorkflowError::WrongState);
        }
        let confirmation = self
            .entry
            .confirmed
            .as_ref()
            .ok_or(WorkflowError::EntryNotConfirmed)?;
        if plan.entry_digest != confirmation.entry_digest
            || Some(&plan.selection_digest) != self.selection_digest.as_ref()
        {
            return Err(WorkflowError::PlanBindingMismatch);
        }
        plan.validate()
            .map_err(|_| WorkflowError::PlanBindingMismatch)?;
        validate_progress(&plan, &progress, true)?;
        if self
            .output_plan
            .as_ref()
            .is_some_and(|current| current.plan_digest != plan.plan_digest)
        {
            self.invalidation_reason = Some(InvalidationReason::OutputBindingChanged);
        }
        self.destination_progress = progress;
        self.output_plan = Some(plan);
        self.approval = None;
        self.advance(WorkflowState::OutputPlanned, now);
        Ok(())
    }

    pub fn approve(
        &mut self,
        expected_revision: u64,
        plan_digest: &Digest,
        now: Timestamp,
    ) -> Result<(), WorkflowError> {
        self.expect_revision(expected_revision)?;
        if self.state != WorkflowState::OutputPlanned {
            return Err(WorkflowError::WrongState);
        }
        let plan = self
            .output_plan
            .as_ref()
            .ok_or(WorkflowError::MissingPlan)?;
        if &plan.plan_digest != plan_digest {
            return Err(WorkflowError::PlanBindingMismatch);
        }
        if self
            .destination_progress
            .iter()
            .any(|progress| progress.status == DestinationStatus::Conflict)
            || plan
                .targets
                .iter()
                .filter(|target| target.kind == DestinationKind::Local)
                .any(|target| {
                    self.destination_progress.iter().any(|progress| {
                        progress.destination == target.destination
                            && progress.status != DestinationStatus::Pending
                    })
                })
        {
            return Err(WorkflowError::RetryNeedsPreview);
        }
        if self
            .destination_progress
            .iter()
            .any(|progress| progress.status == DestinationStatus::Blocked)
            && ensure_every_local_verified(plan, &self.destination_progress).is_ok()
        {
            return Err(WorkflowError::RetryNeedsPreview);
        }
        self.approval = Some(Approval {
            plan_digest: plan_digest.clone(),
            workflow_revision: self.revision + 1,
            approved_at: now,
            destination_only: None,
        });
        self.advance(WorkflowState::Approved, now);
        Ok(())
    }

    pub fn approve_blocked_retry(
        &mut self,
        expected_revision: u64,
        destination: &str,
        rechecked_plan_digest: &Digest,
        now: Timestamp,
    ) -> Result<(), WorkflowError> {
        self.expect_revision(expected_revision)?;
        if self.state != WorkflowState::OutputPlanned {
            return Err(WorkflowError::WrongState);
        }
        let progress = self
            .destination_progress
            .iter()
            .find(|progress| progress.destination == destination)
            .ok_or(WorkflowError::UnknownDestination)?;
        if progress.status != DestinationStatus::Blocked {
            return Err(WorkflowError::RetryNeedsPreview);
        }
        let plan = self
            .output_plan
            .as_ref()
            .ok_or(WorkflowError::MissingPlan)?;
        let every_local_verified = plan
            .targets
            .iter()
            .filter(|target| target.kind == DestinationKind::Local)
            .all(|target| {
                self.destination_progress.iter().any(|progress| {
                    progress.destination == target.destination
                        && progress.status == DestinationStatus::Verified
                })
            });
        if !every_local_verified {
            return Err(WorkflowError::LocalMustBeVerified);
        }
        if &plan.plan_digest != rechecked_plan_digest {
            return Err(WorkflowError::PlanBindingMismatch);
        }
        self.approval = Some(Approval {
            plan_digest: rechecked_plan_digest.clone(),
            workflow_revision: self.revision + 1,
            approved_at: now,
            destination_only: Some(destination.to_owned()),
        });
        self.advance(WorkflowState::Approved, now);
        Ok(())
    }

    /// Replaces one blocked remote target with a newly inspected preview while
    /// preserving verified local progress. Approval is deliberately cleared.
    pub fn preview_blocked_retry(
        &mut self,
        expected_revision: u64,
        destination: &str,
        replacement: crate::DestinationPlan,
        new_plan_id: impl Into<String>,
        now: Timestamp,
    ) -> Result<OutputPlan, WorkflowError> {
        self.expect_revision(expected_revision)?;
        if self.state != WorkflowState::Partial {
            return Err(WorkflowError::WrongState);
        }
        if replacement.kind != DestinationKind::Remote || replacement.destination != destination {
            return Err(WorkflowError::UnknownDestination);
        }
        let progress = self
            .destination_progress
            .iter()
            .find(|item| item.destination == destination)
            .ok_or(WorkflowError::UnknownDestination)?;
        if progress.status != DestinationStatus::Blocked {
            return Err(WorkflowError::RetryNeedsPreview);
        }
        let old_plan = self
            .output_plan
            .as_ref()
            .ok_or(WorkflowError::MissingPlan)?;
        ensure_every_local_verified(old_plan, &self.destination_progress)?;
        let mut targets = old_plan.targets.clone();
        let target = targets
            .iter_mut()
            .find(|target| target.destination == destination)
            .ok_or(WorkflowError::UnknownDestination)?;
        *target = replacement;
        let plan = OutputPlan::new(
            new_plan_id,
            old_plan.entry_digest.clone(),
            old_plan.selection_digest.clone(),
            targets,
            now,
        )
        .map_err(|_| WorkflowError::PlanBindingMismatch)?;
        self.output_plan = Some(plan.clone());
        self.approval = None;
        self.advance(WorkflowState::OutputPlanned, now);
        Ok(plan)
    }

    pub fn pending_retry_reuses_approval(
        &self,
        destination: &str,
        rechecked_plan_digest: &Digest,
    ) -> bool {
        if self.state != WorkflowState::Partial {
            return false;
        }
        let locals_verified = self.output_plan.as_ref().is_some_and(|plan| {
            ensure_every_local_verified(plan, &self.destination_progress).is_ok()
        });
        let is_pending = self.destination_progress.iter().any(|progress| {
            progress.destination == destination && progress.status == DestinationStatus::Pending
        });
        locals_verified
            && is_pending
            && self
                .approval
                .as_ref()
                .is_some_and(|approval| &approval.plan_digest == rechecked_plan_digest)
    }

    pub fn begin_pending_retry(
        &mut self,
        expected_revision: u64,
        destination: &str,
        rechecked_plan_digest: &Digest,
        now: Timestamp,
    ) -> Result<(), WorkflowError> {
        self.expect_revision(expected_revision)?;
        if !self.pending_retry_reuses_approval(destination, rechecked_plan_digest) {
            return Err(WorkflowError::RetryNeedsPreview);
        }
        self.approval
            .as_mut()
            .ok_or(WorkflowError::MissingApproval)?
            .destination_only = Some(destination.to_owned());
        self.job_status = Some(JobStatus::Running);
        self.advance(WorkflowState::Publishing, now);
        Ok(())
    }

    pub fn begin_publication(
        &mut self,
        expected_revision: u64,
        now: Timestamp,
    ) -> Result<(), WorkflowError> {
        self.expect_revision(expected_revision)?;
        if self.state != WorkflowState::Approved || self.approval.is_none() {
            return Err(WorkflowError::WrongState);
        }
        self.job_status = Some(JobStatus::Running);
        self.advance(WorkflowState::Publishing, now);
        Ok(())
    }

    pub fn request_cancellation(
        &mut self,
        expected_revision: u64,
        now: Timestamp,
    ) -> Result<(), WorkflowError> {
        self.expect_revision(expected_revision)?;
        if self.state != WorkflowState::Publishing {
            return Err(WorkflowError::WrongState);
        }
        self.job_status = Some(JobStatus::CancellationRequested);
        self.bump(now);
        Ok(())
    }

    pub fn finish_publication(
        &mut self,
        expected_revision: u64,
        progress: Vec<DestinationProgress>,
        now: Timestamp,
    ) -> Result<(), WorkflowError> {
        self.expect_revision(expected_revision)?;
        if self.state != WorkflowState::Publishing {
            return Err(WorkflowError::WrongState);
        }
        if progress.len() != self.destination_progress.len() {
            return Err(WorkflowError::UnknownDestination);
        }
        let expected: BTreeSet<_> = self
            .destination_progress
            .iter()
            .map(|item| item.destination.as_str())
            .collect();
        let actual: BTreeSet<_> = progress
            .iter()
            .map(|item| item.destination.as_str())
            .collect();
        if expected != actual {
            return Err(WorkflowError::UnknownDestination);
        }
        let plan = self
            .output_plan
            .as_ref()
            .ok_or(WorkflowError::MissingPlan)?;
        validate_progress(plan, &progress, false)?;
        let execution_scope = self
            .approval
            .as_ref()
            .ok_or(WorkflowError::MissingApproval)?
            .destination_only
            .as_deref();
        for previous in &self.destination_progress {
            let current = progress
                .iter()
                .find(|item| item.destination == previous.destination)
                .ok_or(WorkflowError::UnknownDestination)?;
            if previous.status == DestinationStatus::Verified
                && current.status != DestinationStatus::Verified
            {
                return Err(WorkflowError::VerifiedDestinationRegressed);
            }
            if execution_scope.is_some_and(|scope| scope != previous.destination)
                && current.status != previous.status
            {
                return Err(WorkflowError::DestinationOutsideExecutionScope);
            }
            if execution_scope.is_none()
                && previous.status == DestinationStatus::Blocked
                && current.status != DestinationStatus::Blocked
            {
                return Err(WorkflowError::DestinationOutsideExecutionScope);
            }
        }
        let local_destinations: BTreeSet<_> = plan
            .targets
            .iter()
            .filter(|target| target.kind == DestinationKind::Local)
            .map(|target| target.destination.as_str())
            .collect();
        let remote_verified = progress.iter().any(|item| {
            item.status == DestinationStatus::Verified
                && plan.targets.iter().any(|target| {
                    target.destination == item.destination && target.kind == DestinationKind::Remote
                })
        });
        let every_local_verified = local_destinations.iter().all(|destination| {
            progress.iter().any(|item| {
                item.destination == *destination && item.status == DestinationStatus::Verified
            })
        });
        if remote_verified && !every_local_verified {
            return Err(WorkflowError::LocalMustBeVerified);
        }
        let all_verified = progress
            .iter()
            .all(|item| item.status == DestinationStatus::Verified);
        self.destination_progress = progress;
        self.job_status = Some(JobStatus::Completed);
        self.all_verified_at = all_verified.then_some(now);
        self.advance(
            if all_verified {
                WorkflowState::Completed
            } else {
                WorkflowState::Partial
            },
            now,
        );
        Ok(())
    }

    pub fn seed_amendment(
        &mut self,
        expected_revision: u64,
        current_local_text: String,
        now: Timestamp,
    ) -> Result<(), WorkflowError> {
        self.expect_revision(expected_revision)?;
        if self.state != WorkflowState::Completed {
            return Err(WorkflowError::WrongState);
        }
        self.entry.text = current_local_text;
        self.entry.version += 1;
        self.entry.confirmed = None;
        self.clear_plan_and_approval();
        self.all_verified_at = None;
        self.advance(WorkflowState::Composing, now);
        Ok(())
    }

    /// Navigation is deliberately side-effect free.
    pub fn navigate_back(&self) {}

    pub fn discard_recoverable_text(&mut self, now: Timestamp) {
        let completed = self.state == WorkflowState::Completed;
        self.revision += 1;
        self.entry.version += 1;
        self.entry.text.clear();
        for snapshot in &mut self.snapshots {
            snapshot.body.clear();
        }
        self.reviewed_memories.clear();
        self.model_draft = None;
        self.entry.confirmed = None;
        self.clear_plan_and_approval();
        if !completed {
            self.state = WorkflowState::Started;
            self.candidates.clear();
            self.snapshots.clear();
            self.selection_digest = None;
            self.explicit_empty_selection = false;
            self.all_verified_at = None;
        }
        self.discarded_at = Some(now);
        self.updated_at = now;
    }

    fn expect_revision(&self, expected: u64) -> Result<(), WorkflowError> {
        if self.revision != expected {
            return Err(WorkflowError::StaleRevision {
                expected,
                actual: self.revision,
            });
        }
        Ok(())
    }

    fn advance(&mut self, state: WorkflowState, now: Timestamp) {
        self.state = state;
        self.bump(now);
    }

    fn bump(&mut self, now: Timestamp) {
        self.revision += 1;
        self.updated_at = now;
        self.discarded_at = None;
    }

    fn ensure_entry_editable(&self) -> Result<(), WorkflowError> {
        if matches!(
            self.state,
            WorkflowState::SourcesConfirmed
                | WorkflowState::Composing
                | WorkflowState::EntryConfirmed
                | WorkflowState::OutputPlanned
                | WorkflowState::Approved
        ) {
            Ok(())
        } else {
            Err(WorkflowError::WrongState)
        }
    }

    fn clear_after_selection(&mut self) {
        self.reviewed_memories.clear();
        self.model_draft = None;
        self.model_material_stale = false;
        self.entry.confirmed = None;
        self.clear_plan_and_approval();
    }

    fn clear_plan_and_approval(&mut self) {
        self.output_plan = None;
        self.approval = None;
        self.destination_progress.clear();
        self.job_status = None;
    }

    fn invalidate_sources(&mut self, reason: InvalidationReason, now: Timestamp) {
        self.snapshots.clear();
        self.selection_digest = None;
        self.explicit_empty_selection = false;
        self.entry.confirmed = None;
        self.clear_after_selection();
        self.invalidation_reason = Some(reason);
        self.advance(WorkflowState::Discovered, now);
    }

    fn invalidate_composition(&mut self, now: Timestamp) {
        self.model_material_stale = true;
        self.entry.confirmed = None;
        self.clear_plan_and_approval();
        self.invalidation_reason = Some(InvalidationReason::CompositionChoiceChanged);
        if matches!(
            self.state,
            WorkflowState::Composing
                | WorkflowState::EntryConfirmed
                | WorkflowState::OutputPlanned
                | WorkflowState::Approved
        ) {
            self.state = WorkflowState::Composing;
        }
        self.bump(now);
    }

    fn validate(&self) -> Result<(), WorkflowError> {
        ensure_unique_candidates(&self.candidates)?;
        let candidate_sources: BTreeSet<_> = self
            .candidates
            .iter()
            .map(|candidate| candidate.source.clone())
            .collect();
        let mut snapshot_sources = BTreeSet::new();
        if self.snapshots.iter().any(|snapshot| {
            !candidate_sources.contains(&snapshot.source)
                || !snapshot_sources.insert(snapshot.source.clone())
        }) {
            return Err(WorkflowError::InvalidPersistedState);
        }
        match &self.selection_digest {
            Some(digest) => {
                if digest != &selection_digest(&self.snapshots)
                    || self.explicit_empty_selection != self.snapshots.is_empty()
                {
                    return Err(WorkflowError::InvalidPersistedState);
                }
            }
            None => {
                if !self.snapshots.is_empty() || self.explicit_empty_selection {
                    return Err(WorkflowError::InvalidPersistedState);
                }
            }
        }
        if self.entry.confirmed.as_ref().is_some_and(|confirmation| {
            confirmation.buffer_version != self.entry.version
                || confirmation.entry_digest != Digest::of_bytes(self.entry.text.as_bytes())
        }) {
            return Err(WorkflowError::InvalidPersistedState);
        }
        if self.output_plan.is_some() && self.entry.confirmed.is_none() {
            return Err(WorkflowError::InvalidPersistedState);
        }
        if self.approval.is_some() && self.output_plan.is_none() {
            return Err(WorkflowError::InvalidPersistedState);
        }
        if let Some(plan) = &self.output_plan {
            plan.validate()
                .map_err(|_| WorkflowError::InvalidPersistedState)?;
            let confirmation = self
                .entry
                .confirmed
                .as_ref()
                .ok_or(WorkflowError::InvalidPersistedState)?;
            if plan.entry_digest != confirmation.entry_digest
                || Some(&plan.selection_digest) != self.selection_digest.as_ref()
            {
                return Err(WorkflowError::InvalidPersistedState);
            }
            validate_progress(plan, &self.destination_progress, false)
                .map_err(|_| WorkflowError::InvalidPersistedState)?;
            if let Some(approval) = &self.approval
                && (approval.plan_digest != plan.plan_digest
                    || approval.workflow_revision > self.revision
                    || approval
                        .destination_only
                        .as_ref()
                        .is_some_and(|destination| {
                            !plan.targets.iter().any(|target| {
                                &target.destination == destination
                                    && target.kind == DestinationKind::Remote
                            })
                        }))
            {
                return Err(WorkflowError::InvalidPersistedState);
            }
        } else if !self.destination_progress.is_empty() {
            return Err(WorkflowError::InvalidPersistedState);
        }

        let stripped_completed = self.state == WorkflowState::Completed
            && self.discarded_at.is_some()
            && self.output_plan.is_none()
            && self.entry.confirmed.is_none();
        let valid_state = match self.state {
            WorkflowState::Started => {
                self.candidates.is_empty()
                    && self.selection_digest.is_none()
                    && self.entry.confirmed.is_none()
                    && self.output_plan.is_none()
            }
            WorkflowState::Discovered => {
                self.selection_digest.is_none()
                    && self.entry.confirmed.is_none()
                    && self.output_plan.is_none()
            }
            WorkflowState::SourcesConfirmed | WorkflowState::Composing => {
                self.selection_digest.is_some()
                    && self.entry.confirmed.is_none()
                    && self.output_plan.is_none()
            }
            WorkflowState::EntryConfirmed => {
                self.entry.confirmed.is_some() && self.output_plan.is_none()
            }
            WorkflowState::OutputPlanned => {
                self.entry.confirmed.is_some()
                    && self.output_plan.is_some()
                    && self.approval.is_none()
            }
            WorkflowState::Approved => {
                self.entry.confirmed.is_some()
                    && self.output_plan.is_some()
                    && self
                        .approval
                        .as_ref()
                        .is_some_and(|approval| approval.workflow_revision == self.revision)
            }
            WorkflowState::Publishing => {
                self.entry.confirmed.is_some()
                    && self.output_plan.is_some()
                    && self.approval.is_some()
                    && matches!(
                        self.job_status,
                        Some(JobStatus::Running | JobStatus::CancellationRequested)
                    )
            }
            WorkflowState::Completed => {
                stripped_completed
                    || (self.entry.confirmed.is_some()
                        && self.output_plan.is_some()
                        && self.all_verified_at.is_some()
                        && self
                            .destination_progress
                            .iter()
                            .all(|progress| progress.status == DestinationStatus::Verified))
            }
            WorkflowState::Partial => {
                self.entry.confirmed.is_some()
                    && self.output_plan.is_some()
                    && self.all_verified_at.is_none()
                    && self
                        .destination_progress
                        .iter()
                        .any(|progress| progress.status != DestinationStatus::Verified)
            }
        };
        if !valid_state
            || (self.state != WorkflowState::Completed && self.all_verified_at.is_some())
        {
            return Err(WorkflowError::InvalidPersistedState);
        }
        Ok(())
    }
}

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
    if candidates
        .iter()
        .any(|candidate| !keys.insert(candidate.source.clone()))
    {
        return Err(WorkflowError::DuplicateSource);
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkflowError {
    StaleRevision { expected: u64, actual: u64 },
    StaleBuffer { expected: u64, actual: u64 },
    WrongState,
    EmptySelectionNeedsConfirmation,
    UnknownSource,
    DuplicateSource,
    NeedsDownload,
    PermissionDeniedSource,
    UnreadableSource,
    ModelStepsDisabled,
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
            Self::NeedsDownload => formatter.write_str("source needs download"),
            Self::PermissionDeniedSource => formatter.write_str("source permission was denied"),
            Self::UnreadableSource => formatter.write_str("source is unreadable"),
            Self::ModelStepsDisabled => formatter.write_str("model steps are disabled"),
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
mod tests {
    use super::*;
    use crate::{
        DateEvidence, DestinationPlan, FreshnessStamp, SourceAccessError, SourceAvailability,
        SourceCheckpointError, SourceReadOutcome, SourceReader, SourceRef, WritingLanguage,
        revalidate_workflow_sources,
    };

    #[test]
    fn state_table_contains_every_architecture_state_and_journey_enters_each() {
        let states: BTreeSet<_> = LEGAL_TRANSITIONS
            .iter()
            .flat_map(|(from, to)| [*from, *to])
            .collect();
        assert_eq!(states.len(), 10);

        let mut workflow = discovered_workflow();
        let snapshot = snapshot("selected", "body");
        workflow
            .confirm_sources(workflow.revision(), vec![snapshot], false, time(2))
            .unwrap();
        assert_eq!(workflow.state(), WorkflowState::SourcesConfirmed);
        workflow
            .begin_manual_composition(workflow.revision(), time(3))
            .unwrap();
        assert_eq!(workflow.state(), WorkflowState::Composing);
        workflow
            .set_language(workflow.revision(), WritingLanguage::EnUs, time(4))
            .unwrap();
        workflow
            .save_entry(0, "Invented day.".to_owned(), time(5))
            .unwrap();
        workflow
            .confirm_entry(workflow.revision(), 1, time(6))
            .unwrap();
        assert_eq!(workflow.state(), WorkflowState::EntryConfirmed);
        let plan = plan_for(&workflow, "plan-1");
        workflow
            .preview_output(workflow.revision(), plan.clone(), time(7))
            .unwrap();
        assert_eq!(workflow.state(), WorkflowState::OutputPlanned);
        workflow
            .approve(workflow.revision(), &plan.plan_digest, time(8))
            .unwrap();
        assert_eq!(workflow.state(), WorkflowState::Approved);
        workflow
            .begin_publication(workflow.revision(), time(9))
            .unwrap();
        assert_eq!(workflow.state(), WorkflowState::Publishing);
        workflow
            .request_cancellation(workflow.revision(), time(9))
            .unwrap();
        assert_eq!(
            workflow.job_status(),
            Some(JobStatus::CancellationRequested)
        );
        workflow
            .finish_publication(
                workflow.revision(),
                vec![DestinationProgress {
                    destination: "local".to_owned(),
                    status: DestinationStatus::Verified,
                }],
                time(10),
            )
            .unwrap();
        assert_eq!(workflow.state(), WorkflowState::Completed);
        workflow
            .seed_amendment(
                workflow.revision(),
                "Edited local text".to_owned(),
                time(11),
            )
            .unwrap();
        assert_eq!(workflow.state(), WorkflowState::Composing);

        let mut partial = journey_to_publishing();
        partial
            .finish_publication(
                partial.revision(),
                vec![DestinationProgress {
                    destination: "local".to_owned(),
                    status: DestinationStatus::Blocked,
                }],
                time(10),
            )
            .unwrap();
        assert_eq!(partial.state(), WorkflowState::Partial);
    }

    #[test]
    fn refused_commands_and_navigation_leave_state_unchanged() {
        let mut workflow = Workflow::new(day(), time(0), &Preferences::default());
        let before = workflow.clone();
        assert_eq!(
            workflow.confirm_sources(0, Vec::new(), true, time(1)),
            Err(WorkflowError::WrongState)
        );
        workflow.navigate_back();
        assert_eq!(workflow, before);
        assert_eq!(
            workflow.discover(9, Vec::new(), time(1)),
            Err(WorkflowError::StaleRevision {
                expected: 9,
                actual: 0
            })
        );
        assert_eq!(workflow, before);
    }

    #[test]
    fn selection_is_independent_of_date_evidence_and_empty_path_disables_model() {
        let mut workflow = Workflow::new(day(), time(0), &Preferences::default());
        let outside = SourceCandidate {
            source: source("outside"),
            date_evidence: DateEvidence {
                directory_day: Some(DiaryDay::parse("2026-10-20").unwrap()),
                header_day: Some(DiaryDay::parse("2026-10-30").unwrap()),
                warning: Some("dates disagree".to_owned()),
            },
            availability: SourceAvailability::Resident,
        };
        workflow.discover(0, vec![outside], time(1)).unwrap();
        workflow
            .confirm_sources(1, vec![snapshot("outside", "included")], false, time(2))
            .unwrap();
        assert!(workflow.model_steps_available());

        let mut empty = discovered_workflow();
        assert_eq!(empty.language(), None);
        assert_eq!(empty.style(), WritingStyle::Concise);
        assert_eq!(empty.composition_path(), CompositionPath::Hosted);
        assert_eq!(
            empty.confirm_sources(1, Vec::new(), false, time(2)),
            Err(WorkflowError::EmptySelectionNeedsConfirmation)
        );
        empty.confirm_sources(1, Vec::new(), true, time(2)).unwrap();
        assert!(!empty.model_steps_available());
        assert_eq!(
            empty.assistant_request(),
            Err(WorkflowError::ModelStepsDisabled)
        );
        empty.begin_manual_composition(2, time(3)).unwrap();
        empty.save_entry(0, "빈 날".to_owned(), time(5)).unwrap();
        assert_eq!(
            empty.confirm_entry(empty.revision(), 1, time(6)),
            Err(WorkflowError::LanguageRequired)
        );
        empty
            .set_language(empty.revision(), WritingLanguage::KoKr, time(7))
            .unwrap();
        empty.confirm_entry(4, 1, time(8)).unwrap();
    }

    #[test]
    fn reselection_voids_downstream_but_preserves_manual_text() {
        let mut workflow = confirmed_workflow();
        workflow
            .confirm_sources(workflow.revision(), Vec::new(), true, time(20))
            .unwrap();
        assert_eq!(workflow.state(), WorkflowState::SourcesConfirmed);
        assert_eq!(workflow.entry().text, "Invented day.");
        assert!(workflow.entry().confirmed.is_none());
        assert!(workflow.output_plan().is_none());
        assert_eq!(
            workflow.invalidation_reason(),
            Some(&InvalidationReason::SelectionChanged)
        );
        assert!(!workflow.model_steps_available());
    }

    #[test]
    fn structured_identities_and_plan_bindings_reject_ambiguity() {
        let left = SourceRef::new("a:b", "c").unwrap();
        let right = SourceRef::new("a", "b:c").unwrap();
        assert_ne!(left, right);
        assert_ne!(left.stable_key(), right.stable_key());

        let duplicate = OutputPlan::new(
            "duplicate-target",
            Digest::of_bytes(b"entry"),
            Digest::of_bytes(b"selection"),
            vec![
                target_kind("same", DestinationKind::Local),
                target_kind("same", DestinationKind::Remote),
            ],
            time(1),
        );
        assert!(duplicate.is_err());

        let workflow = confirmed_workflow();
        let mut corrupt = workflow.persisted();
        corrupt.selection_digest = Some(Digest::of_bytes(b"wrong selection"));
        assert_eq!(
            Workflow::restore(corrupt),
            Err(WorkflowError::InvalidPersistedState)
        );
    }

    #[test]
    fn freshness_invalidation_preserves_manual_text_but_voids_downstream() {
        for outcome in [
            FreshnessOutcome::Changed {
                source: source("selected"),
            },
            FreshnessOutcome::Unverifiable {
                source: source("selected"),
            },
        ] {
            let mut workflow = confirmed_workflow();
            workflow
                .apply_freshness(
                    workflow.revision(),
                    FreshnessCheckpoint::BeforePreview,
                    outcome,
                    time(20),
                )
                .unwrap();
            assert_eq!(workflow.state(), WorkflowState::Discovered);
            assert_eq!(workflow.entry().text, "Invented day.");
            assert!(workflow.entry().confirmed.is_none());
            assert!(workflow.output_plan().is_none());
            assert!(workflow.approval().is_none());
        }
    }

    #[test]
    fn every_freshness_checkpoint_accepts_fresh_without_mutation() {
        for checkpoint in [
            FreshnessCheckpoint::PostRead,
            FreshnessCheckpoint::BeforeExtraction,
            FreshnessCheckpoint::BeforeDrafting,
            FreshnessCheckpoint::BeforeEntryConfirmation,
            FreshnessCheckpoint::BeforePreview,
            FreshnessCheckpoint::BeforeFirstWrite,
        ] {
            let mut workflow = composing_workflow();
            let before = workflow.clone();
            workflow
                .apply_freshness(
                    workflow.revision(),
                    checkpoint,
                    FreshnessOutcome::Fresh,
                    time(20),
                )
                .unwrap();
            assert_eq!(workflow, before);
        }
    }

    #[test]
    fn source_reader_checkpoint_invalidates_then_allows_reread_and_reconfirmation() {
        let mut workflow = confirmed_workflow();
        let reader = FakeSourceReader {
            outcome: Ok(FreshnessOutcome::Changed {
                source: source("selected"),
            }),
        };
        revalidate_workflow_sources(
            &mut workflow,
            &reader,
            FreshnessCheckpoint::BeforePreview,
            time(30),
        )
        .unwrap();
        assert_eq!(workflow.state(), WorkflowState::Discovered);
        assert_eq!(workflow.entry().text, "Invented day.");
        assert!(workflow.entry().confirmed.is_none());

        workflow
            .confirm_sources(
                workflow.revision(),
                vec![snapshot("selected", "fresh body")],
                false,
                time(31),
            )
            .unwrap();
        workflow
            .confirm_entry(workflow.revision(), workflow.entry().version, time(32))
            .unwrap();
        assert_eq!(workflow.state(), WorkflowState::EntryConfirmed);
    }

    struct FakeSourceReader {
        outcome: Result<FreshnessOutcome, SourceAccessError>,
    }

    impl SourceReader for FakeSourceReader {
        fn read_selected(
            &self,
            _source: &SourceRef,
        ) -> Result<SourceReadOutcome, SourceAccessError> {
            unreachable!()
        }

        fn revalidate(
            &self,
            _snapshot: &SourceSnapshot,
        ) -> Result<FreshnessOutcome, SourceAccessError> {
            self.outcome.clone()
        }

        fn request_download(&self, _source: &SourceRef) -> Result<(), SourceAccessError> {
            unreachable!()
        }
    }

    #[test]
    fn source_reader_access_failure_invalidates_before_returning_the_error() {
        let mut workflow = confirmed_workflow();
        let reader = FakeSourceReader {
            outcome: Err(SourceAccessError::TimedOut),
        };
        assert_eq!(
            revalidate_workflow_sources(
                &mut workflow,
                &reader,
                FreshnessCheckpoint::BeforePreview,
                time(30),
            ),
            Err(SourceCheckpointError::Access(SourceAccessError::TimedOut))
        );
        assert_eq!(workflow.state(), WorkflowState::Discovered);
        assert_eq!(workflow.entry().text, "Invented day.");
        assert!(workflow.entry().confirmed.is_none());
        assert!(matches!(
            workflow.invalidation_reason(),
            Some(InvalidationReason::FreshnessUnverifiable(failed_source))
                if failed_source == &source("selected")
        ));
    }

    #[test]
    fn entry_versions_are_independent_and_confirmation_checks_both_versions() {
        let mut workflow = composing_workflow();
        let workflow_revision = workflow.revision();
        workflow.save_entry(0, "first".to_owned(), time(5)).unwrap();
        assert_eq!(workflow.revision(), workflow_revision);
        assert_eq!(
            workflow.save_entry(0, "racing".to_owned(), time(6)),
            Err(WorkflowError::StaleBuffer {
                expected: 0,
                actual: 1
            })
        );
        assert_eq!(
            workflow.confirm_entry(workflow_revision, 0, time(7)),
            Err(WorkflowError::StaleBuffer {
                expected: 0,
                actual: 1
            })
        );
        workflow
            .confirm_entry(workflow_revision, 1, time(8))
            .unwrap();
        let confirmed_revision = workflow.revision();
        workflow
            .save_entry(1, "second".to_owned(), time(9))
            .unwrap();
        assert_eq!(workflow.revision(), confirmed_revision + 1);
        assert_eq!(workflow.state(), WorkflowState::Composing);
        assert!(workflow.entry().confirmed.is_none());
    }

    #[test]
    fn blank_and_inferred_reflection_text_cannot_be_confirmed() {
        let mut workflow = composing_workflow();
        workflow.save_entry(0, "  \n".to_owned(), time(5)).unwrap();
        assert_eq!(
            workflow.confirm_entry(workflow.revision(), 1, time(6)),
            Err(WorkflowError::BlankEntry)
        );
        workflow
            .save_entry(
                1,
                format!("Review {INFERRED_REFLECTION_OPEN}maybe"),
                time(7),
            )
            .unwrap();
        assert_eq!(
            workflow.confirm_entry(workflow.revision(), 2, time(8)),
            Err(WorkflowError::UnresolvedInferenceMarker)
        );
    }

    #[test]
    fn language_and_style_changes_stale_model_material_without_losing_text() {
        let mut workflow = confirmed_workflow();
        workflow
            .set_style(workflow.revision(), WritingStyle::Reflective, time(20))
            .unwrap();
        assert_eq!(workflow.state(), WorkflowState::Composing);
        assert_eq!(workflow.entry().text, "Invented day.");
        assert!(workflow.entry().confirmed.is_none());
        workflow
            .set_language(workflow.revision(), WritingLanguage::KoKr, time(21))
            .unwrap();
        assert_eq!(workflow.entry().text, "Invented day.");
    }

    #[test]
    fn plan_and_approval_are_bound_to_exact_inputs_and_retry_policy() {
        let mut workflow = confirmed_workflow();
        let wrong = OutputPlan::new(
            "wrong",
            Digest::of_bytes(b"different entry"),
            workflow.selection_digest().unwrap().clone(),
            vec![target("local")],
            time(20),
        )
        .unwrap();
        assert_eq!(
            workflow.preview_output(workflow.revision(), wrong, time(20)),
            Err(WorkflowError::PlanBindingMismatch)
        );

        let plan = plan_for(&workflow, "plan-1");
        workflow
            .preview_output(workflow.revision(), plan.clone(), time(21))
            .unwrap();
        let other_digest = Digest::of_bytes(b"other plan");
        assert_eq!(
            workflow.approve(workflow.revision(), &other_digest, time(22)),
            Err(WorkflowError::PlanBindingMismatch)
        );
        workflow
            .approve(workflow.revision(), &plan.plan_digest, time(23))
            .unwrap();
        let mut corrupt_approval = workflow.persisted();
        corrupt_approval.approval.as_mut().unwrap().destination_only = Some("local".to_owned());
        assert_eq!(
            Workflow::restore(corrupt_approval),
            Err(WorkflowError::InvalidPersistedState)
        );
        assert!(!workflow.pending_retry_reuses_approval("local", &plan.plan_digest));
        assert!(!workflow.pending_retry_reuses_approval("local", &other_digest));

        let replacement = plan_for(&workflow, "plan-2");
        let replacement_digest = replacement.plan_digest.clone();
        workflow
            .preview_output(workflow.revision(), replacement, time(24))
            .unwrap();
        assert_eq!(workflow.state(), WorkflowState::OutputPlanned);
        assert!(workflow.approval().is_none());
        workflow
            .approve(workflow.revision(), &replacement_digest, time(25))
            .unwrap();
        workflow
            .save_entry(1, "Changed after approval.".to_owned(), time(26))
            .unwrap();
        assert_eq!(workflow.state(), WorkflowState::Composing);
        assert!(workflow.output_plan().is_none());
        assert!(workflow.approval().is_none());
    }

    #[test]
    fn local_first_and_blocked_remote_retry_require_exact_new_approval() {
        let mut workflow = confirmed_workflow();
        let plan = OutputPlan::new(
            "two-target-plan",
            workflow
                .entry()
                .confirmed
                .as_ref()
                .unwrap()
                .entry_digest
                .clone(),
            workflow.selection_digest().unwrap().clone(),
            vec![
                target_kind("remote", DestinationKind::Remote),
                target_kind("local", DestinationKind::Local),
            ],
            time(20),
        )
        .unwrap();
        workflow
            .preview_output(workflow.revision(), plan.clone(), time(21))
            .unwrap();
        workflow
            .approve(workflow.revision(), &plan.plan_digest, time(22))
            .unwrap();
        workflow
            .begin_publication(workflow.revision(), time(23))
            .unwrap();
        assert_eq!(
            workflow.finish_publication(
                workflow.revision(),
                vec![
                    DestinationProgress {
                        destination: "local".to_owned(),
                        status: DestinationStatus::Pending,
                    },
                    DestinationProgress {
                        destination: "remote".to_owned(),
                        status: DestinationStatus::Verified,
                    },
                ],
                time(24),
            ),
            Err(WorkflowError::LocalMustBeVerified)
        );
        workflow
            .finish_publication(
                workflow.revision(),
                vec![
                    DestinationProgress {
                        destination: "local".to_owned(),
                        status: DestinationStatus::Verified,
                    },
                    DestinationProgress {
                        destination: "remote".to_owned(),
                        status: DestinationStatus::Blocked,
                    },
                ],
                time(25),
            )
            .unwrap();
        assert_eq!(workflow.state(), WorkflowState::Partial);
        let retry_plan = workflow
            .preview_blocked_retry(
                workflow.revision(),
                "remote",
                target_kind("remote", DestinationKind::Remote),
                "remote-retry-plan",
                time(26),
            )
            .unwrap();
        assert_eq!(workflow.state(), WorkflowState::OutputPlanned);
        assert!(workflow.approval().is_none());
        assert_eq!(
            workflow.approve_blocked_retry(
                workflow.revision(),
                "remote",
                &Digest::of_bytes(b"changed plan"),
                time(27),
            ),
            Err(WorkflowError::PlanBindingMismatch)
        );
        assert_eq!(
            workflow.approve(workflow.revision(), &retry_plan.plan_digest, time(28),),
            Err(WorkflowError::RetryNeedsPreview)
        );
        workflow
            .approve_blocked_retry(
                workflow.revision(),
                "remote",
                &retry_plan.plan_digest,
                time(29),
            )
            .unwrap();
        assert_eq!(
            workflow.approval().unwrap().destination_only.as_deref(),
            Some("remote")
        );
    }

    #[test]
    fn pending_remote_reuses_approval_only_after_local_verifies_and_digest_matches() {
        let (mut pending, plan) = two_target_publishing();
        pending
            .finish_publication(
                pending.revision(),
                vec![
                    DestinationProgress {
                        destination: "local".to_owned(),
                        status: DestinationStatus::Verified,
                    },
                    DestinationProgress {
                        destination: "remote".to_owned(),
                        status: DestinationStatus::Pending,
                    },
                ],
                time(30),
            )
            .unwrap();
        assert!(pending.pending_retry_reuses_approval("remote", &plan.plan_digest));
        assert!(!pending.pending_retry_reuses_approval(
            "remote",
            &Digest::of_bytes(b"changed destination state")
        ));
        assert_eq!(
            pending.begin_pending_retry(
                pending.revision(),
                "remote",
                &Digest::of_bytes(b"changed destination state"),
                time(31),
            ),
            Err(WorkflowError::RetryNeedsPreview)
        );
        pending
            .begin_pending_retry(pending.revision(), "remote", &plan.plan_digest, time(32))
            .unwrap();
        assert_eq!(pending.state(), WorkflowState::Publishing);
        assert_eq!(
            pending.approval().unwrap().destination_only.as_deref(),
            Some("remote")
        );
        assert_eq!(
            pending.finish_publication(
                pending.revision(),
                vec![
                    DestinationProgress {
                        destination: "local".to_owned(),
                        status: DestinationStatus::Pending,
                    },
                    DestinationProgress {
                        destination: "remote".to_owned(),
                        status: DestinationStatus::Verified,
                    },
                ],
                time(33),
            ),
            Err(WorkflowError::VerifiedDestinationRegressed)
        );
        pending
            .finish_publication(
                pending.revision(),
                vec![
                    DestinationProgress {
                        destination: "local".to_owned(),
                        status: DestinationStatus::Verified,
                    },
                    DestinationProgress {
                        destination: "remote".to_owned(),
                        status: DestinationStatus::Verified,
                    },
                ],
                time(34),
            )
            .unwrap();
        assert_eq!(pending.state(), WorkflowState::Completed);

        let (mut conflicted, conflicted_plan) = two_target_publishing();
        conflicted
            .finish_publication(
                conflicted.revision(),
                vec![
                    DestinationProgress {
                        destination: "local".to_owned(),
                        status: DestinationStatus::Conflict,
                    },
                    DestinationProgress {
                        destination: "remote".to_owned(),
                        status: DestinationStatus::Pending,
                    },
                ],
                time(30),
            )
            .unwrap();
        assert!(!conflicted.pending_retry_reuses_approval("remote", &conflicted_plan.plan_digest));
    }

    fn two_target_publishing() -> (Workflow, OutputPlan) {
        let mut workflow = confirmed_workflow();
        let plan = OutputPlan::new(
            "two-target-retry",
            workflow
                .entry()
                .confirmed
                .as_ref()
                .unwrap()
                .entry_digest
                .clone(),
            workflow.selection_digest().unwrap().clone(),
            vec![
                target_kind("local", DestinationKind::Local),
                target_kind("remote", DestinationKind::Remote),
            ],
            time(20),
        )
        .unwrap();
        workflow
            .preview_output(workflow.revision(), plan.clone(), time(21))
            .unwrap();
        workflow
            .approve(workflow.revision(), &plan.plan_digest, time(22))
            .unwrap();
        workflow
            .begin_publication(workflow.revision(), time(23))
            .unwrap();
        (workflow, plan)
    }

    fn journey_to_publishing() -> Workflow {
        let mut workflow = confirmed_workflow();
        let plan = plan_for(&workflow, "plan-1");
        workflow
            .preview_output(workflow.revision(), plan.clone(), time(7))
            .unwrap();
        workflow
            .approve(workflow.revision(), &plan.plan_digest, time(8))
            .unwrap();
        workflow
            .begin_publication(workflow.revision(), time(9))
            .unwrap();
        workflow
    }

    fn confirmed_workflow() -> Workflow {
        let mut workflow = composing_workflow();
        workflow
            .save_entry(0, "Invented day.".to_owned(), time(5))
            .unwrap();
        workflow
            .confirm_entry(workflow.revision(), 1, time(6))
            .unwrap();
        workflow
    }

    fn composing_workflow() -> Workflow {
        let mut workflow = discovered_workflow();
        workflow
            .confirm_sources(
                1,
                vec![snapshot("selected", "invented note")],
                false,
                time(2),
            )
            .unwrap();
        workflow.begin_manual_composition(2, time(3)).unwrap();
        workflow
            .set_language(3, WritingLanguage::EnUs, time(4))
            .unwrap();
        workflow
    }

    fn discovered_workflow() -> Workflow {
        let mut workflow = Workflow::new(day(), time(0), &Preferences::default());
        workflow
            .discover(
                0,
                vec![SourceCandidate {
                    source: source("selected"),
                    date_evidence: DateEvidence {
                        directory_day: Some(day()),
                        header_day: Some(day()),
                        warning: None,
                    },
                    availability: SourceAvailability::Resident,
                }],
                time(1),
            )
            .unwrap();
        workflow
    }

    fn plan_for(workflow: &Workflow, id: &str) -> OutputPlan {
        OutputPlan::new(
            id,
            workflow
                .entry()
                .confirmed
                .as_ref()
                .unwrap()
                .entry_digest
                .clone(),
            workflow.selection_digest().unwrap().clone(),
            vec![target("local")],
            time(7),
        )
        .unwrap()
    }

    fn target(name: &str) -> DestinationPlan {
        target_kind(name, DestinationKind::Local)
    }

    fn target_kind(name: &str, kind: DestinationKind) -> DestinationPlan {
        DestinationPlan::new(
            kind,
            name,
            format!("{name}-target"),
            Digest::of_bytes(format!("{name}-state").as_bytes()),
            Digest::of_bytes(format!("{name}-payload").as_bytes()),
            Digest::of_bytes(format!("{name}-inspected").as_bytes()),
        )
        .unwrap()
    }

    fn snapshot(id: &str, body: &str) -> SourceSnapshot {
        SourceSnapshot {
            source: source(id),
            stamp: FreshnessStamp::new(
                format!("file-{id}"),
                body.len() as u64,
                time(1),
                Digest::of_bytes(body.as_bytes()),
            )
            .unwrap(),
            body: body.to_owned(),
        }
    }

    fn source(id: &str) -> SourceRef {
        SourceRef::new("fake", id).unwrap()
    }

    fn day() -> DiaryDay {
        DiaryDay::parse("2026-10-28").unwrap()
    }

    const fn time(value: i64) -> Timestamp {
        Timestamp(value)
    }
}
