//! Amendment of a `Completed` day: local-first, digest-guarded replacement of
//! only the content the app itself wrote and receipted.

use diary_core::{AmendmentSeed, NotionReplaceRequest, WorkflowState};

use super::*;

/// What the person sees before approving a replacement.
#[derive(Clone, Debug)]
pub struct AmendmentPreview {
    pub workflow: Workflow,
    pub local_target: String,
    pub local_current: String,
    pub local_payload: String,
    pub local_status: DestinationStatus,
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

/// As [`versioned`], for a Notion receipt: the attempt keeps it unique across
/// the other days of the same month, which share the month page ID.
pub(super) fn versioned_for_attempt(receipt_id: &str, attempt_id: &str, version: u32) -> String {
    format!("{}#{attempt_id}#v{version}", base_receipt_id(receipt_id))
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

pub(super) struct AmendNotionDestination<'a> {
    pub(super) output: &'a dyn NotionOutputPort,
    pub(super) request: &'a NotionReplaceRequest,
    pub(super) cancelled: &'a AtomicBool,
    pub(super) version: u32,
}

impl Destination for AmendNotionDestination<'_> {
    fn write(&mut self, intent: &OperationIntent) -> WriteOutcome {
        match self.output.replace_section(self.request, self.cancelled) {
            WriteOutcome::Verified {
                receipt_id,
                returned_ids,
            } => WriteOutcome::Verified {
                receipt_id: versioned_for_attempt(&receipt_id, &intent.attempt_id, self.version),
                returned_ids,
            },
            other => other,
        }
    }

    fn inspect(&mut self, intent: &OperationIntent) -> InspectionOutcome {
        let versioned = |outcome: InspectionOutcome| match outcome {
            InspectionOutcome::Matching {
                receipt_id,
                returned_ids,
            } => InspectionOutcome::Matching {
                receipt_id: versioned_for_attempt(&receipt_id, &intent.attempt_id, self.version),
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

impl CommandService {}

mod mirror;
mod preview;
mod publication;
mod reconciliation;

impl CommandService {
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
