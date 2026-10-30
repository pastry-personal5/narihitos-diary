//! The Help Overlay (P1-M16): a modal pane over the whole frame listing the
//! Sources-stage shortcuts. It sizes itself to the frame with a margin, wraps
//! its text to the pane, and scrolls when the text is longer than the pane.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, Clear, Paragraph, Widget};

use crate::display::{truncate, width};
use crate::sanitize::sanitize_for_display;
use crate::theme::Palette;

/// Columns and rows left around the overlay: 76x26 inside the 80x30 minimum.
const MARGIN: u16 = 2;

#[must_use]
pub fn overlay_area(frame: Rect) -> Rect {
    Rect {
        x: frame.x + MARGIN.min(frame.width / 2),
        y: frame.y + MARGIN.min(frame.height / 2),
        width: frame.width.saturating_sub(MARGIN * 2),
        height: frame.height.saturating_sub(MARGIN * 2),
    }
}

/// Wraps every line to `max_width` display columns, breaking at spaces and
/// hard-splitting a word that is wider than the pane. A blank line stays blank
/// and a leading indent is kept on continuation lines.
#[must_use]
pub fn wrap_lines(lines: &[String], max_width: u16) -> Vec<String> {
    let limit = usize::from(max_width.max(1));
    let mut out = Vec::new();
    for line in lines {
        if width(line) <= limit {
            out.push(line.clone());
            continue;
        }
        let indent: String = line.chars().take_while(|ch| *ch == ' ').collect();
        let mut current = String::new();
        for word in line.split_whitespace() {
            let candidate = if current.is_empty() {
                format!("{indent}{word}")
            } else {
                format!("{current} {word}")
            };
            if width(&candidate) <= limit {
                current = candidate;
                continue;
            }
            if !current.is_empty() {
                out.push(std::mem::take(&mut current));
            }
            let mut rest = format!("{indent}{word}");
            while width(&rest) > limit {
                let head = truncate(&rest, max_width.max(1));
                if head.is_empty() {
                    break;
                }
                rest = rest[head.len()..].to_owned();
                out.push(head);
            }
            current = rest;
        }
        if !current.is_empty() {
            out.push(current);
        }
    }
    out
}

pub struct HelpOverlayWidget<'a> {
    title: &'a str,
    lines: &'a [String],
    scroll: u16,
    palette: Palette,
}

impl<'a> HelpOverlayWidget<'a> {
    #[must_use]
    pub const fn new(title: &'a str, lines: &'a [String], scroll: u16) -> Self {
        Self {
            title,
            lines,
            scroll,
            palette: Palette::new(true),
        }
    }

    #[must_use]
    pub const fn palette(mut self, palette: Palette) -> Self {
        self.palette = palette;
        self
    }

    /// The furthest useful scroll offset for `area`: a scroll past it would
    /// only show blank rows, so callers clamp their stored offset to it.
    #[must_use]
    pub fn max_scroll(lines: &[String], area: Rect) -> u16 {
        let inner_width = area.width.saturating_sub(2);
        let inner_height = area.height.saturating_sub(2);
        let total = wrap_lines(lines, inner_width).len();
        u16::try_from(total.saturating_sub(usize::from(inner_height))).unwrap_or(u16::MAX)
    }
}

impl Widget for HelpOverlayWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        Clear.render(area, buf);
        let inner_width = area.width.saturating_sub(2);
        let wrapped = wrap_lines(self.lines, inner_width);
        let scroll = self.scroll.min(Self::max_scroll(self.lines, area));
        let more = if scroll < Self::max_scroll(self.lines, area) {
            " · more below"
        } else {
            ""
        };
        let body = wrapped
            .iter()
            .map(|line| sanitize_for_display(line))
            .collect::<Vec<_>>()
            .join("\n");
        Paragraph::new(body)
            .block(
                Block::bordered()
                    .title(format!("{}{more}", self.title))
                    .border_style(self.palette.border(true)),
            )
            .scroll((scroll, 0))
            .render(area, buf);
    }
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;
    use crate::ime::ImeState;
    use crate::source_shortcuts::{SHORTCUTS, help_lines};

    #[test]
    fn overlay_area_leaves_a_margin_and_is_76_by_26_at_the_minimum_size() {
        let area = overlay_area(Rect::new(0, 0, 80, 30));
        assert_eq!((area.x, area.y, area.width, area.height), (2, 2, 76, 26));
        let big = overlay_area(Rect::new(0, 0, 200, 60));
        assert_eq!((big.width, big.height), (196, 56));
    }

    #[test]
    fn wrapping_never_exceeds_the_pane_and_keeps_every_word() {
        let lines = help_lines(ImeState::English);
        for pane in [20_u16, 40, 74, 120] {
            let wrapped = wrap_lines(&lines, pane);
            assert!(wrapped.iter().all(|line| width(line) <= usize::from(pane)));
            let original: String = lines.concat().split_whitespace().collect();
            let after: String = wrapped.concat().split_whitespace().collect();
            assert_eq!(original, after, "pane {pane}");
        }
    }

    fn rendered(width_cols: u16, height_rows: u16, scroll: u16) -> String {
        let lines = help_lines(ImeState::English);
        let mut terminal = Terminal::new(TestBackend::new(width_cols, height_rows)).unwrap();
        terminal
            .draw(|frame| {
                let area = overlay_area(frame.area());
                frame.render_widget(
                    HelpOverlayWidget::new("Sources shortcuts", &lines, scroll),
                    area,
                );
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn minimum_size_shows_the_title_and_scrolls_to_reveal_every_shortcut() {
        let first = rendered(80, 30, 0);
        assert!(first.contains("Sources shortcuts"));
        let lines = help_lines(ImeState::English);
        let max = HelpOverlayWidget::max_scroll(&lines, overlay_area(Rect::new(0, 0, 80, 30)));
        let mut seen = String::new();
        for offset in 0..=max {
            seen.push_str(&rendered(80, 30, offset));
        }
        for entry in SHORTCUTS {
            assert!(
                seen.contains(entry.label),
                "{} not reachable at 80x30",
                entry.label
            );
        }
    }

    #[test]
    fn a_large_terminal_needs_no_scrolling() {
        let lines = help_lines(ImeState::English);
        let max = HelpOverlayWidget::max_scroll(&lines, overlay_area(Rect::new(0, 0, 120, 60)));
        assert_eq!(max, 0);
        let screen = rendered(120, 60, 0);
        for entry in SHORTCUTS {
            assert!(screen.contains(entry.label));
        }
    }

    #[test]
    fn overscroll_is_clamped_to_the_last_page() {
        assert_eq!(
            rendered(80, 30, 500),
            rendered(80, 30, {
                let lines = help_lines(ImeState::English);
                HelpOverlayWidget::max_scroll(&lines, overlay_area(Rect::new(0, 0, 80, 30)))
            })
        );
    }
}
