//! Reconciliation use cases.

use super::*;

impl CommandService {
    pub(super) fn verify_local_receipt(&self, workflow: &Workflow) -> Result<(), CommandError> {
        let output = self
            .local_output
            .as_ref()
            .ok_or(CommandError::LocalOutput(LocalOutputError::Unavailable))?;
        let language = workflow
            .language()
            .ok_or(CommandError::Workflow(WorkflowError::LanguageRequired))?;
        let payload = render_local_markdown(workflow.day(), language, &workflow.entry().text);
        let payload_digest = Digest::of_bytes(payload.as_bytes());
        if output
            .inspect(workflow.day(), &payload)
            .map_err(CommandError::LocalOutput)?
            != LocalOutputState::Identical
        {
            return Err(CommandError::Workflow(WorkflowError::LocalMustBeVerified));
        }
        let confirmation = workflow
            .entry()
            .confirmed
            .as_ref()
            .ok_or(CommandError::Workflow(WorkflowError::EntryNotConfirmed))?;
        let selection = workflow
            .selection_digest()
            .ok_or(CommandError::Workflow(WorkflowError::PlanBindingMismatch))?;
        let target = output.target(workflow.day());
        let receipted = self
            .store
            .receipts(workflow.day())
            .map_err(CommandError::Store)?
            .iter()
            .any(|receipt| {
                receipt.destination == "local"
                    && receipt.target_identity == target
                    && receipt.payload_digest == payload_digest
                    && receipt.entry_digest == confirmation.entry_digest
                    && &receipt.selection_digest == selection
            });
        if !receipted {
            return Err(CommandError::Workflow(WorkflowError::LocalMustBeVerified));
        }
        Ok(())
    }

    /// Recover uncertain local effects before accepting commands for their day.
    pub fn reconcile_local_intents(&self) -> Result<(), CommandError> {
        let Some(output) = &self.local_output else {
            return Ok(());
        };
        for intent in self
            .store
            .unresolved_intents(None)
            .map_err(CommandError::Store)?
        {
            if intent.destination != "local" {
                continue;
            }
            let mut workflow = self.load_workflow(&intent.day)?;
            let Some(language) = workflow.language() else {
                continue;
            };
            let payload = render_local_markdown(&intent.day, language, &workflow.entry().text);
            if Digest::of_bytes(payload.as_bytes()) != intent.payload_digest
                || output.target(&intent.day) != intent.target_identity
            {
                continue;
            }
            let mut destination = LocalDestination {
                output: output.as_ref(),
                day: &intent.day,
                payload: &payload,
                receipt_id: format!("local-{}", intent.attempt_id),
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
            if workflow.state() == diary_core::WorkflowState::Publishing {
                let revision = workflow.revision();
                workflow
                    .finish_publication(
                        revision,
                        progress_with(
                            &workflow,
                            &[("local", status), ("notion", DestinationStatus::Blocked)],
                        ),
                        self.clock.now(),
                    )
                    .map_err(CommandError::Workflow)?;
                self.store
                    .compare_and_swap(revision, None, &workflow)
                    .map_err(CommandError::Store)?;
            }
        }
        // A crash may land between persisting `Publishing` and recording its
        // intent, or between committing a receipt and advancing the workflow.
        for summary in self
            .store
            .list_summaries(diary_control_contract::MAX_WORKFLOWS)
            .map_err(CommandError::Store)?
        {
            if summary.state != diary_core::WorkflowState::Publishing {
                continue;
            }
            let day = summary.day;
            if !self
                .store
                .unresolved_intents(Some(&day))
                .map_err(CommandError::Store)?
                .is_empty()
            {
                continue;
            }
            let mut workflow = self.load_workflow(&day)?;
            if workflow.amendment().is_some() {
                continue;
            }
            let Some(plan) = workflow.output_plan() else {
                continue;
            };
            let Some(target) = plan
                .targets
                .iter()
                .find(|target| target.destination == "local")
            else {
                continue;
            };
            let Some(language) = workflow.language() else {
                continue;
            };
            let payload = render_local_markdown(&day, language, &workflow.entry().text);
            if Digest::of_bytes(payload.as_bytes()) != target.payload_digest {
                continue;
            }
            let receipted = self
                .store
                .receipts(&day)
                .map_err(CommandError::Store)?
                .iter()
                .any(|receipt| {
                    receipt.destination == "local"
                        && receipt.target_identity == target.target_identity
                        && receipt.payload_digest == target.payload_digest
                        && receipt.entry_digest == plan.entry_digest
                        && receipt.selection_digest == plan.selection_digest
                });
            let status = match output.inspect(&day, &payload) {
                Ok(LocalOutputState::Identical) if receipted => DestinationStatus::Verified,
                Ok(LocalOutputState::Absent) if !receipted => DestinationStatus::Pending,
                Ok(_) => DestinationStatus::Conflict,
                Err(_) => continue,
            };
            let revision = workflow.revision();
            workflow
                .finish_publication(
                    revision,
                    progress_with(
                        &workflow,
                        &[("local", status), ("notion", DestinationStatus::Blocked)],
                    ),
                    self.clock.now(),
                )
                .map_err(CommandError::Workflow)?;
            self.store
                .compare_and_swap(revision, None, &workflow)
                .map_err(CommandError::Store)?;
        }
        self.reconcile_amendment_intents()
    }

    /// Reconciles every uncertain Notion effect before that diary day accepts
    /// another command. Exact active intents may adopt a unique matching
    /// section; unrelated same-day content remains a conflict.
    pub fn reconcile_notion_intents(&self) -> Result<(), CommandError> {
        let Some(output) = &self.notion_output else {
            return Ok(());
        };
        for intent in self
            .store
            .unresolved_intents(None)
            .map_err(CommandError::Store)?
        {
            if intent.destination != "notion" {
                continue;
            }
            let mut workflow = self.load_workflow(&intent.day)?;
            if workflow.amendment().is_some() {
                continue;
            }
            let Some(plan) = workflow.output_plan() else {
                continue;
            };
            let Some(target) = plan
                .targets
                .iter()
                .find(|target| target.destination == "notion")
            else {
                continue;
            };
            if target.target_identity != intent.target_identity
                || target.payload_digest != intent.payload_digest
            {
                continue;
            }
            let request = NotionRequest {
                day: intent.day.clone(),
                markdown: workflow.entry().text.clone(),
                expected_target_identity: target.target_identity.clone(),
                expected_payload_digest: target.payload_digest.clone(),
                expected_state_digest: target.inspected_state_digest.clone(),
            };
            let not_cancelled = AtomicBool::new(false);
            let mut destination = NotionDestination {
                output: output.as_ref(),
                request: &request,
                cancelled: &not_cancelled,
            };
            let result = PublicationCoordinator::reconcile(
                self.store.as_ref(),
                &mut destination,
                &intent,
                2,
                self.clock.now(),
            )
            .map_err(|_| CommandError::NotionUnavailable)?;
            let status = match result {
                PublicationResult::Verified(_) => DestinationStatus::Verified,
                PublicationResult::Conflict => DestinationStatus::Conflict,
                PublicationResult::SafeToRetry => DestinationStatus::Pending,
                _ => continue,
            };
            if workflow.state() == diary_core::WorkflowState::Publishing {
                let revision = workflow.revision();
                workflow
                    .finish_publication(
                        revision,
                        vec![
                            DestinationProgress {
                                destination: "local".into(),
                                status: DestinationStatus::Verified,
                            },
                            DestinationProgress {
                                destination: "notion".into(),
                                status,
                            },
                        ],
                        self.clock.now(),
                    )
                    .map_err(CommandError::Workflow)?;
                self.store
                    .compare_and_swap(revision, None, &workflow)
                    .map_err(CommandError::Store)?;
            }
        }
        Ok(())
    }

    pub fn save_local_conflict_under_day(
        &self,
        source_day: &DiaryDay,
        expected_revision: u64,
        new_day: DiaryDay,
    ) -> Result<Workflow, CommandError> {
        self.ensure_clear(source_day)?;
        self.ensure_clear(&new_day)?;
        let source = self.load_workflow(source_day)?;
        ensure_revision(&source, expected_revision)?;
        if source_day == &new_day {
            return Err(CommandError::PreconditionFailed);
        }
        let preferences = self.store.load_preferences().map_err(CommandError::Store)?;
        let carried = source
            .carry_conflict_to_day(new_day.clone(), self.clock.now(), &preferences)
            .map_err(CommandError::Workflow)?;
        match self
            .store
            .start_or_resume(new_day.clone(), self.clock.now(), &preferences)
            .map_err(CommandError::Store)?
        {
            StartOutcome::Created(created) => {
                let output = self
                    .local_output
                    .as_ref()
                    .ok_or(CommandError::LocalOutput(LocalOutputError::Unavailable))?;
                let candidate_payload = render_local_markdown(
                    &new_day,
                    source.language().unwrap_or(WritingLanguage::EnUs),
                    &source.entry().text,
                );
                if output
                    .inspect(&new_day, &candidate_payload)
                    .map_err(CommandError::LocalOutput)?
                    != LocalOutputState::Absent
                {
                    return Ok(created);
                }
                self.store
                    .compare_and_swap(0, None, &carried)
                    .map_err(CommandError::Store)?;
                Ok(carried)
            }
            StartOutcome::Resumed(existing) | StartOutcome::Completed(existing) => Ok(existing),
        }
    }
}
