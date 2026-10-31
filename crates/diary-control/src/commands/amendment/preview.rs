//! Amendment preview use cases.

use super::*;

impl CommandService {
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

    /// Rebinds the amendment to the file as it is now, keeping the person's
    /// text, and requires reconfirmation.
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

    /// Back to `Completed`; receipts and files are untouched.
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

    /// Previews the current file against the replacement. The preview never
    /// stops at a conflict: it shows the file as it is now, and approval binds
    /// exactly that content, so anything changed since is refused at approval
    /// and previewed again. Remotes are not part of an amendment.
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
        ensure_amending(&workflow)?;
        let language = workflow
            .language()
            .ok_or(CommandError::Workflow(WorkflowError::LanguageRequired))?;
        let payload = render_local_markdown(day, language, &workflow.entry().text);
        let payload_digest = Digest::of_bytes(payload.as_bytes());
        let (current, observed) =
            match output.inspect(day, "").map_err(CommandError::LocalOutput)? {
                LocalOutputState::Different(text) => {
                    let digest = Digest::of_bytes(text.as_bytes());
                    (text, digest)
                }
                LocalOutputState::Identical => (String::new(), Digest::of_bytes(b"")),
                LocalOutputState::Absent => (String::new(), Digest::of_bytes(b"absent")),
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
            vec![local_plan, blocked],
            self.clock.now(),
        )
        .map_err(|_| CommandError::LocalOutput(LocalOutputError::Unavailable))?;
        let progress = vec![
            DestinationProgress {
                destination: "local".into(),
                status: DestinationStatus::Pending,
            },
            DestinationProgress {
                destination: "notion".into(),
                status: DestinationStatus::Blocked,
            },
        ];
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
            local_status: DestinationStatus::Pending,
        })
    }
}
