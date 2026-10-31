//! Amendment publication use cases.

use super::*;

impl CommandService {
    /// The replacement is approved against the file exactly as the preview
    /// showed it; a file that changed since is refused and previewed again.
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
        ensure_amending(&workflow)?;
        let plan = workflow
            .output_plan()
            .ok_or(CommandError::Workflow(WorkflowError::MissingPlan))?;
        if &plan.plan_digest != digest {
            return Err(CommandError::PreconditionFailed);
        }
        let local = plan
            .targets
            .iter()
            .find(|target| target.destination == "local")
            .ok_or(CommandError::Workflow(WorkflowError::MissingPlan))?;
        let current = match output.inspect(day, "").map_err(CommandError::LocalOutput)? {
            LocalOutputState::Different(text) => Digest::of_bytes(text.as_bytes()),
            LocalOutputState::Identical => Digest::of_bytes(b""),
            LocalOutputState::Absent => Digest::of_bytes(b"absent"),
        };
        if current != local.inspected_state_digest {
            return Err(CommandError::PreconditionFailed);
        }
        workflow
            .approve(expected_revision, digest, self.clock.now())
            .map_err(CommandError::Workflow)?;
        self.store
            .compare_and_swap(expected_revision, None, &workflow)
            .map_err(CommandError::Store)?;
        Ok(workflow)
    }

    /// Replaces the local file, guarded by the digest the approved plan was
    /// built from. Remotes are not touched; push them again afterward.
    pub fn publish_amendment(
        &self,
        day: &DiaryDay,
        expected_revision: u64,
        _cancelled: &AtomicBool,
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
        let receipt_id = versioned(&format!("local-{}", intent.attempt_id), version);
        let result = if local_target.inspected_state_digest == Digest::of_bytes(b"absent") {
            // The file is gone: write it again instead of replacing it.
            let mut destination = LocalDestination {
                output: output.as_ref(),
                day,
                payload: &payload,
                receipt_id,
            };
            PublicationCoordinator::execute(
                self.store.as_ref(),
                &mut destination,
                &intent,
                version,
                self.clock.now(),
            )
        } else {
            let mut destination = AmendLocalDestination {
                output: output.as_ref(),
                day,
                payload: &payload,
                seed_digest: Some(&local_target.inspected_state_digest),
                receipt_id,
            };
            PublicationCoordinator::execute(
                self.store.as_ref(),
                &mut destination,
                &intent,
                version,
                self.clock.now(),
            )
        }
        .map_err(|_| CommandError::LocalOutput(LocalOutputError::Unavailable))?;
        let local_status = match result {
            PublicationResult::Verified(_) => DestinationStatus::Verified,
            PublicationResult::Conflict => DestinationStatus::Changed,
            PublicationResult::DefiniteNoEffect => DestinationStatus::Pending,
            _ => return Ok(workflow),
        };
        self.finish_amendment(workflow, &[("local", local_status)])
    }
}
