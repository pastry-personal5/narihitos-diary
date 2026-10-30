//! Amendment mirror use cases.

use super::*;

impl CommandService {
    pub(super) fn receipts_of(&self, day: &DiaryDay) -> Result<Vec<Receipt>, CommandError> {
        self.store.receipts(day).map_err(CommandError::Store)
    }

    /// Latest receipt of `destination` made at or before `version`.
    pub(super) fn owned_receipt(
        receipts: &[Receipt],
        destination: &str,
        version: u32,
    ) -> Option<Receipt> {
        receipts
            .iter()
            .filter(|receipt| receipt.destination == destination && receipt.version <= version)
            .max_by_key(|receipt| receipt.version)
            .cloned()
    }

    /// Digest of the mirror bytes the app itself wrote, from the latest
    /// `icloud` receipt the amendment was seeded from.
    pub(super) fn owned_mirror_digest(
        receipts: &[Receipt],
        seed: &AmendmentSeed,
    ) -> Option<Digest> {
        Self::owned_receipt(receipts, MIRROR_DESTINATION, seed.receipt_version)
            .map(|receipt| receipt.payload_digest)
    }

    /// Classifies the mirror file against the replacement: an absent or
    /// already-identical file and the app's own receipted bytes are
    /// `Pending`; any other content is a `Conflict` and is never replaced.
    pub(super) fn classify_mirror(
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
    pub(super) fn execute_amend_mirror(
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
}
