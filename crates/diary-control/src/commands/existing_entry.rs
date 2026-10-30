//! Append and Overwrite of an entry that already exists at a file destination.
//!
//! The chosen mode is never stored. The plan's creation time and payload
//! digests imply it, so approval, publication, mirror retry, and recovery
//! re-derive the exact bytes the person previewed.

use super::amendment::AmendNotionDestination;
use super::*;
use diary_core::{NotionExistingSection, NotionExtendRequest, NotionReplaceRequest};

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

/// The request an approved Notion plan acts on, rebuilt from the section as it
/// stands now.
pub(super) enum ExistingNotion {
    Replace(NotionReplaceRequest),
    Extend(NotionExtendRequest),
}

pub(super) fn overwrite_request(
    day: &DiaryDay,
    markdown: &str,
    root: &str,
    plan_digest: &Digest,
    section: &NotionExistingSection,
) -> NotionReplaceRequest {
    NotionReplaceRequest {
        day: day.clone(),
        markdown: markdown.to_owned(),
        expected_target_identity: root.to_owned(),
        expected_payload_digest: plan_digest.clone(),
        owned_month_id: section.month_id.clone(),
        owned_block_ids: section.block_ids.clone(),
        owned_payload_digest: section.section_digest.clone(),
    }
}

pub(super) fn extend_request(
    day: &DiaryDay,
    markdown: &str,
    root: &str,
    appended_at: String,
    expected: Option<Digest>,
    section: &NotionExistingSection,
) -> NotionExtendRequest {
    NotionExtendRequest {
        day: day.clone(),
        markdown: markdown.to_owned(),
        appended_at,
        expected_target_identity: root.to_owned(),
        expected_result_digest: expected,
        owned_month_id: section.month_id.clone(),
        owned_block_ids: section.block_ids.clone(),
        owned_section_digest: section.section_digest.clone(),
    }
}

impl CommandService {
    /// Rebuilds the request for a Notion plan made for Append or Overwrite and
    /// checks it still binds: the section must be exactly what was previewed
    /// and the planned digest must be the one this mode produces. Anything
    /// else is a changed binding and nothing is written.
    pub(super) fn bind_existing_notion(
        &self,
        notion: &dyn NotionOutputPort,
        workflow: &Workflow,
        target: &DestinationPlan,
    ) -> Result<ExistingNotion, CommandError> {
        let guard = replacing_guard(target).ok_or(CommandError::PreconditionFailed)?;
        let day = workflow.day();
        let text = &workflow.entry().text;
        let created_at = workflow
            .output_plan()
            .ok_or(CommandError::Workflow(WorkflowError::MissingPlan))?
            .created_at;
        let regular = notion
            .preflight(day, text)
            .map_err(|reason| notion_preflight_error(&reason))?;
        if regular.target_identity != target.target_identity {
            return Err(CommandError::NotionBindingChanged("notion-target-changed"));
        }
        let section = notion
            .observe_existing(day)
            .map_err(|reason| notion_preflight_error(&reason))?
            .ok_or(CommandError::NotionBindingChanged("notion-state-changed"))?;
        if &section.section_digest != guard {
            return Err(CommandError::NotionBindingChanged("notion-state-changed"));
        }
        let overwrite = overwrite_request(
            day,
            text,
            &target.target_identity,
            &regular.payload_digest,
            &section,
        );
        if let Ok(checked) = notion.preflight_replace(&overwrite)
            && checked.preflight.status == NotionPreflightStatus::Ready
            && checked.preflight.payload_digest == target.payload_digest
        {
            return Ok(ExistingNotion::Replace(overwrite));
        }
        let mut append = extend_request(
            day,
            text,
            &target.target_identity,
            append_stamp(created_at),
            None,
            &section,
        );
        if let Ok(checked) = notion.preflight_extend(&append)
            && checked.preflight.status == NotionPreflightStatus::Ready
            && checked.preflight.payload_digest == target.payload_digest
        {
            append.expected_result_digest = Some(checked.preflight.payload_digest);
            return Ok(ExistingNotion::Extend(append));
        }
        Err(CommandError::NotionBindingChanged("notion-payload-changed"))
    }
}

/// Appends under an existing Notion section.
pub(super) struct ExtendNotionDestination<'a> {
    pub output: &'a dyn NotionOutputPort,
    pub request: &'a NotionExtendRequest,
    pub cancelled: &'a AtomicBool,
}

impl Destination for ExtendNotionDestination<'_> {
    fn write(&mut self, intent: &OperationIntent) -> WriteOutcome {
        match self.output.extend_section(self.request, self.cancelled) {
            WriteOutcome::Verified {
                receipt_id,
                returned_ids,
            } => WriteOutcome::Verified {
                receipt_id: format!("{receipt_id}#{}", intent.attempt_id),
                returned_ids,
            },
            other => other,
        }
    }

    fn inspect(&mut self, intent: &OperationIntent) -> InspectionOutcome {
        let Some(expected) = &self.request.expected_result_digest else {
            return InspectionOutcome::Conflict;
        };
        match self.output.observe_existing(&self.request.day) {
            Ok(Some(section)) if &section.section_digest == expected => {
                InspectionOutcome::Matching {
                    receipt_id: format!("{}#{}", section.month_id, intent.attempt_id),
                    returned_ids: section.block_ids,
                }
            }
            // Still exactly what was previewed: the addition never happened.
            Ok(Some(section)) if section.section_digest == self.request.owned_section_digest => {
                InspectionOutcome::Absent
            }
            Ok(_) => InspectionOutcome::Conflict,
            Err(_) => InspectionOutcome::Unavailable,
        }
    }
}

/// The Notion destination for a plain publication, an Overwrite, or an Append.
pub(super) enum NotionPublishTarget<'a> {
    Plain(NotionDestination<'a>),
    Replace(AmendNotionDestination<'a>),
    Extend(ExtendNotionDestination<'a>),
}

impl<'a> NotionPublishTarget<'a> {
    pub(super) fn replacing(
        output: &'a dyn NotionOutputPort,
        request: &'a NotionReplaceRequest,
        cancelled: &'a AtomicBool,
    ) -> Self {
        Self::Replace(AmendNotionDestination {
            output,
            request,
            cancelled,
            version: 1,
        })
    }
}

impl Destination for NotionPublishTarget<'_> {
    fn write(&mut self, intent: &OperationIntent) -> WriteOutcome {
        match self {
            Self::Plain(destination) => destination.write(intent),
            Self::Replace(destination) => destination.write(intent),
            Self::Extend(destination) => destination.write(intent),
        }
    }

    fn inspect(&mut self, intent: &OperationIntent) -> InspectionOutcome {
        match self {
            Self::Plain(destination) => destination.inspect(intent),
            Self::Replace(destination) => destination.inspect(intent),
            Self::Extend(destination) => destination.inspect(intent),
        }
    }
}

impl CommandService {
    /// Previews an Append or Overwrite of a canonical same-day Notion section
    /// the app did not write. Nothing changes until the person approves the
    /// exact plan; a section that changed since falls back to Conflict.
    pub(super) fn preview_notion_existing(
        &self,
        notion: &dyn NotionOutputPort,
        day: &DiaryDay,
        expected_revision: u64,
        mut workflow: Workflow,
        preflight: diary_core::NotionPreflight,
        mode: ExistingEntryMode,
    ) -> Result<NotionPreview, CommandError> {
        fn refused(
            workflow: Workflow,
            preflight: diary_core::NotionPreflight,
            mode: ExistingEntryMode,
            reason: String,
        ) -> NotionPreview {
            NotionPreview {
                workflow,
                root_label: preflight.root_label,
                year_title: preflight.year_title,
                month_title: preflight.month_title,
                year_exists: preflight.year_exists,
                month_exists: preflight.month_exists,
                placement: preflight.placement,
                renderer_version: preflight.renderer_version,
                api_version: preflight.api_version,
                payload_digest: preflight.payload_digest,
                plan_digest: None,
                blocks: preflight.blocks,
                current_blocks: Vec::new(),
                mode,
                status: DestinationStatus::Conflict,
                reason: Some(reason),
            }
        }
        let created_at = self.clock.now();
        let text = workflow.entry().text.clone();
        let section = match notion.observe_existing(day) {
            Ok(Some(section)) => section,
            Ok(None) => {
                let reason = "notion-same-day-conflict".to_owned();
                return Ok(refused(workflow, preflight, mode, reason));
            }
            Err(reason) => return Ok(refused(workflow, preflight, mode, reason)),
        };
        let checked = if mode == ExistingEntryMode::Overwrite {
            notion.preflight_replace(&overwrite_request(
                day,
                &text,
                &preflight.target_identity,
                &preflight.payload_digest,
                &section,
            ))
        } else {
            notion.preflight_extend(&extend_request(
                day,
                &text,
                &preflight.target_identity,
                append_stamp(created_at),
                None,
                &section,
            ))
        };
        let checked = match checked {
            Ok(checked) if checked.preflight.status == NotionPreflightStatus::Ready => checked,
            Ok(checked) => {
                let reason = checked
                    .preflight
                    .reason
                    .unwrap_or_else(|| "notion-same-day-conflict".to_owned());
                return Ok(refused(workflow, preflight, mode, reason));
            }
            Err(reason) => return Ok(refused(workflow, preflight, mode, reason)),
        };
        let prep = checked.preflight;
        let replacement = DestinationPlan::new(
            DestinationKind::Remote,
            "notion",
            prep.target_identity.clone(),
            replacement_intent(&prep.observed_state_digest, &prep.payload_digest),
            prep.payload_digest.clone(),
            prep.observed_state_digest.clone(),
        )
        .map_err(|_| CommandError::NotionUnavailable)?;
        let plan = workflow
            .preview_blocked_retry(
                expected_revision,
                "notion",
                replacement,
                random_id("plan_").map_err(|_| CommandError::NotionUnavailable)?,
                created_at,
            )
            .map_err(CommandError::Workflow)?;
        self.store
            .compare_and_swap(expected_revision, None, &workflow)
            .map_err(CommandError::Store)?;
        Ok(NotionPreview {
            workflow,
            root_label: prep.root_label,
            year_title: prep.year_title,
            month_title: prep.month_title,
            year_exists: prep.year_exists,
            month_exists: prep.month_exists,
            placement: prep.placement,
            renderer_version: prep.renderer_version,
            api_version: prep.api_version,
            payload_digest: prep.payload_digest,
            plan_digest: Some(plan.plan_digest),
            blocks: prep.blocks,
            current_blocks: checked.current_blocks,
            mode,
            status: DestinationStatus::Pending,
            reason: None,
        })
    }
}
