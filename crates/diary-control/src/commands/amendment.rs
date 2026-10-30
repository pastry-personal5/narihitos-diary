//! Amendment of a `Completed` day: local-first, digest-guarded replacement of
//! only the content the app itself wrote and receipted.

use diary_core::{
    AmendmentSeed, NotionReplacePreflight, NotionReplaceRequest, Receipt, WorkflowState,
};

use super::*;

/// What the person sees before approving a replacement.
#[derive(Clone, Debug)]
pub struct AmendmentPreview {
    pub workflow: Workflow,
    pub local_target: String,
    pub local_current: String,
    pub local_payload: String,
    pub local_status: DestinationStatus,
    pub notion: Option<AmendmentNotionPreview>,
}

#[derive(Clone, Debug)]
pub struct AmendmentNotionPreview {
    pub current_blocks: Vec<NotionBlockPreview>,
    pub replacement_blocks: Vec<NotionBlockPreview>,
    pub status: DestinationStatus,
    pub reason: Option<String>,
}

/// The editor seed: the file body without its re-rendered front matter.
#[derive(Clone, Debug)]
pub struct AmendmentSeeded {
    pub workflow: Workflow,
    pub hand_edited: bool,
}

/// Splits the app's front matter off a local file. A file without it is
/// returned whole; the caller flags such a file as hand-edited anyway.
#[must_use]
pub fn local_body(text: &str) -> String {
    let body = text
        .strip_prefix("---\n")
        .and_then(|rest| rest.split_once("\n---\n"))
        .map_or(text, |(_, body)| body);
    body.trim_start_matches('\n')
        .trim_end_matches('\n')
        .to_owned()
}

/// Receipt IDs are primary keys, and the original Notion receipt's ID is the
/// month page ID, so each amendment receipt carries a version suffix.
fn versioned(receipt_id: &str, version: u32) -> String {
    format!("{}#v{version}", base_receipt_id(receipt_id))
}

fn base_receipt_id(receipt_id: &str) -> &str {
    receipt_id.split('#').next().unwrap_or(receipt_id)
}

fn ensure_amending(workflow: &Workflow) -> Result<&AmendmentSeed, CommandError> {
    workflow
        .amendment()
        .ok_or(CommandError::Workflow(WorkflowError::WrongState))
}

struct AmendLocalDestination<'a> {
    output: &'a dyn LocalOutputPort,
    day: &'a DiaryDay,
    payload: &'a str,
    /// Digest of the bytes the app itself wrote and may replace; `None` when
    /// no receipt exists, so nothing present is ever replaceable.
    seed_digest: Option<&'a Digest>,
    receipt_id: String,
}

impl Destination for AmendLocalDestination<'_> {
    fn write(&mut self, _: &OperationIntent) -> WriteOutcome {
        let Some(seed_digest) = self.seed_digest else {
            return WriteOutcome::Conflict;
        };
        match self.output.replace(self.day, seed_digest, self.payload) {
            // The file already holds exactly the approved bytes (an unchanged
            // amendment or a replayed attempt), which is a verified result.
            Ok(
                diary_core::LocalReplaceOutcome::Replaced
                | diary_core::LocalReplaceOutcome::AlreadyReplaced,
            ) => WriteOutcome::Verified {
                receipt_id: self.receipt_id.clone(),
                returned_ids: Vec::new(),
            },
            Ok(diary_core::LocalReplaceOutcome::Changed) => WriteOutcome::Conflict,
            Err(_) => WriteOutcome::Unavailable,
        }
    }

    fn inspect(&mut self, _: &OperationIntent) -> InspectionOutcome {
        match self.output.inspect(self.day, self.payload) {
            Ok(LocalOutputState::Identical) => InspectionOutcome::Matching {
                receipt_id: self.receipt_id.clone(),
                returned_ids: Vec::new(),
            },
            // The file is still exactly what the amendment was seeded from, so
            // the replacement never happened and may be tried again.
            Ok(LocalOutputState::Different(text))
                if Some(&Digest::of_bytes(text.as_bytes())) == self.seed_digest =>
            {
                InspectionOutcome::Absent
            }
            Ok(_) => InspectionOutcome::Conflict,
            Err(_) => InspectionOutcome::Unavailable,
        }
    }
}

struct AmendNotionDestination<'a> {
    output: &'a dyn NotionOutputPort,
    request: &'a NotionReplaceRequest,
    cancelled: &'a AtomicBool,
    version: u32,
}

impl Destination for AmendNotionDestination<'_> {
    fn write(&mut self, _: &OperationIntent) -> WriteOutcome {
        match self.output.replace_section(self.request, self.cancelled) {
            WriteOutcome::Verified {
                receipt_id,
                returned_ids,
            } => WriteOutcome::Verified {
                receipt_id: versioned(&receipt_id, self.version),
                returned_ids,
            },
            other => other,
        }
    }

    fn inspect(&mut self, _: &OperationIntent) -> InspectionOutcome {
        let versioned = |outcome: InspectionOutcome| match outcome {
            InspectionOutcome::Matching {
                receipt_id,
                returned_ids,
            } => InspectionOutcome::Matching {
                receipt_id: versioned(&receipt_id, self.version),
                returned_ids,
            },
            other => other,
        };
        match versioned(self.output.reconcile_replace(self.request)) {
            // A partial insert is resumed, never replayed: the adapter reads
            // the section back before each step.
            InspectionOutcome::Unavailable => {
                match self.output.replace_section(self.request, self.cancelled) {
                    WriteOutcome::Verified {
                        receipt_id,
                        returned_ids,
                    } => versioned(InspectionOutcome::Matching {
                        receipt_id,
                        returned_ids,
                    }),
                    WriteOutcome::Conflict => InspectionOutcome::Conflict,
                    _ => InspectionOutcome::Unavailable,
                }
            }
            other => other,
        }
    }
}

impl CommandService {
    fn receipts_of(&self, day: &DiaryDay) -> Result<Vec<Receipt>, CommandError> {
        self.store.receipts(day).map_err(CommandError::Store)
    }

    /// Latest receipt of `destination` made at or before `version`.
    fn owned_receipt(receipts: &[Receipt], destination: &str, version: u32) -> Option<Receipt> {
        receipts
            .iter()
            .filter(|receipt| receipt.destination == destination && receipt.version <= version)
            .max_by_key(|receipt| receipt.version)
            .cloned()
    }

    /// Digest of the mirror bytes the app itself wrote, from the latest
    /// `icloud` receipt the amendment was seeded from.
    fn owned_mirror_digest(receipts: &[Receipt], seed: &AmendmentSeed) -> Option<Digest> {
        Self::owned_receipt(receipts, MIRROR_DESTINATION, seed.receipt_version)
            .map(|receipt| receipt.payload_digest)
    }

    /// Classifies the mirror file against the replacement: an absent or
    /// already-identical file and the app's own receipted bytes are
    /// `Pending`; any other content is a `Conflict` and is never replaced.
    fn classify_mirror(
        mirror: &dyn LocalOutputPort,
        day: &DiaryDay,
        payload: &str,
        owned: Option<&Digest>,
    ) -> (Digest, DestinationStatus) {
        match mirror.inspect(day, payload) {
            Ok(LocalOutputState::Absent) => {
                (Digest::of_bytes(b"absent"), DestinationStatus::Pending)
            }
            Ok(LocalOutputState::Identical) => (
                Digest::of_bytes(payload.as_bytes()),
                DestinationStatus::Pending,
            ),
            Ok(LocalOutputState::Different(text)) => {
                let digest = Digest::of_bytes(text.as_bytes());
                let status = if Some(&digest) == owned {
                    DestinationStatus::Pending
                } else {
                    DestinationStatus::Conflict
                };
                (digest, status)
            }
            Err(_) => (
                Digest::of_bytes(b"unverifiable"),
                DestinationStatus::Blocked,
            ),
        }
    }

    /// Writes or replaces the mirror after the verified local replacement.
    /// Every failure is contained as a non-verified status, never an error.
    fn execute_amend_mirror(
        &self,
        plan: &OutputPlan,
        day: &DiaryDay,
        payload: &str,
        owned: Option<&Digest>,
        version: u32,
    ) -> DestinationStatus {
        let (Some(mirror), Some(target)) = (
            &self.mirror_output,
            plan.targets
                .iter()
                .find(|target| target.kind == DestinationKind::Mirror),
        ) else {
            return DestinationStatus::Blocked;
        };
        let replacing = match mirror.inspect(day, payload) {
            Err(_) => return DestinationStatus::Blocked,
            Ok(LocalOutputState::Different(text)) => {
                if Some(&Digest::of_bytes(text.as_bytes())) != owned {
                    return DestinationStatus::Conflict;
                }
                true
            }
            Ok(_) => false,
        };
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
        let result = if replacing {
            let mut destination = AmendLocalDestination {
                output: mirror.as_ref(),
                day,
                payload,
                seed_digest: owned,
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
            let mut destination = LocalDestination {
                output: mirror.as_ref(),
                day,
                payload,
                receipt_id,
            };
            PublicationCoordinator::execute(
                self.store.as_ref(),
                &mut destination,
                &intent,
                version,
                self.clock.now(),
            )
        };
        match result {
            Ok(PublicationResult::Verified(_)) => DestinationStatus::Verified,
            Ok(PublicationResult::Conflict) => DestinationStatus::Conflict,
            // An uncertain write keeps its intent for startup reconciliation.
            _ => DestinationStatus::Pending,
        }
    }

    /// Retries only the mirror of a `Partial` amendment whose local file
    /// already holds the new text.
    pub fn publish_amendment_mirror(
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
        if workflow.state() != WorkflowState::Partial {
            return Err(CommandError::Workflow(WorkflowError::WrongState));
        }
        let seed = ensure_amending(&workflow)?.clone();
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
        let owned = Self::owned_mirror_digest(&self.receipts_of(day)?, &seed);
        let status = self.execute_amend_mirror(
            &plan,
            day,
            &payload,
            owned.as_ref(),
            seed.receipt_version + 1,
        );
        workflow
            .finish_mirror_retry(expected_revision, status, self.clock.now())
            .map_err(CommandError::Workflow)?;
        self.store
            .compare_and_swap(expected_revision, None, &workflow)
            .map_err(CommandError::Store)?;
        Ok(workflow)
    }

    fn notion_replace_request(
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
    fn preview_amendment_notion(
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

    fn notion_amendment_target(
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
                .map_err(|_| CommandError::NotionUnavailable)?;
            if fresh.preflight.status != NotionPreflightStatus::Ready
                || fresh.preflight.observed_state_digest != target.inspected_state_digest
                || fresh.preflight.payload_digest != target.payload_digest
            {
                return Err(CommandError::PreconditionFailed);
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
                .map_err(|_| CommandError::NotionUnavailable)?;
            if fresh.preflight.status != NotionPreflightStatus::Ready
                || fresh.preflight.observed_state_digest != notion.inspected_state_digest
                || fresh.preflight.payload_digest != notion.payload_digest
            {
                return Err(CommandError::PreconditionFailed);
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
            .map_err(|_| CommandError::NotionUnavailable)?;
        // After a local success the owned section is unchanged until our own
        // replacement lands, so the bound state digest must still match.
        if fresh.preflight.status != NotionPreflightStatus::Ready
            || fresh.preflight.payload_digest != target.payload_digest
            || fresh.preflight.observed_state_digest != target.inspected_state_digest
        {
            return Err(CommandError::PreconditionFailed);
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
    fn run_amendment_notion(
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

    fn finish_amendment(
        &self,
        mut workflow: Workflow,
        overrides: &[(&str, DestinationStatus)],
    ) -> Result<Workflow, CommandError> {
        let revision = workflow.revision();
        let progress = progress_with(&workflow, overrides);
        workflow
            .finish_publication(revision, progress, self.clock.now())
            .map_err(CommandError::Workflow)?;
        self.store
            .compare_and_swap(revision, None, &workflow)
            .map_err(CommandError::Store)?;
        Ok(workflow)
    }
}

/// Which destination an unresolved amendment intent belonged to.
#[derive(Clone, Copy)]
enum Reconciled {
    Local,
    Mirror,
    Notion,
}

fn local_verified(workflow: &Workflow) -> bool {
    progress_status(workflow, "local") == DestinationStatus::Verified
}

fn progress_status(workflow: &Workflow, destination: &str) -> DestinationStatus {
    workflow
        .destination_progress()
        .iter()
        .find(|item| item.destination == destination)
        .map_or(DestinationStatus::Blocked, |item| item.status)
}
