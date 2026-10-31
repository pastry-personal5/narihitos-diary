//! Notion output planning, publication, and reconciliation.

use super::*;

/// Production Notion adapter. Its Debug representation never contains the
/// bearer token or page identity.
pub struct NotionClient {
    pub(super) remote: Arc<dyn NotionRemote>,
    pub(super) root_page_id: String,
}

impl std::fmt::Debug for NotionClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NotionClient")
            .field("remote", &"[REDACTED]")
            .field("root_page_id", &"[REDACTED]")
            .finish()
    }
}

impl NotionClient {
    pub fn new(
        token: impl Into<String>,
        root_page_id: impl Into<String>,
    ) -> Result<Self, NotionError> {
        let token = token.into();
        let root_page_id = root_page_id.into();
        if token.is_empty()
            || root_page_id.len() != 32
            || !root_page_id.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(NotionError::InvalidPlan);
        }
        Ok(Self {
            remote: Arc::new(HttpRemote::new(token)?),
            root_page_id: root_page_id.to_ascii_lowercase(),
        })
    }

    #[cfg(test)]
    pub(super) fn scripted(remote: Arc<dyn NotionRemote>, root_page_id: &str) -> Self {
        Self {
            remote,
            root_page_id: root_page_id.to_owned(),
        }
    }

    pub(super) fn resolve_path(
        &self,
        plan: &NotionPlan,
        cancelled: &AtomicBool,
    ) -> Result<ResolvedPath, RemoteError> {
        let root_children = self.remote.list_children(&self.root_page_id, cancelled)?;
        let year =
            resolve_child(&root_children, &plan.year_title, true).map_err(map_hierarchy_error)?;
        let month = if let Some(year) = &year {
            let year_children = self.remote.list_children(&year.id, cancelled)?;
            resolve_child(&year_children, &plan.month_title, true).map_err(map_hierarchy_error)?
        } else {
            None
        };
        Ok(ResolvedPath { year, month })
    }

    fn preflight_plan(
        &self,
        plan: &NotionPlan,
        cancelled: &AtomicBool,
    ) -> Result<NotionPreflight, RemoteError> {
        let path = self.resolve_path(plan, cancelled)?;
        let mut observed = Vec::new();
        observed.extend_from_slice(self.root_page_id.as_bytes());
        observed.extend_from_slice(plan.year_title.as_bytes());
        observed.extend_from_slice(plan.month_title.as_bytes());
        let mut status = NotionPreflightStatus::Ready;
        let mut reason = None;
        let mut placement = "end".to_owned();
        if let Some(year) = &path.year {
            observed.extend_from_slice(year.id.as_bytes());
        }
        if let Some(month) = &path.month {
            observed.extend_from_slice(month.id.as_bytes());
            let blocks = self.remote.list_blocks(&month.id, cancelled)?;
            for block in &blocks {
                observed.extend_from_slice(block.id.as_bytes());
                observed.extend_from_slice(block.block.display_text().as_bytes());
            }
            let bare: Vec<_> = blocks.iter().map(|item| item.block.clone()).collect();
            if !matches!(classify_same_day(&bare, plan), SameDayState::Absent) {
                status = NotionPreflightStatus::Existing;
                reason = Some("notion-same-day-conflict".to_owned());
            } else {
                placement = plan_insertion(&bare, plan).label;
            }
        }
        Ok(NotionPreflight {
            target_identity: self.root_page_id.clone(),
            root_label: "Notion root".to_owned(),
            year_title: plan.year_title.clone(),
            month_title: plan.month_title.clone(),
            year_exists: path.year.is_some(),
            month_exists: path.month.is_some(),
            placement,
            renderer_version: plan.renderer_version,
            api_version: plan.api_version.clone(),
            payload_digest: Digest::parse(plan.payload_digest.clone())
                .map_err(|_| RemoteError::Invalid)?,
            observed_state_digest: Digest::of_bytes(&observed),
            blocks: flatten_preview(&plan.blocks),
            status,
            reason,
        })
    }

    fn ensure_path(
        &self,
        plan: &NotionPlan,
        cancelled: &AtomicBool,
    ) -> Result<ResolvedPath, RemoteError> {
        let mut path = self.resolve_path(plan, cancelled)?;
        if path.year.is_none() {
            let _ = self
                .remote
                .create_page(&self.root_page_id, &plan.year_title, cancelled)?;
            path = self.resolve_path(plan, cancelled)?;
            if path.year.is_none() {
                return Err(RemoteError::Ambiguous);
            }
        }
        if path.month.is_none() {
            let year_id = &path.year.as_ref().ok_or(RemoteError::Ambiguous)?.id;
            let _ = self
                .remote
                .create_page(year_id, &plan.month_title, cancelled)?;
            path = self.resolve_path(plan, cancelled)?;
            if path.month.is_none() {
                return Err(RemoteError::Ambiguous);
            }
        }
        Ok(path)
    }

    fn inspect_exact(
        &self,
        plan: &NotionPlan,
        cancelled: &AtomicBool,
    ) -> Result<Option<Vec<String>>, RemoteError> {
        let path = self.resolve_path(plan, cancelled)?;
        let Some(month) = path.month else {
            return Ok(None);
        };
        self.inspect_prefix_in_month(&month.id, plan, plan.blocks.len(), cancelled)
    }

    fn inspect_prefix_in_month(
        &self,
        month_id: &str,
        plan: &NotionPlan,
        expected_count: usize,
        cancelled: &AtomicBool,
    ) -> Result<Option<Vec<String>>, RemoteError> {
        let remote = self.remote.list_blocks(month_id, cancelled)?;
        let blocks: Vec<_> = remote.iter().map(|item| item.block.clone()).collect();
        let SameDayState::Canonical { index } = classify_same_day(&blocks, plan) else {
            return match classify_same_day(&blocks, plan) {
                SameDayState::Absent => Ok(None),
                _ => Err(RemoteError::Invalid),
            };
        };
        let end = index.saturating_add(expected_count);
        let section_end = remote
            .iter()
            .enumerate()
            .skip(index + 1)
            .find(|(_, item)| matches!(item.block.kind, BlockKind::Heading1 | BlockKind::Heading2))
            .map_or(remote.len(), |(index, _)| index);
        if end > remote.len()
            || end != section_end
            || remote[index..end]
                .iter()
                .map(|item| &item.block)
                .ne(plan.blocks[..expected_count].iter())
        {
            return Err(RemoteError::Invalid);
        }
        Ok(Some(
            remote[index..end]
                .iter()
                .map(|item| item.id.clone())
                .collect(),
        ))
    }
}

impl NotionOutputPort for NotionClient {
    fn preflight(&self, day: &DiaryDay, markdown: &str) -> Result<NotionPreflight, String> {
        let plan = render_notion(day, markdown).map_err(|_| "notion-render-invalid".to_owned())?;
        self.preflight_plan(&plan, &AtomicBool::new(false))
            .map_err(remote_reason)
    }

    fn month_page_url(&self, day: &DiaryDay) -> Option<String> {
        let plan = render_notion(day, "").ok()?;
        let path = self.resolve_path(&plan, &AtomicBool::new(false)).ok()?;
        let id = path.month?.id.replace('-', "");
        Some(format!("https://www.notion.so/{id}"))
    }

    fn write(
        &self,
        request: &NotionRequest,
        cancelled: &std::sync::atomic::AtomicBool,
    ) -> WriteOutcome {
        use std::sync::atomic::Ordering;

        if cancelled.load(Ordering::Acquire) {
            return WriteOutcome::DefiniteNoEffect;
        }
        let plan = match render_notion(&request.day, &request.markdown) {
            Ok(plan) => plan,
            Err(_) => return WriteOutcome::Unavailable,
        };
        if plan.payload_digest != request.expected_payload_digest.as_str()
            || request.expected_target_identity != self.root_page_id
        {
            return WriteOutcome::Conflict;
        }
        let fresh = match self.preflight_plan(&plan, cancelled) {
            Ok(fresh) => fresh,
            Err(RemoteError::Ambiguous) => return WriteOutcome::AmbiguousWrite,
            Err(RemoteError::Pending) => return WriteOutcome::DefiniteNoEffect,
            Err(_) => return WriteOutcome::Unavailable,
        };
        if fresh.observed_state_digest != request.expected_state_digest {
            return WriteOutcome::Conflict;
        }
        if fresh.status != NotionPreflightStatus::Ready {
            return WriteOutcome::Conflict;
        }
        if cancelled.load(Ordering::Acquire) {
            return WriteOutcome::DefiniteNoEffect;
        }
        let path = match self.ensure_path(&plan, cancelled) {
            Ok(path) => path,
            Err(RemoteError::Ambiguous) => return WriteOutcome::AmbiguousWrite,
            Err(RemoteError::Pending) => return WriteOutcome::DefiniteNoEffect,
            Err(_) => return WriteOutcome::Unavailable,
        };
        let month = match path.month {
            Some(month) => month,
            None => return WriteOutcome::AmbiguousWrite,
        };
        let existing = match self.remote.list_blocks(&month.id, cancelled) {
            Ok(blocks) => blocks,
            Err(_) => return WriteOutcome::Unavailable,
        };
        let bare: Vec<_> = existing.iter().map(|item| item.block.clone()).collect();
        if !matches!(classify_same_day(&bare, &plan), SameDayState::Absent) {
            return WriteOutcome::Conflict;
        }
        // Only the first chunk uses the dated insertion point; later chunks
        // chain after the last read-back-verified block.
        let mut place = match plan_insertion(&bare, &plan).point {
            InsertionPoint::End => Placement::End,
            InsertionPoint::Start => Placement::Start,
            InsertionPoint::After(index) => Placement::After(existing[index].id.clone()),
        };
        let mut ids = Vec::new();
        for chunk in plan.append_chunks() {
            if cancelled.load(Ordering::Acquire) {
                return if ids.is_empty() {
                    WriteOutcome::DefiniteNoEffect
                } else {
                    WriteOutcome::AmbiguousWrite
                };
            }
            let appended = match self
                .remote
                .append_blocks(&month.id, chunk, &place, cancelled)
            {
                Ok(appended) => appended,
                Err(RemoteError::Pending) if ids.is_empty() => {
                    return WriteOutcome::DefiniteNoEffect;
                }
                Err(_) => return WriteOutcome::AmbiguousWrite,
            };
            if appended.len() != chunk.len()
                || appended.iter().map(|item| &item.block).ne(chunk.iter())
            {
                return WriteOutcome::AmbiguousWrite;
            }
            let expected_count = ids.len().saturating_add(chunk.len());
            let verified =
                match self.inspect_prefix_in_month(&month.id, &plan, expected_count, cancelled) {
                    Ok(Some(verified)) => verified,
                    _ => return WriteOutcome::AmbiguousWrite,
                };
            ids = verified;
            if let Some(last) = ids.last() {
                place = Placement::After(last.clone());
            }
        }
        match self.inspect_exact(&plan, cancelled) {
            Ok(Some(verified)) if verified == ids => WriteOutcome::Verified {
                receipt_id: month.id,
                returned_ids: ids,
            },
            Ok(Some(_)) | Ok(None) | Err(RemoteError::Ambiguous | RemoteError::Pending) => {
                WriteOutcome::AmbiguousWrite
            }
            Err(_) => WriteOutcome::Conflict,
        }
    }

    fn inspect(&self, request: &NotionRequest) -> InspectionOutcome {
        let plan = match render_notion(&request.day, &request.markdown) {
            Ok(plan)
                if plan.payload_digest == request.expected_payload_digest.as_str()
                    && request.expected_target_identity == self.root_page_id =>
            {
                plan
            }
            _ => return InspectionOutcome::Conflict,
        };
        match self.inspect_exact(&plan, &AtomicBool::new(false)) {
            Ok(Some(ids)) => InspectionOutcome::Matching {
                receipt_id: "notion-section".to_owned(),
                returned_ids: ids,
            },
            Ok(None) => InspectionOutcome::Absent,
            Err(RemoteError::Invalid | RemoteError::Blocked(_)) => InspectionOutcome::Conflict,
            Err(RemoteError::Ambiguous | RemoteError::Pending) => InspectionOutcome::Unavailable,
        }
    }

    fn preflight_replace(
        &self,
        request: &NotionReplaceRequest,
    ) -> Result<NotionReplacePreflight, String> {
        let plan = render_notion(&request.day, &request.markdown)
            .map_err(|_| "notion-render-invalid".to_owned())?;
        let cancelled = AtomicBool::new(false);
        if request.expected_target_identity != self.root_page_id {
            return Err("notion-target-changed".to_owned());
        }
        let (status, reason, section, current) =
            match self.read_owned_section(request, &plan, &cancelled) {
                Ok((OwnedSection::Original { .. }, remote)) => {
                    let current = owned_blocks(&remote, request);
                    (NotionPreflightStatus::Ready, None, true, current)
                }
                Ok(_) => (NotionPreflightStatus::Ready, None, true, Vec::new()),
                Err(RemoteError::Invalid) => (
                    NotionPreflightStatus::Existing,
                    Some("notion-section-not-owned".to_owned()),
                    false,
                    Vec::new(),
                ),
                Err(error) => return Err(remote_reason(error)),
            };
        let observed = if section {
            request.owned_payload_digest.clone()
        } else {
            Digest::of_bytes(b"notion-section-not-owned")
        };
        Ok(NotionReplacePreflight {
            preflight: NotionPreflight {
                target_identity: self.root_page_id.clone(),
                root_label: "Notion root".to_owned(),
                year_title: plan.year_title.clone(),
                month_title: plan.month_title.clone(),
                year_exists: true,
                month_exists: true,
                placement: "in place".to_owned(),
                renderer_version: plan.renderer_version,
                api_version: plan.api_version.clone(),
                payload_digest: Digest::parse(&plan.payload_digest)
                    .map_err(|_| "notion-render-invalid".to_owned())?,
                observed_state_digest: observed,
                blocks: flatten_preview(&plan.blocks),
                status,
                reason,
            },
            current_blocks: flatten_preview(&current),
        })
    }

    fn replace_section(
        &self,
        request: &NotionReplaceRequest,
        cancelled: &AtomicBool,
    ) -> WriteOutcome {
        if request.expected_target_identity != self.root_page_id {
            return WriteOutcome::Conflict;
        }
        let plan = match Self::replace_plan(request) {
            Ok(plan) => plan,
            Err(outcome) => return outcome,
        };
        let wanted = &plan.blocks[1..];
        let mut wrote = false;
        loop {
            if cancelled.load(Ordering::Acquire) {
                return if wrote {
                    WriteOutcome::AmbiguousWrite
                } else {
                    WriteOutcome::DefiniteNoEffect
                };
            }
            let state = match self.read_owned_section(request, &plan, cancelled) {
                Ok((state, _)) => state,
                Err(RemoteError::Invalid) => return WriteOutcome::Conflict,
                Err(RemoteError::Pending) if !wrote => return WriteOutcome::DefiniteNoEffect,
                Err(RemoteError::Blocked(_)) if !wrote => return WriteOutcome::Unavailable,
                Err(_) => return WriteOutcome::AmbiguousWrite,
            };
            let (heading_id, fresh_ids, stale_ids) = match state {
                OwnedSection::Original { .. } => (
                    request.owned_block_ids[0].clone(),
                    Vec::new(),
                    request.owned_block_ids[1..].to_vec(),
                ),
                OwnedSection::Replacing {
                    heading_id,
                    fresh_ids,
                    stale_ids,
                } => (heading_id, fresh_ids, stale_ids),
            };
            if fresh_ids.len() < wanted.len() {
                // Insert the next chunk directly after the last verified block.
                let remaining = &wanted[fresh_ids.len()..];
                let chunk = chunk_blocks(remaining)[0];
                let after = fresh_ids.last().unwrap_or(&heading_id).clone();
                match self.remote.append_blocks(
                    &request.owned_month_id,
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
                continue;
            }
            if !stale_ids.is_empty() {
                if self.trash_stale(&stale_ids, cancelled).is_err() {
                    return WriteOutcome::AmbiguousWrite;
                }
                wrote = true;
                continue;
            }
            let mut returned = vec![heading_id];
            returned.extend(fresh_ids);
            return WriteOutcome::Verified {
                receipt_id: request.owned_month_id.clone(),
                returned_ids: returned,
            };
        }
    }

    fn observe_existing(&self, day: &DiaryDay) -> Result<Option<NotionExistingSection>, String> {
        self.observe_section(day)
    }

    fn preflight_extend(
        &self,
        request: &NotionExtendRequest,
    ) -> Result<NotionReplacePreflight, String> {
        self.preflight_extension(request)
    }

    fn extend_section(
        &self,
        request: &NotionExtendRequest,
        cancelled: &AtomicBool,
    ) -> WriteOutcome {
        self.extend(request, cancelled)
    }

    fn reconcile_replace(&self, request: &NotionReplaceRequest) -> InspectionOutcome {
        let plan = match Self::replace_plan(request) {
            Ok(plan) => plan,
            Err(_) => return InspectionOutcome::Conflict,
        };
        let cancelled = AtomicBool::new(false);
        let wanted = plan.blocks.len() - 1;
        match self.read_owned_section(request, &plan, &cancelled) {
            Ok((OwnedSection::Original { .. }, _)) => InspectionOutcome::Absent,
            Ok((
                OwnedSection::Replacing {
                    heading_id,
                    fresh_ids,
                    stale_ids,
                },
                _,
            )) => {
                if fresh_ids.len() < wanted {
                    // A partial insert is neither absent nor complete; the
                    // caller resumes it with `replace_section`.
                    return InspectionOutcome::Unavailable;
                }
                if self.trash_stale(&stale_ids, &cancelled).is_err() {
                    return InspectionOutcome::Unavailable;
                }
                let mut returned = vec![heading_id];
                returned.extend(fresh_ids);
                InspectionOutcome::Matching {
                    receipt_id: request.owned_month_id.clone(),
                    returned_ids: returned,
                }
            }
            Err(RemoteError::Invalid | RemoteError::Blocked(_)) => InspectionOutcome::Conflict,
            Err(RemoteError::Ambiguous | RemoteError::Pending) => InspectionOutcome::Unavailable,
        }
    }
}

fn owned_blocks(remote: &[RemoteBlock], request: &NotionReplaceRequest) -> Vec<NotionBlock> {
    remote
        .iter()
        .filter(|item| request.owned_block_ids.contains(&item.id))
        .map(|item| item.block.clone())
        .collect()
}

/// Where an owned section stands relative to its receipt and the wanted body.
#[derive(Clone, Debug, Eq, PartialEq)]
enum OwnedSection {
    /// Heading and body still match the receipt exactly; nothing replaced.
    Original { body_ids: Vec<String> },
    /// `fresh` leading body blocks are the wanted body (possibly only a
    /// prefix) and `stale` receipted body blocks have not been trashed yet.
    Replacing {
        heading_id: String,
        fresh_ids: Vec<String>,
        stale_ids: Vec<String>,
    },
}

impl NotionClient {
    fn read_owned_section(
        &self,
        request: &NotionReplaceRequest,
        plan: &NotionPlan,
        cancelled: &AtomicBool,
    ) -> Result<(OwnedSection, Vec<RemoteBlock>), RemoteError> {
        let month = self
            .resolve_path(plan, cancelled)?
            .month
            .ok_or(RemoteError::Invalid)?;
        if month.id != request.owned_month_id {
            return Err(RemoteError::Invalid);
        }
        let remote = self.remote.list_blocks(&month.id, cancelled)?;
        let heading_id = request
            .owned_block_ids
            .first()
            .ok_or(RemoteError::Invalid)?;
        let index = remote
            .iter()
            .position(|item| &item.id == heading_id)
            .ok_or(RemoteError::Invalid)?;
        let end = remote
            .iter()
            .enumerate()
            .skip(index + 1)
            .find(|(_, item)| matches!(item.block.kind, BlockKind::Heading1 | BlockKind::Heading2))
            .map_or(remote.len(), |(position, _)| position);
        let section = &remote[index..end];
        if section[0].block.display_text() != plan.heading {
            return Err(RemoteError::Invalid);
        }
        let owned_body = &request.owned_block_ids[1..];
        let body = &section[1..];
        let (fresh, stale): (Vec<_>, Vec<_>) =
            body.iter().partition(|item| !owned_body.contains(&item.id));
        let wanted = &plan.blocks[1..];
        if fresh.is_empty() {
            let ids: Vec<_> = body.iter().map(|item| item.id.clone()).collect();
            let blocks: Vec<_> = section.iter().map(|item| item.block.clone()).collect();
            let digest = plan_digest(
                RENDERER_VERSION,
                API_VERSION,
                &plan.year_title,
                &plan.month_title,
                &plan.heading,
                &blocks,
            )
            .map_err(|_| RemoteError::Invalid)?;
            if ids != owned_body || digest != request.owned_payload_digest.as_str() {
                return Err(RemoteError::Invalid);
            }
            return Ok((OwnedSection::Original { body_ids: ids }, remote));
        }
        // Every receipted body block that remains must still be in receipt
        // order, and the inserted blocks must come first and be a prefix of
        // the wanted body.
        let stale_ids: Vec<_> = stale.iter().map(|item| item.id.clone()).collect();
        let order_ok = stale_ids
            .iter()
            .filter_map(|id| owned_body.iter().position(|owned| owned == id))
            .collect::<Vec<_>>()
            .windows(2)
            .all(|pair| pair[0] < pair[1]);
        let fresh_first = body
            .iter()
            .take(fresh.len())
            .all(|item| !owned_body.contains(&item.id));
        if !order_ok
            || !fresh_first
            || fresh.len() > wanted.len()
            || fresh
                .iter()
                .map(|item| &item.block)
                .ne(wanted[..fresh.len()].iter())
        {
            return Err(RemoteError::Invalid);
        }
        Ok((
            OwnedSection::Replacing {
                heading_id: heading_id.clone(),
                fresh_ids: fresh.iter().map(|item| item.id.clone()).collect(),
                stale_ids,
            },
            remote,
        ))
    }

    fn replace_plan(request: &NotionReplaceRequest) -> Result<NotionPlan, WriteOutcome> {
        let plan = render_notion(&request.day, &request.markdown)
            .map_err(|_| WriteOutcome::Unavailable)?;
        if plan.payload_digest != request.expected_payload_digest.as_str() {
            return Err(WriteOutcome::Conflict);
        }
        Ok(plan)
    }

    fn trash_stale(&self, stale_ids: &[String], cancelled: &AtomicBool) -> Result<(), RemoteError> {
        for id in stale_ids {
            self.remote.trash_block(id, cancelled)?;
        }
        Ok(())
    }
}

pub(super) struct ResolvedPath {
    year: Option<NotionPage>,
    pub(super) month: Option<NotionPage>,
}

fn map_hierarchy_error(_: HierarchyError) -> RemoteError {
    RemoteError::Blocked(SetupReason::HierarchyAmbiguous)
}

pub(super) fn remote_reason(error: RemoteError) -> String {
    match error {
        RemoteError::Blocked(reason) => reason.as_str().to_owned(),
        RemoteError::Ambiguous => "notion-write-ambiguous".to_owned(),
        RemoteError::Pending => "notion-retry-deferred".to_owned(),
        RemoteError::Invalid => "notion-response-invalid".to_owned(),
    }
}

pub(super) fn flatten_preview(blocks: &[NotionBlock]) -> Vec<NotionBlockPreview> {
    fn append(output: &mut Vec<NotionBlockPreview>, blocks: &[NotionBlock], depth: u8) {
        for block in blocks {
            output.push(NotionBlockPreview {
                kind: block.kind.as_str().to_owned(),
                text: block.display_text(),
                depth,
            });
            append(output, &block.children, depth.saturating_add(1));
        }
    }
    let mut output = Vec::new();
    append(&mut output, blocks, 0);
    output
}
