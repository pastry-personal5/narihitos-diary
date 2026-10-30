//! Amendment preview use cases.

use super::*;

impl CommandService {
    pub(super) fn notion_replace_request(
        &self,
        workflow: &Workflow,
        seed: &AmendmentSeed,
        receipts: &[Receipt],
        expected_payload_digest: Digest,
    ) -> Option<NotionReplaceRequest> {
        let owned = Self::owned_receipt(receipts, "notion", seed.receipt_version)?;
        Some(NotionReplaceRequest {
            day: workflow.day().clone(),
            markdown: workflow.entry().text.clone(),
            expected_target_identity: owned.target_identity.clone(),
            expected_payload_digest,
            owned_month_id: base_receipt_id(&owned.receipt_id).to_owned(),
            owned_block_ids: owned.returned_ids.clone(),
            owned_payload_digest: owned.payload_digest.clone(),
        })
    }

    /// Offers an amendment of a `Completed` day: seeds the editor with the
    /// local file body only and records the seed-time file digest. No source is
    /// read and no freshness check runs.
    pub fn seed_amendment(
        &self,
        day: &DiaryDay,
        expected_revision: u64,
    ) -> Result<AmendmentSeeded, CommandError> {
        self.ensure_clear(day)?;
        let output = self
            .local_output
            .as_ref()
            .ok_or(CommandError::LocalOutput(LocalOutputError::Unavailable))?;
        let mut workflow = self.load_workflow(day)?;
        ensure_revision(&workflow, expected_revision)?;
        if workflow.state() != WorkflowState::Completed {
            return Err(CommandError::Workflow(WorkflowError::WrongState));
        }
        let text = match output.inspect(day, "").map_err(CommandError::LocalOutput)? {
            LocalOutputState::Different(text) => text,
            LocalOutputState::Identical => String::new(),
            LocalOutputState::Absent => return Err(CommandError::PreconditionFailed),
        };
        let receipts = self.receipts_of(day)?;
        let local = Self::owned_receipt(&receipts, "local", u32::MAX)
            .ok_or(CommandError::PreconditionFailed)?;
        let file_digest = Digest::of_bytes(text.as_bytes());
        let hand_edited = file_digest != local.payload_digest || !text.starts_with("---\n");
        let seed = AmendmentSeed {
            file_digest,
            receipt_version: receipts
                .iter()
                .map(|receipt| receipt.version)
                .max()
                .unwrap_or(1),
            hand_edited,
        };
        workflow
            .seed_amendment(expected_revision, local_body(&text), seed, self.clock.now())
            .map_err(CommandError::Workflow)?;
        self.store
            .compare_and_swap(expected_revision, None, &workflow)
            .map_err(CommandError::Store)?;
        Ok(AmendmentSeeded {
            workflow,
            hand_edited,
        })
    }

    /// Conflict resolution "reread": rebinds the amendment to the file as it
    /// is now, keeping the person's text, and requires reconfirmation.
    pub fn reread_amendment(
        &self,
        day: &DiaryDay,
        expected_revision: u64,
    ) -> Result<AmendmentSeeded, CommandError> {
        self.ensure_clear(day)?;
        let output = self
            .local_output
            .as_ref()
            .ok_or(CommandError::LocalOutput(LocalOutputError::Unavailable))?;
        let mut workflow = self.load_workflow(day)?;
        ensure_revision(&workflow, expected_revision)?;
        let seed = ensure_amending(&workflow)?.clone();
        if local_verified(&workflow) {
            // The local file already holds the new text; only the Notion
            // section can have changed. Preview it again against its receipt.
            let preview = self.preview_amendment_notion(day, expected_revision, workflow)?;
            return Ok(AmendmentSeeded {
                workflow: preview.workflow,
                hand_edited: seed.hand_edited,
            });
        }
        let text = match output.inspect(day, "").map_err(CommandError::LocalOutput)? {
            LocalOutputState::Different(text) => text,
            LocalOutputState::Identical => String::new(),
            LocalOutputState::Absent => return Err(CommandError::PreconditionFailed),
        };
        let receipts = self.receipts_of(day)?;
        let local = Self::owned_receipt(&receipts, "local", u32::MAX)
            .ok_or(CommandError::PreconditionFailed)?;
        let file_digest = Digest::of_bytes(text.as_bytes());
        let rebound = AmendmentSeed {
            hand_edited: file_digest != local.payload_digest,
            file_digest,
            receipt_version: seed.receipt_version,
        };
        let hand_edited = rebound.hand_edited;
        workflow
            .rebind_amendment(expected_revision, rebound, self.clock.now())
            .map_err(CommandError::Workflow)?;
        self.store
            .compare_and_swap(expected_revision, None, &workflow)
            .map_err(CommandError::Store)?;
        Ok(AmendmentSeeded {
            workflow,
            hand_edited,
        })
    }

    /// Conflict resolution "abandon": back to `Completed`; receipts and files
    /// are untouched.
    pub fn abandon_amendment(
        &self,
        day: &DiaryDay,
        expected_revision: u64,
    ) -> Result<Workflow, CommandError> {
        self.ensure_clear(day)?;
        let mut workflow = self.load_workflow(day)?;
        ensure_revision(&workflow, expected_revision)?;
        workflow
            .abandon_amendment(expected_revision, self.clock.now())
            .map_err(CommandError::Workflow)?;
        self.store
            .compare_and_swap(expected_revision, None, &workflow)
            .map_err(CommandError::Store)?;
        Ok(workflow)
    }

    /// Previews current against replacement content per destination. Approval
    /// binds the seed-time file digest and the verified Notion section digest.
    pub fn preview_amendment(
        &self,
        day: &DiaryDay,
        expected_revision: u64,
    ) -> Result<AmendmentPreview, CommandError> {
        self.ensure_clear(day)?;
        let output = self
            .local_output
            .as_ref()
            .ok_or(CommandError::LocalOutput(LocalOutputError::Unavailable))?;
        let mut workflow = self.load_workflow(day)?;
        ensure_revision(&workflow, expected_revision)?;
        let seed = ensure_amending(&workflow)?.clone();
        if local_verified(&workflow) {
            return self.preview_amendment_notion(day, expected_revision, workflow);
        }
        let language = workflow
            .language()
            .ok_or(CommandError::Workflow(WorkflowError::LanguageRequired))?;
        let payload = render_local_markdown(day, language, &workflow.entry().text);
        let payload_digest = Digest::of_bytes(payload.as_bytes());
        let (current, observed, local_status) =
            match output.inspect(day, "").map_err(CommandError::LocalOutput)? {
                LocalOutputState::Different(text) => {
                    let digest = Digest::of_bytes(text.as_bytes());
                    let status = if digest == seed.file_digest {
                        DestinationStatus::Pending
                    } else {
                        DestinationStatus::Conflict
                    };
                    (text, digest, status)
                }
                LocalOutputState::Identical => {
                    let digest = Digest::of_bytes(b"");
                    let status = if digest == seed.file_digest {
                        DestinationStatus::Pending
                    } else {
                        DestinationStatus::Conflict
                    };
                    (String::new(), digest, status)
                }
                LocalOutputState::Absent => (
                    String::new(),
                    Digest::of_bytes(b"absent"),
                    DestinationStatus::Conflict,
                ),
            };
        let target = output.target(day);
        let local_plan = DestinationPlan::new(
            DestinationKind::Local,
            "local",
            target.clone(),
            payload_digest.clone(),
            payload_digest,
            observed,
        )
        .map_err(|_| CommandError::LocalOutput(LocalOutputError::Unavailable))?;

        let receipts = self.receipts_of(day)?;
        let mut targets = vec![local_plan];
        let mut mirror_progress = None;
        if let Some(mirror) = &self.mirror_output {
            let owned = Self::owned_mirror_digest(&receipts, &seed);
            let (observed, status) =
                Self::classify_mirror(mirror.as_ref(), day, &payload, owned.as_ref());
            targets.push(
                DestinationPlan::new(
                    DestinationKind::Mirror,
                    MIRROR_DESTINATION,
                    mirror.target(day),
                    Digest::of_bytes(payload.as_bytes()),
                    Digest::of_bytes(payload.as_bytes()),
                    observed,
                )
                .map_err(|_| CommandError::LocalOutput(LocalOutputError::Unavailable))?,
            );
            mirror_progress = Some(DestinationProgress {
                destination: MIRROR_DESTINATION.into(),
                status,
            });
        }
        let (notion_plan, notion_progress, notion_preview) =
            self.notion_amendment_target(&workflow, &seed, &receipts);
        targets.push(notion_plan);
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
            targets,
            self.clock.now(),
        )
        .map_err(|_| CommandError::LocalOutput(LocalOutputError::Unavailable))?;
        let mut progress = vec![DestinationProgress {
            destination: "local".into(),
            status: local_status,
        }];
        progress.extend(mirror_progress);
        progress.push(DestinationProgress {
            destination: "notion".into(),
            status: notion_progress,
        });
        workflow
            .preview_output_with_progress(expected_revision, plan, progress, self.clock.now())
            .map_err(CommandError::Workflow)?;
        self.store
            .compare_and_swap(expected_revision, None, &workflow)
            .map_err(CommandError::Store)?;
        Ok(AmendmentPreview {
            workflow,
            local_target: target,
            local_current: current,
            local_payload: payload,
            local_status,
            notion: notion_preview,
        })
    }

    /// Previews the Notion section again after the local file was replaced. A
    /// section that now matches its receipt gets a fresh plan needing a new
    /// approval; one that still does not is reported unchanged.
    pub(super) fn preview_amendment_notion(
        &self,
        day: &DiaryDay,
        expected_revision: u64,
        mut workflow: Workflow,
    ) -> Result<AmendmentPreview, CommandError> {
        let output = self
            .local_output
            .as_ref()
            .ok_or(CommandError::LocalOutput(LocalOutputError::Unavailable))?;
        let seed = ensure_amending(&workflow)?.clone();
        let language = workflow
            .language()
            .ok_or(CommandError::Workflow(WorkflowError::LanguageRequired))?;
        let payload = render_local_markdown(day, language, &workflow.entry().text);
        let receipts = self.receipts_of(day)?;
        let (plan, status, preview) = self.notion_amendment_target(&workflow, &seed, &receipts);
        if status == DestinationStatus::Pending {
            workflow
                .preview_blocked_retry(
                    expected_revision,
                    "notion",
                    plan,
                    random_id("plan_").map_err(|_| CommandError::NotionUnavailable)?,
                    self.clock.now(),
                )
                .map_err(CommandError::Workflow)?;
            self.store
                .compare_and_swap(expected_revision, None, &workflow)
                .map_err(CommandError::Store)?;
        }
        let current = match output.inspect(day, "").map_err(CommandError::LocalOutput)? {
            LocalOutputState::Different(text) => text,
            _ => String::new(),
        };
        Ok(AmendmentPreview {
            workflow,
            local_target: output.target(day),
            local_current: current,
            local_payload: payload,
            local_status: DestinationStatus::Verified,
            notion: preview,
        })
    }

    pub(super) fn notion_amendment_target(
        &self,
        workflow: &Workflow,
        seed: &AmendmentSeed,
        receipts: &[Receipt],
    ) -> (
        DestinationPlan,
        DestinationStatus,
        Option<AmendmentNotionPreview>,
    ) {
        let blocked = |reason: &str| {
            (
                DestinationPlan::new(
                    DestinationKind::Remote,
                    "notion",
                    "unavailable",
                    Digest::of_bytes(b"blocked"),
                    Digest::of_bytes(b"blocked"),
                    Digest::of_bytes(b"uninspected"),
                )
                .expect("static blocked plan is valid"),
                DestinationStatus::Blocked,
                Some(AmendmentNotionPreview {
                    current_blocks: Vec::new(),
                    replacement_blocks: Vec::new(),
                    status: DestinationStatus::Blocked,
                    reason: Some(reason.to_owned()),
                }),
            )
        };
        let (Some(notion), Some(request)) = (
            self.notion_output.as_ref(),
            // Preflight ignores the expected digest and reports the real one.
            self.notion_replace_request(workflow, seed, receipts, Digest::of_bytes(b"preflight")),
        ) else {
            return blocked("notion-not-published");
        };
        let preflight: NotionReplacePreflight = match notion.preflight_replace(&request) {
            Ok(preflight) => preflight,
            Err(reason) => return blocked(&reason),
        };
        let status = match preflight.preflight.status {
            NotionPreflightStatus::Ready => DestinationStatus::Pending,
            NotionPreflightStatus::Blocked => DestinationStatus::Blocked,
            NotionPreflightStatus::Conflict => DestinationStatus::Conflict,
        };
        let Ok(plan) = DestinationPlan::new(
            DestinationKind::Remote,
            "notion",
            preflight.preflight.target_identity.clone(),
            preflight.preflight.payload_digest.clone(),
            preflight.preflight.payload_digest.clone(),
            preflight.preflight.observed_state_digest.clone(),
        ) else {
            return blocked("notion-unavailable");
        };
        (
            plan,
            status,
            Some(AmendmentNotionPreview {
                current_blocks: preflight.current_blocks,
                replacement_blocks: preflight.preflight.blocks,
                status,
                reason: preflight.preflight.reason,
            }),
        )
    }
}
