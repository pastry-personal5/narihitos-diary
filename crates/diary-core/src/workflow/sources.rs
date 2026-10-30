//! Workflow sources rules.

use super::*;

impl Workflow {
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
        let direct_explicit_empty = self.state == WorkflowState::Started
            && snapshots.is_empty()
            && explicitly_continue_empty;
        if !direct_explicit_empty
            && !matches!(
                self.state,
                WorkflowState::Discovered
                    | WorkflowState::SourcesConfirmed
                    | WorkflowState::Composing
                    | WorkflowState::EntryConfirmed
                    | WorkflowState::OutputPlanned
                    | WorkflowState::Approved
            )
        {
            return Err(WorkflowError::WrongState);
        }
        let replaces_confirmation = !matches!(
            self.state,
            WorkflowState::Started | WorkflowState::Discovered
        );
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
                | WorkflowState::Partial
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
}
