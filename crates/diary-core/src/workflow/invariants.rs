//! Workflow invariants rules.

use super::*;

impl Workflow {
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
        self.assistant_draft = None;
        self.memory_candidate_sets.clear();
        self.reviewed_memory_artifact = None;
        self.draft_proposals.clear();
        self.paragraph_suggestions.clear();
        self.source_refine_suggestions.clear();
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

    pub(super) fn expect_revision(&self, expected: u64) -> Result<(), WorkflowError> {
        if self.revision != expected {
            return Err(WorkflowError::StaleRevision {
                expected,
                actual: self.revision,
            });
        }
        Ok(())
    }

    pub(super) fn advance(&mut self, state: WorkflowState, now: Timestamp) {
        self.state = state;
        self.bump(now);
    }

    pub(super) fn bump(&mut self, now: Timestamp) {
        self.revision += 1;
        self.updated_at = now;
        self.discarded_at = None;
    }

    pub(super) fn ensure_entry_editable(&self) -> Result<(), WorkflowError> {
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

    pub(super) fn ensure_writing_settings_editable(&self) -> Result<(), WorkflowError> {
        if matches!(
            self.state,
            WorkflowState::Started | WorkflowState::Discovered
        ) {
            Ok(())
        } else {
            self.ensure_entry_editable()
        }
    }

    pub(super) fn clear_after_selection(&mut self) {
        for candidates in &mut self.memory_candidate_sets {
            candidates.stale = true;
        }
        if let Some(reviewed) = &mut self.reviewed_memory_artifact {
            reviewed.stale = true;
        }
        for draft in &mut self.draft_proposals {
            draft.stale = true;
        }
        for suggestion in &mut self.paragraph_suggestions {
            suggestion.stale = true;
        }
        self.assistant_material_stale = true;
        self.entry.confirmed = None;
        self.clear_plan_and_approval();
    }

    pub(super) fn clear_plan_and_approval(&mut self) {
        self.output_plan = None;
        self.approval = None;
        self.destination_progress.clear();
        self.job_status = None;
    }

    pub(super) fn invalidate_sources(&mut self, reason: InvalidationReason, now: Timestamp) {
        self.snapshots.clear();
        self.selection_digest = None;
        self.explicit_empty_selection = false;
        self.entry.confirmed = None;
        self.clear_after_selection();
        self.invalidation_reason = Some(reason);
        self.advance(WorkflowState::Discovered, now);
    }

    pub(super) fn invalidate_composition(&mut self, now: Timestamp) {
        self.assistant_material_stale = true;
        for draft in &mut self.draft_proposals {
            draft.stale = true;
        }
        for suggestion in &mut self.paragraph_suggestions {
            suggestion.stale = true;
        }
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

    pub(super) fn validate(&self) -> Result<(), WorkflowError> {
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
                if (self.state != WorkflowState::Discovered && !self.snapshots.is_empty())
                    || self.explicit_empty_selection
                {
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

        let mut artifact_ids = BTreeSet::new();
        for set in &self.memory_candidate_sets {
            let mut candidate_ids = BTreeSet::new();
            if !artifact_ids.insert((&set.id, "memory-set"))
                || set.candidates.len() > crate::MAX_MEMORY_CANDIDATES
                || set.candidates.iter().any(|candidate| {
                    !candidate_ids.insert(&candidate.id)
                        || candidate.text.is_empty()
                        || candidate.text.len() > crate::MAX_MEMORY_CANDIDATE_BYTES
                        || candidate.source_snapshot_digests.is_empty()
                        || candidate.source_snapshot_digests.len() > crate::MAX_SOURCE_ORDINALS
                })
            {
                return Err(WorkflowError::InvalidPersistedState);
            }
        }
        if let Some(reviewed) = &self.reviewed_memory_artifact
            && (reviewed.text.len() > crate::MAX_REVIEWED_MEMORY_BYTES
                || reviewed.content_digest != Digest::of_bytes(reviewed.text.as_bytes()))
        {
            return Err(WorkflowError::InvalidPersistedState);
        }
        for draft in &self.draft_proposals {
            if draft.text.is_empty()
                || draft.text.len() > crate::MAX_DRAFT_RESULT_BYTES
                || draft.reflection_ranges.len() > crate::MAX_REFLECTION_RANGES
                || draft.content_digest != Digest::of_bytes(draft.text.as_bytes())
            {
                return Err(WorkflowError::InvalidPersistedState);
            }
        }
        for suggestion in &self.paragraph_suggestions {
            if suggestion.replacement.is_empty()
                || suggestion.replacement.len() > crate::MAX_PARAGRAPH_BYTES
                || suggestion.reflection_ranges.len() > crate::MAX_REFLECTION_RANGES
                || suggestion
                    .explanation
                    .as_ref()
                    .is_some_and(|value| value.len() > crate::MAX_SUGGESTION_EXPLANATION_BYTES)
                || suggestion.content_digest != Digest::of_bytes(suggestion.replacement.as_bytes())
                || suggestion.resolution.is_some() != suggestion.resolved_at.is_some()
            {
                return Err(WorkflowError::InvalidPersistedState);
            }
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
        let amendment_allowed = self.amendment.is_none()
            || matches!(
                self.state,
                WorkflowState::Composing
                    | WorkflowState::EntryConfirmed
                    | WorkflowState::OutputPlanned
                    | WorkflowState::Approved
                    | WorkflowState::Publishing
                    | WorkflowState::Partial
            );
        if !valid_state
            || !amendment_allowed
            || (self.state != WorkflowState::Completed && self.all_verified_at.is_some())
        {
            return Err(WorkflowError::InvalidPersistedState);
        }
        Ok(())
    }
}
