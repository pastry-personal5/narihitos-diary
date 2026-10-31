//! iCloud write and recovery used by the iCloud remote.

use super::remote_publication::RemoteBinding;
use super::*;

impl CommandService {
    /// Writes the iCloud copy for `target`, replacing existing content only
    /// when the target was planned to (Overwrite) and the file still holds
    /// what the person saw. Every failure is contained as a status.
    pub(super) fn execute_mirror(
        &self,
        day: &DiaryDay,
        binding: &RemoteBinding,
        target: &DestinationPlan,
        payload: &str,
    ) -> DestinationStatus {
        let Some(mirror) = &self.mirror_output else {
            return DestinationStatus::Blocked;
        };
        let replacing = existing_entry::replacing_guard(target);
        // Read-only preflight: never record an intent for a mirror that
        // cannot be inspected, so a missing folder cannot gate anything.
        match mirror.inspect(day, payload) {
            Err(_) => return DestinationStatus::Blocked,
            Ok(LocalOutputState::Different(text)) => {
                if replacing != Some(&Digest::of_bytes(text.as_bytes())) {
                    return DestinationStatus::Conflict;
                }
            }
            Ok(_) => {}
        }
        let Ok(attempt) = random_id("attempt_") else {
            return DestinationStatus::Blocked;
        };
        let receipt_id = format!("icloud-{attempt}");
        let Ok(intent) = OperationIntent::new(
            attempt,
            binding.plan_id.clone(),
            day.clone(),
            target,
            binding.entry_digest.clone(),
            binding.selection_digest.clone(),
            self.clock.now(),
        ) else {
            return DestinationStatus::Blocked;
        };
        let mut destination = existing_entry::ReplacingDestination {
            output: mirror.as_ref(),
            day,
            payload,
            receipt_id,
            replacing,
            backup: true,
        };
        match PublicationCoordinator::execute(
            self.store.as_ref(),
            &mut destination,
            &intent,
            1,
            self.clock.now(),
        ) {
            Ok(PublicationResult::Verified(_)) => DestinationStatus::Verified,
            Ok(PublicationResult::Conflict) => DestinationStatus::Conflict,
            // An uncertain write keeps its intent; startup reconciliation
            // resolves it before any retry.
            _ => DestinationStatus::Pending,
        }
    }

    /// Recovers uncertain iCloud effects left by an interrupted push. The
    /// outcome is read back from the file itself; no workflow row changes.
    pub fn reconcile_mirror_intents(&self) -> Result<(), CommandError> {
        let Some(mirror) = &self.mirror_output else {
            return Ok(());
        };
        for intent in self
            .store
            .unresolved_intents(None)
            .map_err(CommandError::Store)?
        {
            if intent.destination != MIRROR_DESTINATION
                || mirror.target(&intent.day) != intent.target_identity
            {
                continue;
            }
            let Some(payload) = self.mirror_recovery_payload(&intent) else {
                continue;
            };
            let mut destination = LocalDestination {
                output: mirror.as_ref(),
                day: &intent.day,
                payload: &payload,
                receipt_id: format!("icloud-{}", intent.attempt_id),
            };
            PublicationCoordinator::reconcile(
                self.store.as_ref(),
                &mut destination,
                &intent,
                1,
                self.clock.now(),
            )
            .map_err(|error| match error {
                PublicationError::Store(error) => CommandError::Store(error),
                PublicationError::InvalidOutcome => {
                    CommandError::LocalOutput(LocalOutputError::Unavailable)
                }
            })?;
        }
        Ok(())
    }

    /// The bytes an uncertain push meant to write: the verified local file,
    /// or the iCloud file itself when it already holds the intended bytes
    /// (an Append result is not derivable from the entry alone).
    fn mirror_recovery_payload(&self, intent: &OperationIntent) -> Option<String> {
        let read = |output: &Arc<dyn LocalOutputPort>| match output.inspect(&intent.day, "").ok()? {
            LocalOutputState::Different(text) => Some(text),
            _ => None,
        };
        let mirrored = self.mirror_output.as_ref().and_then(read);
        match mirrored {
            Some(text) if Digest::of_bytes(text.as_bytes()) == intent.payload_digest => Some(text),
            // Otherwise the write did not land: recover against the verified
            // local bytes so the intent resolves rather than gating the day.
            _ => self.local_output.as_ref().and_then(read),
        }
    }
}
