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
            let (status, scope) = match intent.destination.as_str() {
                MIRROR_DESTINATION => {
                    let Some(mirror) = self.mirror_output.as_ref() else {
                        continue;
                    };
                    let Some(language) = workflow.language() else {
                        continue;
                    };
                    let payload =
                        render_local_markdown(&intent.day, language, &workflow.entry().text);
                    if Digest::of_bytes(payload.as_bytes()) != intent.payload_digest
                        || mirror.target(&intent.day) != intent.target_identity
                    {
                        continue;
                    }
                    let owned = Self::owned_mirror_digest(&self.receipts_of(&intent.day)?, &seed);
                    let receipt_id = format!("icloud-{}", intent.attempt_id);
                    // A present, different file is the receipted old mirror
                    // (replace attempt); an absent or identical one a create.
                    let result = if matches!(
                        mirror.inspect(&intent.day, &payload),
                        Ok(LocalOutputState::Different(_))
                    ) {
                        let mut destination = AmendLocalDestination {
                            output: mirror.as_ref(),
                            day: &intent.day,
                            payload: &payload,
                            seed_digest: owned.as_ref(),
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
                        let mut destination = LocalDestination {
                            output: mirror.as_ref(),
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
                    }
                    .map_err(|_| CommandError::LocalOutput(LocalOutputError::Unavailable))?;
                    let status = match result {
                        PublicationResult::Verified(_) => DestinationStatus::Verified,
                        PublicationResult::Conflict => DestinationStatus::Conflict,
                        PublicationResult::SafeToRetry => DestinationStatus::Pending,
                        _ => continue,
                    };
                    (status, Reconciled::Mirror)
                }
                "local" => {
                    let language = workflow
                        .language()
                        .ok_or(CommandError::Workflow(WorkflowError::LanguageRequired))?;
                    let payload =
                        render_local_markdown(&intent.day, language, &workflow.entry().text);
                    if Digest::of_bytes(payload.as_bytes()) != intent.payload_digest {
                        continue;
                    }
                    let mut destination = AmendLocalDestination {
                        output: output.as_ref(),
                        day: &intent.day,
                        payload: &payload,
                        seed_digest: Some(&seed.file_digest),
                        receipt_id: versioned("local", version),
                    };
                    let result = PublicationCoordinator::reconcile(
                        self.store.as_ref(),
                        &mut destination,
                        &intent,
                        version,
                        self.clock.now(),
                    )
                    .map_err(|_| CommandError::LocalOutput(LocalOutputError::Unavailable))?;
                    let status = match result {
                        PublicationResult::Verified(_) => DestinationStatus::Verified,
                        PublicationResult::Conflict => DestinationStatus::Conflict,
                        PublicationResult::SafeToRetry => DestinationStatus::Pending,
                        _ => continue,
                    };
                    (status, Reconciled::Local)
                }
                "notion" => {
                    let Some(notion) = self.notion_output.as_ref() else {
                        continue;
                    };
                    let receipts = self.receipts_of(&intent.day)?;
                    let Some(target) = plan
                        .targets
                        .iter()
                        .find(|target| target.destination == "notion")
                    else {
                        continue;
                    };
                    let Some(request) = self.notion_replace_request(
                        &workflow,
                        &seed,
                        &receipts,
                        target.payload_digest.clone(),
                    ) else {
                        continue;
                    };
                    let not_cancelled = AtomicBool::new(false);
                    let mut destination = AmendNotionDestination {
                        output: notion.as_ref(),
                        request: &request,
                        cancelled: &not_cancelled,
                        version,
                    };
                    let result = PublicationCoordinator::reconcile(
                        self.store.as_ref(),
                        &mut destination,
                        &intent,
                        version,
                        self.clock.now(),
                    )
                    .map_err(|_| CommandError::NotionUnavailable)?;
                    let status = match result {
                        PublicationResult::Verified(_) => DestinationStatus::Verified,
                        PublicationResult::Conflict => DestinationStatus::Conflict,
                        PublicationResult::SafeToRetry => DestinationStatus::Pending,
                        _ => continue,
                    };
                    (status, Reconciled::Notion)
                }
                _ => continue,
            };
            match (workflow.state(), scope) {
                (WorkflowState::Publishing, Reconciled::Local) => {
                    self.finish_amendment(workflow, &[("local", status)])?;
                }
                (WorkflowState::Publishing, Reconciled::Mirror) => {
                    self.finish_amendment(
                        workflow,
                        &[
                            ("local", DestinationStatus::Verified),
                            (MIRROR_DESTINATION, status),
                        ],
                    )?;
                }
                (WorkflowState::Publishing, Reconciled::Notion) => {
                    self.finish_amendment(
                        workflow,
                        &[("local", DestinationStatus::Verified), ("notion", status)],
                    )?;
                }
                (WorkflowState::Partial, Reconciled::Mirror) => {
                    let mut workflow = workflow;
                    let revision = workflow.revision();
                    workflow
                        .finish_mirror_retry(revision, status, self.clock.now())
                        .map_err(CommandError::Workflow)?;
                    self.store
                        .compare_and_swap(revision, None, &workflow)
                        .map_err(CommandError::Store)?;
                }
                _ => {}
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
                    if Digest::of_bytes(text.as_bytes()) == seed.file_digest =>
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
