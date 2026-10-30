//! Append and Overwrite of an entry that already exists at a file destination.
//!
//! The chosen mode is never stored. The plan's creation time and payload
//! digests imply it, so approval, publication, mirror retry, and recovery
//! re-derive the exact bytes the person previewed.

use super::*;

/// What a preview does when a destination already holds different content.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ExistingEntryMode {
    /// Report the differing content as a conflict, as before.
    #[default]
    Refuse,
    /// Keep the existing content and add the new text under a dated separator.
    Append,
    /// Replace the existing content, keeping a backup of a file.
    Overwrite,
}

impl ExistingEntryMode {
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "append" => Some(Self::Append),
            "overwrite" => Some(Self::Overwrite),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Refuse => "refuse",
            Self::Append => "append",
            Self::Overwrite => "overwrite",
        }
    }

    pub(super) const fn chosen(self) -> bool {
        !matches!(self, Self::Refuse)
    }
}

/// Mac-local `YYYY-MM-DD HH:MM` of the plan's creation time, fixed when the
/// plan is built so the previewed and the written bytes agree.
pub(super) fn append_stamp(created_at: Timestamp) -> String {
    crate::local_time::local_minute(created_at.0)
}

/// A destination that replaces existing content instead of creating a file.
///
/// The write is guarded by the digest of the content the person previewed;
/// content that changed since then is a conflict and nothing is written.
pub(super) struct ReplacingDestination<'a> {
    pub output: &'a dyn LocalOutputPort,
    pub day: &'a DiaryDay,
    pub payload: &'a str,
    pub receipt_id: String,
    /// Digest of the previewed existing content; `None` creates the file.
    pub replacing: Option<&'a Digest>,
    /// Copy the replaced file beside it first (Overwrite and the mirror).
    pub backup: bool,
}

impl ReplacingDestination<'_> {
    fn verified(&self) -> WriteOutcome {
        WriteOutcome::Verified {
            receipt_id: self.receipt_id.clone(),
            returned_ids: Vec::new(),
        }
    }
}

impl Destination for ReplacingDestination<'_> {
    fn write(&mut self, intent: &OperationIntent) -> WriteOutcome {
        let Some(expected) = self.replacing else {
            return LocalDestination {
                output: self.output,
                day: self.day,
                payload: self.payload,
                receipt_id: self.receipt_id.clone(),
            }
            .write(intent);
        };
        if self.backup {
            match self.output.backup(self.day, expected) {
                Ok(true) => {}
                // Absent or changed: a replay that already landed is verified,
                // anything else is a conflict and nothing was written.
                Ok(false) => {
                    return match self.output.inspect(self.day, self.payload) {
                        Ok(LocalOutputState::Identical) => self.verified(),
                        Ok(_) => WriteOutcome::Conflict,
                        Err(_) => WriteOutcome::Unavailable,
                    };
                }
                Err(_) => return WriteOutcome::Unavailable,
            }
        }
        match self.output.replace(self.day, expected, self.payload) {
            Ok(
                diary_core::LocalReplaceOutcome::Replaced
                | diary_core::LocalReplaceOutcome::AlreadyReplaced,
            ) => self.verified(),
            Ok(diary_core::LocalReplaceOutcome::Changed) => WriteOutcome::Conflict,
            Err(_) => WriteOutcome::Unavailable,
        }
    }

    fn inspect(&mut self, intent: &OperationIntent) -> InspectionOutcome {
        let Some(expected) = self.replacing else {
            return LocalDestination {
                output: self.output,
                day: self.day,
                payload: self.payload,
                receipt_id: self.receipt_id.clone(),
            }
            .inspect(intent);
        };
        match self.output.inspect(self.day, self.payload) {
            Ok(LocalOutputState::Identical) => InspectionOutcome::Matching {
                receipt_id: self.receipt_id.clone(),
                returned_ids: Vec::new(),
            },
            // Still exactly what was previewed: the replacement never happened.
            Ok(LocalOutputState::Different(text))
                if &Digest::of_bytes(text.as_bytes()) == expected =>
            {
                InspectionOutcome::Absent
            }
            Ok(_) => InspectionOutcome::Conflict,
            Err(_) => InspectionOutcome::Unavailable,
        }
    }
}

impl CommandService {
    /// The exact bytes a plan publishes at its local target: the plain
    /// rendering (Overwrite, or no existing file), the file itself once it
    /// already holds the planned bytes, or the existing file plus the Append
    /// block built from the plan's creation time.
    pub(super) fn planned_local_payload(
        &self,
        output: &dyn LocalOutputPort,
        workflow: &Workflow,
        expected: &Digest,
    ) -> Option<String> {
        let day = workflow.day();
        let body = &workflow.entry().text;
        let plain = render_local_markdown(day, workflow.language()?, body);
        if &Digest::of_bytes(plain.as_bytes()) == expected {
            return Some(plain);
        }
        let LocalOutputState::Different(text) = output.inspect(day, &plain).ok()? else {
            return None;
        };
        if &Digest::of_bytes(text.as_bytes()) == expected {
            return Some(text);
        }
        let created_at = workflow.output_plan()?.created_at;
        let candidate = diary_core::append_to_existing(&text, body, &append_stamp(created_at));
        (&Digest::of_bytes(candidate.as_bytes()) == expected).then_some(candidate)
    }
}

/// The intended-state digest of a target that replaces existing content. A
/// plan that merely met a conflict keeps the payload digest there, so only a
/// plan built for Append or Overwrite can ever replace anything.
pub(super) fn replacement_intent(existing: &Digest, payload: &Digest) -> Digest {
    Digest::of_bytes(format!("replace:{}:{}", existing.as_str(), payload.as_str()).as_bytes())
}

/// The digest to guard a replacement with, when the plan target was built to
/// replace the content the person previewed.
pub(super) fn replacing_guard(target: &DestinationPlan) -> Option<&Digest> {
    (target.intended_state_digest
        == replacement_intent(&target.inspected_state_digest, &target.payload_digest))
    .then_some(&target.inspected_state_digest)
}
