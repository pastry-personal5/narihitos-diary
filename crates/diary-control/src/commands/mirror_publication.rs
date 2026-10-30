//! Mirror publication use cases.

use super::*;

impl CommandService {
    /// A plan carries an `icloud` target exactly when a mirror is configured,
    /// for the same target and bytes; otherwise a new preview is required.
    pub(super) fn ensure_mirror_binding(
        &self,
        plan: &OutputPlan,
        day: &DiaryDay,
        payload_digest: &Digest,
    ) -> Result<(), CommandError> {
        let planned = plan
            .targets
            .iter()
            .find(|target| target.kind == DestinationKind::Mirror);
        match (&self.mirror_output, planned) {
            (None, None) => Ok(()),
            (Some(mirror), Some(target))
                if target.destination == MIRROR_DESTINATION
                    && target.target_identity == mirror.target(day)
                    && &target.payload_digest == payload_digest =>
            {
                Ok(())
            }
            _ => Err(CommandError::PreconditionFailed),
        }
    }

    /// Runs the mirror write after a verified local write. Every failure is
    /// contained: it yields a non-verified status and never an error.
    pub(super) fn run_mirror(
        &self,
        workflow: &Workflow,
        plan: &OutputPlan,
        day: &DiaryDay,
        payload: &str,
    ) -> DestinationStatus {
        let planned = workflow
            .destination_progress()
            .iter()
            .find(|item| item.destination == MIRROR_DESTINATION)
            .map_or(DestinationStatus::Blocked, |item| item.status);
        if planned != DestinationStatus::Pending {
            return planned;
        }
        self.execute_mirror(plan, day, payload)
    }

    pub(super) fn execute_mirror(
        &self,
        plan: &OutputPlan,
        day: &DiaryDay,
        payload: &str,
    ) -> DestinationStatus {
        let (Some(mirror), Some(target)) = (
            &self.mirror_output,
            plan.targets
                .iter()
                .find(|target| target.kind == DestinationKind::Mirror),
        ) else {
            return DestinationStatus::Blocked;
        };
        // Read-only preflight: never record an intent for a mirror that
        // cannot be inspected, so a missing folder cannot gate the day.
        match mirror.inspect(day, payload) {
            Err(_) => return DestinationStatus::Blocked,
            Ok(LocalOutputState::Different(_)) => return DestinationStatus::Conflict,
            Ok(_) => {}
        }
        let Ok(attempt) = random_id("attempt_") else {
            return DestinationStatus::Blocked;
        };
        let receipt_id = format!("icloud-{attempt}");
        let Ok(intent) = OperationIntent::new(
            attempt,
            plan.plan_id.clone(),
            day.clone(),
            target,
            plan.entry_digest.clone(),
            plan.selection_digest.clone(),
            self.clock.now(),
        ) else {
            return DestinationStatus::Blocked;
        };
        let mut destination = LocalDestination {
            output: mirror.as_ref(),
            day,
            payload,
            receipt_id,
        };
        match PublicationCoordinator::execute(
            self.store.as_ref(),
            &mut destination,
            &intent,
            1,
            self.clock.now(),
        ) {
            Ok(PublicationResult::Verified(_)) => DestinationStatus::Verified,
            Ok(PublicationResult::Conflict) => DestinationStatus::Conflict,
            // An uncertain write keeps its intent; startup reconciliation
            // resolves it before any retry.
            _ => DestinationStatus::Pending,
        }
    }

    /// Retries only the iCloud mirror from the verified local file.
    pub fn publish_mirror_guarded(
        &self,
        day: &DiaryDay,
        expected_revision: u64,
        commit: impl FnOnce() -> bool,
    ) -> Result<Workflow, CommandError> {
        self.ensure_clear(day)?;
        if self.mirror_output.is_none() {
            return Err(CommandError::LocalOutput(LocalOutputError::Unavailable));
        }
        let mut workflow = self.load_workflow(day)?;
        ensure_revision(&workflow, expected_revision)?;
        if workflow.amendment().is_some() {
            // The mirror holds the app's earlier bytes: replace, never create.
            return self.publish_amendment_mirror(day, expected_revision, commit);
        }
        if workflow.state() != diary_core::WorkflowState::Partial {
            return Err(CommandError::Workflow(WorkflowError::WrongState));
        }
        self.verify_local_receipt(&workflow)?;
        let plan = workflow
            .output_plan()
            .ok_or(CommandError::Workflow(WorkflowError::MissingPlan))?
            .clone();
        let payload = render_local_markdown(
            day,
            workflow
                .language()
                .ok_or(CommandError::Workflow(WorkflowError::LanguageRequired))?,
            &workflow.entry().text,
        );
        self.ensure_mirror_binding(&plan, day, &Digest::of_bytes(payload.as_bytes()))?;
        if !commit() {
            return Err(CommandError::Cancelled);
        }
        let status = self.execute_mirror(&plan, day, &payload);
        workflow
            .finish_mirror_retry(expected_revision, status, self.clock.now())
            .map_err(CommandError::Workflow)?;
        self.store
            .compare_and_swap(expected_revision, None, &workflow)
            .map_err(CommandError::Store)?;
        Ok(workflow)
    }

    /// Recovers uncertain mirror effects. An interrupted publication job
    /// finishes through `finish_publication`; a mirror-only retry through
    /// `finish_mirror_retry`.
    pub fn reconcile_mirror_intents(&self) -> Result<(), CommandError> {
        let Some(mirror) = &self.mirror_output else {
            return Ok(());
        };
        for intent in self
            .store
            .unresolved_intents(None)
            .map_err(CommandError::Store)?
        {
            if intent.destination != MIRROR_DESTINATION {
                continue;
            }
            let mut workflow = self.load_workflow(&intent.day)?;
            if workflow.amendment().is_some() {
                continue;
            }
            let Some(language) = workflow.language() else {
                continue;
            };
            let payload = render_local_markdown(&intent.day, language, &workflow.entry().text);
            if Digest::of_bytes(payload.as_bytes()) != intent.payload_digest
                || mirror.target(&intent.day) != intent.target_identity
            {
                continue;
            }
            let mut destination = LocalDestination {
                output: mirror.as_ref(),
                day: &intent.day,
                payload: &payload,
                receipt_id: format!("icloud-{}", intent.attempt_id),
            };
            let result = PublicationCoordinator::reconcile(
                self.store.as_ref(),
                &mut destination,
                &intent,
                1,
                self.clock.now(),
            )
            .map_err(|error| match error {
                PublicationError::Store(error) => CommandError::Store(error),
                PublicationError::InvalidOutcome => {
                    CommandError::LocalOutput(LocalOutputError::Unavailable)
                }
            })?;
            let status = match result {
                PublicationResult::Verified(_) => DestinationStatus::Verified,
                PublicationResult::Conflict => DestinationStatus::Conflict,
                PublicationResult::SafeToRetry => DestinationStatus::Pending,
                _ => continue,
            };
            let revision = workflow.revision();
            match workflow.state() {
                diary_core::WorkflowState::Publishing => {
                    workflow
                        .finish_publication(
                            revision,
                            progress_with(
                                &workflow,
                                &[
                                    ("local", DestinationStatus::Verified),
                                    (MIRROR_DESTINATION, status),
                                ],
                            ),
                            self.clock.now(),
                        )
                        .map_err(CommandError::Workflow)?;
                }
                diary_core::WorkflowState::Partial => {
                    workflow
                        .finish_mirror_retry(revision, status, self.clock.now())
                        .map_err(CommandError::Workflow)?;
                }
                _ => continue,
            }
            self.store
                .compare_and_swap(revision, None, &workflow)
                .map_err(CommandError::Store)?;
        }
        Ok(())
    }
}
