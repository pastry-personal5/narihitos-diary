//! Amendment reconciliation use cases.

use super::*;

impl CommandService {
    /// Resolves every uncertain amendment effect, and every amendment stuck in
    /// `Publishing` without an intent, before its day accepts another command.
    /// It trashes only the stale set and never replays an insert.
    pub fn reconcile_amendment_intents(&self) -> Result<(), CommandError> {
        let Some(output) = self.local_output.as_ref() else {
            return Ok(());
        };
        for intent in self
            .store
            .unresolved_intents(None)
            .map_err(CommandError::Store)?
        {
            let workflow = self.load_workflow(&intent.day)?;
            let Some(seed) = workflow.amendment().cloned() else {
                continue;
            };
            let Some(plan) = workflow.output_plan().cloned() else {
                continue;
            };
            let version = seed.receipt_version + 1;
            if intent.destination != "local" {
                continue;
            }
            let Some(local) = plan
                .targets
                .iter()
                .find(|target| target.destination == "local")
            else {
                continue;
            };
            let language = workflow
                .language()
                .ok_or(CommandError::Workflow(WorkflowError::LanguageRequired))?;
            let payload = render_local_markdown(&intent.day, language, &workflow.entry().text);
            if Digest::of_bytes(payload.as_bytes()) != intent.payload_digest {
                continue;
            }
            let receipt_id = versioned(&format!("local-{}", intent.attempt_id), version);
            let result = if local.inspected_state_digest == Digest::of_bytes(b"absent") {
                let mut destination = LocalDestination {
                    output: output.as_ref(),
                    day: &intent.day,
                    payload: &payload,
                    receipt_id,
                };
                PublicationCoordinator::reconcile(
                    self.store.as_ref(),
                    &mut destination,
                    &intent,
                    version,
                    self.clock.now(),
                )
            } else {
                let mut destination = AmendLocalDestination {
                    output: output.as_ref(),
                    day: &intent.day,
                    payload: &payload,
                    seed_digest: Some(&local.inspected_state_digest),
                    receipt_id,
                };
                PublicationCoordinator::reconcile(
                    self.store.as_ref(),
                    &mut destination,
                    &intent,
                    version,
                    self.clock.now(),
                )
            }
            .map_err(|_| CommandError::LocalOutput(LocalOutputError::Unavailable))?;
            let status = match result {
                PublicationResult::Verified(_) => DestinationStatus::Verified,
                PublicationResult::Conflict => DestinationStatus::Conflict,
                PublicationResult::SafeToRetry => DestinationStatus::Pending,
                _ => continue,
            };
            if workflow.state() == WorkflowState::Publishing {
                self.finish_amendment(workflow, &[("local", status)])?;
            }
        }
        // A crash between persisting `Publishing` and recording its intent.
        for summary in self
            .store
            .list_summaries(diary_control_contract::MAX_WORKFLOWS)
            .map_err(CommandError::Store)?
        {
            if summary.state != WorkflowState::Publishing
                || !self
                    .store
                    .unresolved_intents(Some(&summary.day))
                    .map_err(CommandError::Store)?
                    .is_empty()
            {
                continue;
            }
            let workflow = self.load_workflow(&summary.day)?;
            let Some(seed) = workflow.amendment().cloned() else {
                continue;
            };
            let Some(observed) = workflow.output_plan().and_then(|plan| {
                plan.targets
                    .iter()
                    .find(|target| target.destination == "local")
                    .map(|target| target.inspected_state_digest.clone())
            }) else {
                continue;
            };
            let Some(language) = workflow.language() else {
                continue;
            };
            let payload = render_local_markdown(&summary.day, language, &workflow.entry().text);
            let payload_digest = Digest::of_bytes(payload.as_bytes());
            let receipted = self.receipts_of(&summary.day)?.iter().any(|receipt| {
                receipt.destination == "local"
                    && receipt.version > seed.receipt_version
                    && receipt.payload_digest == payload_digest
            });
            let status = match output.inspect(&summary.day, &payload) {
                Ok(LocalOutputState::Identical) if receipted => DestinationStatus::Verified,
                Ok(LocalOutputState::Different(text))
                    if Digest::of_bytes(text.as_bytes()) == observed =>
                {
                    DestinationStatus::Pending
                }
                Ok(_) => DestinationStatus::Conflict,
                Err(_) => continue,
            };
            self.finish_amendment(workflow, &[("local", status)])?;
        }
        Ok(())
    }
}
