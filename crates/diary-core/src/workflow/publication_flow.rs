//! Workflow publication flow rules.

use super::*;

impl Workflow {
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
        let rereading_local_conflict = self.state == WorkflowState::Partial
            && self.destination_progress.iter().any(|item| {
                item.status == DestinationStatus::Conflict
                    && self.output_plan.as_ref().is_some_and(|plan| {
                        plan.targets.iter().any(|target| {
                            target.kind == DestinationKind::Local
                                && target.destination == item.destination
                        })
                    })
            });
        if !matches!(
            self.state,
            WorkflowState::EntryConfirmed | WorkflowState::OutputPlanned | WorkflowState::Approved
        ) && !rereading_local_conflict
        {
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
        // A mirror conflict is isolated: it never gates local or Notion.
        let is_mirror = |destination: &str| {
            plan.targets.iter().any(|target| {
                target.destination == destination && target.kind == DestinationKind::Mirror
            })
        };
        if self.destination_progress.iter().any(|progress| {
            progress.status == DestinationStatus::Conflict && !is_mirror(&progress.destination)
        }) || plan
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
        if !matches!(
            progress.status,
            DestinationStatus::Blocked | DestinationStatus::Pending
        ) {
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

    /// Replaces a blocked remote target, or a pending target whose complete
    /// binding changed, with a newly inspected preview while preserving
    /// verified local progress. Approval is deliberately cleared.
    pub fn preview_blocked_retry(
        &mut self,
        expected_revision: u64,
        destination: &str,
        replacement: crate::DestinationPlan,
        new_plan_id: impl Into<String>,
        now: Timestamp,
    ) -> Result<OutputPlan, WorkflowError> {
        self.expect_revision(expected_revision)?;
        // A stale, unapproved preview (`OutputPlanned`) may be re-previewed.
        if !matches!(
            self.state,
            WorkflowState::Partial | WorkflowState::OutputPlanned
        ) {
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
        // An amendment's Notion conflict is resolved by rereading: the section
        // is previewed again against its receipt and needs a fresh approval.
        let rereading_amendment =
            self.amendment.is_some() && progress.status == DestinationStatus::Conflict;
        if !matches!(
            progress.status,
            DestinationStatus::Blocked | DestinationStatus::Pending
        ) && !rereading_amendment
        {
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
        if rereading_amendment {
            for item in &mut self.destination_progress {
                if item.destination == destination {
                    item.status = DestinationStatus::Pending;
                }
            }
        }
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

    /// Re-enters a previously approved local write after reconciliation proved
    /// that an uncertain attempt had no effect.
    pub fn begin_local_retry(
        &mut self,
        expected_revision: u64,
        now: Timestamp,
    ) -> Result<(), WorkflowError> {
        self.expect_revision(expected_revision)?;
        if self.state != WorkflowState::Partial
            || self.approval.is_none()
            || !self.destination_progress.iter().any(|item| {
                item.destination == "local" && item.status == DestinationStatus::Pending
            })
        {
            return Err(WorkflowError::RetryNeedsPreview);
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
        if all_verified {
            self.amendment = None;
        }
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

    /// Completes a mirror-only retry from `Partial`. Only the `icloud`
    /// destination changes; a verified destination never regresses.
    pub fn finish_mirror_retry(
        &mut self,
        expected_revision: u64,
        status: DestinationStatus,
        now: Timestamp,
    ) -> Result<(), WorkflowError> {
        self.expect_revision(expected_revision)?;
        if self.state != WorkflowState::Partial || self.approval.is_none() {
            return Err(WorkflowError::WrongState);
        }
        let status_of = |destination: &str| {
            self.destination_progress
                .iter()
                .find(|item| item.destination == destination)
                .map(|item| item.status)
        };
        if status_of("local") != Some(DestinationStatus::Verified) {
            return Err(WorkflowError::LocalMustBeVerified);
        }
        match status_of("icloud") {
            None => return Err(WorkflowError::UnknownDestination),
            Some(DestinationStatus::Verified) => {
                return Err(WorkflowError::VerifiedDestinationRegressed);
            }
            Some(_) => {}
        }
        for item in &mut self.destination_progress {
            if item.destination == "icloud" {
                item.status = status;
            }
        }
        self.job_status = Some(JobStatus::Completed);
        let all_verified = self
            .destination_progress
            .iter()
            .all(|item| item.status == DestinationStatus::Verified);
        self.all_verified_at = all_verified.then_some(now);
        if all_verified {
            self.amendment = None;
            self.advance(WorkflowState::Completed, now);
        } else {
            self.bump(now);
        }
        Ok(())
    }
}
