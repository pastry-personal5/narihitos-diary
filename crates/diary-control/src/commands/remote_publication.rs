//! Remote publication: what a completed day offers after its local file is
//! saved. iCloud is stateless: its plan is derived from the verified local
//! file and the folder as it stands, so a push is its own approval. Notion
//! keeps its stored plan and is pushed through the same entry points.

use super::readiness::progress_of;
use super::*;

/// Where one remote stands for the day.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RemoteStatus {
    NotPushed,
    /// The remote holds content this app did not write.
    Exists,
    Pushed,
    Blocked,
}

impl RemoteStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotPushed => "not-pushed",
            Self::Exists => "exists",
            Self::Pushed => "pushed",
            Self::Blocked => "blocked",
        }
    }
}

/// The bindings an iCloud intent records, taken from the confirmed entry.
pub(super) struct RemoteBinding {
    pub plan_id: String,
    pub entry_digest: Digest,
    pub selection_digest: Digest,
}

#[derive(Clone, Debug)]
pub struct RemoteSummary {
    pub id: &'static str,
    pub label: &'static str,
    pub status: RemoteStatus,
    /// A closed content-free token when the remote is blocked.
    pub reason: Option<String>,
}

/// The exact iCloud result, derived without storing anything.
#[derive(Clone, Debug)]
pub struct IcloudPreview {
    pub status: RemoteStatus,
    pub target: String,
    pub payload: String,
    pub existing_text: Option<String>,
    pub intended_changes: Vec<String>,
    pub mode: ExistingEntryMode,
    pub plan_digest: Digest,
    pub reason: Option<String>,
    observed: Digest,
    replacing: bool,
}

impl CommandService {
    /// The verified local file's exact bytes: what every remote receives.
    pub(super) fn local_final_text(&self, workflow: &Workflow) -> Result<String, CommandError> {
        self.verify_local_receipt(workflow)?;
        let output = self
            .local_output
            .as_ref()
            .ok_or(CommandError::LocalOutput(LocalOutputError::Unavailable))?;
        match output
            .inspect(workflow.day(), "")
            .map_err(CommandError::LocalOutput)?
        {
            LocalOutputState::Different(text) => Ok(text),
            LocalOutputState::Identical => Ok(String::new()),
            LocalOutputState::Absent => {
                Err(CommandError::Workflow(WorkflowError::LocalMustBeVerified))
            }
        }
    }

    /// Lists the remotes and where each stands. Read-only.
    pub fn remotes(&self, day: &DiaryDay) -> Result<Vec<RemoteSummary>, CommandError> {
        self.ensure_clear(day)?;
        let workflow = self.load_workflow(day)?;
        if progress_of(&workflow, "local") != Some(DestinationStatus::Verified) {
            let blocked = |id, label| RemoteSummary {
                id,
                label,
                status: RemoteStatus::Blocked,
                reason: Some("local-not-published".to_owned()),
            };
            return Ok(vec![
                blocked(MIRROR_DESTINATION, "iCloud"),
                blocked("notion", "Notion"),
            ]);
        }
        let icloud = match self.preview_icloud(day, ExistingEntryMode::Refuse) {
            Ok(preview) => RemoteSummary {
                id: MIRROR_DESTINATION,
                label: "iCloud",
                status: preview.status,
                reason: preview.reason,
            },
            Err(_) => RemoteSummary {
                id: MIRROR_DESTINATION,
                label: "iCloud",
                status: RemoteStatus::Blocked,
                reason: Some("icloud-unavailable".to_owned()),
            },
        };
        let notion = if let Some(notion) = self.notion_output.as_ref() {
            let (status, reason) = match notion.observe_existing(day) {
                Ok(Some(_)) if self.notion_section_is_ours(day) => {
                    if self.notion_receipt_matches_entry(day, &workflow) {
                        (RemoteStatus::Pushed, None)
                    } else {
                        (RemoteStatus::NotPushed, None)
                    }
                }
                Ok(Some(_)) => (RemoteStatus::Exists, None),
                Ok(None) => (RemoteStatus::NotPushed, None),
                Err(error) => (
                    RemoteStatus::Blocked,
                    Some(
                        match error.as_str() {
                            "notion-same-day-conflict" => "notion-same-day-conflict",
                            "notion-hierarchy-ambiguous" => "notion-hierarchy-ambiguous",
                            _ => "notion-unavailable",
                        }
                        .to_owned(),
                    ),
                ),
            };
            RemoteSummary {
                id: "notion",
                label: "Notion",
                status,
                reason,
            }
        } else {
            RemoteSummary {
                id: "notion",
                label: "Notion",
                status: RemoteStatus::Blocked,
                reason: Some("notion-not-configured".to_owned()),
            }
        };
        Ok(vec![icloud, notion])
    }

    /// Previews the iCloud copy. A chosen `mode` plans an Overwrite of
    /// content this app did not write; nothing is stored.
    pub fn preview_icloud(
        &self,
        day: &DiaryDay,
        mode: ExistingEntryMode,
    ) -> Result<IcloudPreview, CommandError> {
        if mode == ExistingEntryMode::Append {
            return Err(CommandError::PreconditionFailed);
        }
        self.ensure_clear(day)?;
        let mirror = self
            .mirror_output
            .as_ref()
            .ok_or(CommandError::LocalOutput(LocalOutputError::Unavailable))?;
        let workflow = self.load_workflow(day)?;
        let local = self.local_final_text(&workflow)?;
        let local_digest = Digest::of_bytes(local.as_bytes());
        let target = mirror.target(day);
        let owned = Self::owned_receipt(&self.receipts_of(day)?, MIRROR_DESTINATION, u32::MAX)
            .map(|receipt| receipt.payload_digest);
        let payload = local.clone();
        let mut existing_text = None;
        let mut replacing = false;
        let mut reason = None;
        let mut intended_changes = Vec::new();
        let (status, observed) = match mirror.inspect(day, &local) {
            Err(_) => {
                reason = Some("icloud-folder-unavailable".to_owned());
                (RemoteStatus::Blocked, Digest::of_bytes(b"unverifiable"))
            }
            Ok(LocalOutputState::Absent) => {
                intended_changes.push(format!("Create {target}"));
                (RemoteStatus::NotPushed, Digest::of_bytes(b"absent"))
            }
            Ok(LocalOutputState::Identical) => (RemoteStatus::Pushed, local_digest.clone()),
            Ok(LocalOutputState::Different(text)) => {
                let observed = Digest::of_bytes(text.as_bytes());
                if Some(&observed) == owned.as_ref() {
                    // The copy this app wrote earlier: update it, keeping a backup.
                    replacing = true;
                    intended_changes.push(format!(
                        "Back up the earlier copy to {}, then update {target}",
                        diary_core::backup_file_name(&target, &observed)
                    ));
                    (RemoteStatus::NotPushed, observed)
                } else {
                    existing_text = Some(text.clone());
                    match mode {
                        ExistingEntryMode::Append => unreachable!("rejected above"),
                        ExistingEntryMode::Overwrite => {
                            replacing = true;
                            intended_changes.push(format!(
                                "Back up the existing file to {}, then replace {target}",
                                diary_core::backup_file_name(&target, &observed)
                            ));
                        }
                        ExistingEntryMode::Refuse => {}
                    }
                    (RemoteStatus::Exists, observed)
                }
            }
        };
        let plan_digest = Digest::of_bytes(
            format!(
                "icloud|{target}|{}|{}|{}",
                observed.as_str(),
                local_digest.as_str(),
                mode.as_str()
            )
            .as_bytes(),
        );
        Ok(IcloudPreview {
            status,
            target,
            payload,
            existing_text,
            intended_changes,
            mode,
            plan_digest,
            reason,
            observed,
            replacing,
        })
    }

    /// Pushes the iCloud copy. The push is the approval: it acts only on the
    /// plan the person previewed (`expected`), and a changed folder is
    /// reported as [`CommandError::PreconditionFailed`] so the client can
    /// show the refreshed preview instead of stalling.
    pub fn push_icloud(
        &self,
        day: &DiaryDay,
        mode: ExistingEntryMode,
        expected: &Digest,
        commit: impl FnOnce() -> bool,
    ) -> Result<DestinationStatus, CommandError> {
        let workflow = self.load_workflow(day)?;
        let preview = self.preview_icloud(day, mode)?;
        if &preview.plan_digest != expected {
            return Err(CommandError::PreconditionFailed);
        }
        match preview.status {
            RemoteStatus::Pushed => return Ok(DestinationStatus::Verified),
            RemoteStatus::Blocked => return Ok(DestinationStatus::Blocked),
            RemoteStatus::Exists if !mode.chosen() => {
                return Err(CommandError::PreconditionFailed);
            }
            _ => {}
        }
        if !commit() {
            return Err(CommandError::Cancelled);
        }
        let payload_digest = Digest::of_bytes(preview.payload.as_bytes());
        let intent = if preview.replacing {
            existing_entry::replacement_intent(&preview.observed, &payload_digest)
        } else {
            payload_digest.clone()
        };
        let target = DestinationPlan::new(
            DestinationKind::Mirror,
            MIRROR_DESTINATION,
            preview.target.clone(),
            intent,
            payload_digest,
            preview.observed.clone(),
        )
        .map_err(|_| CommandError::LocalOutput(LocalOutputError::Unavailable))?;
        let binding = RemoteBinding {
            plan_id: random_id("plan_")
                .map_err(|_| CommandError::LocalOutput(LocalOutputError::Unavailable))?,
            entry_digest: workflow
                .entry()
                .confirmed
                .as_ref()
                .ok_or(CommandError::Workflow(WorkflowError::EntryNotConfirmed))?
                .entry_digest
                .clone(),
            selection_digest: workflow
                .selection_digest()
                .ok_or(CommandError::Workflow(WorkflowError::PlanBindingMismatch))?
                .clone(),
        };
        Ok(self.execute_mirror(day, &binding, &target, &preview.payload))
    }

    /// The Notion month page URL, offered only when that page blocks a push
    /// so the person can fix it by hand in Notion.
    #[must_use]
    pub fn notion_fix_url(&self, day: &DiaryDay) -> Option<String> {
        self.notion_output.as_ref()?.month_page_url(day)
    }

    /// Read-only observation for the remote menu. A bare selection must not
    /// save an Append plan or change the completed workflow state.
    pub fn notion_existing_section(
        &self,
        day: &DiaryDay,
    ) -> Result<Option<diary_core::NotionExistingSection>, String> {
        self.notion_output
            .as_ref()
            .ok_or_else(|| "notion-not-configured".to_owned())?
            .observe_existing(day)
    }

    /// Whether the Notion section for this day is the one this app wrote: the
    /// same page and blocks as its latest Notion receipt, still unedited. Such
    /// a section is updated in place by a push, with no choice to make.
    #[must_use]
    pub fn notion_section_is_ours(&self, day: &DiaryDay) -> bool {
        let Some(notion) = self.notion_output.as_ref() else {
            return false;
        };
        let Ok(Some(section)) = notion.observe_existing(day) else {
            return false;
        };
        let Ok(receipts) = self.receipts_of(day) else {
            return false;
        };
        let Some(owned) = Self::owned_receipt(&receipts, "notion", u32::MAX) else {
            return false;
        };
        owned.receipt_id.split('#').next() == Some(section.month_id.as_str())
            && owned.returned_ids == section.block_ids
            && owned.payload_digest == section.section_digest
    }

    #[must_use]
    pub fn notion_receipt_matches_entry(&self, day: &DiaryDay, workflow: &Workflow) -> bool {
        let Some(entry_digest) = workflow.entry().confirmed.as_ref().map(|c| &c.entry_digest)
        else {
            return false;
        };
        self.receipts_of(day)
            .ok()
            .and_then(|receipts| Self::owned_receipt(&receipts, "notion", u32::MAX))
            .is_some_and(|receipt| &receipt.entry_digest == entry_digest)
    }

    /// Pushes Notion: approves the stored preview plan and publishes it.
    pub fn push_notion(
        &self,
        day: &DiaryDay,
        expected_revision: u64,
        plan_digest: &Digest,
        cancelled: &AtomicBool,
    ) -> Result<Workflow, CommandError> {
        let approved = self.approve_notion(day, expected_revision, plan_digest)?;
        self.publish_notion_guarded(day, approved.revision(), cancelled)
    }
}
