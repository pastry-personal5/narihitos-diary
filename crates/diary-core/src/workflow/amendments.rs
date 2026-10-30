//! Workflow amendments rules.

use super::*;

impl Workflow {
    /// Opens an amendment on a `Completed` day. The editor is seeded with the
    /// local file body; the original source snapshots and selection digest
    /// stay, so no source is read and source freshness has nothing to check.
    pub fn seed_amendment(
        &mut self,
        expected_revision: u64,
        current_local_body: String,
        seed: AmendmentSeed,
        now: Timestamp,
    ) -> Result<(), WorkflowError> {
        self.expect_revision(expected_revision)?;
        if self.state != WorkflowState::Completed || self.selection_digest.is_none() {
            return Err(WorkflowError::WrongState);
        }
        self.entry.text = current_local_body;
        self.entry.version += 1;
        self.entry.confirmed = None;
        self.clear_plan_and_approval();
        self.all_verified_at = None;
        self.amendment = Some(seed);
        self.advance(WorkflowState::Composing, now);
        Ok(())
    }

    /// Rebinds the amendment to the destination as it is now (the "reread"
    /// resolution). The person's text is kept; the entry must be reconfirmed
    /// and re-previewed against the new seed.
    pub fn rebind_amendment(
        &mut self,
        expected_revision: u64,
        seed: AmendmentSeed,
        now: Timestamp,
    ) -> Result<(), WorkflowError> {
        self.expect_revision(expected_revision)?;
        let local_unverified = self
            .destination_progress
            .iter()
            .any(|item| item.destination == "local" && item.status != DestinationStatus::Verified);
        if self.amendment.is_none()
            || !(matches!(
                self.state,
                WorkflowState::Composing
                    | WorkflowState::EntryConfirmed
                    | WorkflowState::OutputPlanned
                    | WorkflowState::Approved
            ) || (self.state == WorkflowState::Partial && local_unverified))
        {
            return Err(WorkflowError::WrongState);
        }
        self.job_status = None;
        self.amendment = Some(seed);
        self.entry.confirmed = None;
        self.clear_plan_and_approval();
        self.invalidation_reason = None;
        self.advance(WorkflowState::Composing, now);
        Ok(())
    }

    /// Discards the amendment revision and returns to `Completed`. Receipts
    /// and files are untouched; the workflow keeps only its stripped form,
    /// like an expired one, and the amendment can be offered again.
    pub fn abandon_amendment(
        &mut self,
        expected_revision: u64,
        now: Timestamp,
    ) -> Result<(), WorkflowError> {
        self.expect_revision(expected_revision)?;
        if self.amendment.is_none() || self.state == WorkflowState::Publishing {
            return Err(WorkflowError::WrongState);
        }
        self.amendment = None;
        self.state = WorkflowState::Completed;
        self.strip_for_abandon(now);
        Ok(())
    }

    pub(super) fn strip_for_abandon(&mut self, now: Timestamp) {
        self.revision += 1;
        self.entry.version += 1;
        self.entry.text.clear();
        self.entry.confirmed = None;
        self.reviewed_memories.clear();
        self.assistant_draft = None;
        self.memory_candidate_sets.clear();
        self.reviewed_memory_artifact = None;
        self.draft_proposals.clear();
        self.paragraph_suggestions.clear();
        self.source_refine_suggestions.clear();
        for snapshot in &mut self.snapshots {
            snapshot.body.clear();
        }
        self.clear_plan_and_approval();
        self.all_verified_at = Some(now);
        self.discarded_at = Some(now);
        self.updated_at = now;
    }
}
