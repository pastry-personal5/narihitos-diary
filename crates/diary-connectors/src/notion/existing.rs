//! Append and Overwrite of a same-day section the app did not write.
//!
//! Overwrite reuses the amendment replacement with a request built from
//! [`NotionClient::observe_section`]. Append inserts a separator, a dated line
//! and the new body after the section's last block, resuming safely: every
//! step reads the section back first.

use super::output::{flatten_preview, remote_reason};
use super::projection::{split_rich_text, text_block};
use super::*;

/// The canonical same-day section as read from the month page.
struct SectionRead {
    plan: NotionPlan,
    month_id: String,
    remote: Vec<RemoteBlock>,
    start: usize,
    end: usize,
}

impl SectionRead {
    fn blocks(&self) -> Vec<NotionBlock> {
        self.remote[self.start..self.end]
            .iter()
            .map(|item| item.block.clone())
            .collect()
    }

    fn ids(&self) -> Vec<String> {
        self.remote[self.start..self.end]
            .iter()
            .map(|item| item.id.clone())
            .collect()
    }

    fn digest(&self, blocks: &[NotionBlock]) -> Result<Digest, RemoteError> {
        let digest = plan_digest(
            RENDERER_VERSION,
            API_VERSION,
            &self.plan.year_title,
            &self.plan.month_title,
            &self.plan.heading,
            blocks,
        )
        .map_err(|_| RemoteError::Invalid)?;
        Digest::parse(digest).map_err(|_| RemoteError::Invalid)
    }
}

/// The blocks an Append adds: a literal rule (the projection has no divider
/// block), an italic dated line, then the new body without its own heading.
fn extension_blocks(
    day: &DiaryDay,
    markdown: &str,
    appended_at: &str,
) -> Result<Vec<NotionBlock>, NotionError> {
    let plan = render_notion(day, markdown)?;
    let dated = NotionBlock {
        kind: BlockKind::Paragraph,
        rich_text: split_rich_text(
            &format!("Appended {appended_at}"),
            NotionTextAnnotations {
                italic: true,
                ..NotionTextAnnotations::default()
            },
            None,
        ),
        language: None,
        children: Vec::new(),
    };
    let mut added = vec![text_block(BlockKind::Paragraph, "---".to_owned()), dated];
    added.extend(plan.blocks.into_iter().skip(1));
    Ok(added)
}

impl NotionClient {
    fn read_section(
        &self,
        day: &DiaryDay,
        cancelled: &AtomicBool,
    ) -> Result<Option<SectionRead>, RemoteError> {
        let plan = render_notion(day, "").map_err(|_| RemoteError::Invalid)?;
        let Some(month) = self.resolve_path(&plan, cancelled)?.month else {
            return Ok(None);
        };
        let remote = self.remote.list_blocks(&month.id, cancelled)?;
        let bare: Vec<_> = remote.iter().map(|item| item.block.clone()).collect();
        match classify_same_day(&bare, &plan) {
            SameDayState::Absent => Ok(None),
            SameDayState::Canonical { index } => {
                let end = remote
                    .iter()
                    .enumerate()
                    .skip(index + 1)
                    .find(|(_, item)| {
                        matches!(item.block.kind, BlockKind::Heading1 | BlockKind::Heading2)
                    })
                    .map_or(remote.len(), |(position, _)| position);
                Ok(Some(SectionRead {
                    plan,
                    month_id: month.id,
                    remote,
                    start: index,
                    end,
                }))
            }
            SameDayState::Ambiguous { .. } => Err(RemoteError::Invalid),
        }
    }

    pub(super) fn observe_section(
        &self,
        day: &DiaryDay,
    ) -> Result<Option<NotionExistingSection>, String> {
        let cancelled = AtomicBool::new(false);
        match self.read_section(day, &cancelled) {
            Ok(None) => Ok(None),
            Ok(Some(section)) => {
                let blocks = section.blocks();
                Ok(Some(NotionExistingSection {
                    month_id: section.month_id.clone(),
                    block_ids: section.ids(),
                    section_digest: section.digest(&blocks).map_err(remote_reason)?,
                    blocks: flatten_preview(&blocks),
                }))
            }
            Err(RemoteError::Invalid) => Err("notion-same-day-conflict".to_owned()),
            Err(error) => Err(remote_reason(error)),
        }
    }

    pub(super) fn preflight_extension(
        &self,
        request: &NotionExtendRequest,
    ) -> Result<NotionReplacePreflight, String> {
        if request.expected_target_identity != self.root_page_id {
            return Err("notion-target-changed".to_owned());
        }
        let added = extension_blocks(&request.day, &request.markdown, &request.appended_at)
            .map_err(|_| "notion-render-invalid".to_owned())?;
        let cancelled = AtomicBool::new(false);
        let plan = render_notion(&request.day, &request.markdown)
            .map_err(|_| "notion-render-invalid".to_owned())?;
        let (status, reason, current, result) = match self.read_section(&request.day, &cancelled) {
            Ok(Some(section))
                if section.month_id == request.owned_month_id
                    && section.ids() == request.owned_block_ids
                    && section.digest(&section.blocks()).as_ref()
                        == Ok(&request.owned_section_digest) =>
            {
                let current = section.blocks();
                let mut whole = current.clone();
                whole.extend(added.iter().cloned());
                let result = section.digest(&whole).map_err(remote_reason)?;
                (NotionPreflightStatus::Ready, None, current, result)
            }
            Ok(_) | Err(RemoteError::Invalid) => (
                NotionPreflightStatus::Existing,
                Some("notion-section-not-owned".to_owned()),
                Vec::new(),
                Digest::of_bytes(b"notion-section-not-owned"),
            ),
            Err(error) => return Err(remote_reason(error)),
        };
        Ok(NotionReplacePreflight {
            preflight: NotionPreflight {
                target_identity: self.root_page_id.clone(),
                root_label: "Notion root".to_owned(),
                year_title: plan.year_title.clone(),
                month_title: plan.month_title.clone(),
                year_exists: true,
                month_exists: true,
                placement: "append to existing section".to_owned(),
                renderer_version: plan.renderer_version,
                api_version: plan.api_version.clone(),
                payload_digest: result,
                observed_state_digest: request.owned_section_digest.clone(),
                blocks: flatten_preview(&added),
                status,
                reason,
            },
            current_blocks: flatten_preview(&current),
        })
    }

    pub(super) fn extend(
        &self,
        request: &NotionExtendRequest,
        cancelled: &AtomicBool,
    ) -> WriteOutcome {
        let Some(expected) = &request.expected_result_digest else {
            return WriteOutcome::Conflict;
        };
        if request.expected_target_identity != self.root_page_id {
            return WriteOutcome::Conflict;
        }
        let Ok(added) = extension_blocks(&request.day, &request.markdown, &request.appended_at)
        else {
            return WriteOutcome::Unavailable;
        };
        let owned = request.owned_block_ids.len();
        let mut wrote = false;
        loop {
            if cancelled.load(Ordering::Acquire) {
                return if wrote {
                    WriteOutcome::AmbiguousWrite
                } else {
                    WriteOutcome::DefiniteNoEffect
                };
            }
            let section = match self.read_section(&request.day, cancelled) {
                Ok(Some(section)) => section,
                Ok(None) | Err(RemoteError::Invalid) => return WriteOutcome::Conflict,
                Err(RemoteError::Pending) if !wrote => return WriteOutcome::DefiniteNoEffect,
                Err(RemoteError::Blocked(_)) if !wrote => return WriteOutcome::Unavailable,
                Err(_) => return WriteOutcome::AmbiguousWrite,
            };
            let ids = section.ids();
            let blocks = section.blocks();
            let fresh = blocks.get(owned..).unwrap_or_default();
            if section.month_id != request.owned_month_id
                || ids.get(..owned) != Some(request.owned_block_ids.as_slice())
                || fresh.len() > added.len()
                || fresh.iter().ne(added[..fresh.len()].iter())
            {
                return WriteOutcome::Conflict;
            }
            if fresh.is_empty()
                && section.digest(&blocks).as_ref() != Ok(&request.owned_section_digest)
            {
                return WriteOutcome::Conflict;
            }
            if fresh.len() == added.len() {
                return match section.digest(&blocks) {
                    Ok(digest) if &digest == expected => WriteOutcome::Verified {
                        receipt_id: section.month_id,
                        returned_ids: ids,
                    },
                    _ => WriteOutcome::Conflict,
                };
            }
            let chunk = chunk_blocks(&added[fresh.len()..])[0];
            let after = ids.last().cloned().unwrap_or_default();
            match self.remote.append_blocks(
                &section.month_id,
                chunk,
                &Placement::After(after),
                cancelled,
            ) {
                Ok(_) => {}
                Err(RemoteError::Pending) if !wrote => return WriteOutcome::DefiniteNoEffect,
                Err(_) => return WriteOutcome::AmbiguousWrite,
            }
            // Loop: the next pass reads the insert back before anything else.
            wrote = true;
        }
    }
}
