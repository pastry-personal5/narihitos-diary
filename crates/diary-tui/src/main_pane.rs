//! The right Main Pane: the active resource's content for the current
//! stage. The Start stage shows a fixed instructional message; later
//! stages render prepared source bodies, the Draft editor, and the entry
//! review in separate work.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Paragraph, Widget, Wrap};

use crate::sanitize::sanitize_for_display;
use crate::theme::Palette;

/// Indents semantic diary prose beneath its heading without changing the
/// stored entry. Empty lines retain the same visual inset.
#[must_use]
pub fn indent_diary_block(text: &str) -> String {
    text.split('\n')
        .map(|line| format!("  {line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

pub struct MainPaneWidget<'a> {
    title: &'a str,
    body: &'a str,
    focused: bool,
    scroll: u16,
    diary_entry: Option<&'a str>,
    existing_entry: Option<&'a str>,
    palette: Palette,
}

impl<'a> MainPaneWidget<'a> {
    #[must_use]
    pub const fn new(title: &'a str, body: &'a str) -> Self {
        Self {
            title,
            body,
            focused: false,
            scroll: 0,
            diary_entry: None,
            existing_entry: None,
            palette: Palette::new(true),
        }
    }

    #[must_use]
    pub const fn focused(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }

    #[must_use]
    pub const fn scroll(mut self, scroll: u16) -> Self {
        self.scroll = scroll;
        self
    }

    /// Highlights the last exact occurrence of the confirmed diary entry in
    /// the body. Callers put that semantic block after their status/controls.
    #[must_use]
    pub const fn diary_entry(mut self, diary_entry: Option<&'a str>) -> Self {
        self.diary_entry = diary_entry;
        self
    }

    /// Highlights content already present at the selected destination.
    #[must_use]
    pub const fn existing_entry(mut self, existing_entry: Option<&'a str>) -> Self {
        self.existing_entry = existing_entry;
        self
    }

    #[must_use]
    pub const fn palette(mut self, palette: Palette) -> Self {
        self.palette = palette;
        self
    }
}

impl Widget for MainPaneWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // `body` may be untrusted source/entry text; sanitize only the
        // rendered copy, never the caller's own stored value.
        let title = if self.focused {
            format!("{} [focus]", self.title)
        } else {
            self.title.to_owned()
        };
        let body = sanitize_for_display(self.body);
        let diary_entry = self
            .diary_entry
            .filter(|entry| !entry.is_empty())
            .map(sanitize_for_display);
        let existing_entry = self
            .existing_entry
            .filter(|entry| !entry.is_empty())
            .map(sanitize_for_display);
        Paragraph::new(styled_body(
            body,
            diary_entry.as_deref(),
            existing_entry.as_deref(),
            self.palette.diary_entry(),
            self.palette.existing_entry(),
        ))
        .block(
            Block::bordered()
                .title(title)
                .border_style(self.palette.border(self.focused)),
        )
        .wrap(Wrap { trim: false })
        .scroll((self.scroll, 0))
        .render(area, buf);
    }
}

fn styled_body(
    body: String,
    diary_entry: Option<&str>,
    existing_entry: Option<&str>,
    diary_style: Style,
    existing_style: Style,
) -> Text<'static> {
    let mut highlights = Vec::new();
    if let Some(entry) = existing_entry
        && let Some(start) = body.find(entry)
    {
        highlights.push((start, start + entry.len(), existing_style));
    }
    if let Some(entry) = diary_entry
        && let Some(start) = body.rfind(entry)
    {
        highlights.push((start, start + entry.len(), diary_style));
    }
    highlights.sort_by_key(|(start, _, _)| *start);
    highlights.dedup_by(|left, right| left.0 == right.0 && left.1 == right.1);
    if highlights.is_empty() {
        return Text::raw(body);
    }
    let mut lines = Vec::new();
    let mut current = Vec::new();
    let mut cursor = 0;
    for (start, end, style) in highlights {
        if start < cursor {
            continue;
        }
        push_segment(&mut lines, &mut current, &body[cursor..start], Style::new());
        push_segment(&mut lines, &mut current, &body[start..end], style);
        cursor = end;
    }
    push_segment(&mut lines, &mut current, &body[cursor..], Style::new());
    lines.push(Line::from(current));
    Text::from(lines)
}

fn push_segment(
    lines: &mut Vec<Line<'static>>,
    current: &mut Vec<Span<'static>>,
    segment: &str,
    style: Style,
) {
    for part in segment.split_inclusive('\n') {
        let (text, newline) = part
            .strip_suffix('\n')
            .map_or((part, false), |text| (text, true));
        if !text.is_empty() {
            current.push(Span::styled(text.to_owned(), style));
        }
        if newline {
            lines.push(Line::from(std::mem::take(current)));
        }
    }
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;

    #[test]
    fn it_renders_the_title_and_body_without_panicking() {
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                frame.render_widget(
                    MainPaneWidget::new("Main", "Choose a workflow to continue."),
                    frame.area(),
                );
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let row: String = (1..buffer.area.width - 1)
            .map(|x| buffer[(x, 1)].symbol())
            .collect();
        assert!(row.contains("Choose a workflow"));
    }

    #[test]
    fn diary_entry_uses_its_semantic_foreground_only() {
        let backend = TestBackend::new(50, 8);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                frame.render_widget(
                    MainPaneWidget::new(
                        "Main",
                        "Status: ready\nAlready there:\n  An older diary line\n\nDiary entry to publish:\n  A vivid diary line",
                    )
                    .diary_entry(Some("  A vivid diary line"))
                    .existing_entry(Some("  An older diary line"))
                    .palette(Palette::new(false)),
                    frame.area(),
                );
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let status = &buffer[(1, 1)];
        let existing = &buffer[(1, 3)];
        let entry = &buffer[(1, 6)];
        assert_eq!(status.fg, ratatui::style::Color::Reset);
        assert_eq!(existing.fg, ratatui::style::Color::Cyan);
        assert_eq!(entry.fg, ratatui::style::Color::Magenta);
    }

    #[test]
    fn diary_block_indents_every_line_including_blank_lines() {
        assert_eq!(indent_diary_block("first\n\nthird"), "  first\n  \n  third");
    }
}
