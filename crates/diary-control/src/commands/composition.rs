//! Composition use cases.

use super::*;

impl CommandService {
    pub fn begin_manual_composition(
        &self,
        day: &DiaryDay,
        expected_revision: u64,
    ) -> Result<Workflow, CommandError> {
        self.ensure_clear(day)?;
        let mut workflow = self.load_workflow(day)?;
        ensure_revision(&workflow, expected_revision)?;
        workflow
            .begin_manual_composition(expected_revision, self.clock.now())
            .map_err(CommandError::Workflow)?;
        self.store
            .compare_and_swap(expected_revision, None, &workflow)
            .map_err(CommandError::Store)?;
        Ok(workflow)
    }

    pub fn save_entry(
        &self,
        day: &DiaryDay,
        expected_buffer: u64,
        text: String,
    ) -> Result<Workflow, CommandError> {
        self.ensure_clear(day)?;
        if text.len() > diary_control_contract::MAX_ENTRY_BYTES {
            return Err(CommandError::LimitExceeded);
        }
        for attempt in 0..2 {
            let mut workflow = self.load_workflow(day)?;
            if workflow.entry().version != expected_buffer {
                return Err(CommandError::PreconditionFailed);
            }
            let revision = workflow.revision();
            workflow
                .save_entry(expected_buffer, text.clone(), self.clock.now())
                .map_err(CommandError::Workflow)?;
            match self
                .store
                .compare_and_swap(revision, Some(expected_buffer), &workflow)
            {
                Ok(()) => return Ok(workflow),
                Err(StoreError::StaleRevision) if attempt == 0 => continue,
                Err(StoreError::StaleBuffer | StoreError::StaleRevision) => {
                    return Err(CommandError::PreconditionFailed);
                }
                Err(error) => return Err(CommandError::Store(error)),
            }
        }
        Err(CommandError::PreconditionFailed)
    }

    pub fn confirm_entry(
        &self,
        day: &DiaryDay,
        expected_revision: u64,
        expected_buffer: u64,
    ) -> Result<Workflow, CommandError> {
        self.ensure_clear(day)?;
        let mut workflow = self.load_workflow(day)?;
        ensure_revision(&workflow, expected_revision)?;
        if workflow.entry().version != expected_buffer {
            return Err(CommandError::PreconditionFailed);
        }
        let before_revision = workflow.revision();
        let freshness = revalidate_workflow_sources(
            &mut workflow,
            self.reader.as_ref(),
            FreshnessCheckpoint::BeforeEntryConfirmation,
            self.clock.now(),
        );
        if freshness.is_err() || workflow.revision() != before_revision {
            if workflow.revision() != before_revision {
                self.store
                    .compare_and_swap(before_revision, Some(expected_buffer), &workflow)
                    .map_err(CommandError::Store)?;
            }
            return Err(CommandError::SourcesInvalidated);
        }
        workflow
            .confirm_entry(expected_revision, expected_buffer, self.clock.now())
            .map_err(CommandError::Workflow)?;
        self.store
            .compare_and_swap(expected_revision, Some(expected_buffer), &workflow)
            .map_err(CommandError::Store)?;
        Ok(workflow)
    }

    pub fn now(&self) -> Timestamp {
        self.clock.now()
    }
}
