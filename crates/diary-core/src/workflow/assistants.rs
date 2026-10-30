//! Workflow assistants rules.

use super::*;

impl Workflow {
    #[must_use]
    pub fn assistant_actions_available(&self) -> bool {
        !self.snapshots.is_empty()
            && matches!(
                self.state,
                WorkflowState::SourcesConfirmed | WorkflowState::Composing
            )
    }

    pub fn memory_extraction_request(&self) -> Result<MemoryExtractionRequest, WorkflowError> {
        if !self.assistant_actions_available() {
            return Err(WorkflowError::AssistantActionsDisabled);
        }
        MemoryExtractionRequest::new(
            self.snapshots
                .iter()
                .map(|snapshot| snapshot.body.clone())
                .collect(),
        )
        .map_err(|_| WorkflowError::AssistantRequestInvalid)
    }

    #[must_use]
    pub fn memory_candidate_sets(&self) -> &[MemoryCandidateSet] {
        &self.memory_candidate_sets
    }

    #[must_use]
    pub fn reviewed_memory_artifact(&self) -> Option<&ReviewedMemoryArtifact> {
        self.reviewed_memory_artifact.as_ref()
    }

    #[must_use]
    pub fn draft_proposals(&self) -> &[DraftProposal] {
        &self.draft_proposals
    }

    #[must_use]
    pub fn paragraph_suggestions(&self) -> &[ParagraphSuggestion] {
        &self.paragraph_suggestions
    }

    #[must_use]
    pub fn source_refine_suggestion(
        &self,
        source: &crate::SourceRef,
    ) -> Option<&SourceRefineSuggestion> {
        self.source_refine_suggestions
            .iter()
            .find(|value| &value.source == source)
    }

    #[must_use]
    pub fn source_refine_suggestions(&self) -> &[SourceRefineSuggestion] {
        &self.source_refine_suggestions
    }

    pub fn record_source_refine_suggestion(
        &mut self,
        source: &crate::SourceRef,
        original_body: String,
        suggested_body: String,
        binding_digest: Digest,
        provenance: ArtifactProvenance,
        now: Timestamp,
    ) -> Result<(), WorkflowError> {
        if !self
            .candidates
            .iter()
            .any(|candidate| &candidate.source == source)
        {
            return Err(WorkflowError::UnknownSource);
        }
        self.source_refine_suggestions
            .retain(|value| &value.source != source);
        self.source_refine_suggestions.push(SourceRefineSuggestion {
            source: source.clone(),
            original_body,
            suggested_body,
            binding_digest,
            provenance,
            created_at: now,
        });
        self.bump(now);
        Ok(())
    }

    pub fn take_source_refine_suggestion(
        &mut self,
        expected_revision: u64,
        source: &crate::SourceRef,
        binding_digest: &Digest,
        accept: bool,
        now: Timestamp,
    ) -> Result<Option<String>, WorkflowError> {
        self.expect_revision(expected_revision)?;
        let index = self
            .source_refine_suggestions
            .iter()
            .position(|value| &value.source == source)
            .ok_or(WorkflowError::AssistantArtifactNotFound)?;
        if &self.source_refine_suggestions[index].binding_digest != binding_digest {
            return Err(WorkflowError::AssistantBindingStale);
        }
        let suggestion = self.source_refine_suggestions.remove(index);
        self.bump(now);
        Ok(accept.then_some(suggestion.suggested_body))
    }

    pub fn source_body_changed(
        &mut self,
        expected_revision: u64,
        source: &crate::SourceRef,
        now: Timestamp,
    ) -> Result<(), WorkflowError> {
        self.expect_revision(expected_revision)?;
        if !self
            .candidates
            .iter()
            .any(|candidate| &candidate.source == source)
        {
            return Err(WorkflowError::UnknownSource);
        }
        self.source_refine_suggestions
            .retain(|value| &value.source != source);
        if self
            .snapshots
            .iter()
            .any(|snapshot| &snapshot.source == source)
        {
            self.invalidate_sources(InvalidationReason::SourceChanged(source.clone()), now);
        } else {
            self.bump(now);
        }
        Ok(())
    }

    pub fn source_deleted(
        &mut self,
        expected_revision: u64,
        source: &crate::SourceRef,
        now: Timestamp,
    ) -> Result<(), WorkflowError> {
        self.expect_revision(expected_revision)?;
        if !self
            .candidates
            .iter()
            .any(|candidate| &candidate.source == source)
        {
            return Err(WorkflowError::UnknownSource);
        }
        let was_selected = self
            .snapshots
            .iter()
            .any(|snapshot| &snapshot.source == source);
        self.candidates
            .retain(|candidate| &candidate.source != source);
        self.source_refine_suggestions
            .retain(|value| &value.source != source);
        if was_selected {
            self.invalidate_sources(InvalidationReason::SourceChanged(source.clone()), now);
        } else {
            self.bump(now);
        }
        Ok(())
    }

    pub fn commit_memory_extraction(
        &mut self,
        artifact_id: String,
        provenance: ArtifactProvenance,
        captured_source_digest: &Digest,
        result: &MemoryExtractionResult,
        now: Timestamp,
    ) -> Result<(), WorkflowError> {
        validate_artifact_id(&artifact_id)?;
        if self.selection_digest.as_ref() != Some(captured_source_digest) {
            return Err(WorkflowError::AssistantBindingStale);
        }
        let candidates = result
            .candidates()
            .iter()
            .enumerate()
            .map(|(index, candidate)| {
                let source_snapshot_digests = candidate
                    .source_ordinals()
                    .iter()
                    .map(|ordinal| {
                        self.snapshots
                            .get(usize::from(*ordinal).saturating_sub(1))
                            .map(|snapshot| snapshot.stamp.content_digest.clone())
                            .ok_or(WorkflowError::AssistantRequestInvalid)
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(MemoryCandidateArtifact {
                    id: format!("{artifact_id}-{}", index + 1),
                    text: candidate.text().to_owned(),
                    source_snapshot_digests,
                })
            })
            .collect::<Result<Vec<_>, WorkflowError>>()?;
        let mut digest_input = Vec::new();
        for candidate in &candidates {
            push_framed(&mut digest_input, candidate.text.as_bytes());
            for digest in &candidate.source_snapshot_digests {
                push_framed(&mut digest_input, digest.as_str().as_bytes());
            }
        }
        for set in &mut self.memory_candidate_sets {
            set.stale = true;
        }
        if let Some(reviewed) = &mut self.reviewed_memory_artifact {
            reviewed.stale = true;
        }
        self.memory_candidate_sets.push(MemoryCandidateSet {
            id: artifact_id,
            provenance,
            created_at: now,
            content_digest: Digest::of_bytes(&digest_input),
            binding: MemoryCandidateBinding {
                source_selection_digest: captured_source_digest.clone(),
            },
            candidates,
            stale: false,
        });
        self.bump(now);
        Ok(())
    }

    pub fn replace_reviewed_memory(
        &mut self,
        expected_revision: u64,
        artifact_id: String,
        candidate_set_id: &str,
        text: String,
        now: Timestamp,
    ) -> Result<(), WorkflowError> {
        self.expect_revision(expected_revision)?;
        validate_artifact_id(&artifact_id)?;
        if text.len() > crate::MAX_REVIEWED_MEMORY_BYTES {
            return Err(WorkflowError::AssistantRequestInvalid);
        }
        let candidate_set = self
            .memory_candidate_sets
            .iter()
            .find(|set| set.id == candidate_set_id)
            .ok_or(WorkflowError::AssistantArtifactNotFound)?;
        if candidate_set.stale
            || Some(&candidate_set.binding.source_selection_digest)
                != self.selection_digest.as_ref()
        {
            return Err(WorkflowError::AssistantBindingStale);
        }
        let revision = self
            .reviewed_memory_artifact
            .as_ref()
            .map_or(0, |artifact| artifact.revision.saturating_add(1));
        self.reviewed_memory_artifact = Some(ReviewedMemoryArtifact {
            id: artifact_id,
            provenance: Some(candidate_set.provenance.clone()),
            created_at: now,
            content_digest: Digest::of_bytes(text.as_bytes()),
            binding: Some(ReviewedMemoryBinding {
                candidate_set_digest: candidate_set.content_digest.clone(),
            }),
            revision,
            text,
            stale: false,
        });
        for draft in &mut self.draft_proposals {
            draft.stale = true;
        }
        self.bump(now);
        Ok(())
    }

    pub fn draft_entry_request(
        &self,
        include_prior_draft: bool,
    ) -> Result<(DraftEntryRequest, DraftProposalBinding), WorkflowError> {
        let selection = self
            .assistant_selection
            .as_ref()
            .ok_or(WorkflowError::AssistantBackendRequired)?;
        let source_selection_digest = self
            .selection_digest
            .clone()
            .ok_or(WorkflowError::AssistantActionsDisabled)?;
        let reviewed = self
            .reviewed_memory_artifact
            .as_ref()
            .filter(|artifact| !artifact.stale)
            .ok_or(WorkflowError::ReviewedMemoryRequired)?;
        let language = self.language.ok_or(WorkflowError::LanguageRequired)?;
        // Stored draft text carries inferred-reflection markers, which the
        // request rejects; the markers are single characters inserted at char
        // boundaries, so removing them recovers the assistant's plain text.
        let prior_draft = include_prior_draft
            .then(|| self.draft_proposals.last())
            .flatten()
            .map(|draft| {
                draft
                    .text
                    .chars()
                    .filter(|c| !matches!(*c, INFERRED_REFLECTION_OPEN | INFERRED_REFLECTION_CLOSE))
                    .collect::<String>()
            });
        let request =
            DraftEntryRequest::new(reviewed.text.clone(), prior_draft, language, self.style)
                .map_err(|_| WorkflowError::AssistantDraftRequestInvalid)?;
        Ok((
            request,
            DraftProposalBinding {
                source_selection_digest,
                language,
                style: self.style,
                backend_id: selection.backend_id.clone(),
                reviewed_memory_digest: reviewed.content_digest.clone(),
                reviewed_memory_revision: reviewed.revision,
                captured_buffer_version: self.entry.version,
            },
        ))
    }

    pub fn commit_draft_proposal(
        &mut self,
        artifact_id: String,
        provenance: ArtifactProvenance,
        binding: DraftProposalBinding,
        result: &DraftEntryResult,
        now: Timestamp,
    ) -> Result<bool, WorkflowError> {
        validate_artifact_id(&artifact_id)?;
        let rendered = result
            .rendered_text()
            .map_err(|_| WorkflowError::AssistantDraftResultInvalid)?;
        let fresh = self.draft_binding_matches(&binding);
        let populated_buffer_version = if fresh {
            self.entry.text = rendered.clone();
            self.entry.version = self.entry.version.saturating_add(1);
            self.entry.confirmed = None;
            Some(self.entry.version)
        } else {
            None
        };
        self.draft_proposals.push(DraftProposal {
            id: artifact_id,
            provenance: Some(provenance),
            created_at: now,
            content_digest: Digest::of_bytes(rendered.as_bytes()),
            binding: Some(binding),
            text: rendered,
            reflection_ranges: result.reflection_ranges().to_vec(),
            stale: !fresh,
            populated_buffer_version,
        });
        self.assistant_draft = self.draft_proposals.last().map(|draft| draft.text.clone());
        self.state = WorkflowState::Composing;
        self.bump(now);
        Ok(fresh)
    }

    pub(super) fn draft_binding_matches(&self, binding: &DraftProposalBinding) -> bool {
        self.selection_digest.as_ref() == Some(&binding.source_selection_digest)
            && self.language == Some(binding.language)
            && self.style == binding.style
            && self
                .assistant_selection
                .as_ref()
                .is_some_and(|selection| selection.backend_id == binding.backend_id)
            && self.entry.version == binding.captured_buffer_version
            && self
                .reviewed_memory_artifact
                .as_ref()
                .is_some_and(|reviewed| {
                    !reviewed.stale
                        && reviewed.revision == binding.reviewed_memory_revision
                        && reviewed.content_digest == binding.reviewed_memory_digest
                })
    }

    pub fn paragraph_review_request(
        &self,
        range: ReflectionRange,
    ) -> Result<(ParagraphReviewRequest, ParagraphSuggestionBinding), WorkflowError> {
        if range.start >= range.end
            || range.end > self.entry.text.len()
            || !self.entry.text.is_char_boundary(range.start)
            || !self.entry.text.is_char_boundary(range.end)
        {
            return Err(WorkflowError::AssistantParagraphRangeInvalid);
        }
        let paragraph = &self.entry.text[range.start..range.end];
        let language = self.language.ok_or(WorkflowError::LanguageRequired)?;
        let selection = self
            .assistant_selection
            .as_ref()
            .ok_or(WorkflowError::AssistantBackendRequired)?;
        // A drafted entry buffer still carries inferred-reflection markers,
        // which the request rejects. The binding keeps buffer coordinates and
        // the digest of the marked slice; only the text sent drops the markers.
        let plain: String = paragraph
            .chars()
            .filter(|c| !matches!(*c, INFERRED_REFLECTION_OPEN | INFERRED_REFLECTION_CLOSE))
            .collect();
        let request = ParagraphReviewRequest::new(plain, language, self.style)
            .map_err(|_| WorkflowError::AssistantParagraphRequestInvalid)?;
        Ok((
            request,
            ParagraphSuggestionBinding {
                source_selection_digest: self
                    .selection_digest
                    .clone()
                    .ok_or(WorkflowError::AssistantActionsDisabled)?,
                language,
                style: self.style,
                backend_id: selection.backend_id.clone(),
                buffer_version: self.entry.version,
                range,
                selected_text_digest: Digest::of_bytes(paragraph.as_bytes()),
            },
        ))
    }

    pub fn commit_paragraph_suggestion(
        &mut self,
        suggestion_id: String,
        provenance: ArtifactProvenance,
        binding: ParagraphSuggestionBinding,
        result: &ParagraphReviewResult,
        now: Timestamp,
    ) -> Result<(), WorkflowError> {
        validate_artifact_id(&suggestion_id)?;
        let replacement = result
            .rendered_replacement()
            .map_err(|_| WorkflowError::AssistantParagraphResultInvalid)?;
        let stale = !self.paragraph_binding_matches(&binding);
        self.paragraph_suggestions.push(ParagraphSuggestion {
            id: suggestion_id,
            provenance,
            created_at: now,
            content_digest: Digest::of_bytes(replacement.as_bytes()),
            binding,
            replacement,
            reflection_ranges: result.reflection_ranges().to_vec(),
            explanation: result.explanation().map(str::to_owned),
            stale,
            resolution: None,
            resolved_at: None,
        });
        self.bump(now);
        Ok(())
    }

    pub fn resolve_paragraph_suggestion(
        &mut self,
        expected_revision: u64,
        suggestion_id: &str,
        resolution: SuggestionResolution,
        now: Timestamp,
    ) -> Result<(), WorkflowError> {
        self.expect_revision(expected_revision)?;
        let index = self
            .paragraph_suggestions
            .iter()
            .position(|suggestion| suggestion.id == suggestion_id)
            .ok_or(WorkflowError::AssistantArtifactNotFound)?;
        if self.paragraph_suggestions[index].resolution.is_some() {
            return Err(WorkflowError::AssistantSuggestionAlreadyResolved);
        }
        let binding = self.paragraph_suggestions[index].binding.clone();
        if self.paragraph_suggestions[index].stale || !self.paragraph_binding_matches(&binding) {
            return Err(WorkflowError::AssistantBindingStale);
        }
        if resolution == SuggestionResolution::Accepted {
            self.entry.text.replace_range(
                binding.range.start..binding.range.end,
                &self.paragraph_suggestions[index].replacement,
            );
            self.entry.version = self.entry.version.saturating_add(1);
            for (other, suggestion) in self.paragraph_suggestions.iter_mut().enumerate() {
                if other != index {
                    suggestion.stale = true;
                }
            }
        }
        self.paragraph_suggestions[index].resolution = Some(resolution);
        self.paragraph_suggestions[index].resolved_at = Some(now);
        self.bump(now);
        Ok(())
    }

    pub(super) fn paragraph_binding_matches(&self, binding: &ParagraphSuggestionBinding) -> bool {
        self.selection_digest.as_ref() == Some(&binding.source_selection_digest)
            && self.language == Some(binding.language)
            && self.style == binding.style
            && self
                .assistant_selection
                .as_ref()
                .is_some_and(|selection| selection.backend_id == binding.backend_id)
            && self.entry.version == binding.buffer_version
            && self
                .entry
                .text
                .get(binding.range.start..binding.range.end)
                .is_some_and(|text| {
                    Digest::of_bytes(text.as_bytes()) == binding.selected_text_digest
                })
    }
}
