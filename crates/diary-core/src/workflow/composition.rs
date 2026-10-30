//! Workflow composition rules.

use super::*;

impl Workflow {
    pub fn begin_manual_composition(
        &mut self,
        expected_revision: u64,
        now: Timestamp,
    ) -> Result<(), WorkflowError> {
        self.expect_revision(expected_revision)?;
        if !matches!(
            self.state,
            WorkflowState::SourcesConfirmed | WorkflowState::Composing
        ) {
            return Err(WorkflowError::WrongState);
        }
        self.advance(WorkflowState::Composing, now);
        Ok(())
    }

    pub fn set_language(
        &mut self,
        expected_revision: u64,
        language: WritingLanguage,
        now: Timestamp,
    ) -> Result<(), WorkflowError> {
        self.expect_revision(expected_revision)?;
        self.ensure_writing_settings_editable()?;
        if self.language == Some(language) {
            return Ok(());
        }
        self.language = Some(language);
        self.invalidate_composition(now);
        Ok(())
    }

    pub fn set_style(
        &mut self,
        expected_revision: u64,
        style: WritingStyle,
        now: Timestamp,
    ) -> Result<(), WorkflowError> {
        self.expect_revision(expected_revision)?;
        self.ensure_writing_settings_editable()?;
        if self.style == style {
            return Ok(());
        }
        self.style = style;
        self.invalidate_composition(now);
        Ok(())
    }

    pub fn set_assistant_selection(
        &mut self,
        expected_revision: u64,
        selection: Option<AssistantSelection>,
        now: Timestamp,
    ) -> Result<(), WorkflowError> {
        self.expect_revision(expected_revision)?;
        self.ensure_writing_settings_editable()?;
        if self.assistant_selection == selection {
            return Ok(());
        }
        self.assistant_selection = selection;
        self.invalidate_composition(now);
        Ok(())
    }

    pub fn set_writing_settings(
        &mut self,
        expected_revision: u64,
        language: WritingLanguage,
        style: WritingStyle,
        assistant_selection: Option<AssistantSelection>,
        now: Timestamp,
    ) -> Result<(), WorkflowError> {
        self.expect_revision(expected_revision)?;
        self.ensure_writing_settings_editable()?;
        if self.language == Some(language)
            && self.style == style
            && self.assistant_selection == assistant_selection
        {
            return Ok(());
        }
        self.language = Some(language);
        self.style = style;
        self.assistant_selection = assistant_selection;
        self.invalidate_composition(now);
        Ok(())
    }

    pub fn save_entry(
        &mut self,
        expected_buffer_version: u64,
        text: String,
        now: Timestamp,
    ) -> Result<(), WorkflowError> {
        self.ensure_entry_editable()?;
        if self.entry.version != expected_buffer_version {
            return Err(WorkflowError::StaleBuffer {
                expected: expected_buffer_version,
                actual: self.entry.version,
            });
        }
        if self.entry.text == text {
            return Ok(());
        }
        let invalidates_confirmation = self.entry.confirmed.is_some();
        self.entry.text = text;
        self.entry.version += 1;
        for suggestion in &mut self.paragraph_suggestions {
            suggestion.stale = true;
        }
        self.updated_at = now;
        if invalidates_confirmation {
            self.entry.confirmed = None;
            self.clear_plan_and_approval();
            self.state = WorkflowState::Composing;
            self.invalidation_reason = Some(InvalidationReason::EntryEdited);
            self.revision += 1;
        }
        Ok(())
    }

    pub fn confirm_entry(
        &mut self,
        expected_revision: u64,
        expected_buffer_version: u64,
        now: Timestamp,
    ) -> Result<(), WorkflowError> {
        self.expect_revision(expected_revision)?;
        if self.entry.version != expected_buffer_version {
            return Err(WorkflowError::StaleBuffer {
                expected: expected_buffer_version,
                actual: self.entry.version,
            });
        }
        if !matches!(
            self.state,
            WorkflowState::SourcesConfirmed | WorkflowState::Composing
        ) {
            return Err(WorkflowError::WrongState);
        }
        if self.language.is_none() {
            return Err(WorkflowError::LanguageRequired);
        }
        if self.entry.text.trim().is_empty() {
            return Err(WorkflowError::BlankEntry);
        }
        if self.entry.text.contains(INFERRED_REFLECTION_OPEN)
            || self.entry.text.contains(INFERRED_REFLECTION_CLOSE)
        {
            return Err(WorkflowError::UnresolvedInferenceMarker);
        }
        self.entry.confirmed = Some(EntryConfirmation {
            buffer_version: self.entry.version,
            entry_digest: Digest::of_bytes(self.entry.text.as_bytes()),
            confirmed_at: now,
        });
        self.clear_plan_and_approval();
        self.advance(WorkflowState::EntryConfirmed, now);
        Ok(())
    }
}
