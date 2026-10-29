//! Provider-neutral assistant requests, structured results, and artifact bindings.

use std::collections::BTreeSet;

use crate::{
    AssistantBackendId, CompositionPath, Digest, Timestamp, ValueError, WritingLanguage,
    WritingStyle,
};

pub const MAX_MEMORY_CANDIDATES: usize = 128;
pub const MAX_MEMORY_CANDIDATE_BYTES: usize = 4 * 1024;
pub const MAX_REVIEWED_MEMORY_BYTES: usize = 512 * 1024;
pub const MAX_SOURCE_ORDINALS: usize = 64;
pub const MAX_DRAFT_RESULT_BYTES: usize = 512 * 1024;
pub const MAX_PARAGRAPH_BYTES: usize = 32 * 1024;
pub const MAX_SUGGESTION_EXPLANATION_BYTES: usize = 4 * 1024;
pub const MAX_REFLECTION_RANGES: usize = 32;
const MAX_SOURCE_BODY_BYTES: usize = 8 * 1024 * 1024;
const MAX_SOURCE_BODY_TOTAL_BYTES: usize = 16 * 1024 * 1024;
const REFLECTION_OPEN: char = '⟪';
const REFLECTION_CLOSE: char = '⟫';

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemoryExtractionSource {
    ordinal: u16,
    body: String,
}

impl MemoryExtractionSource {
    #[must_use]
    pub const fn ordinal(&self) -> u16 {
        self.ordinal
    }

    #[must_use]
    pub fn body(&self) -> &str {
        &self.body
    }
}

/// Extraction contains only request-local ordinals and confirmed readable bodies.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemoryExtractionRequest {
    sources: Vec<MemoryExtractionSource>,
}

impl MemoryExtractionRequest {
    pub fn new(bodies: Vec<String>) -> Result<Self, ValueError> {
        if bodies.is_empty() || bodies.len() > MAX_SOURCE_ORDINALS {
            return Err(ValueError::new("invalid extraction source count"));
        }
        let mut total = 0_usize;
        let mut sources = Vec::with_capacity(bodies.len());
        for (index, body) in bodies.into_iter().enumerate() {
            if body.is_empty() || body.len() > MAX_SOURCE_BODY_BYTES {
                return Err(ValueError::new("invalid extraction source body"));
            }
            total = total
                .checked_add(body.len())
                .ok_or_else(|| ValueError::new("extraction source bodies are too large"))?;
            if total > MAX_SOURCE_BODY_TOTAL_BYTES {
                return Err(ValueError::new("extraction source bodies are too large"));
            }
            sources.push(MemoryExtractionSource {
                ordinal: u16::try_from(index + 1)
                    .map_err(|_| ValueError::new("invalid extraction source ordinal"))?,
                body,
            });
        }
        Ok(Self { sources })
    }

    #[must_use]
    pub fn sources(&self) -> &[MemoryExtractionSource] {
        &self.sources
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemoryCandidate {
    text: String,
    source_ordinals: Vec<u16>,
}

impl MemoryCandidate {
    pub fn new(
        text: impl Into<String>,
        source_ordinals: Vec<u16>,
        source_count: usize,
    ) -> Result<Self, ValueError> {
        let text = text.into();
        if text.is_empty() || text.len() > MAX_MEMORY_CANDIDATE_BYTES {
            return Err(ValueError::new("invalid memory candidate text"));
        }
        if source_ordinals.is_empty() || source_ordinals.len() > MAX_SOURCE_ORDINALS {
            return Err(ValueError::new("invalid memory candidate source ordinals"));
        }
        let distinct = source_ordinals.iter().copied().collect::<BTreeSet<_>>();
        if distinct.len() != source_ordinals.len()
            || source_ordinals
                .iter()
                .any(|ordinal| *ordinal == 0 || usize::from(*ordinal) > source_count)
        {
            return Err(ValueError::new("invalid memory candidate source ordinals"));
        }
        Ok(Self {
            text,
            source_ordinals,
        })
    }

    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    #[must_use]
    pub fn source_ordinals(&self) -> &[u16] {
        &self.source_ordinals
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemoryExtractionResult {
    candidates: Vec<MemoryCandidate>,
}

impl MemoryExtractionResult {
    pub fn new(candidates: Vec<MemoryCandidate>) -> Result<Self, ValueError> {
        if candidates.len() > MAX_MEMORY_CANDIDATES {
            return Err(ValueError::new("too many memory candidates"));
        }
        Ok(Self { candidates })
    }

    #[must_use]
    pub fn candidates(&self) -> &[MemoryCandidate] {
        &self.candidates
    }
}

/// Drafting contains no source bodies or identities.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DraftEntryRequest {
    reviewed_memory: String,
    prior_draft: Option<String>,
    language: WritingLanguage,
    style: WritingStyle,
}

impl DraftEntryRequest {
    pub fn new(
        reviewed_memory: impl Into<String>,
        prior_draft: Option<String>,
        language: WritingLanguage,
        style: WritingStyle,
    ) -> Result<Self, ValueError> {
        let reviewed_memory = reviewed_memory.into();
        if reviewed_memory.len() > MAX_REVIEWED_MEMORY_BYTES
            || prior_draft
                .as_ref()
                .is_some_and(|draft| draft.len() > MAX_DRAFT_RESULT_BYTES)
        {
            return Err(ValueError::new("draft request text is too large"));
        }
        reject_reserved_markers(&reviewed_memory)?;
        if let Some(draft) = &prior_draft {
            reject_reserved_markers(draft)?;
        }
        Ok(Self {
            reviewed_memory,
            prior_draft,
            language,
            style,
        })
    }

    #[must_use]
    pub fn reviewed_memory(&self) -> &str {
        &self.reviewed_memory
    }

    #[must_use]
    pub fn prior_draft(&self) -> Option<&str> {
        self.prior_draft.as_deref()
    }

    #[must_use]
    pub const fn language(&self) -> WritingLanguage {
        self.language
    }

    #[must_use]
    pub const fn style(&self) -> WritingStyle {
        self.style
    }
}

/// Paragraph review contains exactly the displayed target and writing settings.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParagraphReviewRequest {
    paragraph: String,
    language: WritingLanguage,
    style: WritingStyle,
}

/// Source refinement contains one readable Ayumi body and no source identity,
/// path, metadata, or audio.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceRefinementRequest {
    body: String,
}

impl SourceRefinementRequest {
    pub fn new(body: impl Into<String>) -> Result<Self, ValueError> {
        let body = body.into();
        if body.is_empty() || body.len() > MAX_SOURCE_BODY_BYTES {
            return Err(ValueError::new("invalid source refinement body"));
        }
        Ok(Self { body })
    }

    #[must_use]
    pub fn body(&self) -> &str {
        &self.body
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceRefinementResult {
    body: String,
}

impl SourceRefinementResult {
    pub fn new(body: impl Into<String>) -> Result<Self, ValueError> {
        let body = body.into();
        if body.is_empty() || body.len() > MAX_SOURCE_BODY_BYTES {
            return Err(ValueError::new("invalid source refinement result"));
        }
        Ok(Self { body })
    }

    #[must_use]
    pub fn body(&self) -> &str {
        &self.body
    }
}

impl ParagraphReviewRequest {
    pub fn new(
        paragraph: impl Into<String>,
        language: WritingLanguage,
        style: WritingStyle,
    ) -> Result<Self, ValueError> {
        let paragraph = paragraph.into();
        if paragraph.is_empty() || paragraph.len() > MAX_PARAGRAPH_BYTES {
            return Err(ValueError::new("invalid paragraph review target"));
        }
        reject_reserved_markers(&paragraph)?;
        Ok(Self {
            paragraph,
            language,
            style,
        })
    }

    #[must_use]
    pub fn paragraph(&self) -> &str {
        &self.paragraph
    }

    #[must_use]
    pub const fn language(&self) -> WritingLanguage {
        self.language
    }

    #[must_use]
    pub const fn style(&self) -> WritingStyle {
        self.style
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReflectionRange {
    pub start: usize,
    pub end: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DraftEntryResult {
    text: String,
    reflection_ranges: Vec<ReflectionRange>,
}

impl DraftEntryResult {
    pub fn new(
        text: impl Into<String>,
        reflection_ranges: Vec<ReflectionRange>,
    ) -> Result<Self, ValueError> {
        let text = text.into();
        validate_structured_text(&text, &reflection_ranges, MAX_DRAFT_RESULT_BYTES)?;
        Ok(Self {
            text,
            reflection_ranges,
        })
    }

    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    #[must_use]
    pub fn reflection_ranges(&self) -> &[ReflectionRange] {
        &self.reflection_ranges
    }

    pub fn rendered_text(&self) -> Result<String, ValueError> {
        insert_reflection_markers(&self.text, &self.reflection_ranges, MAX_DRAFT_RESULT_BYTES)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParagraphReviewResult {
    replacement: String,
    reflection_ranges: Vec<ReflectionRange>,
    explanation: Option<String>,
}

impl ParagraphReviewResult {
    pub fn new(
        replacement: impl Into<String>,
        reflection_ranges: Vec<ReflectionRange>,
        explanation: Option<String>,
    ) -> Result<Self, ValueError> {
        let replacement = replacement.into();
        validate_structured_text(&replacement, &reflection_ranges, MAX_PARAGRAPH_BYTES)?;
        if explanation
            .as_ref()
            .is_some_and(|value| value.len() > MAX_SUGGESTION_EXPLANATION_BYTES)
        {
            return Err(ValueError::new("suggestion explanation is too large"));
        }
        Ok(Self {
            replacement,
            reflection_ranges,
            explanation,
        })
    }

    #[must_use]
    pub fn replacement(&self) -> &str {
        &self.replacement
    }

    #[must_use]
    pub fn reflection_ranges(&self) -> &[ReflectionRange] {
        &self.reflection_ranges
    }

    #[must_use]
    pub fn explanation(&self) -> Option<&str> {
        self.explanation.as_deref()
    }

    pub fn rendered_replacement(&self) -> Result<String, ValueError> {
        insert_reflection_markers(
            &self.replacement,
            &self.reflection_ranges,
            MAX_PARAGRAPH_BYTES,
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AssistantRequest {
    ExtractMemories(MemoryExtractionRequest),
    DraftEntry(DraftEntryRequest),
    ReviewParagraph(ParagraphReviewRequest),
    RefineSource(SourceRefinementRequest),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AssistantResponse {
    Memories(MemoryExtractionResult),
    EntryDraft(DraftEntryResult),
    ParagraphReview(ParagraphReviewResult),
    SourceRefinement(SourceRefinementResult),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactProvenance {
    pub backend_id: AssistantBackendId,
    pub composition_path: CompositionPath,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemoryCandidateBinding {
    pub source_selection_digest: Digest,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewedMemoryBinding {
    pub candidate_set_digest: Digest,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DraftProposalBinding {
    pub source_selection_digest: Digest,
    pub language: WritingLanguage,
    pub style: WritingStyle,
    pub backend_id: AssistantBackendId,
    pub reviewed_memory_digest: Digest,
    pub reviewed_memory_revision: u64,
    pub captured_buffer_version: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParagraphSuggestionBinding {
    pub source_selection_digest: Digest,
    pub language: WritingLanguage,
    pub style: WritingStyle,
    pub backend_id: AssistantBackendId,
    pub buffer_version: u64,
    pub range: ReflectionRange,
    pub selected_text_digest: Digest,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemoryCandidateArtifact {
    pub id: String,
    pub text: String,
    pub source_snapshot_digests: Vec<Digest>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemoryCandidateSet {
    pub id: String,
    pub provenance: ArtifactProvenance,
    pub created_at: Timestamp,
    pub content_digest: Digest,
    pub binding: MemoryCandidateBinding,
    pub candidates: Vec<MemoryCandidateArtifact>,
    pub stale: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewedMemoryArtifact {
    pub id: String,
    pub provenance: Option<ArtifactProvenance>,
    pub created_at: Timestamp,
    pub content_digest: Digest,
    pub binding: Option<ReviewedMemoryBinding>,
    pub revision: u64,
    pub text: String,
    pub stale: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DraftProposal {
    pub id: String,
    pub provenance: Option<ArtifactProvenance>,
    pub created_at: Timestamp,
    pub content_digest: Digest,
    pub binding: Option<DraftProposalBinding>,
    pub text: String,
    pub reflection_ranges: Vec<ReflectionRange>,
    pub stale: bool,
    pub populated_buffer_version: Option<u64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SuggestionResolution {
    Accepted,
    Rejected,
}

impl SuggestionResolution {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Rejected => "rejected",
        }
    }

    pub fn parse(value: &str) -> Result<Self, ValueError> {
        match value {
            "accepted" => Ok(Self::Accepted),
            "rejected" => Ok(Self::Rejected),
            _ => Err(ValueError::new("invalid suggestion resolution")),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParagraphSuggestion {
    pub id: String,
    pub provenance: ArtifactProvenance,
    pub created_at: Timestamp,
    pub content_digest: Digest,
    pub binding: ParagraphSuggestionBinding,
    pub replacement: String,
    pub reflection_ranges: Vec<ReflectionRange>,
    pub explanation: Option<String>,
    pub stale: bool,
    pub resolution: Option<SuggestionResolution>,
    pub resolved_at: Option<Timestamp>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceRefineSuggestion {
    pub source: crate::SourceRef,
    pub original_body: String,
    pub suggested_body: String,
    pub binding_digest: Digest,
    pub provenance: ArtifactProvenance,
    pub created_at: Timestamp,
}

fn reject_reserved_markers(text: &str) -> Result<(), ValueError> {
    if text.contains(REFLECTION_OPEN) || text.contains(REFLECTION_CLOSE) {
        return Err(ValueError::new(
            "text contains an unresolved reflection marker",
        ));
    }
    Ok(())
}

fn validate_structured_text(
    text: &str,
    ranges: &[ReflectionRange],
    max_bytes: usize,
) -> Result<(), ValueError> {
    if text.is_empty() || text.len() > max_bytes || ranges.len() > MAX_REFLECTION_RANGES {
        return Err(ValueError::new("invalid assistant structured text"));
    }
    reject_reserved_markers(text)?;
    let mut previous_end = 0;
    for (index, range) in ranges.iter().enumerate() {
        if range.start >= range.end
            || range.end > text.len()
            || !text.is_char_boundary(range.start)
            || !text.is_char_boundary(range.end)
            || (index > 0 && range.start < previous_end)
        {
            return Err(ValueError::new("invalid reflection range"));
        }
        previous_end = range.end;
    }
    let marker_bytes = (REFLECTION_OPEN.len_utf8() + REFLECTION_CLOSE.len_utf8())
        .checked_mul(ranges.len())
        .ok_or_else(|| ValueError::new("assistant structured text is too large"))?;
    if text.len().saturating_add(marker_bytes) > max_bytes {
        return Err(ValueError::new("assistant structured text is too large"));
    }
    Ok(())
}

fn insert_reflection_markers(
    text: &str,
    ranges: &[ReflectionRange],
    max_bytes: usize,
) -> Result<String, ValueError> {
    validate_structured_text(text, ranges, max_bytes)?;
    let mut rendered = text.to_owned();
    for range in ranges.iter().rev() {
        rendered.insert(range.end, REFLECTION_CLOSE);
        rendered.insert(range.start, REFLECTION_OPEN);
    }
    Ok(rendered)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_requests_expose_only_their_allowed_fields() {
        let extraction = MemoryExtractionRequest::new(vec!["source".to_owned()]).unwrap();
        assert_eq!(extraction.sources()[0].ordinal(), 1);
        assert_eq!(extraction.sources()[0].body(), "source");

        let draft = DraftEntryRequest::new(
            "reviewed",
            Some("prior".to_owned()),
            WritingLanguage::KoKr,
            WritingStyle::Reflective,
        )
        .unwrap();
        assert_eq!(draft.reviewed_memory(), "reviewed");
        assert_eq!(draft.prior_draft(), Some("prior"));

        let paragraph =
            ParagraphReviewRequest::new("target", WritingLanguage::EnUs, WritingStyle::Standard)
                .unwrap();
        assert_eq!(paragraph.paragraph(), "target");
    }

    #[test]
    fn candidates_require_distinct_in_range_source_ordinals() {
        assert!(MemoryCandidate::new("memory", vec![1, 2], 2).is_ok());
        assert!(MemoryCandidate::new("memory", vec![], 2).is_err());
        assert!(MemoryCandidate::new("memory", vec![1, 1], 2).is_err());
        assert!(MemoryCandidate::new("memory", vec![3], 2).is_err());
    }

    #[test]
    fn core_validates_ranges_and_inserts_reserved_markers() {
        let result =
            DraftEntryResult::new("one two", vec![ReflectionRange { start: 4, end: 7 }]).unwrap();
        assert_eq!(result.rendered_text().unwrap(), "one ⟪two⟫");
        assert!(DraftEntryResult::new("é", vec![ReflectionRange { start: 1, end: 2 }]).is_err());
        assert!(DraftEntryResult::new("⟪raw⟫", vec![]).is_err());
    }

    #[test]
    fn revision_and_paragraph_requests_reject_unresolved_markers() {
        assert!(
            DraftEntryRequest::new(
                "memory",
                Some("⟪unresolved⟫".to_owned()),
                WritingLanguage::EnUs,
                WritingStyle::Standard,
            )
            .is_err()
        );
        assert!(
            ParagraphReviewRequest::new(
                "⟪unresolved⟫",
                WritingLanguage::EnUs,
                WritingStyle::Standard,
            )
            .is_err()
        );
    }
}
