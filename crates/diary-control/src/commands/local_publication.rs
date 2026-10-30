//! Local publication use cases.

use super::*;

impl CommandService {
    pub fn preview_local(
        &self,
        day: &DiaryDay,
        expected_revision: u64,
    ) -> Result<LocalPreview, CommandError> {
        self.ensure_clear(day)?;
        let output = self
            .local_output
            .as_ref()
            .ok_or(CommandError::LocalOutput(LocalOutputError::Unavailable))?;
        let mut workflow = self.load_workflow(day)?;
        ensure_revision(&workflow, expected_revision)?;
        let prior_revision = workflow.revision();
        let freshness = revalidate_workflow_sources(
            &mut workflow,
            self.reader.as_ref(),
            FreshnessCheckpoint::BeforePreview,
            self.clock.now(),
        );
        if freshness.is_err() || workflow.revision() != prior_revision {
            if workflow.revision() != prior_revision {
                self.store
                    .compare_and_swap(prior_revision, None, &workflow)
                    .map_err(CommandError::Store)?;
            }
            return Err(CommandError::SourcesInvalidated);
        }
        let language = workflow
            .language()
            .ok_or(CommandError::Workflow(WorkflowError::LanguageRequired))?;
        let payload = render_local_markdown(day, language, &workflow.entry().text);
        let payload_digest = Digest::of_bytes(payload.as_bytes());
        let state = output.inspect(day, &payload);
        let (observed, status, existing_text) = match state {
            Ok(LocalOutputState::Absent) => (
                Digest::of_bytes(b"absent"),
                DestinationStatus::Pending,
                None,
            ),
            Ok(LocalOutputState::Identical) => {
                (payload_digest.clone(), DestinationStatus::Pending, None)
            }
            Ok(LocalOutputState::Different(text)) => (
                Digest::of_bytes(text.as_bytes()),
                DestinationStatus::Conflict,
                Some(text),
            ),
            Err(_) => (
                Digest::of_bytes(b"unverifiable"),
                DestinationStatus::Blocked,
                None,
            ),
        };
        let target = output.target(day);
        let intended_changes = if status == DestinationStatus::Pending {
            if existing_text.is_none() && observed == Digest::of_bytes(b"absent") {
                vec![
                    format!("Create parent directories as needed for {target}"),
                    format!("Create {target} without replacing an existing file"),
                ]
            } else {
                vec![format!("Verify identical existing file at {target}")]
            }
        } else {
            Vec::new()
        };
        let local = DestinationPlan::new(
            DestinationKind::Local,
            "local",
            target.clone(),
            payload_digest.clone(),
            payload_digest.clone(),
            observed,
        )
        .map_err(|_| CommandError::LocalOutput(LocalOutputError::Unavailable))?;
        let mut targets = vec![local];
        let mut mirror_target = None;
        let mut mirror_status = None;
        if let Some(mirror) = &self.mirror_output {
            let (observed, status) = match mirror.inspect(day, &payload) {
                Ok(LocalOutputState::Absent) => {
                    (Digest::of_bytes(b"absent"), DestinationStatus::Pending)
                }
                Ok(LocalOutputState::Identical) => {
                    (payload_digest.clone(), DestinationStatus::Pending)
                }
                Ok(LocalOutputState::Different(text)) => (
                    Digest::of_bytes(text.as_bytes()),
                    DestinationStatus::Conflict,
                ),
                Err(_) => (
                    Digest::of_bytes(b"unverifiable"),
                    DestinationStatus::Blocked,
                ),
            };
            let identity = mirror.target(day);
            targets.push(
                DestinationPlan::new(
                    DestinationKind::Mirror,
                    MIRROR_DESTINATION,
                    identity.clone(),
                    payload_digest.clone(),
                    payload_digest.clone(),
                    observed,
                )
                .map_err(|_| CommandError::LocalOutput(LocalOutputError::Unavailable))?,
            );
            mirror_target = Some(identity);
            mirror_status = Some(status);
        }
        let blocked = DestinationPlan::new(
            DestinationKind::Remote,
            "notion",
            "unavailable",
            Digest::of_bytes(b"blocked"),
            Digest::of_bytes(b"blocked"),
            Digest::of_bytes(b"uninspected"),
        )
        .map_err(|_| CommandError::LocalOutput(LocalOutputError::Unavailable))?;
        let plan = OutputPlan::new(
            random_id("plan_")
                .map_err(|_| CommandError::LocalOutput(LocalOutputError::Unavailable))?,
            workflow
                .entry()
                .confirmed
                .as_ref()
                .ok_or(CommandError::Workflow(WorkflowError::EntryNotConfirmed))?
                .entry_digest
                .clone(),
            workflow
                .selection_digest()
                .ok_or(CommandError::Workflow(WorkflowError::PlanBindingMismatch))?
                .clone(),
            {
                targets.push(blocked);
                targets
            },
            self.clock.now(),
        )
        .map_err(|_| CommandError::LocalOutput(LocalOutputError::Unavailable))?;
        let mut progress = vec![DestinationProgress {
            destination: "local".into(),
            status,
        }];
        if let Some(status) = mirror_status {
            progress.push(DestinationProgress {
                destination: MIRROR_DESTINATION.into(),
                status,
            });
        }
        progress.push(DestinationProgress {
            destination: "notion".into(),
            status: DestinationStatus::Blocked,
        });
        workflow
            .preview_output_with_progress(expected_revision, plan, progress, self.clock.now())
            .map_err(CommandError::Workflow)?;
        self.store
            .compare_and_swap(expected_revision, None, &workflow)
            .map_err(CommandError::Store)?;
        Ok(LocalPreview {
            workflow,
            target,
            payload,
            payload_digest,
            existing_text,
            intended_changes,
            mirror_target,
            mirror_status,
        })
    }

    pub fn approve_local(
        &self,
        day: &DiaryDay,
        expected_revision: u64,
        digest: &Digest,
    ) -> Result<Workflow, CommandError> {
        self.ensure_clear(day)?;
        let mut workflow = self.load_workflow(day)?;
        ensure_revision(&workflow, expected_revision)?;
        let prior_revision = workflow.revision();
        let freshness = revalidate_workflow_sources(
            &mut workflow,
            self.reader.as_ref(),
            FreshnessCheckpoint::BeforePreview,
            self.clock.now(),
        );
        if freshness.is_err() || workflow.revision() != prior_revision {
            if workflow.revision() != prior_revision {
                self.store
                    .compare_and_swap(prior_revision, None, &workflow)
                    .map_err(CommandError::Store)?;
            }
            return Err(CommandError::SourcesInvalidated);
        }
        let plan = workflow
            .output_plan()
            .ok_or(CommandError::Workflow(WorkflowError::MissingPlan))?;
        let target = plan
            .targets
            .iter()
            .find(|target| target.kind == DestinationKind::Local)
            .ok_or(CommandError::Workflow(WorkflowError::MissingPlan))?;
        let output = self
            .local_output
            .as_ref()
            .ok_or(CommandError::LocalOutput(LocalOutputError::Unavailable))?;
        let payload = render_local_markdown(
            day,
            workflow
                .language()
                .ok_or(CommandError::Workflow(WorkflowError::LanguageRequired))?,
            &workflow.entry().text,
        );
        if Digest::of_bytes(payload.as_bytes()) != target.payload_digest
            || output.target(day) != target.target_identity
            || observed_digest(
                output
                    .inspect(day, &payload)
                    .map_err(CommandError::LocalOutput)?,
                &payload,
            ) != target.inspected_state_digest
        {
            return Err(CommandError::PreconditionFailed);
        }
        self.ensure_mirror_binding(plan, day, &target.payload_digest)?;
        workflow
            .approve(expected_revision, digest, self.clock.now())
            .map_err(CommandError::Workflow)?;
        self.store
            .compare_and_swap(expected_revision, None, &workflow)
            .map_err(CommandError::Store)?;
        Ok(workflow)
    }

    pub fn publish_local(
        &self,
        day: &DiaryDay,
        expected_revision: u64,
    ) -> Result<Workflow, CommandError> {
        self.publish_local_guarded(day, expected_revision, || true)
    }

    pub fn publish_local_guarded(
        &self,
        day: &DiaryDay,
        expected_revision: u64,
        commit: impl FnOnce() -> bool,
    ) -> Result<Workflow, CommandError> {
        self.ensure_clear(day)?;
        let output = self
            .local_output
            .as_ref()
            .ok_or(CommandError::LocalOutput(LocalOutputError::Unavailable))?;
        let mut workflow = self.load_workflow(day)?;
        ensure_revision(&workflow, expected_revision)?;
        let plan = workflow
            .output_plan()
            .ok_or(CommandError::Workflow(WorkflowError::MissingPlan))?
            .clone();
        let target = plan
            .targets
            .iter()
            .find(|target| target.kind == DestinationKind::Local)
            .ok_or(CommandError::Workflow(WorkflowError::MissingPlan))?;
        let payload = render_local_markdown(
            day,
            workflow
                .language()
                .ok_or(CommandError::Workflow(WorkflowError::LanguageRequired))?,
            &workflow.entry().text,
        );
        if Digest::of_bytes(payload.as_bytes()) != target.payload_digest
            || output.target(day) != target.target_identity
            || observed_digest(
                output
                    .inspect(day, &payload)
                    .map_err(CommandError::LocalOutput)?,
                &payload,
            ) != target.inspected_state_digest
        {
            return Err(CommandError::PreconditionFailed);
        }
        self.ensure_mirror_binding(&plan, day, &target.payload_digest)?;
        let prior = workflow.revision();
        let fresh = revalidate_workflow_sources(
            &mut workflow,
            self.reader.as_ref(),
            FreshnessCheckpoint::BeforeFirstWrite,
            self.clock.now(),
        );
        if fresh.is_err() || workflow.revision() != prior {
            if workflow.revision() != prior {
                self.store
                    .compare_and_swap(prior, None, &workflow)
                    .map_err(CommandError::Store)?;
            }
            return Err(CommandError::SourcesInvalidated);
        }
        if !commit() {
            return Err(CommandError::Cancelled);
        }
        if workflow.state() == diary_core::WorkflowState::Partial {
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
            target,
            plan.entry_digest.clone(),
            plan.selection_digest.clone(),
            self.clock.now(),
        )
        .map_err(|_| CommandError::LocalOutput(LocalOutputError::Unavailable))?;
        let mut destination = LocalDestination {
            output: output.as_ref(),
            day,
            payload: &payload,
            receipt_id: "local".into(),
        };
        let result = PublicationCoordinator::execute(
            self.store.as_ref(),
            &mut destination,
            &intent,
            1,
            self.clock.now(),
        )
        .map_err(|_| CommandError::LocalOutput(LocalOutputError::Unavailable))?;
        let status = match result {
            PublicationResult::Verified(_) => DestinationStatus::Verified,
            PublicationResult::Conflict => DestinationStatus::Conflict,
            _ => DestinationStatus::Reconciling,
        };
        if status == DestinationStatus::Reconciling {
            return Ok(workflow);
        }
        let mut overrides = vec![("local", status), ("notion", DestinationStatus::Blocked)];
        if status == DestinationStatus::Verified && self.mirror_output.is_some() {
            overrides.push((
                MIRROR_DESTINATION,
                self.run_mirror(&workflow, &plan, day, &payload),
            ));
        }
        let progress = progress_with(&workflow, &overrides);
        workflow
            .finish_publication(workflow.revision(), progress, self.clock.now())
            .map_err(CommandError::Workflow)?;
        self.store
            .compare_and_swap(workflow.revision() - 1, None, &workflow)
            .map_err(CommandError::Store)?;
        Ok(workflow)
    }
}
