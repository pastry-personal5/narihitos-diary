//! Deterministic Notion projection, hierarchy classification, and retry policy.
//!
//! This module deliberately keeps credentials and remote identifiers out of
//! its public preview types. The HTTP adapter consumes the same [`NotionPlan`]
//! that Preview displays and read-back verifies.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use diary_core::{
    DiaryDay, Digest, InspectionOutcome, NotionBlockPreview, NotionOutputPort, NotionPreflight,
    NotionPreflightStatus, NotionRequest, WriteOutcome,
};
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use serde::{Deserialize, Serialize};

pub const API_VERSION: &str = "2026-03-11";
pub const RENDERER_VERSION: u32 = 1;
pub const MAX_RICH_TEXT_CHARS: usize = 2_000;
pub const MAX_APPEND_BLOCKS: usize = 100;
pub const MAX_CHILDREN: usize = 100;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NotionPlan {
    pub renderer_version: u32,
    pub api_version: String,
    pub year_title: String,
    pub month_title: String,
    pub heading: String,
    pub blocks: Vec<NotionBlock>,
    pub payload_digest: String,
}

impl NotionPlan {
    #[must_use]
    pub fn append_chunks(&self) -> Vec<&[NotionBlock]> {
        let mut chunks = Vec::new();
        let mut start = 0;
        while start < self.blocks.len() {
            let mut end = start;
            let mut bytes: usize = 0;
            while end < self.blocks.len() && end - start < MAX_APPEND_BLOCKS {
                let block_bytes =
                    serde_json::to_vec(&self.blocks[end]).map_or(usize::MAX, |value| value.len());
                if end > start && bytes.saturating_add(block_bytes) > 450 * 1024 {
                    break;
                }
                bytes = bytes.saturating_add(block_bytes);
                end += 1;
            }
            chunks.push(&self.blocks[start..end]);
            start = end;
        }
        chunks
    }

    pub fn validate(&self) -> Result<(), NotionError> {
        if self.renderer_version != RENDERER_VERSION || self.api_version != API_VERSION {
            return Err(NotionError::InvalidPlan);
        }
        if self.blocks.first().map(|block| block.kind) != Some(BlockKind::Heading2)
            || self.blocks.first().and_then(NotionBlock::plain_text) != Some(self.heading.as_str())
        {
            return Err(NotionError::InvalidPlan);
        }
        validate_blocks(&self.blocks)?;
        let digest = plan_digest(
            self.renderer_version,
            &self.api_version,
            &self.year_title,
            &self.month_title,
            &self.heading,
            &self.blocks,
        )?;
        if digest != self.payload_digest {
            return Err(NotionError::InvalidPlan);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockKind {
    Paragraph,
    Heading1,
    Heading2,
    Heading3,
    BulletedListItem,
    NumberedListItem,
    Quote,
    Code,
}

impl BlockKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Paragraph => "paragraph",
            Self::Heading1 => "heading_1",
            Self::Heading2 => "heading_2",
            Self::Heading3 => "heading_3",
            Self::BulletedListItem => "bulleted_list_item",
            Self::NumberedListItem => "numbered_list_item",
            Self::Quote => "quote",
            Self::Code => "code",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NotionBlock {
    pub kind: BlockKind,
    pub rich_text: Vec<NotionRichText>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<NotionBlock>,
}

impl NotionBlock {
    #[must_use]
    pub fn plain_text(&self) -> Option<&str> {
        (self.rich_text.len() == 1).then(|| self.rich_text[0].text.as_str())
    }

    #[must_use]
    pub fn display_text(&self) -> String {
        self.rich_text
            .iter()
            .map(|item| item.text.as_str())
            .collect()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NotionRichText {
    pub text: String,
    pub annotations: NotionTextAnnotations,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct NotionTextAnnotations {
    pub bold: bool,
    pub italic: bool,
    pub strikethrough: bool,
    pub code: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NotionError {
    InvalidPlan,
    UnsupportedSize,
    Serialization,
}

/// Convert confirmed Markdown into the sole canonical native-block plan.
pub fn render_notion(day: &DiaryDay, markdown: &str) -> Result<NotionPlan, NotionError> {
    let (year_title, month_title, heading) = canonical_titles(day)?;
    let mut blocks = vec![text_block(BlockKind::Heading2, heading.clone())];
    let body = Renderer::new(markdown).render()?;
    blocks.extend(body);
    validate_blocks(&blocks)?;
    let payload_digest = plan_digest(
        RENDERER_VERSION,
        API_VERSION,
        &year_title,
        &month_title,
        &heading,
        &blocks,
    )?;
    Ok(NotionPlan {
        renderer_version: RENDERER_VERSION,
        api_version: API_VERSION.to_owned(),
        year_title,
        month_title,
        heading,
        blocks,
        payload_digest,
    })
}

fn canonical_titles(day: &DiaryDay) -> Result<(String, String, String), NotionError> {
    let value = day.as_str();
    let year = value[0..4].to_owned();
    let month = value[0..7].to_owned();
    let weekday = weekday(day)?;
    Ok((year, month, format!("{value} ({weekday})")))
}

fn weekday(day: &DiaryDay) -> Result<&'static str, NotionError> {
    let bytes = day.as_str().as_bytes();
    let year = parse_digits(&bytes[0..4])? as i64;
    let month = parse_digits(&bytes[5..7])? as i64;
    let date = parse_digits(&bytes[8..10])? as i64;
    // Sakamoto's Gregorian algorithm, where 0 is Sunday.
    let offsets = [0_i64, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let adjusted_year = if month < 3 { year - 1 } else { year };
    let index = (adjusted_year + adjusted_year / 4 - adjusted_year / 100
        + adjusted_year / 400
        + offsets[(month - 1) as usize]
        + date)
        .rem_euclid(7) as usize;
    Ok(["SUN", "MON", "TUE", "WED", "THU", "FRI", "SAT"][index])
}

fn parse_digits(bytes: &[u8]) -> Result<u32, NotionError> {
    bytes.iter().try_fold(0_u32, |value, byte| {
        byte.is_ascii_digit()
            .then(|| value * 10 + u32::from(byte - b'0'))
            .ok_or(NotionError::InvalidPlan)
    })
}

fn plan_digest(
    renderer_version: u32,
    api_version: &str,
    year: &str,
    month: &str,
    heading: &str,
    blocks: &[NotionBlock],
) -> Result<String, NotionError> {
    #[derive(Serialize)]
    struct Canonical<'a> {
        renderer_version: u32,
        api_version: &'a str,
        year: &'a str,
        month: &'a str,
        heading: &'a str,
        blocks: &'a [NotionBlock],
    }
    let bytes = serde_json::to_vec(&Canonical {
        renderer_version,
        api_version,
        year,
        month,
        heading,
        blocks,
    })
    .map_err(|_| NotionError::Serialization)?;
    Ok(Digest::of_bytes(&bytes).as_str().to_owned())
}

fn validate_blocks(blocks: &[NotionBlock]) -> Result<(), NotionError> {
    for block in blocks {
        if block.children.len() > MAX_CHILDREN || block.rich_text.len() > MAX_CHILDREN {
            return Err(NotionError::UnsupportedSize);
        }
        if block.rich_text.iter().any(|item| {
            item.text.chars().count() > MAX_RICH_TEXT_CHARS
                || item.text.contains('\0')
                || item.link.as_ref().is_some_and(|link| link.len() > 2_000)
        }) {
            return Err(NotionError::UnsupportedSize);
        }
        validate_blocks(&block.children)?;
    }
    Ok(())
}

fn text_block(kind: BlockKind, text: String) -> NotionBlock {
    NotionBlock {
        kind,
        rich_text: split_rich_text(&text, NotionTextAnnotations::default(), None),
        language: None,
        children: Vec::new(),
    }
}

fn split_rich_text(
    text: &str,
    annotations: NotionTextAnnotations,
    link: Option<String>,
) -> Vec<NotionRichText> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut result = Vec::new();
    let mut start = 0;
    let mut count = 0;
    for (index, _) in text.char_indices() {
        if count == MAX_RICH_TEXT_CHARS {
            result.push(NotionRichText {
                text: text[start..index].to_owned(),
                annotations,
                link: link.clone(),
            });
            start = index;
            count = 0;
        }
        count += 1;
    }
    result.push(NotionRichText {
        text: text[start..].to_owned(),
        annotations,
        link,
    });
    result
}

#[derive(Clone)]
struct InlineState {
    annotations: NotionTextAnnotations,
    link: Option<String>,
}

struct OpenBlock {
    kind: BlockKind,
    rich_text: Vec<NotionRichText>,
    language: Option<String>,
    children: Vec<NotionBlock>,
}

struct Renderer<'a> {
    markdown: &'a str,
    blocks: Vec<NotionBlock>,
    open: Vec<OpenBlock>,
    inline: Vec<InlineState>,
    lists: Vec<bool>,
    unsupported: bool,
}

impl<'a> Renderer<'a> {
    fn new(markdown: &'a str) -> Self {
        Self {
            markdown,
            blocks: Vec::new(),
            open: Vec::new(),
            inline: vec![InlineState {
                annotations: NotionTextAnnotations::default(),
                link: None,
            }],
            lists: Vec::new(),
            unsupported: false,
        }
    }

    fn render(mut self) -> Result<Vec<NotionBlock>, NotionError> {
        let options = Options::ENABLE_STRIKETHROUGH;
        for event in Parser::new_ext(self.markdown, options) {
            self.event(event);
            if self.unsupported {
                // Literal Markdown is the lossless fallback for any construct
                // whose native meaning cannot be represented.
                return Ok(vec![text_block(
                    BlockKind::Paragraph,
                    self.markdown.to_owned(),
                )]);
            }
        }
        if !self.open.is_empty() || self.inline.len() != 1 || !self.lists.is_empty() {
            return Err(NotionError::InvalidPlan);
        }
        Ok(self.blocks)
    }

    fn event(&mut self, event: Event<'_>) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) => self.push_text(&text),
            Event::Code(text) => {
                self.push_inline(text.as_ref(), |annotations| annotations.code = true)
            }
            Event::SoftBreak => self.push_text("\n"),
            Event::HardBreak => self.push_text("\n"),
            Event::Html(_)
            | Event::InlineHtml(_)
            | Event::FootnoteReference(_)
            | Event::TaskListMarker(_)
            | Event::InlineMath(_)
            | Event::DisplayMath(_)
            | Event::Rule => self.unsupported = true,
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph => self.open(BlockKind::Paragraph, None),
            Tag::Heading { level, .. } => match level {
                HeadingLevel::H1 => self.open(BlockKind::Heading1, None),
                HeadingLevel::H2 => self.open(BlockKind::Heading2, None),
                HeadingLevel::H3 => self.open(BlockKind::Heading3, None),
                _ => self.unsupported = true,
            },
            Tag::BlockQuote(None) => self.open(BlockKind::Quote, None),
            Tag::CodeBlock(kind) => {
                let language = match kind {
                    CodeBlockKind::Indented => None,
                    CodeBlockKind::Fenced(value) if value.is_empty() => None,
                    CodeBlockKind::Fenced(value) => Some(value.into_string()),
                };
                self.open(BlockKind::Code, language);
            }
            Tag::List(first) => self.lists.push(first.is_some()),
            Tag::Item => self.open(
                if self.lists.last().copied().unwrap_or(false) {
                    BlockKind::NumberedListItem
                } else {
                    BlockKind::BulletedListItem
                },
                None,
            ),
            Tag::Emphasis => self.push_annotation(|value| value.italic = true),
            Tag::Strong => self.push_annotation(|value| value.bold = true),
            Tag::Strikethrough => self.push_annotation(|value| value.strikethrough = true),
            Tag::Link { dest_url, .. } => {
                let mut next = self.inline.last().cloned().unwrap();
                next.link = Some(dest_url.into_string());
                self.inline.push(next);
            }
            Tag::HtmlBlock
            | Tag::FootnoteDefinition(_)
            | Tag::Table(_)
            | Tag::TableHead
            | Tag::TableRow
            | Tag::TableCell
            | Tag::Image { .. }
            | Tag::MetadataBlock(_)
            | Tag::DefinitionList
            | Tag::DefinitionListTitle
            | Tag::DefinitionListDefinition
            | Tag::Superscript
            | Tag::Subscript
            | Tag::BlockQuote(Some(_)) => self.unsupported = true,
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph
            | TagEnd::Heading(_)
            | TagEnd::BlockQuote(_)
            | TagEnd::CodeBlock
            | TagEnd::Item => self.close(),
            TagEnd::List(_) => {
                self.lists.pop();
            }
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough | TagEnd::Link => {
                if self.inline.len() > 1 {
                    self.inline.pop();
                } else {
                    self.unsupported = true;
                }
            }
            _ => self.unsupported = true,
        }
    }

    fn open(&mut self, kind: BlockKind, language: Option<String>) {
        self.open.push(OpenBlock {
            kind,
            rich_text: Vec::new(),
            language,
            children: Vec::new(),
        });
    }

    fn close(&mut self) {
        let Some(block) = self.open.pop() else {
            self.unsupported = true;
            return;
        };
        let block = NotionBlock {
            kind: block.kind,
            rich_text: block.rich_text,
            language: block.language,
            children: block.children,
        };
        if let Some(parent) = self.open.last_mut() {
            if parent.rich_text.is_empty() && matches!(block.kind, BlockKind::Paragraph) {
                parent.rich_text = block.rich_text;
            } else {
                parent.children.push(block);
            }
        } else {
            self.blocks.push(block);
        }
    }

    fn push_text(&mut self, text: &str) {
        let state = self.inline.last().cloned().unwrap();
        let Some(block) = self.open.last_mut() else {
            if !text.trim().is_empty() {
                self.unsupported = true;
            }
            return;
        };
        block
            .rich_text
            .extend(split_rich_text(text, state.annotations, state.link));
    }

    fn push_inline(&mut self, text: &str, change: impl FnOnce(&mut NotionTextAnnotations)) {
        let mut next = self.inline.last().cloned().unwrap();
        change(&mut next.annotations);
        let Some(block) = self.open.last_mut() else {
            self.unsupported = true;
            return;
        };
        block
            .rich_text
            .extend(split_rich_text(text, next.annotations, next.link));
    }

    fn push_annotation(&mut self, change: impl FnOnce(&mut NotionTextAnnotations)) {
        let mut next = self.inline.last().cloned().unwrap();
        change(&mut next.annotations);
        self.inline.push(next);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PageKind {
    ChildPage,
    Other,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PageChild {
    pub id: String,
    pub kind: PageKind,
    pub title: Option<String>,
    pub trashed: bool,
    pub accessible: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NotionPage {
    pub id: String,
    pub title: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HierarchyPlan {
    pub year: Option<NotionPage>,
    pub month: Option<NotionPage>,
    pub year_will_be_created: bool,
    pub month_will_be_created: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HierarchyError {
    Duplicate,
    WrongKind,
    Trashed,
    Inaccessible,
    IncompleteListing,
}

/// Classifies one fully paginated direct-child listing.
pub fn resolve_child(
    children: &[PageChild],
    title: &str,
    listing_complete: bool,
) -> Result<Option<NotionPage>, HierarchyError> {
    if !listing_complete {
        return Err(HierarchyError::IncompleteListing);
    }
    let matches: Vec<_> = children
        .iter()
        .filter(|child| child.title.as_deref() == Some(title))
        .collect();
    if matches.len() > 1 {
        return Err(HierarchyError::Duplicate);
    }
    let Some(found) = matches.first() else {
        return Ok(None);
    };
    if found.kind != PageKind::ChildPage {
        return Err(HierarchyError::WrongKind);
    }
    if found.trashed {
        return Err(HierarchyError::Trashed);
    }
    if !found.accessible {
        return Err(HierarchyError::Inaccessible);
    }
    Ok(Some(NotionPage {
        id: found.id.clone(),
        title: title.to_owned(),
    }))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SameDayState {
    Absent,
    Canonical { index: usize },
    Conflict { indices: Vec<usize> },
}

/// Finds canonical, ISO-only, and wrong-weekday headings for the target day.
pub fn classify_same_day(blocks: &[NotionBlock], plan: &NotionPlan) -> SameDayState {
    let day = &plan.heading[..10];
    let mut canonical = Vec::new();
    let mut collision = Vec::new();
    for (index, block) in blocks.iter().enumerate() {
        if block.kind != BlockKind::Heading2 {
            continue;
        }
        let text = block.display_text();
        if text == plan.heading {
            canonical.push(index);
        } else if text == *day
            || (text.starts_with(day)
                && text.as_bytes().get(10) == Some(&b' ')
                && text.as_bytes().get(11) == Some(&b'('))
        {
            collision.push(index);
        }
    }
    if canonical.len() == 1 && collision.is_empty() {
        SameDayState::Canonical {
            index: canonical[0],
        }
    } else if canonical.is_empty() && collision.is_empty() {
        SameDayState::Absent
    } else {
        collision.extend(canonical);
        collision.sort_unstable();
        SameDayState::Conflict { indices: collision }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RetryAction {
    RetryAfter(Duration),
    Pending,
    InspectAmbiguousWrite,
    Blocked,
    Conflict,
    Exhausted,
}

#[derive(Clone, Debug)]
pub struct RetryPolicy {
    attempts: u8,
    maximum_attempts: u8,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            attempts: 0,
            maximum_attempts: 5,
        }
    }
}

impl RetryPolicy {
    #[must_use]
    pub fn classify(
        &mut self,
        status: Option<u16>,
        retry_after: Option<Duration>,
        is_write: bool,
    ) -> RetryAction {
        self.attempts = self.attempts.saturating_add(1);
        if self.attempts >= self.maximum_attempts {
            return RetryAction::Exhausted;
        }
        match status {
            Some(429 | 529) => match retry_after {
                Some(delay) if delay > Duration::from_secs(60) => RetryAction::Pending,
                Some(delay) => RetryAction::RetryAfter(delay),
                None => RetryAction::RetryAfter(Duration::from_secs(
                    1_u64 << u32::from(self.attempts.saturating_sub(1)),
                )),
            },
            Some(500 | 502 | 503 | 504) if is_write => RetryAction::InspectAmbiguousWrite,
            Some(500 | 502 | 503 | 504) => RetryAction::RetryAfter(Duration::from_secs(
                1_u64 << u32::from(self.attempts.saturating_sub(1)),
            )),
            Some(401 | 403 | 404) => RetryAction::Blocked,
            Some(400) => RetryAction::Blocked,
            Some(409) => RetryAction::Conflict,
            None if is_write => RetryAction::InspectAmbiguousWrite,
            None => RetryAction::RetryAfter(Duration::from_secs(
                1_u64 << u32::from(self.attempts.saturating_sub(1)),
            )),
            Some(_) => RetryAction::Blocked,
        }
    }
}

/// Stable, content-free reason tokens accepted at connector boundaries.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SetupReason {
    CredentialInvalid,
    PermissionDenied,
    RootUnverifiable,
    HierarchyAmbiguous,
    RemoteUnavailable,
    ResponseInvalid,
}

#[must_use]
pub fn content_free_reason(status: Option<u16>) -> SetupReason {
    match status {
        Some(401) => SetupReason::CredentialInvalid,
        Some(403) => SetupReason::PermissionDenied,
        Some(404) => SetupReason::RootUnverifiable,
        Some(400 | 409) => SetupReason::ResponseInvalid,
        Some(_) | None => SetupReason::RemoteUnavailable,
    }
}

impl SetupReason {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CredentialInvalid => "notion-credential-invalid",
            Self::PermissionDenied => "notion-permission-denied",
            Self::RootUnverifiable => "notion-root-unverifiable",
            Self::HierarchyAmbiguous => "notion-hierarchy-ambiguous",
            Self::RemoteUnavailable => "notion-remote-unavailable",
            Self::ResponseInvalid => "notion-response-invalid",
        }
    }
}

#[derive(Clone, Debug)]
struct RemoteBlock {
    id: String,
    block: NotionBlock,
}

trait NotionRemote: Send + Sync {
    fn list_children(
        &self,
        parent_id: &str,
        cancelled: &AtomicBool,
    ) -> Result<Vec<PageChild>, RemoteError>;
    fn list_blocks(
        &self,
        parent_id: &str,
        cancelled: &AtomicBool,
    ) -> Result<Vec<RemoteBlock>, RemoteError>;
    fn create_page(
        &self,
        parent_id: &str,
        title: &str,
        cancelled: &AtomicBool,
    ) -> Result<String, RemoteError>;
    fn append_blocks(
        &self,
        parent_id: &str,
        blocks: &[NotionBlock],
        after: Option<&str>,
        cancelled: &AtomicBool,
    ) -> Result<Vec<RemoteBlock>, RemoteError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RemoteError {
    Blocked(SetupReason),
    Ambiguous,
    Pending,
    Invalid,
}

/// Production Notion adapter. Its Debug representation never contains the
/// bearer token or page identity.
pub struct NotionClient {
    remote: Arc<dyn NotionRemote>,
    root_page_id: String,
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
    fn scripted(remote: Arc<dyn NotionRemote>, root_page_id: &str) -> Self {
        Self {
            remote,
            root_page_id: root_page_id.to_owned(),
        }
    }

    fn resolve_path(
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
            if !matches!(
                classify_same_day(
                    &blocks
                        .iter()
                        .map(|item| item.block.clone())
                        .collect::<Vec<_>>(),
                    plan,
                ),
                SameDayState::Absent
            ) {
                status = NotionPreflightStatus::Conflict;
                reason = Some("notion-same-day-conflict".to_owned());
            }
        }
        Ok(NotionPreflight {
            target_identity: self.root_page_id.clone(),
            root_label: "Notion root".to_owned(),
            year_title: plan.year_title.clone(),
            month_title: plan.month_title.clone(),
            year_exists: path.year.is_some(),
            month_exists: path.month.is_some(),
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
        if !matches!(
            classify_same_day(
                &existing
                    .iter()
                    .map(|item| item.block.clone())
                    .collect::<Vec<_>>(),
                &plan,
            ),
            SameDayState::Absent
        ) {
            return WriteOutcome::Conflict;
        }
        let mut ids = Vec::new();
        let mut after = None;
        for chunk in plan.append_chunks() {
            if cancelled.load(Ordering::Acquire) {
                return if ids.is_empty() {
                    WriteOutcome::DefiniteNoEffect
                } else {
                    WriteOutcome::AmbiguousWrite
                };
            }
            let appended =
                match self
                    .remote
                    .append_blocks(&month.id, chunk, after.as_deref(), cancelled)
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
            after = ids.last().cloned();
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
}

struct ResolvedPath {
    year: Option<NotionPage>,
    month: Option<NotionPage>,
}

fn map_hierarchy_error(_: HierarchyError) -> RemoteError {
    RemoteError::Blocked(SetupReason::HierarchyAmbiguous)
}

fn remote_reason(error: RemoteError) -> String {
    match error {
        RemoteError::Blocked(reason) => reason.as_str().to_owned(),
        RemoteError::Ambiguous => "notion-write-ambiguous".to_owned(),
        RemoteError::Pending => "notion-retry-deferred".to_owned(),
        RemoteError::Invalid => "notion-response-invalid".to_owned(),
    }
}

fn flatten_preview(blocks: &[NotionBlock]) -> Vec<NotionBlockPreview> {
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

/// First-in-first-out queue shared by every Notion request in the process.
/// A job that finishes one request goes to the back of the line, so a job
/// issuing back-to-back requests cannot starve a waiting one; a waiter that is
/// cancelled leaves the line without blocking those behind it.
struct RequestQueue {
    state: Mutex<QueueState>,
    turn_changed: std::sync::Condvar,
}

#[derive(Default)]
struct QueueState {
    next_ticket: u64,
    waiting: std::collections::VecDeque<u64>,
    busy: bool,
}

struct QueueTurn<'a>(&'a RequestQueue);

fn request_queue() -> &'static RequestQueue {
    static QUEUE: OnceLock<RequestQueue> = OnceLock::new();
    QUEUE.get_or_init(|| RequestQueue {
        state: Mutex::new(QueueState::default()),
        turn_changed: std::sync::Condvar::new(),
    })
}

impl RequestQueue {
    fn enter(&self, cancelled: &AtomicBool) -> Result<QueueTurn<'_>, RemoteError> {
        let mut state = self.state.lock().map_err(|_| RemoteError::Ambiguous)?;
        let ticket = state.next_ticket;
        state.next_ticket = state.next_ticket.wrapping_add(1);
        state.waiting.push_back(ticket);
        loop {
            if cancelled.load(Ordering::Acquire) {
                state.waiting.retain(|waiting| *waiting != ticket);
                self.turn_changed.notify_all();
                return Err(RemoteError::Pending);
            }
            if !state.busy && state.waiting.front() == Some(&ticket) {
                state.waiting.pop_front();
                state.busy = true;
                return Ok(QueueTurn(self));
            }
            // Wake periodically so a cancelled waiter notices without a signal.
            state = self
                .turn_changed
                .wait_timeout(state, Duration::from_millis(10))
                .map_err(|_| RemoteError::Ambiguous)?
                .0;
        }
    }
}

impl Drop for QueueTurn<'_> {
    fn drop(&mut self) {
        if let Ok(mut state) = self.0.state.lock() {
            state.busy = false;
        }
        self.0.turn_changed.notify_all();
    }
}

type Waiter = Arc<dyn Fn(Duration, &AtomicBool) -> Result<(), RemoteError> + Send + Sync>;

struct HttpRemote {
    token: String,
    agent: ureq::Agent,
    base_url: String,
    wait: Waiter,
}

impl HttpRemote {
    fn new(token: String) -> Result<Self, NotionError> {
        let config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(30)))
            .build();
        Ok(Self {
            token,
            agent: config.into(),
            base_url: "https://api.notion.com/v1".to_owned(),
            wait: Arc::new(wait_cancellable),
        })
    }

    /// Points the adapter at a local stub with an injected wait, so the retry
    /// matrix runs without a network or real sleeps.
    #[cfg(test)]
    fn with_stub(mut self, base_url: String, wait: Waiter) -> Self {
        self.base_url = base_url;
        self.wait = wait;
        self
    }

    fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<&serde_json::Value>,
        is_write: bool,
        cancelled: &AtomicBool,
    ) -> Result<serde_json::Value, RemoteError> {
        let _turn = request_queue().enter(cancelled)?;
        let url = format!("{}{path}", self.base_url);
        let mut retry = RetryPolicy::default();
        loop {
            if cancelled.load(Ordering::Acquire) {
                return Err(RemoteError::Pending);
            }
            let authorization = format!("Bearer {}", self.token);
            let encoded = body
                .map(serde_json::to_vec)
                .transpose()
                .map_err(|_| RemoteError::Invalid)?;
            let result = match (method, encoded) {
                ("GET", None) => self
                    .agent
                    .get(&url)
                    .header("Authorization", &authorization)
                    .header("Notion-Version", API_VERSION)
                    .header("Content-Type", "application/json")
                    .call(),
                ("POST", Some(encoded)) => self
                    .agent
                    .post(&url)
                    .header("Authorization", &authorization)
                    .header("Notion-Version", API_VERSION)
                    .header("Content-Type", "application/json")
                    .send(encoded),
                ("PATCH", Some(encoded)) => self
                    .agent
                    .patch(&url)
                    .header("Authorization", &authorization)
                    .header("Notion-Version", API_VERSION)
                    .header("Content-Type", "application/json")
                    .send(encoded),
                _ => return Err(RemoteError::Invalid),
            };
            let mut response = match result {
                Ok(response) => response,
                Err(_) if is_write => return Err(RemoteError::Ambiguous),
                Err(_) => match retry.classify(None, None, false) {
                    RetryAction::RetryAfter(delay) => {
                        (self.wait)(delay, cancelled)?;
                        continue;
                    }
                    _ => return Err(RemoteError::Blocked(SetupReason::RemoteUnavailable)),
                },
            };
            let status = response.status().as_u16();
            let retry_after = response
                .headers()
                .get("Retry-After")
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<u64>().ok())
                .map(Duration::from_secs);
            if !(200..300).contains(&status) {
                match retry.classify(Some(status), retry_after, is_write) {
                    RetryAction::RetryAfter(delay) => {
                        (self.wait)(delay, cancelled)?;
                        continue;
                    }
                    RetryAction::InspectAmbiguousWrite => return Err(RemoteError::Ambiguous),
                    RetryAction::Pending | RetryAction::Exhausted => {
                        return Err(RemoteError::Pending);
                    }
                    RetryAction::Conflict => return Err(RemoteError::Invalid),
                    _ => return Err(RemoteError::Blocked(content_free_reason(Some(status)))),
                }
            }
            let raw = response
                .body_mut()
                .with_config()
                .limit(2 * 1024 * 1024)
                .read_to_string()
                .map_err(|_| {
                    if is_write {
                        RemoteError::Ambiguous
                    } else {
                        RemoteError::Invalid
                    }
                })?;
            return serde_json::from_str(&raw).map_err(|_| RemoteError::Invalid);
        }
    }

    fn list_all(
        &self,
        parent_id: &str,
        cancelled: &AtomicBool,
    ) -> Result<Vec<serde_json::Value>, RemoteError> {
        let mut cursor: Option<String> = None;
        let mut all = Vec::new();
        loop {
            let encoded_cursor = cursor
                .as_deref()
                .map(|value| format!("&start_cursor={}", encode_query_component(value)))
                .unwrap_or_default();
            let response = self.request(
                "GET",
                &format!("/blocks/{parent_id}/children?page_size=100{encoded_cursor}"),
                None,
                false,
                cancelled,
            )?;
            let page = response.as_object().ok_or(RemoteError::Invalid)?;
            let results = page
                .get("results")
                .and_then(serde_json::Value::as_array)
                .ok_or(RemoteError::Invalid)?;
            all.extend(results.iter().cloned());
            let has_more = page
                .get("has_more")
                .and_then(serde_json::Value::as_bool)
                .ok_or(RemoteError::Invalid)?;
            if !has_more {
                if page
                    .get("next_cursor")
                    .is_some_and(|value| !value.is_null())
                {
                    return Err(RemoteError::Invalid);
                }
                break;
            }
            cursor = Some(
                page.get("next_cursor")
                    .and_then(serde_json::Value::as_str)
                    .filter(|value| !value.is_empty())
                    .ok_or(RemoteError::Invalid)?
                    .to_owned(),
            );
        }
        Ok(all)
    }

    fn parse_block_tree(
        &self,
        value: &serde_json::Value,
        cancelled: &AtomicBool,
    ) -> Result<RemoteBlock, RemoteError> {
        let mut parsed = parse_remote_block(value)?;
        let has_children = value
            .get("has_children")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        if has_children {
            parsed.block.children = self
                .list_all(&parsed.id, cancelled)?
                .iter()
                .map(|child| {
                    self.parse_block_tree(child, cancelled)
                        .map(|item| item.block)
                })
                .collect::<Result<Vec<_>, _>>()?;
        }
        Ok(parsed)
    }
}

fn encode_query_component(value: &str) -> String {
    use std::fmt::Write as _;

    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            let _ = write!(&mut encoded, "%{byte:02X}");
        }
    }
    encoded
}

fn wait_cancellable(delay: Duration, cancelled: &AtomicBool) -> Result<(), RemoteError> {
    let started = std::time::Instant::now();
    while started.elapsed() < delay {
        if cancelled.load(Ordering::Acquire) {
            return Err(RemoteError::Pending);
        }
        let remaining = delay.saturating_sub(started.elapsed());
        std::thread::sleep(remaining.min(Duration::from_millis(50)));
    }
    Ok(())
}

impl NotionRemote for HttpRemote {
    fn list_children(
        &self,
        parent_id: &str,
        cancelled: &AtomicBool,
    ) -> Result<Vec<PageChild>, RemoteError> {
        self.list_all(parent_id, cancelled)?
            .into_iter()
            .map(|value| parse_page_child(&value))
            .collect()
    }

    fn list_blocks(
        &self,
        parent_id: &str,
        cancelled: &AtomicBool,
    ) -> Result<Vec<RemoteBlock>, RemoteError> {
        self.list_all(parent_id, cancelled)?
            .iter()
            .map(|value| self.parse_block_tree(value, cancelled))
            .collect()
    }

    fn create_page(
        &self,
        parent_id: &str,
        title: &str,
        cancelled: &AtomicBool,
    ) -> Result<String, RemoteError> {
        let body = serde_json::json!({
            "parent": { "type": "page_id", "page_id": parent_id },
            "properties": {
                "title": {
                    "type": "title",
                    "title": [{ "type": "text", "text": { "content": title } }]
                }
            }
        });
        self.request("POST", "/pages", Some(&body), true, cancelled)?
            .get("id")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
            .ok_or(RemoteError::Invalid)
    }

    fn append_blocks(
        &self,
        parent_id: &str,
        blocks: &[NotionBlock],
        after: Option<&str>,
        cancelled: &AtomicBool,
    ) -> Result<Vec<RemoteBlock>, RemoteError> {
        let children: Vec<_> = blocks.iter().map(block_json).collect();
        let position = after.map_or_else(
            || serde_json::json!({ "type": "end" }),
            |id| serde_json::json!({ "type": "after_block", "after_block": { "id": id } }),
        );
        let body = serde_json::json!({ "children": children, "position": position });
        let response = self.request(
            "PATCH",
            &format!("/blocks/{parent_id}/children"),
            Some(&body),
            true,
            cancelled,
        )?;
        response
            .get("results")
            .and_then(serde_json::Value::as_array)
            .ok_or(RemoteError::Invalid)?
            .iter()
            .map(|value| self.parse_block_tree(value, cancelled))
            .collect()
    }
}

fn parse_page_child(value: &serde_json::Value) -> Result<PageChild, RemoteError> {
    let object = value.as_object().ok_or(RemoteError::Invalid)?;
    let id = object
        .get("id")
        .and_then(serde_json::Value::as_str)
        .ok_or(RemoteError::Invalid)?
        .to_owned();
    let block_type = object
        .get("type")
        .and_then(serde_json::Value::as_str)
        .ok_or(RemoteError::Invalid)?;
    let title = if block_type == "child_page" {
        object
            .get("child_page")
            .and_then(|value| value.get("title"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
    } else {
        parse_rich_text(object.get(block_type))
            .ok()
            .map(|items| items.into_iter().map(|item| item.text).collect::<String>())
    };
    Ok(PageChild {
        id,
        kind: if block_type == "child_page" {
            PageKind::ChildPage
        } else {
            PageKind::Other
        },
        title,
        trashed: object
            .get("in_trash")
            .or_else(|| object.get("archived"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        accessible: true,
    })
}

fn parse_remote_block(value: &serde_json::Value) -> Result<RemoteBlock, RemoteError> {
    let object = value.as_object().ok_or(RemoteError::Invalid)?;
    let id = object
        .get("id")
        .and_then(serde_json::Value::as_str)
        .ok_or(RemoteError::Invalid)?
        .to_owned();
    let block_type = object
        .get("type")
        .and_then(serde_json::Value::as_str)
        .ok_or(RemoteError::Invalid)?;
    let kind = match block_type {
        "paragraph" => BlockKind::Paragraph,
        "heading_1" => BlockKind::Heading1,
        "heading_2" => BlockKind::Heading2,
        "heading_3" => BlockKind::Heading3,
        "bulleted_list_item" => BlockKind::BulletedListItem,
        "numbered_list_item" => BlockKind::NumberedListItem,
        "quote" => BlockKind::Quote,
        "code" => BlockKind::Code,
        _ => return Err(RemoteError::Invalid),
    };
    let fields = object.get(block_type).ok_or(RemoteError::Invalid)?;
    let rich_text = parse_rich_text(Some(fields))?;
    let language = fields
        .get("language")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    let children = fields
        .get("children")
        .and_then(serde_json::Value::as_array)
        .map(|children| {
            children
                .iter()
                .map(parse_remote_block)
                .map(|result| result.map(|item| item.block))
                .collect()
        })
        .transpose()?
        .unwrap_or_default();
    Ok(RemoteBlock {
        id,
        block: NotionBlock {
            kind,
            rich_text,
            language,
            children,
        },
    })
}

fn parse_rich_text(value: Option<&serde_json::Value>) -> Result<Vec<NotionRichText>, RemoteError> {
    value
        .and_then(|value| value.get("rich_text"))
        .and_then(serde_json::Value::as_array)
        .ok_or(RemoteError::Invalid)?
        .iter()
        .map(|item| {
            let object = item.as_object().ok_or(RemoteError::Invalid)?;
            let text = object
                .get("plain_text")
                .and_then(serde_json::Value::as_str)
                .or_else(|| {
                    object
                        .get("text")
                        .and_then(|text| text.get("content"))
                        .and_then(serde_json::Value::as_str)
                })
                .ok_or(RemoteError::Invalid)?
                .to_owned();
            let annotations = object.get("annotations");
            let flag = |name| {
                annotations
                    .and_then(|value| value.get(name))
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false)
            };
            let link = object
                .get("href")
                .and_then(serde_json::Value::as_str)
                .or_else(|| {
                    object
                        .get("text")
                        .and_then(|text| text.get("link"))
                        .and_then(|link| link.get("url"))
                        .and_then(serde_json::Value::as_str)
                })
                .map(str::to_owned);
            Ok(NotionRichText {
                text,
                annotations: NotionTextAnnotations {
                    bold: flag("bold"),
                    italic: flag("italic"),
                    strikethrough: flag("strikethrough"),
                    code: flag("code"),
                },
                link,
            })
        })
        .collect()
}

fn block_json(block: &NotionBlock) -> serde_json::Value {
    let rich_text: Vec<_> = block
        .rich_text
        .iter()
        .map(|item| {
            serde_json::json!({
                "type": "text",
                "text": {
                    "content": item.text,
                    "link": item.link.as_ref().map(|url| serde_json::json!({ "url": url }))
                },
                "annotations": {
                    "bold": item.annotations.bold,
                    "italic": item.annotations.italic,
                    "strikethrough": item.annotations.strikethrough,
                    "underline": false,
                    "code": item.annotations.code,
                    "color": "default"
                }
            })
        })
        .collect();
    let children: Vec<_> = block.children.iter().map(block_json).collect();
    let mut fields = serde_json::json!({ "rich_text": rich_text });
    if !children.is_empty() {
        fields["children"] = serde_json::Value::Array(children);
    }
    if let Some(language) = &block.language {
        fields["language"] = serde_json::Value::String(language.clone());
    }
    let mut object = serde_json::Map::new();
    object.insert("object".into(), serde_json::Value::String("block".into()));
    object.insert(
        "type".into(),
        serde_json::Value::String(block.kind.as_str().into()),
    );
    object.insert(block.kind.as_str().into(), fields);
    serde_json::Value::Object(object)
}

#[allow(dead_code)]
fn canonical_object_for_block(block: &NotionBlock) -> BTreeMap<&'static str, serde_json::Value> {
    let mut value = BTreeMap::new();
    value.insert(
        "type",
        serde_json::Value::String(block.kind.as_str().into()),
    );
    value.insert(
        "rich_text",
        serde_json::to_value(&block.rich_text).unwrap_or(serde_json::Value::Null),
    );
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicBool, AtomicUsize};

    fn day(value: &str) -> DiaryDay {
        DiaryDay::parse(value).unwrap()
    }

    #[test]
    fn weekday_is_fixed_across_leap_and_year_boundaries() {
        for (value, expected) in [
            ("2024-02-29", "2024-02-29 (THU)"),
            ("2025-12-31", "2025-12-31 (WED)"),
            ("2026-01-01", "2026-01-01 (THU)"),
            ("2026-10-30", "2026-10-30 (FRI)"),
        ] {
            assert_eq!(render_notion(&day(value), "").unwrap().heading, expected);
        }
    }

    #[test]
    fn supported_markdown_projects_without_losing_text_or_annotations() {
        let markdown = concat!(
            "# Heading\n\n",
            "soft\nbreak and **bold** *italic* ~~strike~~ `code` ",
            "[link](https://example.invalid).  \nHard.\n\n",
            "- one\n  - nested\n\n1. first\n\n> quote\n\n```rust\nfn main() {}\n```\n",
        );
        let plan = render_notion(&day("2026-10-30"), markdown).unwrap();
        plan.validate().unwrap();
        assert_eq!(plan.blocks[0].kind, BlockKind::Heading2);
        assert!(
            plan.blocks
                .iter()
                .any(|block| block.kind == BlockKind::Heading1)
        );
        assert!(
            plan.blocks
                .iter()
                .any(|block| block.kind == BlockKind::BulletedListItem)
        );
        assert!(
            plan.blocks
                .iter()
                .any(|block| block.kind == BlockKind::NumberedListItem)
        );
        assert!(
            plan.blocks
                .iter()
                .any(|block| block.kind == BlockKind::Quote)
        );
        assert!(
            plan.blocks
                .iter()
                .any(|block| block.kind == BlockKind::Code)
        );
        let rich: Vec<_> = plan
            .blocks
            .iter()
            .flat_map(|block| &block.rich_text)
            .collect();
        assert!(rich.iter().any(|item| item.annotations.bold));
        assert!(rich.iter().any(|item| item.annotations.italic));
        assert!(rich.iter().any(|item| item.annotations.strikethrough));
        assert!(rich.iter().any(|item| item.annotations.code));
        assert!(rich.iter().any(|item| item.link.is_some()));
    }

    #[test]
    fn unicode_splitting_uses_scalar_boundaries_and_preserves_all_text() {
        let text = "한🙂".repeat(1_501);
        let plan = render_notion(&day("2026-10-30"), &text).unwrap();
        assert_eq!(plan.blocks[1].display_text(), text);
        assert!(
            plan.blocks[1]
                .rich_text
                .iter()
                .all(|item| item.text.chars().count() <= MAX_RICH_TEXT_CHARS)
        );
    }

    #[test]
    fn unsupported_construct_is_preserved_as_literal_markdown() {
        let markdown = "before\n\n![alt](https://example.invalid/image.png)\n\nafter";
        let plan = render_notion(&day("2026-10-30"), markdown).unwrap();
        assert_eq!(plan.blocks.len(), 2);
        assert_eq!(plan.blocks[1].display_text(), markdown);
    }

    #[test]
    fn hierarchy_allows_unrelated_but_fails_closed() {
        let unrelated = PageChild {
            id: "x".into(),
            kind: PageKind::Other,
            title: Some("notes".into()),
            trashed: false,
            accessible: true,
        };
        assert_eq!(
            resolve_child(std::slice::from_ref(&unrelated), "2026", true),
            Ok(None)
        );
        let canonical = PageChild {
            id: "year".into(),
            kind: PageKind::ChildPage,
            title: Some("2026".into()),
            trashed: false,
            accessible: true,
        };
        assert_eq!(
            resolve_child(&[unrelated, canonical.clone()], "2026", true),
            Ok(Some(NotionPage {
                id: "year".into(),
                title: "2026".into()
            }))
        );
        assert_eq!(
            resolve_child(&[canonical.clone(), canonical], "2026", true),
            Err(HierarchyError::Duplicate)
        );
        assert_eq!(
            resolve_child(&[], "2026", false),
            Err(HierarchyError::IncompleteListing)
        );
        for (mut child, expected) in [
            (
                PageChild {
                    kind: PageKind::Other,
                    ..PageChild {
                        id: "wrong".into(),
                        kind: PageKind::ChildPage,
                        title: Some("2026".into()),
                        trashed: false,
                        accessible: true,
                    }
                },
                HierarchyError::WrongKind,
            ),
            (
                PageChild {
                    id: "trashed".into(),
                    kind: PageKind::ChildPage,
                    title: Some("2026".into()),
                    trashed: true,
                    accessible: true,
                },
                HierarchyError::Trashed,
            ),
            (
                PageChild {
                    id: "hidden".into(),
                    kind: PageKind::ChildPage,
                    title: Some("2026".into()),
                    trashed: false,
                    accessible: false,
                },
                HierarchyError::Inaccessible,
            ),
        ] {
            child.title = Some("2026".into());
            assert_eq!(resolve_child(&[child], "2026", true), Err(expected));
        }
    }

    #[test]
    fn same_day_legacy_and_wrong_weekday_headings_conflict() {
        let plan = render_notion(&day("2026-10-30"), "body").unwrap();
        assert_eq!(classify_same_day(&[], &plan), SameDayState::Absent);
        assert_eq!(
            classify_same_day(
                &[text_block(BlockKind::Heading2, plan.heading.clone())],
                &plan
            ),
            SameDayState::Canonical { index: 0 }
        );
        for heading in ["2026-10-30", "2026-10-30 (THU)"] {
            assert!(matches!(
                classify_same_day(&[text_block(BlockKind::Heading2, heading.into())], &plan),
                SameDayState::Conflict { .. }
            ));
        }
    }

    #[test]
    fn retry_policy_is_bounded_and_long_wait_becomes_pending() {
        let mut policy = RetryPolicy::default();
        assert_eq!(
            policy.classify(Some(429), Some(Duration::from_secs(61)), false),
            RetryAction::Pending
        );
        let mut write = RetryPolicy::default();
        assert_eq!(
            write.classify(Some(503), None, true),
            RetryAction::InspectAmbiguousWrite
        );
        let mut exhausted = RetryPolicy::default();
        for _ in 0..4 {
            assert!(matches!(
                exhausted.classify(Some(503), None, false),
                RetryAction::RetryAfter(_)
            ));
        }
        assert_eq!(
            exhausted.classify(Some(503), None, false),
            RetryAction::Exhausted
        );
    }

    #[test]
    fn opaque_pagination_cursor_is_encoded_without_interpreting_it() {
        assert_eq!(encode_query_component("a/b+c=?"), "a%2Fb%2Bc%3D%3F");
    }

    #[test]
    fn retry_wait_observes_cancellation_without_sleeping_the_delay() {
        let cancelled = AtomicBool::new(true);
        assert_eq!(
            wait_cancellable(Duration::from_secs(60), &cancelled),
            Err(RemoteError::Pending)
        );
    }

    #[derive(Default)]
    struct FakeState {
        children: BTreeMap<String, Vec<PageChild>>,
        blocks: BTreeMap<String, Vec<RemoteBlock>>,
        next_id: u64,
        lose_next_append_reply: bool,
        /// `(append call number, committed)`: that append's reply is lost,
        /// and the blocks are either stored (`true`) or never reach Notion.
        lose_append_reply_on_call: Option<(usize, bool)>,
        append_anchors: Vec<Option<String>>,
        list_blocks_calls: usize,
        lose_next_create_reply: bool,
        fail_list_blocks_on_call: Option<(usize, RemoteError)>,
        list_errors: BTreeMap<String, RemoteError>,
        creates: usize,
    }

    #[derive(Default)]
    struct FakeRemote(Mutex<FakeState>);

    impl FakeRemote {
        fn lose_next_create_reply(&self) {
            self.0.lock().unwrap().lose_next_create_reply = true;
        }

        fn fail_list_blocks_on_call(&self, call: usize, error: RemoteError) {
            self.0.lock().unwrap().fail_list_blocks_on_call = Some((call, error));
        }

        fn fail_children_of(&self, parent: &str, error: RemoteError) {
            self.0
                .lock()
                .unwrap()
                .list_errors
                .insert(parent.to_owned(), error);
        }

        fn seed_children(&self, parent: &str, children: Vec<PageChild>) {
            self.0
                .lock()
                .unwrap()
                .children
                .insert(parent.to_owned(), children);
        }

        fn seed_blocks(&self, parent: &str, blocks: Vec<RemoteBlock>) {
            self.0
                .lock()
                .unwrap()
                .blocks
                .insert(parent.to_owned(), blocks);
        }

        fn lose_next_append_reply(&self) {
            self.0.lock().unwrap().lose_next_append_reply = true;
        }

        fn lose_append_reply_on_call(&self, call: usize, committed: bool) {
            self.0.lock().unwrap().lose_append_reply_on_call = Some((call, committed));
        }
    }

    impl NotionRemote for FakeRemote {
        fn list_children(
            &self,
            parent_id: &str,
            _: &AtomicBool,
        ) -> Result<Vec<PageChild>, RemoteError> {
            let state = self.0.lock().unwrap();
            if let Some(error) = state.list_errors.get(parent_id) {
                return Err(*error);
            }
            Ok(state.children.get(parent_id).cloned().unwrap_or_default())
        }

        fn list_blocks(
            &self,
            parent_id: &str,
            _: &AtomicBool,
        ) -> Result<Vec<RemoteBlock>, RemoteError> {
            let mut state = self.0.lock().unwrap();
            state.list_blocks_calls += 1;
            if let Some((call, error)) = state.fail_list_blocks_on_call
                && call == state.list_blocks_calls
            {
                state.fail_list_blocks_on_call = None;
                return Err(error);
            }
            Ok(state.blocks.get(parent_id).cloned().unwrap_or_default())
        }

        fn create_page(
            &self,
            parent_id: &str,
            title: &str,
            _: &AtomicBool,
        ) -> Result<String, RemoteError> {
            let mut state = self.0.lock().unwrap();
            state.creates += 1;
            state.next_id += 1;
            let id = format!("page-{}", state.next_id);
            state
                .children
                .entry(parent_id.to_owned())
                .or_default()
                .push(PageChild {
                    id: id.clone(),
                    kind: PageKind::ChildPage,
                    title: Some(title.to_owned()),
                    trashed: false,
                    accessible: true,
                });
            if state.lose_next_create_reply {
                state.lose_next_create_reply = false;
                return Err(RemoteError::Ambiguous);
            }
            Ok(id)
        }

        fn append_blocks(
            &self,
            parent_id: &str,
            blocks: &[NotionBlock],
            after: Option<&str>,
            _: &AtomicBool,
        ) -> Result<Vec<RemoteBlock>, RemoteError> {
            let mut state = self.0.lock().unwrap();
            state.append_anchors.push(after.map(str::to_owned));
            let call = state.append_anchors.len();
            let lost = state
                .lose_append_reply_on_call
                .filter(|(lost_call, _)| *lost_call == call);
            if let Some((_, false)) = lost {
                state.lose_append_reply_on_call = None;
                return Err(RemoteError::Ambiguous);
            }
            let mut appended = Vec::new();
            for block in blocks {
                state.next_id += 1;
                appended.push(RemoteBlock {
                    id: format!("block-{}", state.next_id),
                    block: block.clone(),
                });
            }
            let existing = state.blocks.entry(parent_id.to_owned()).or_default();
            let insertion = after
                .and_then(|id| existing.iter().position(|item| item.id == id))
                .map_or(existing.len(), |index| index + 1);
            existing.splice(insertion..insertion, appended.clone());
            if lost.is_some() {
                state.lose_append_reply_on_call = None;
                return Err(RemoteError::Ambiguous);
            }
            if state.lose_next_append_reply {
                state.lose_next_append_reply = false;
                return Err(RemoteError::Ambiguous);
            }
            Ok(appended)
        }
    }

    #[test]
    fn lost_append_reply_is_inspected_and_never_appended_twice() {
        let root = "0123456789abcdef0123456789abcdef";
        let remote = Arc::new(FakeRemote::default());
        let client = NotionClient::scripted(remote.clone(), root);
        let diary_day = day("2026-10-30");
        let preflight = client.preflight(&diary_day, "body").unwrap();
        assert_eq!(preflight.status, NotionPreflightStatus::Ready);
        assert!(!preflight.year_exists);
        assert!(!preflight.month_exists);
        let request = NotionRequest {
            day: diary_day,
            markdown: "body".into(),
            expected_target_identity: preflight.target_identity,
            expected_payload_digest: preflight.payload_digest,
            expected_state_digest: preflight.observed_state_digest,
        };
        remote.lose_next_append_reply();
        assert_eq!(
            client.write(&request, &AtomicBool::new(false)),
            WriteOutcome::AmbiguousWrite
        );
        let matching = client.inspect(&request);
        let InspectionOutcome::Matching {
            ref returned_ids, ..
        } = matching
        else {
            panic!("lost response must reconcile to the exact committed section");
        };
        assert_eq!(returned_ids.len(), 2);
        assert_eq!(client.inspect(&request), matching);
    }

    #[test]
    fn later_chunk_uses_last_read_back_verified_block_as_its_anchor() {
        let root = "0123456789abcdef0123456789abcdef";
        let remote = Arc::new(FakeRemote::default());
        let client = NotionClient::scripted(remote.clone(), root);
        let diary_day = day("2026-10-30");
        let markdown = (0..120)
            .map(|index| format!("paragraph {index}"))
            .collect::<Vec<_>>()
            .join("\n\n");
        let preflight = client.preflight(&diary_day, &markdown).unwrap();
        let request = NotionRequest {
            day: diary_day,
            markdown,
            expected_target_identity: preflight.target_identity,
            expected_payload_digest: preflight.payload_digest,
            expected_state_digest: preflight.observed_state_digest,
        };
        assert!(matches!(
            client.write(&request, &AtomicBool::new(false)),
            WriteOutcome::Verified { .. }
        ));
        let state = remote.0.lock().unwrap();
        assert_eq!(state.append_anchors.len(), 2);
        assert!(state.append_anchors[0].is_none());
        assert!(state.append_anchors[1].is_some());
        assert!(state.list_blocks_calls >= 4);
    }

    const ROOT: &str = "0123456789abcdef0123456789abcdef";

    fn long_markdown() -> String {
        (0..120)
            .map(|index| format!("paragraph {index}"))
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    fn request_for(client: &NotionClient, value: &str, markdown: &str) -> NotionRequest {
        let diary_day = day(value);
        let preflight = client.preflight(&diary_day, markdown).unwrap();
        NotionRequest {
            day: diary_day,
            markdown: markdown.into(),
            expected_target_identity: preflight.target_identity,
            expected_payload_digest: preflight.payload_digest,
            expected_state_digest: preflight.observed_state_digest,
        }
    }

    fn page(id: &str, title: &str) -> PageChild {
        PageChild {
            id: id.into(),
            kind: PageKind::ChildPage,
            title: Some(title.into()),
            trashed: false,
            accessible: true,
        }
    }

    fn remote_heading(id: &str, text: &str) -> RemoteBlock {
        RemoteBlock {
            id: id.into(),
            block: text_block(BlockKind::Heading2, text.into()),
        }
    }

    #[test]
    fn lost_reply_on_a_later_chunk_that_committed_is_adopted_without_another_append() {
        let remote = Arc::new(FakeRemote::default());
        let client = NotionClient::scripted(remote.clone(), ROOT);
        let request = request_for(&client, "2026-10-30", &long_markdown());
        remote.lose_append_reply_on_call(2, true);
        assert_eq!(
            client.write(&request, &AtomicBool::new(false)),
            WriteOutcome::AmbiguousWrite
        );
        assert_eq!(remote.0.lock().unwrap().append_anchors.len(), 2);
        let InspectionOutcome::Matching { returned_ids, .. } = client.inspect(&request) else {
            panic!("a committed later chunk must reconcile to the exact section");
        };
        assert_eq!(returned_ids.len(), 121);
        // Reconciling never appends; a new write sees the section and refuses.
        assert_eq!(
            client.write(&request, &AtomicBool::new(false)),
            WriteOutcome::Conflict
        );
        assert_eq!(remote.0.lock().unwrap().append_anchors.len(), 2);
    }

    #[test]
    fn lost_reply_on_a_later_chunk_that_never_landed_is_a_conflict_not_a_silent_completion() {
        let remote = Arc::new(FakeRemote::default());
        let client = NotionClient::scripted(remote.clone(), ROOT);
        let request = request_for(&client, "2026-10-30", &long_markdown());
        remote.lose_append_reply_on_call(2, false);
        assert_eq!(
            client.write(&request, &AtomicBool::new(false)),
            WriteOutcome::AmbiguousWrite
        );
        // Only the first chunk is present: inspection must not adopt it, and
        // nothing may append the missing remainder on its own.
        assert_eq!(client.inspect(&request), InspectionOutcome::Conflict);
        assert_eq!(remote.0.lock().unwrap().append_anchors.len(), 2);
        let blocks = remote
            .0
            .lock()
            .unwrap()
            .blocks
            .values()
            .map(Vec::len)
            .sum::<usize>();
        assert_eq!(blocks, 100);
    }

    #[derive(Clone, Copy)]
    enum Reply {
        Ok,
        Status(u16, Option<&'static str>),
        Drop,
    }

    struct Stub {
        base_url: String,
        requests: Arc<AtomicUsize>,
        waits: Arc<Mutex<Vec<Duration>>>,
    }

    impl Stub {
        fn serving(script: Vec<Reply>) -> Self {
            use std::io::{Read, Write};

            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let base_url = format!(
                "http://127.0.0.1:{}/v1",
                listener.local_addr().unwrap().port()
            );
            let requests = Arc::new(AtomicUsize::new(0));
            let counter = Arc::clone(&requests);
            std::thread::spawn(move || {
                for reply in script {
                    let Ok((mut stream, _)) = listener.accept() else {
                        return;
                    };
                    let mut received = Vec::new();
                    let mut chunk = [0_u8; 1024];
                    while !received.windows(4).any(|window| window == b"\r\n\r\n") {
                        match stream.read(&mut chunk) {
                            Ok(0) | Err(_) => break,
                            Ok(count) => received.extend_from_slice(&chunk[..count]),
                        }
                    }
                    let head = String::from_utf8_lossy(&received).to_ascii_lowercase();
                    let length = head
                        .split("content-length:")
                        .nth(1)
                        .and_then(|rest| rest.split("\r\n").next())
                        .and_then(|value| value.trim().parse::<usize>().ok())
                        .unwrap_or(0);
                    let body_start = received
                        .windows(4)
                        .position(|window| window == b"\r\n\r\n")
                        .map_or(received.len(), |index| index + 4);
                    let mut have = received.len().saturating_sub(body_start);
                    while have < length {
                        match stream.read(&mut chunk) {
                            Ok(0) | Err(_) => break,
                            Ok(count) => have += count,
                        }
                    }
                    counter.fetch_add(1, Ordering::SeqCst);
                    let (code, retry_after, body) = match reply {
                        Reply::Drop => continue,
                        Reply::Ok => (
                            200,
                            None,
                            r#"{"results":[],"has_more":false,"next_cursor":null}"#,
                        ),
                        Reply::Status(code, retry_after) => (code, retry_after, "{}"),
                    };
                    let retry_after = retry_after
                        .map(|value| format!("Retry-After: {value}\r\n"))
                        .unwrap_or_default();
                    let _ = write!(
                        stream,
                        "HTTP/1.1 {code} X\r\nContent-Type: application/json\r\n{retry_after}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                }
            });
            Self {
                base_url,
                requests,
                waits: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn remote(&self) -> HttpRemote {
            let waits = Arc::clone(&self.waits);
            HttpRemote::new("secret".to_owned()).unwrap().with_stub(
                self.base_url.clone(),
                Arc::new(move |delay, _| {
                    waits.lock().unwrap().push(delay);
                    Ok(())
                }),
            )
        }

        fn requests(&self) -> usize {
            self.requests.load(Ordering::SeqCst)
        }

        fn waits(&self) -> Vec<Duration> {
            self.waits.lock().unwrap().clone()
        }
    }

    fn read(remote: &HttpRemote) -> Result<serde_json::Value, RemoteError> {
        remote.request(
            "GET",
            "/blocks/x/children",
            None,
            false,
            &AtomicBool::new(false),
        )
    }

    fn write(remote: &HttpRemote) -> Result<serde_json::Value, RemoteError> {
        remote.request(
            "PATCH",
            "/blocks/x/children",
            Some(&serde_json::json!({})),
            true,
            &AtomicBool::new(false),
        )
    }

    fn secs(values: &[u64]) -> Vec<Duration> {
        values
            .iter()
            .map(|value| Duration::from_secs(*value))
            .collect()
    }

    #[test]
    fn rate_limits_honor_retry_after_up_to_sixty_seconds_then_become_pending() {
        for (status, header, expected) in [
            (429, Some("2"), secs(&[2])),
            (529, Some("2"), secs(&[2])),
            (429, Some("60"), secs(&[60])),
            (429, Some("0"), secs(&[0])),
            // No valid header falls back to the exponential schedule.
            (429, None, secs(&[1])),
            (529, None, secs(&[1])),
            (429, Some("soon"), secs(&[1])),
        ] {
            let stub = Stub::serving(vec![Reply::Status(status, header), Reply::Ok]);
            assert!(read(&stub.remote()).is_ok(), "{status} {header:?}");
            assert_eq!(stub.requests(), 2, "{status} {header:?}");
            assert_eq!(stub.waits(), expected, "{status} {header:?}");
        }
        for status in [429, 529] {
            let stub = Stub::serving(vec![Reply::Status(status, Some("61"))]);
            assert_eq!(read(&stub.remote()), Err(RemoteError::Pending));
            assert_eq!(stub.requests(), 1);
            assert!(stub.waits().is_empty(), "a long wait must not be slept");
        }
    }

    #[test]
    fn repeated_rate_limits_back_off_exponentially_within_the_attempt_bound() {
        let stub = Stub::serving(vec![
            Reply::Status(429, None),
            Reply::Status(529, None),
            Reply::Status(429, None),
            Reply::Ok,
        ]);
        assert!(read(&stub.remote()).is_ok());
        assert_eq!(stub.waits(), secs(&[1, 2, 4]));
    }

    #[test]
    fn idempotent_reads_retry_5xx_and_transport_errors_and_stop_at_five_attempts() {
        for status in [500, 502, 503, 504] {
            let stub = Stub::serving(vec![Reply::Status(status, None), Reply::Ok]);
            assert!(read(&stub.remote()).is_ok(), "{status}");
            assert_eq!(stub.requests(), 2);
            assert_eq!(stub.waits(), secs(&[1]));
        }
        let stub = Stub::serving(vec![Reply::Drop, Reply::Ok]);
        assert!(read(&stub.remote()).is_ok());
        assert_eq!(stub.requests(), 2);
        assert_eq!(stub.waits(), secs(&[1]));

        let exhausted = Stub::serving(vec![Reply::Status(503, None); 5]);
        assert_eq!(read(&exhausted.remote()), Err(RemoteError::Pending));
        assert_eq!(exhausted.requests(), 5, "five total attempts, no sixth");
        assert_eq!(exhausted.waits(), secs(&[1, 2, 4, 8]));
    }

    #[test]
    fn writes_are_never_blindly_retried_after_5xx_or_a_lost_response() {
        for status in [500, 502, 503, 504] {
            let stub = Stub::serving(vec![Reply::Status(status, None), Reply::Ok]);
            assert_eq!(
                write(&stub.remote()),
                Err(RemoteError::Ambiguous),
                "{status}"
            );
            assert_eq!(stub.requests(), 1, "{status}");
            assert!(stub.waits().is_empty());
        }
        let stub = Stub::serving(vec![Reply::Drop, Reply::Ok]);
        assert_eq!(write(&stub.remote()), Err(RemoteError::Ambiguous));
        assert_eq!(stub.requests(), 1);
        // A rate limit means the write was refused, so it may be retried.
        let limited = Stub::serving(vec![Reply::Status(429, Some("1")), Reply::Ok]);
        assert!(write(&limited.remote()).is_ok());
        assert_eq!(limited.requests(), 2);
    }

    #[test]
    fn auth_permission_and_validation_errors_are_not_retried() {
        for (status, expected) in [
            (401, RemoteError::Blocked(SetupReason::CredentialInvalid)),
            (403, RemoteError::Blocked(SetupReason::PermissionDenied)),
            (404, RemoteError::Blocked(SetupReason::RootUnverifiable)),
            (400, RemoteError::Blocked(SetupReason::ResponseInvalid)),
            (409, RemoteError::Invalid),
            (418, RemoteError::Blocked(SetupReason::RemoteUnavailable)),
        ] {
            for is_write in [false, true] {
                let stub = Stub::serving(vec![Reply::Status(status, None), Reply::Ok]);
                let remote = stub.remote();
                let result = if is_write {
                    write(&remote)
                } else {
                    read(&remote)
                };
                assert_eq!(result, Err(expected), "{status} write={is_write}");
                assert_eq!(stub.requests(), 1, "{status} write={is_write}");
                assert!(stub.waits().is_empty());
            }
        }
    }

    #[test]
    fn cancellation_stops_before_a_request_and_during_a_retry_wait() {
        let stub = Stub::serving(Vec::new());
        let cancelled = AtomicBool::new(true);
        assert_eq!(
            stub.remote()
                .request("GET", "/blocks/x/children", None, false, &cancelled),
            Err(RemoteError::Pending)
        );
        assert_eq!(stub.requests(), 0);

        let stub = Stub::serving(vec![Reply::Status(503, None), Reply::Ok]);
        let remote = HttpRemote::new("secret".to_owned()).unwrap().with_stub(
            stub.base_url.clone(),
            Arc::new(|_, cancelled| {
                cancelled.store(true, Ordering::Release);
                Err(RemoteError::Pending)
            }),
        );
        assert_eq!(read(&remote), Err(RemoteError::Pending));
        assert_eq!(stub.requests(), 1, "no request after the wait is cancelled");
    }

    #[test]
    fn a_job_issuing_back_to_back_requests_cannot_starve_a_waiting_job() {
        const BURST: usize = 60;
        let stub = Stub::serving(vec![Reply::Ok; BURST + 1]);
        let busy = Arc::new(stub.remote());
        let waiting = Arc::new(stub.remote());
        let finished = Arc::new(AtomicUsize::new(0));
        let waiter_saw = Arc::new(AtomicUsize::new(usize::MAX));

        let burst = {
            let (busy, finished) = (Arc::clone(&busy), Arc::clone(&finished));
            std::thread::spawn(move || {
                for _ in 0..BURST {
                    read(&busy).unwrap();
                    finished.fetch_add(1, Ordering::SeqCst);
                }
            })
        };
        while finished.load(Ordering::SeqCst) < 3 {
            std::thread::sleep(Duration::from_millis(1));
        }
        let started_after = finished.load(Ordering::SeqCst);
        let waiter = {
            let (waiting, finished, saw) = (
                Arc::clone(&waiting),
                Arc::clone(&finished),
                Arc::clone(&waiter_saw),
            );
            std::thread::spawn(move || {
                read(&waiting).unwrap();
                saw.store(finished.load(Ordering::SeqCst), Ordering::SeqCst);
            })
        };
        burst.join().unwrap();
        waiter.join().unwrap();
        let served_after = waiter_saw.load(Ordering::SeqCst) - started_after;
        assert!(
            served_after <= 3,
            "the waiting job was served only after {served_after} more requests from the busy job"
        );
    }

    #[test]
    fn a_cancelled_waiter_leaves_the_line_without_blocking_those_behind_it() {
        let queue = Arc::new(RequestQueue {
            state: Mutex::new(QueueState::default()),
            turn_changed: std::sync::Condvar::new(),
        });
        let first = queue.enter(&AtomicBool::new(false)).unwrap();
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancelled_waiter = {
            let (queue, cancelled) = (Arc::clone(&queue), Arc::clone(&cancelled));
            std::thread::spawn(move || queue.enter(&cancelled).is_err())
        };
        std::thread::sleep(Duration::from_millis(30));
        let patient_waiter = {
            let queue = Arc::clone(&queue);
            std::thread::spawn(move || queue.enter(&AtomicBool::new(false)).is_ok())
        };
        std::thread::sleep(Duration::from_millis(30));
        cancelled.store(true, Ordering::Release);
        assert!(
            cancelled_waiter.join().unwrap(),
            "cancellation ends the wait"
        );
        drop(first);
        assert!(
            patient_waiter.join().unwrap(),
            "the next waiter still gets its turn"
        );
    }

    #[test]
    fn hierarchy_failures_fail_closed_at_every_level() {
        type Breaker = fn(&mut PageChild);
        let broken: [(&str, Breaker); 4] = [
            ("wrong kind", |child| child.kind = PageKind::Other),
            ("trashed", |child| child.trashed = true),
            ("inaccessible", |child| child.accessible = false),
            ("duplicate", |_| {}),
        ];
        for level in ["year", "month"] {
            for (label, break_child) in broken {
                let remote = Arc::new(FakeRemote::default());
                let client = NotionClient::scripted(remote.clone(), ROOT);
                let mut bad = page("bad", if level == "year" { "2026" } else { "2026-10" });
                break_child(&mut bad);
                let mut listing = vec![bad.clone()];
                if label == "duplicate" {
                    listing.push(bad);
                }
                if level == "year" {
                    remote.seed_children(ROOT, listing);
                } else {
                    remote.seed_children(ROOT, vec![page("year", "2026")]);
                    remote.seed_children("year", listing);
                }
                assert_eq!(
                    client.preflight(&day("2026-10-30"), "body").unwrap_err(),
                    "notion-hierarchy-ambiguous",
                    "{level} {label}"
                );
                let request = NotionRequest {
                    day: day("2026-10-30"),
                    markdown: "body".into(),
                    expected_target_identity: ROOT.into(),
                    expected_payload_digest: Digest::parse(
                        render_notion(&day("2026-10-30"), "body")
                            .unwrap()
                            .payload_digest,
                    )
                    .unwrap(),
                    expected_state_digest: Digest::of_bytes(b"stale"),
                };
                assert_eq!(
                    client.write(&request, &AtomicBool::new(false)),
                    WriteOutcome::Unavailable,
                    "{level} {label}"
                );
                assert_eq!(remote.0.lock().unwrap().creates, 0, "{level} {label}");
            }
        }
    }

    #[test]
    fn remote_errors_map_to_content_free_reasons_at_every_level() {
        for (error, reason) in [
            (
                RemoteError::Blocked(SetupReason::CredentialInvalid),
                "notion-credential-invalid",
            ),
            (
                RemoteError::Blocked(SetupReason::PermissionDenied),
                "notion-permission-denied",
            ),
            (
                RemoteError::Blocked(SetupReason::RootUnverifiable),
                "notion-root-unverifiable",
            ),
            (
                RemoteError::Blocked(SetupReason::RemoteUnavailable),
                "notion-remote-unavailable",
            ),
            (RemoteError::Pending, "notion-retry-deferred"),
            (RemoteError::Invalid, "notion-response-invalid"),
        ] {
            for parent in [ROOT, "year"] {
                let remote = Arc::new(FakeRemote::default());
                remote.seed_children(ROOT, vec![page("year", "2026")]);
                remote.fail_children_of(parent, error);
                let client = NotionClient::scripted(remote, ROOT);
                assert_eq!(
                    client.preflight(&day("2026-10-30"), "body").unwrap_err(),
                    reason
                );
            }
        }
    }

    #[test]
    fn unrelated_siblings_do_not_block_publication() {
        let remote = Arc::new(FakeRemote::default());
        remote.seed_children(ROOT, vec![page("misc", "Notes"), page("year", "2026")]);
        remote.seed_children(
            "year",
            vec![page("other", "Trips"), page("month", "2026-10")],
        );
        remote.seed_blocks(
            "month",
            vec![
                remote_heading("old", "2026-10-29 (THU)"),
                remote_heading("other", "scratch"),
            ],
        );
        let client = NotionClient::scripted(remote.clone(), ROOT);
        let request = request_for(&client, "2026-10-30", "body");
        assert!(matches!(
            client.write(&request, &AtomicBool::new(false)),
            WriteOutcome::Verified { .. }
        ));
        let state = remote.0.lock().unwrap();
        assert_eq!(state.creates, 0);
        assert_eq!(state.blocks["month"].len(), 4);
        assert_eq!(state.blocks["month"][0].id, "old");
    }

    #[test]
    fn lost_year_or_month_create_reply_is_inspected_before_any_further_write() {
        for lost_on in [1, 2] {
            let remote = Arc::new(FakeRemote::default());
            let client = NotionClient::scripted(remote.clone(), ROOT);
            if lost_on == 2 {
                // The year already exists; only the month reply is lost.
                remote.seed_children(ROOT, vec![page("year", "2026")]);
            }
            let request = request_for(&client, "2026-10-30", "body");
            remote.lose_next_create_reply();
            assert_eq!(
                client.write(&request, &AtomicBool::new(false)),
                WriteOutcome::AmbiguousWrite,
                "lost create #{lost_on}"
            );
            let creates_after_loss = remote.0.lock().unwrap().creates;
            assert_eq!(creates_after_loss, 1);
            assert!(remote.0.lock().unwrap().blocks.is_empty());
            // The committed page is adopted by a fresh inspection, never recreated.
            let retry = request_for(&client, "2026-10-30", "body");
            assert!(matches!(
                client.write(&retry, &AtomicBool::new(false)),
                WriteOutcome::Verified { .. }
            ));
            let state = remote.0.lock().unwrap();
            let pages: usize = state.children.values().map(Vec::len).sum();
            assert_eq!(pages, 2, "exactly one year and one month page");
        }
    }

    #[test]
    fn lost_read_back_reply_reconciles_without_a_second_append() {
        let remote = Arc::new(FakeRemote::default());
        let client = NotionClient::scripted(remote.clone(), ROOT);
        let request = request_for(&client, "2026-10-30", "body");
        // Block listings during a write: same-day scan (1), read-back after append (2).
        remote.fail_list_blocks_on_call(2, RemoteError::Ambiguous);
        assert_eq!(
            client.write(&request, &AtomicBool::new(false)),
            WriteOutcome::AmbiguousWrite
        );
        assert_eq!(remote.0.lock().unwrap().append_anchors.len(), 1);
        assert!(matches!(
            client.inspect(&request),
            InspectionOutcome::Matching { .. }
        ));
        assert_eq!(remote.0.lock().unwrap().append_anchors.len(), 1);
    }

    #[test]
    fn unowned_same_day_heading_is_a_conflict_and_nothing_is_written() {
        let remote = Arc::new(FakeRemote::default());
        remote.seed_children(ROOT, vec![page("year", "2026")]);
        remote.seed_children("year", vec![page("month", "2026-10")]);
        remote.seed_blocks("month", vec![remote_heading("theirs", "2026-10-30 (FRI)")]);
        let client = NotionClient::scripted(remote.clone(), ROOT);
        let preflight = client.preflight(&day("2026-10-30"), "body").unwrap();
        assert_eq!(preflight.status, NotionPreflightStatus::Conflict);
        let request = NotionRequest {
            day: day("2026-10-30"),
            markdown: "body".into(),
            expected_target_identity: preflight.target_identity,
            expected_payload_digest: preflight.payload_digest,
            expected_state_digest: preflight.observed_state_digest,
        };
        assert_eq!(
            client.write(&request, &AtomicBool::new(false)),
            WriteOutcome::Conflict
        );
        assert!(remote.0.lock().unwrap().append_anchors.is_empty());
        assert_eq!(remote.0.lock().unwrap().blocks["month"].len(), 1);
    }

    #[test]
    fn changed_remote_state_or_cancellation_voids_the_write() {
        let remote = Arc::new(FakeRemote::default());
        let client = NotionClient::scripted(remote.clone(), ROOT);
        let request = request_for(&client, "2026-10-30", "body");
        assert_eq!(
            client.write(&request, &AtomicBool::new(true)),
            WriteOutcome::DefiniteNoEffect
        );
        remote.seed_children(ROOT, vec![page("year", "2026")]);
        assert_eq!(
            client.write(&request, &AtomicBool::new(false)),
            WriteOutcome::Conflict
        );
        let state = remote.0.lock().unwrap();
        assert_eq!(state.creates, 0);
        assert!(state.append_anchors.is_empty());
    }

    #[test]
    fn verified_section_is_adopted_again_without_another_write() {
        let remote = Arc::new(FakeRemote::default());
        let client = NotionClient::scripted(remote.clone(), ROOT);
        let request = request_for(&client, "2026-10-30", "body");
        let WriteOutcome::Verified { returned_ids, .. } =
            client.write(&request, &AtomicBool::new(false))
        else {
            panic!("first write verifies");
        };
        let InspectionOutcome::Matching {
            returned_ids: again,
            ..
        } = client.inspect(&request)
        else {
            panic!("restart inspection matches the receipt");
        };
        assert_eq!(returned_ids, again);
        assert_eq!(remote.0.lock().unwrap().append_anchors.len(), 1);
    }
}
