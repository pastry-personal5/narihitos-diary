//! Canonical Markdown projection and payload digest.

use super::*;

pub(super) fn chunk_blocks(blocks: &[NotionBlock]) -> Vec<&[NotionBlock]> {
    let mut chunks = Vec::new();
    let mut start = 0;
    while start < blocks.len() {
        let mut end = start;
        let mut bytes: usize = 0;
        while end < blocks.len() && end - start < MAX_APPEND_BLOCKS {
            let block_bytes =
                serde_json::to_vec(&blocks[end]).map_or(usize::MAX, |value| value.len());
            if end > start && bytes.saturating_add(block_bytes) > 450 * 1024 {
                break;
            }
            bytes = bytes.saturating_add(block_bytes);
            end += 1;
        }
        chunks.push(&blocks[start..end]);
        start = end;
    }
    chunks
}

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
        chunk_blocks(&self.blocks)
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

pub(super) fn canonical_titles(day: &DiaryDay) -> Result<(String, String, String), NotionError> {
    let value = day.as_str();
    let year = value[0..4].to_owned();
    let month = value[0..7].to_owned();
    let weekday = weekday(day)?;
    Ok((year, month, format!("{value} ({weekday})")))
}

pub(super) fn weekday(day: &DiaryDay) -> Result<&'static str, NotionError> {
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

pub(super) fn parse_digits(bytes: &[u8]) -> Result<u32, NotionError> {
    bytes.iter().try_fold(0_u32, |value, byte| {
        byte.is_ascii_digit()
            .then(|| value * 10 + u32::from(byte - b'0'))
            .ok_or(NotionError::InvalidPlan)
    })
}

pub(super) fn plan_digest(
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

pub(super) fn validate_blocks(blocks: &[NotionBlock]) -> Result<(), NotionError> {
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

pub(super) fn text_block(kind: BlockKind, text: String) -> NotionBlock {
    NotionBlock {
        kind,
        rich_text: split_rich_text(&text, NotionTextAnnotations::default(), None),
        language: None,
        children: Vec::new(),
    }
}

pub(super) fn split_rich_text(
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
pub(super) struct InlineState {
    annotations: NotionTextAnnotations,
    link: Option<String>,
}

pub(super) struct OpenBlock {
    kind: BlockKind,
    rich_text: Vec<NotionRichText>,
    language: Option<String>,
    children: Vec<NotionBlock>,
}

pub(super) struct Renderer<'a> {
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
