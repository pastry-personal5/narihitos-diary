//! Sources use cases.

use super::*;

impl CommandService {
    pub fn discover(
        &self,
        day: &DiaryDay,
        expected_revision: u64,
        explicit_date: Option<DiaryDay>,
    ) -> Result<Workflow, CommandError> {
        self.discover_guarded(day, expected_revision, explicit_date, || true)
    }

    pub fn discover_guarded(
        &self,
        day: &DiaryDay,
        expected_revision: u64,
        explicit_date: Option<DiaryDay>,
        commit: impl FnOnce() -> bool,
    ) -> Result<Workflow, CommandError> {
        self.ensure_clear(day)?;
        let mut workflow = self.load_workflow(day)?;
        ensure_revision(&workflow, expected_revision)?;
        let requested = explicit_date.clone();
        let request = explicit_date
            .map(CatalogRequest::ExplicitDate)
            .unwrap_or_else(|| CatalogRequest::Automatic(day.clone()));
        let mut discovery = self
            .catalog
            .discover(request)
            .map_err(CommandError::Source)?;
        if let Some(requested) = &requested {
            discovery
                .candidates
                .retain(|candidate| on_local_day(candidate, requested));
        }
        if discovery.candidates.len() > diary_control_contract::MAX_CANDIDATES {
            return Err(CommandError::LimitExceeded);
        }
        workflow
            .discover(expected_revision, discovery.candidates, self.clock.now())
            .map_err(CommandError::Workflow)?;
        if !commit() {
            return Err(CommandError::Cancelled);
        }
        self.store
            .compare_and_swap(expected_revision, None, &workflow)
            .map_err(CommandError::Store)?;
        Ok(workflow)
    }

    pub fn prepare_sources(
        &self,
        day: &DiaryDay,
        expected_revision: u64,
        sources: &[SourceRef],
    ) -> Result<Vec<SelectedSource>, CommandError> {
        self.ensure_clear(day)?;
        let workflow = self.load_workflow(day)?;
        ensure_revision(&workflow, expected_revision)?;
        if sources.is_empty() {
            return Err(CommandError::EmptySelection);
        }
        if sources.len() > diary_control_contract::MAX_SELECTED_SOURCES {
            return Err(CommandError::LimitExceeded);
        }
        for source in sources {
            if !workflow
                .candidates()
                .iter()
                .any(|candidate| &candidate.source == source)
            {
                return Err(CommandError::Workflow(WorkflowError::UnknownSource));
            }
        }
        let mut selected = Vec::with_capacity(sources.len());
        let mut total_bytes = 0_usize;
        for source in sources {
            let mut outcome = self
                .reader
                .read_selected(source)
                .map_err(CommandError::Source)?;
            if matches!(outcome, SourceReadOutcome::NeedsDownload { .. }) {
                self.reader
                    .request_download(source)
                    .map_err(CommandError::Source)?;
                outcome = self
                    .reader
                    .read_selected(source)
                    .map_err(CommandError::Source)?;
            }
            let item = match outcome {
                SourceReadOutcome::Read(item) => item,
                SourceReadOutcome::NeedsDownload { .. } => {
                    return Err(CommandError::Source(SourceAccessError::NeedsDownload));
                }
                SourceReadOutcome::PermissionDenied { .. } => {
                    return Err(CommandError::Source(SourceAccessError::PermissionDenied));
                }
                SourceReadOutcome::Unreadable { .. } => {
                    return Err(CommandError::Workflow(WorkflowError::UnreadableSource));
                }
                SourceReadOutcome::Unverifiable { .. } => {
                    return Err(CommandError::SourcesInvalidated);
                }
            };
            total_bytes = total_bytes.saturating_add(item.snapshot.body.len());
            if total_bytes > diary_control_contract::MAX_PREPARED_BYTES {
                return Err(CommandError::LimitExceeded);
            }
            match self
                .reader
                .revalidate(&item.snapshot)
                .map_err(CommandError::Source)?
            {
                FreshnessOutcome::Fresh => selected.push(item),
                _ => return Err(CommandError::SourcesInvalidated),
            }
        }
        let current = self.load_workflow(day)?;
        ensure_revision(&current, expected_revision)?;
        Ok(selected)
    }

    pub fn confirm_selection(
        &self,
        day: &DiaryDay,
        expected_revision: u64,
        selected: Vec<SelectedSource>,
    ) -> Result<Workflow, CommandError> {
        self.ensure_clear(day)?;
        if selected.is_empty() {
            return Err(CommandError::EmptySelection);
        }
        let mut workflow = self.load_workflow(day)?;
        ensure_revision(&workflow, expected_revision)?;
        for item in &selected {
            match self
                .reader
                .revalidate(&item.snapshot)
                .map_err(CommandError::Source)?
            {
                FreshnessOutcome::Fresh => {}
                _ => return Err(CommandError::SourcesInvalidated),
            }
        }
        workflow
            .confirm_selected_sources(expected_revision, selected, false, self.clock.now())
            .map_err(CommandError::Workflow)?;
        self.store
            .compare_and_swap(expected_revision, None, &workflow)
            .map_err(CommandError::Store)?;
        Ok(workflow)
    }

    pub fn confirm_empty_selection(
        &self,
        day: &DiaryDay,
        expected_revision: u64,
    ) -> Result<Workflow, CommandError> {
        self.ensure_clear(day)?;
        let mut workflow = self.load_workflow(day)?;
        ensure_revision(&workflow, expected_revision)?;
        workflow
            .confirm_selected_sources(expected_revision, Vec::new(), true, self.clock.now())
            .map_err(CommandError::Workflow)?;
        self.store
            .compare_and_swap(expected_revision, None, &workflow)
            .map_err(CommandError::Store)?;
        Ok(workflow)
    }
}
