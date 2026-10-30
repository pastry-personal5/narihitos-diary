//! Amendment publication use cases.

use super::*;

impl CommandService {
    pub fn approve_amendment(
        &self,
        day: &DiaryDay,
        expected_revision: u64,
        digest: &Digest,
    ) -> Result<Workflow, CommandError> {
        self.ensure_clear(day)?;
        let output = self
            .local_output
            .as_ref()
            .ok_or(CommandError::LocalOutput(LocalOutputError::Unavailable))?;
        let mut workflow = self.load_workflow(day)?;
        ensure_revision(&workflow, expected_revision)?;
        let seed = ensure_amending(&workflow)?.clone();
        let plan = workflow
            .output_plan()
            .ok_or(CommandError::Workflow(WorkflowError::MissingPlan))?;
        if &plan.plan_digest != digest {
            return Err(CommandError::PreconditionFailed);
        }
        if local_verified(&workflow) {
            let target = plan
                .targets
                .iter()
                .find(|target| target.destination == "notion")
                .ok_or(CommandError::Workflow(WorkflowError::MissingPlan))?;
            let receipts = self.receipts_of(day)?;
            let request = self
                .notion_replace_request(&workflow, &seed, &receipts, target.payload_digest.clone())
                .ok_or(CommandError::PreconditionFailed)?;
            let fresh = self
                .notion_output
                .as_ref()
                .ok_or(CommandError::NotionUnavailable)?
                .preflight_replace(&request)
                .map_err(|reason| notion_preflight_error(&reason))?;
            if let Some(reason) = notion_binding_refusal(&fresh.preflight, target, false) {
                return Err(CommandError::NotionBindingChanged(reason));
            }
            workflow
                .approve_blocked_retry(expected_revision, "notion", digest, self.clock.now())
                .map_err(CommandError::Workflow)?;
            self.store
                .compare_and_swap(expected_revision, None, &workflow)
                .map_err(CommandError::Store)?;
            return Ok(workflow);
        }
        let local = plan
            .targets
            .iter()
            .find(|target| target.destination == "local")
            .ok_or(CommandError::Workflow(WorkflowError::MissingPlan))?;
        let current = match output.inspect(day, "").map_err(CommandError::LocalOutput)? {
            LocalOutputState::Different(text) => Digest::of_bytes(text.as_bytes()),
            LocalOutputState::Identical => Digest::of_bytes(b""),
            LocalOutputState::Absent => return Err(CommandError::PreconditionFailed),
        };
        if current != seed.file_digest || current != local.inspected_state_digest {
            return Err(CommandError::PreconditionFailed);
        }
        let receipts = self.receipts_of(day)?;
        let payload = render_local_markdown(
            day,
            workflow
                .language()
                .ok_or(CommandError::Workflow(WorkflowError::LanguageRequired))?,
            &workflow.entry().text,
        );
        self.ensure_mirror_binding(plan, day, &Digest::of_bytes(payload.as_bytes()))?;
        if let (Some(mirror), Some(planned)) = (
            &self.mirror_output,
            plan.targets
                .iter()
                .find(|target| target.kind == DestinationKind::Mirror),
        ) {
            let owned = Self::owned_mirror_digest(&receipts, &seed);
            let (observed, _) =
                Self::classify_mirror(mirror.as_ref(), day, &payload, owned.as_ref());
            if observed != planned.inspected_state_digest {
                return Err(CommandError::PreconditionFailed);
            }
        }
        let notion_status = workflow
            .destination_progress()
            .iter()
            .find(|item| item.destination == "notion")
            .map(|item| item.status);
        if notion_status == Some(DestinationStatus::Pending) {
            let notion = plan
                .targets
                .iter()
                .find(|target| target.destination == "notion")
                .ok_or(CommandError::Workflow(WorkflowError::MissingPlan))?;
            let request = self
                .notion_replace_request(&workflow, &seed, &receipts, notion.payload_digest.clone())
                .ok_or(CommandError::PreconditionFailed)?;
            let fresh = self
                .notion_output
                .as_ref()
                .ok_or(CommandError::NotionUnavailable)?
                .preflight_replace(&request)
                .map_err(|reason| notion_preflight_error(&reason))?;
            if let Some(reason) = notion_binding_refusal(&fresh.preflight, notion, false) {
                return Err(CommandError::NotionBindingChanged(reason));
            }
        }
        workflow
            .approve(expected_revision, digest, self.clock.now())
            .map_err(CommandError::Workflow)?;
        self.store
            .compare_and_swap(expected_revision, None, &workflow)
            .map_err(CommandError::Store)?;
        Ok(workflow)
    }

    /// Replaces the local file first, then Notion. A Notion failure or
    /// uncertain write leaves the day `Partial` with the local file already
    /// holding the new text.
    pub fn publish_amendment(
        &self,
        day: &DiaryDay,
        expected_revision: u64,
        cancelled: &AtomicBool,
    ) -> Result<Workflow, CommandError> {
        self.ensure_clear(day)?;
        let output = self
            .local_output
            .as_ref()
            .ok_or(CommandError::LocalOutput(LocalOutputError::Unavailable))?;
        let mut workflow = self.load_workflow(day)?;
        ensure_revision(&workflow, expected_revision)?;
        let seed = ensure_amending(&workflow)?.clone();
        let plan = workflow
            .output_plan()
            .ok_or(CommandError::Workflow(WorkflowError::MissingPlan))?
            .clone();
        let language = workflow
            .language()
            .ok_or(CommandError::Workflow(WorkflowError::LanguageRequired))?;
        let payload = render_local_markdown(day, language, &workflow.entry().text);
        let local_target = plan
            .targets
            .iter()
            .find(|target| target.destination == "local")
            .ok_or(CommandError::Workflow(WorkflowError::MissingPlan))?;
        if Digest::of_bytes(payload.as_bytes()) != local_target.payload_digest {
            return Err(CommandError::PreconditionFailed);
        }
        let version = seed.receipt_version + 1;
        let local_done = local_verified(&workflow);
        if workflow.state() == WorkflowState::Partial {
            workflow
                .begin_local_retry(expected_revision, self.clock.now())
                .map_err(CommandError::Workflow)?;
        } else {
            workflow
                .begin_publication(expected_revision, self.clock.now())
                .map_err(CommandError::Workflow)?;
        }
        self.store
            .compare_and_swap(expected_revision, None, &workflow)
            .map_err(CommandError::Store)?;

        if local_done {
            // A reread Notion preview was approved: the local file already
            // holds the new text, so only Notion is attempted.
            let Some(status) =
                self.run_amendment_notion(&workflow, &seed, &plan, version, cancelled)?
            else {
                return Ok(workflow);
            };
            return self.finish_amendment(
                workflow,
                &[("local", DestinationStatus::Verified), ("notion", status)],
            );
        }
        let intent = OperationIntent::new(
            random_id("attempt_")
                .map_err(|_| CommandError::LocalOutput(LocalOutputError::Unavailable))?,
            plan.plan_id.clone(),
            day.clone(),
            local_target,
            plan.entry_digest.clone(),
            plan.selection_digest.clone(),
            self.clock.now(),
        )
        .map_err(|_| CommandError::LocalOutput(LocalOutputError::Unavailable))?;
        let mut destination = AmendLocalDestination {
            output: output.as_ref(),
            day,
            payload: &payload,
            seed_digest: Some(&seed.file_digest),
            receipt_id: versioned("local", version),
        };
        let result = PublicationCoordinator::execute(
            self.store.as_ref(),
            &mut destination,
            &intent,
            version,
            self.clock.now(),
        )
        .map_err(|_| CommandError::LocalOutput(LocalOutputError::Unavailable))?;
        let local_status = match result {
            PublicationResult::Verified(_) => DestinationStatus::Verified,
            PublicationResult::Conflict => DestinationStatus::Conflict,
            PublicationResult::DefiniteNoEffect => DestinationStatus::Pending,
            _ => return Ok(workflow),
        };
        let mut overrides = vec![("local", local_status)];
        if local_status == DestinationStatus::Verified
            && self.mirror_output.is_some()
            && progress_status(&workflow, MIRROR_DESTINATION) == DestinationStatus::Pending
        {
            let owned = Self::owned_mirror_digest(&self.receipts_of(day)?, &seed);
            overrides.push((
                MIRROR_DESTINATION,
                self.execute_amend_mirror(&plan, day, &payload, owned.as_ref(), version),
            ));
        }
        let mut notion_status = progress_status(&workflow, "notion");
        if local_status == DestinationStatus::Verified
            && notion_status == DestinationStatus::Pending
        {
            match self.run_amendment_notion(&workflow, &seed, &plan, version, cancelled)? {
                Some(status) => notion_status = status,
                None => return Ok(workflow),
            }
        }
        overrides.push(("notion", notion_status));
        self.finish_amendment(workflow, &overrides)
    }

    /// Retries Notion alone after the local file was replaced. The existing
    /// approval is reused only while the plan is unchanged.
    pub fn retry_amendment_notion(
        &self,
        day: &DiaryDay,
        expected_revision: u64,
        cancelled: &AtomicBool,
    ) -> Result<Workflow, CommandError> {
        self.ensure_clear(day)?;
        let mut workflow = self.load_workflow(day)?;
        ensure_revision(&workflow, expected_revision)?;
        let seed = ensure_amending(&workflow)?.clone();
        let plan = workflow
            .output_plan()
            .ok_or(CommandError::Workflow(WorkflowError::MissingPlan))?
            .clone();
        let target = plan
            .targets
            .iter()
            .find(|target| target.destination == "notion")
            .ok_or(CommandError::Workflow(WorkflowError::MissingPlan))?;
        let receipts = self.receipts_of(day)?;
        let request = self
            .notion_replace_request(&workflow, &seed, &receipts, target.payload_digest.clone())
            .ok_or(CommandError::PreconditionFailed)?;
        let fresh = self
            .notion_output
            .as_ref()
            .ok_or(CommandError::NotionUnavailable)?
            .preflight_replace(&request)
            .map_err(|reason| notion_preflight_error(&reason))?;
        // After a local success the owned section is unchanged until our own
        // replacement lands, so the bound state digest must still match.
        if let Some(reason) = notion_binding_refusal(&fresh.preflight, target, false) {
            return Err(CommandError::NotionBindingChanged(reason));
        }
        workflow
            .begin_pending_retry(
                expected_revision,
                "notion",
                &plan.plan_digest,
                self.clock.now(),
            )
            .map_err(CommandError::Workflow)?;
        self.store
            .compare_and_swap(expected_revision, None, &workflow)
            .map_err(CommandError::Store)?;
        let version = seed.receipt_version + 1;
        let Some(status) =
            self.run_amendment_notion(&workflow, &seed, &plan, version, cancelled)?
        else {
            return Ok(workflow);
        };
        self.finish_amendment(
            workflow,
            &[("local", DestinationStatus::Verified), ("notion", status)],
        )
    }

    /// `None` means the effect is uncertain and the day stays gated.
    pub(super) fn run_amendment_notion(
        &self,
        workflow: &Workflow,
        seed: &AmendmentSeed,
        plan: &OutputPlan,
        version: u32,
        cancelled: &AtomicBool,
    ) -> Result<Option<DestinationStatus>, CommandError> {
        let notion = self
            .notion_output
            .as_ref()
            .ok_or(CommandError::NotionUnavailable)?;
        let target = plan
            .targets
            .iter()
            .find(|target| target.destination == "notion")
            .ok_or(CommandError::Workflow(WorkflowError::MissingPlan))?;
        let receipts = self.receipts_of(workflow.day())?;
        let request = self
            .notion_replace_request(workflow, seed, &receipts, target.payload_digest.clone())
            .ok_or(CommandError::PreconditionFailed)?;
        let intent = OperationIntent::new(
            random_id("attempt_").map_err(|_| CommandError::NotionUnavailable)?,
            plan.plan_id.clone(),
            workflow.day().clone(),
            target,
            plan.entry_digest.clone(),
            plan.selection_digest.clone(),
            self.clock.now(),
        )
        .map_err(|_| CommandError::NotionUnavailable)?;
        let mut destination = AmendNotionDestination {
            output: notion.as_ref(),
            request: &request,
            cancelled,
            version,
        };
        let result = PublicationCoordinator::execute(
            self.store.as_ref(),
            &mut destination,
            &intent,
            version,
            self.clock.now(),
        )
        .map_err(|_| CommandError::NotionUnavailable)?;
        Ok(match result {
            PublicationResult::Verified(_) => Some(DestinationStatus::Verified),
            PublicationResult::Conflict => Some(DestinationStatus::Conflict),
            PublicationResult::DefiniteNoEffect | PublicationResult::SafeToRetry => {
                Some(DestinationStatus::Pending)
            }
            _ => None,
        })
    }
}
