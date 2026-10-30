//! AI prompts embedded from `assets/prompts/`.
//!
//! Prompt text, provider, model, and reasoning effort live in asset files, not
//! in code. See `assets/prompts/README.md` for the file format.

use crate::AssistantAction;

const CODEX_EXTRACT: &str = include_str!("../../../assets/prompts/codex/extract-memories.md");
const CODEX_DRAFT: &str = include_str!("../../../assets/prompts/codex/draft-entry.md");
const CODEX_REVIEW: &str = include_str!("../../../assets/prompts/codex/review-paragraph.md");
const CODEX_REFINE: &str = include_str!("../../../assets/prompts/codex/refine-source.md");

const APPLE_SESSION: &str =
    include_str!("../../../assets/prompts/apple-foundation-models/session-instructions.md");
const APPLE_EXTRACT: &str =
    include_str!("../../../assets/prompts/apple-foundation-models/extract-memories.md");
const APPLE_DRAFT: &str =
    include_str!("../../../assets/prompts/apple-foundation-models/draft-entry.md");
const APPLE_REVIEW: &str =
    include_str!("../../../assets/prompts/apple-foundation-models/review-paragraph.md");
const APPLE_REFINE: &str =
    include_str!("../../../assets/prompts/apple-foundation-models/refine-source.md");

/// Front-matter value meaning "the provider's own default applies".
const DEFAULT_VALUE: &str = "default";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PromptAssetError {
    Malformed(&'static str),
    UnknownPlaceholder(String),
    MissingValue(String),
}

/// One parsed prompt asset: the (provider, model, reasoning effort, prompt)
/// tuple. `model` and `reasoning_effort` are `None` when the asset says
/// `default`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PromptAsset {
    provider: String,
    model: Option<String>,
    reasoning_effort: Option<String>,
    template: String,
}

impl PromptAsset {
    pub fn parse(source: &str) -> Result<Self, PromptAssetError> {
        let rest = source
            .strip_prefix("---\n")
            .ok_or(PromptAssetError::Malformed("missing front matter"))?;
        let (header, body) = rest
            .split_once("\n---\n")
            .ok_or(PromptAssetError::Malformed("unterminated front matter"))?;
        let (mut provider, mut model, mut effort) = (None, None, None);
        for line in header.lines() {
            let (key, value) = line
                .split_once(':')
                .ok_or(PromptAssetError::Malformed("bad front matter line"))?;
            let slot = match key.trim() {
                "provider" => &mut provider,
                "model" => &mut model,
                "reasoning_effort" => &mut effort,
                _ => return Err(PromptAssetError::Malformed("unknown front matter key")),
            };
            let value = value.trim();
            if value.is_empty() || slot.replace(value.to_owned()).is_some() {
                return Err(PromptAssetError::Malformed("empty or repeated key"));
            }
        }
        let missing = PromptAssetError::Malformed("missing front matter key");
        let non_default = |value: String| (value != DEFAULT_VALUE).then_some(value);
        let template = body.strip_suffix('\n').unwrap_or(body);
        if template.is_empty() {
            return Err(PromptAssetError::Malformed("empty prompt"));
        }
        Ok(Self {
            provider: provider.ok_or(missing.clone())?,
            model: non_default(model.ok_or(missing.clone())?),
            reasoning_effort: non_default(effort.ok_or(missing)?),
            template: template.to_owned(),
        })
    }

    pub fn provider(&self) -> &str {
        &self.provider
    }

    pub fn model(&self) -> Option<&str> {
        self.model.as_deref()
    }

    pub fn reasoning_effort(&self) -> Option<&str> {
        self.reasoning_effort.as_deref()
    }

    /// The prompt with its `{{name}}` placeholders unexpanded.
    pub fn template(&self) -> &str {
        &self.template
    }

    /// Expands `{{name}}` placeholders in a single pass, so substituted values
    /// (diary text) are never scanned for further placeholders.
    pub fn render(&self, values: &[(&str, &str)]) -> Result<String, PromptAssetError> {
        let mut out = String::with_capacity(self.template.len());
        let mut rest = self.template.as_str();
        while let Some(start) = rest.find("{{") {
            out.push_str(&rest[..start]);
            let after = &rest[start + 2..];
            let end = after
                .find("}}")
                .ok_or(PromptAssetError::Malformed("unterminated placeholder"))?;
            let name = &after[..end];
            let (_, value) = values
                .iter()
                .find(|(key, _)| *key == name)
                .ok_or_else(|| PromptAssetError::MissingValue(name.to_owned()))?;
            out.push_str(value);
            rest = &after[end + 2..];
        }
        out.push_str(rest);
        Ok(out)
    }
}

/// The Codex prompt for `action`.
pub fn codex_prompt(action: AssistantAction) -> Result<PromptAsset, PromptAssetError> {
    PromptAsset::parse(match action {
        AssistantAction::ExtractMemories => CODEX_EXTRACT,
        AssistantAction::DraftEntry => CODEX_DRAFT,
        AssistantAction::ReviewParagraph => CODEX_REVIEW,
        AssistantAction::RefineSource => CODEX_REFINE,
    })
}

/// The Apple Foundation Models per-action prompt template.
pub fn apple_prompt(action: AssistantAction) -> Result<PromptAsset, PromptAssetError> {
    PromptAsset::parse(match action {
        AssistantAction::ExtractMemories => APPLE_EXTRACT,
        AssistantAction::DraftEntry => APPLE_DRAFT,
        AssistantAction::ReviewParagraph => APPLE_REVIEW,
        AssistantAction::RefineSource => APPLE_REFINE,
    })
}

/// The Apple Foundation Models session instructions template.
pub fn apple_session_instructions() -> Result<PromptAsset, PromptAssetError> {
    PromptAsset::parse(APPLE_SESSION)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ACTIONS: [AssistantAction; 4] = [
        AssistantAction::ExtractMemories,
        AssistantAction::DraftEntry,
        AssistantAction::ReviewParagraph,
        AssistantAction::RefineSource,
    ];

    #[test]
    fn every_shipped_asset_parses_with_expected_placeholders() {
        for action in ACTIONS {
            let codex = codex_prompt(action).unwrap();
            assert_eq!(codex.provider(), "codex");
            assert_eq!(
                codex
                    .render(&[("data", "\u{1}")])
                    .unwrap()
                    .matches('\u{1}')
                    .count(),
                1
            );
            let apple = apple_prompt(action).unwrap();
            assert_eq!(apple.provider(), "apple-foundation-models");
            assert_eq!(apple.model(), None);
            assert_eq!(apple.reasoning_effort(), Some("not-applicable"));
            assert!(apple.template().contains("{{data}}"));
        }
        let session = apple_session_instructions().unwrap();
        assert!(
            session
                .render(&[("locale", "ko-KR")])
                .unwrap()
                .contains("ko-KR")
        );
    }

    #[test]
    fn substituted_values_are_not_expanded_again() {
        let asset = PromptAsset::parse(
            "---\nprovider: p\nmodel: m\nreasoning_effort: high\n---\nA {{data}} B {{locale}}\n",
        )
        .unwrap();
        assert_eq!(asset.model(), Some("m"));
        assert_eq!(asset.reasoning_effort(), Some("high"));
        let out = asset
            .render(&[("data", "{{locale}}"), ("locale", "en-US")])
            .unwrap();
        assert_eq!(out, "A {{locale}} B en-US");
    }

    #[test]
    fn rejects_malformed_assets() {
        assert!(PromptAsset::parse("no front matter").is_err());
        assert!(PromptAsset::parse("---\nprovider: p\n---\nbody").is_err());
        let asset = PromptAsset::parse(
            "---\nprovider: p\nmodel: default\nreasoning_effort: default\n---\n{{x}}",
        )
        .unwrap();
        assert_eq!(asset.model(), None);
        assert_eq!(
            asset.render(&[]),
            Err(PromptAssetError::MissingValue("x".into()))
        );
    }
}
