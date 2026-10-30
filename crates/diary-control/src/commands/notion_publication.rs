//! Notion publication use cases.

use super::*;

impl CommandService {
    /// Re-preflights the blocked remote after the local receipt is verified.
    pub fn preview_notion(
        &self,
        day: &DiaryDay,
        expected_revision: u64,
    ) -> Result<NotionPreview, CommandError> {
        self.ensure_clear(day)?;
        let notion = self
            .notion_output
            .as_ref()
            .ok_or(CommandError::NotionUnavailable)?;
        let mut workflow = self.load_workflow(day)?;
        ensure_revision(&workflow, expected_revision)?;
        self.verify_local_receipt(&workflow)?;
        let preflight = match notion.preflight(day, &workflow.entry().text) {
            Ok(preflight) => preflight,
            Err(reason) => {
                return Ok(NotionPreview {
                    workflow,
                    root_label: "Notion root".into(),
                    year_title: day.as_str()[..4].to_owned(),
                    month_title: day.as_str()[..7].to_owned(),
                    year_exists: false,
                    month_exists: false,
                    renderer_version: 0,
                    api_version: String::new(),
                    payload_digest: Digest::of_bytes(b"notion-unavailable"),
                    plan_digest: None,
                    blocks: Vec::new(),
                    status: DestinationStatus::Blocked,
                    reason: Some(reason),
                });
            }
        };
        let status = match preflight.status {
            NotionPreflightStatus::Ready => DestinationStatus::Pending,
            NotionPreflightStatus::Blocked => DestinationStatus::Blocked,
            NotionPreflightStatus::Conflict => DestinationStatus::Conflict,
        };
        if status != DestinationStatus::Pending {
            return Ok(NotionPreview {
                workflow,
                root_label: preflight.root_label,
                year_title: preflight.year_title,
                month_title: preflight.month_title,
                year_exists: preflight.year_exists,
                month_exists: preflight.month_exists,
                renderer_version: preflight.renderer_version,
                api_version: preflight.api_version,
                payload_digest: preflight.payload_digest,
                plan_digest: None,
                blocks: preflight.blocks,
                status,
                reason: preflight.reason,
            });
        }
        let replacement = DestinationPlan::new(
            DestinationKind::Remote,
            "notion",
            preflight.target_identity,
            preflight.payload_digest.clone(),
            preflight.payload_digest.clone(),
            preflight.observed_state_digest,
        )
        .map_err(|_| CommandError::NotionUnavailable)?;
        let plan = workflow
            .preview_blocked_retry(
                expected_revision,
                "notion",
                replacement,
                random_id("plan_").map_err(|_| CommandError::NotionUnavailable)?,
                self.clock.now(),
            )
            .map_err(CommandError::Workflow)?;
        self.store
            .compare_and_swap(expected_revision, None, &workflow)
            .map_err(CommandError::Store)?;
        Ok(NotionPreview {
            workflow,
            root_label: preflight.root_label,
            year_title: preflight.year_title,
            month_title: preflight.month_title,
            year_exists: preflight.year_exists,
            month_exists: preflight.month_exists,
            renderer_version: preflight.renderer_version,
            api_version: preflight.api_version,
            payload_digest: preflight.payload_digest,
            plan_digest: Some(plan.plan_digest),
            blocks: preflight.blocks,
            status,
            reason: None,
        })
    }

    pub fn approve_notion(
        &self,
        day: &DiaryDay,
        expected_revision: u64,
        digest: &Digest,
    ) -> Result<Workflow, CommandError> {
        self.ensure_clear(day)?;
        let notion = self
            .notion_output
            .as_ref()
            .ok_or(CommandError::NotionUnavailable)?;
        let mut workflow = self.load_workflow(day)?;
        ensure_revision(&workflow, expected_revision)?;
        self.verify_local_receipt(&workflow)?;
        let plan = workflow
            .output_plan()
            .ok_or(CommandError::Workflow(WorkflowError::MissingPlan))?;
        if &plan.plan_digest != digest {
            return Err(CommandError::PreconditionFailed);
        }
        let target = plan
            .targets
            .iter()
            .find(|target| target.destination == "notion")
            .ok_or(CommandError::Workflow(WorkflowError::MissingPlan))?;
        let preflight = notion
            .preflight(day, &workflow.entry().text)
            .map_err(|reason| notion_preflight_error(&reason))?;
        if let Some(reason) = notion_binding_refusal(&preflight, target, true) {
            return Err(CommandError::NotionBindingChanged(reason));
        }
        workflow
            .approve_blocked_retry(expected_revision, "notion", digest, self.clock.now())
            .map_err(CommandError::Workflow)?;
        self.store
            .compare_and_swap(expected_revision, None, &workflow)
            .map_err(CommandError::Store)?;
        Ok(workflow)
    }

    pub fn publish_notion(
        &self,
        day: &DiaryDay,
        expected_revision: u64,
    ) -> Result<Workflow, CommandError> {
        self.publish_notion_guarded(day, expected_revision, &AtomicBool::new(false))
    }

    pub fn publish_notion_guarded(
        &self,
        day: &DiaryDay,
        expected_revision: u64,
        cancelled: &AtomicBool,
    ) -> Result<Workflow, CommandError> {
        self.ensure_clear(day)?;
        let notion = self
            .notion_output
            .as_ref()
            .ok_or(CommandError::NotionUnavailable)?;
        let mut workflow = self.load_workflow(day)?;
        ensure_revision(&workflow, expected_revision)?;
        self.verify_local_receipt(&workflow)?;
        let plan = workflow
            .output_plan()
            .ok_or(CommandError::Workflow(WorkflowError::MissingPlan))?
            .clone();
        let target = plan
            .targets
            .iter()
            .find(|target| target.destination == "notion")
            .ok_or(CommandError::Workflow(WorkflowError::MissingPlan))?;
        let request = NotionRequest {
            day: day.clone(),
            markdown: workflow.entry().text.clone(),
            expected_target_identity: target.target_identity.clone(),
            expected_payload_digest: target.payload_digest.clone(),
            expected_state_digest: target.inspected_state_digest.clone(),
        };
        // Revalidate the approved binding immediately before the first effect.
        let preflight = notion
            .preflight(day, &workflow.entry().text)
            .map_err(|_| CommandError::NotionUnavailable)?;
        if let Some(reason) = notion_binding_refusal(&preflight, target, true) {
            return Err(CommandError::NotionBindingChanged(reason));
        }
        if workflow.state() == diary_core::WorkflowState::Partial {
            workflow
                .begin_pending_retry(
                    expected_revision,
                    "notion",
                    &plan.plan_digest,
                    self.clock.now(),
                )
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
            random_id("attempt_").map_err(|_| CommandError::NotionUnavailable)?,
            plan.plan_id,
            day.clone(),
            target,
            plan.entry_digest,
            plan.selection_digest,
            self.clock.now(),
        )
        .map_err(|_| CommandError::NotionUnavailable)?;
        let mut destination = NotionDestination {
            output: notion.as_ref(),
            request: &request,
            cancelled,
        };
        let result = PublicationCoordinator::execute(
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
            PublicationResult::DefiniteNoEffect | PublicationResult::SafeToRetry => {
                DestinationStatus::Pending
            }
            _ => DestinationStatus::Reconciling,
        };
        if status == DestinationStatus::Reconciling {
            return Ok(workflow);
        }
        let revision = workflow.revision();
        workflow
            .finish_publication(
                revision,
                progress_with(
                    &workflow,
                    &[("local", DestinationStatus::Verified), ("notion", status)],
                ),
                self.clock.now(),
            )
            .map_err(CommandError::Workflow)?;
        self.store
            .compare_and_swap(revision, None, &workflow)
            .map_err(CommandError::Store)?;
        Ok(workflow)
    }
}
