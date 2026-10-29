//! The full-width Work Status pane: always exactly four content rows
//! (excluding its border and title), each one a fixed priority that
//! truncates by display width rather than shrinking the pane. See the
//! architecture doc's "Work Status and compact status" section.

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, Widget};

use crate::display::truncate;
use crate::theme::Palette;
/// The four fixed priorities, in render order. An empty priority renders as
/// a blank row; it never causes the other rows to shift or the pane to
/// shrink.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct WorkStatusModel {
    pub current_work: Option<String>,
    pub pending_or_blocked: Option<String>,
    pub connection_and_save_safety: Option<String>,
    pub notice: Option<String>,
}

impl WorkStatusModel {
    /// Returns the four rows, each truncated to fit `width` display
    /// columns. `width` is the pane's inner width (excluding the border).
    #[must_use]
    pub fn rows(&self, width: u16) -> [String; 4] {
        [
            truncate(self.current_work.as_deref().unwrap_or(""), width),
            truncate(self.pending_or_blocked.as_deref().unwrap_or(""), width),
            truncate(
                self.connection_and_save_safety.as_deref().unwrap_or(""),
                width,
            ),
            truncate(self.notice.as_deref().unwrap_or(""), width),
        ]
    }
}

pub struct WorkStatusWidget<'a> {
    model: &'a WorkStatusModel,
    palette: Palette,
}

impl<'a> WorkStatusWidget<'a> {
    #[must_use]
    pub const fn new(model: &'a WorkStatusModel) -> Self {
        Self {
            model,
            palette: Palette::new(true),
        }
    }

    #[must_use]
    pub const fn palette(mut self, palette: Palette) -> Self {
        self.palette = palette;
        self
    }

    /// Renders into `frame` at `area`. A convenience over the `Widget`
    /// impl for call sites that already have a `Frame`.
    pub fn render_in(self, frame: &mut Frame<'_>, area: Rect) {
        frame.render_widget(self, area);
    }
}

impl Widget for WorkStatusWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let block = Block::bordered()
            .title("Work Status")
            .border_style(self.palette.border(false));
        let inner = block.inner(area);
        block.render(area, buf);
        for (row_index, row) in self.model.rows(inner.width).into_iter().enumerate() {
            let Some(y) = inner.y.checked_add(row_index as u16) else {
                break;
            };
            if y >= inner.y + inner.height {
                break;
            }
            buf.set_string(inner.x, y, row, self.palette.plain());
        }
    }
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::text::Line;

    use super::*;
    use crate::ime::ImeState;
    use crate::status_line::StatusLineModel;

    #[test]
    fn four_rows_render_in_fixed_priority_order_inside_the_border() {
        let model = WorkStatusModel {
            current_work: Some("Starting workflow".to_owned()),
            pending_or_blocked: Some("Preparing 2 sources".to_owned()),
            connection_and_save_safety: Some("Connected, saved".to_owned()),
            notice: Some("Press Ctrl-G for Command Line".to_owned()),
        };
        let backend = TestBackend::new(40, 6);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                WorkStatusWidget::new(&model).render_in(frame, frame.area());
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        assert_buffer_row_contains(buffer, 1, "Starting workflow");
        assert_buffer_row_contains(buffer, 2, "Preparing 2 sources");
        assert_buffer_row_contains(buffer, 3, "Connected, saved");
        assert_buffer_row_contains(buffer, 4, "Press Ctrl-G for Command Line");
    }

    #[test]
    fn an_empty_priority_renders_a_blank_row_rather_than_shifting_others() {
        let model = WorkStatusModel {
            current_work: Some("Starting workflow".to_owned()),
            pending_or_blocked: None,
            connection_and_save_safety: Some("Connected".to_owned()),
            notice: None,
        };
        let backend = TestBackend::new(30, 6);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                WorkStatusWidget::new(&model).render_in(frame, frame.area());
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        assert_buffer_row_contains(buffer, 1, "Starting workflow");
        assert_buffer_row_contains(buffer, 3, "Connected");
        // Row 2 (pending_or_blocked) is blank; the pane keeps its four
        // rows rather than collapsing to two.
        assert!(row_text(buffer, 2).trim().is_empty());
        assert!(row_text(buffer, 4).trim().is_empty());
    }

    #[test]
    fn a_long_message_truncates_by_display_width_without_panicking() {
        let model = WorkStatusModel {
            current_work: Some("x".repeat(500)),
            ..WorkStatusModel::default()
        };
        let rows = model.rows(20);
        assert_eq!(rows[0].chars().count(), 20);
    }

    #[test]
    fn display_width_truncation_counts_wide_korean_syllables_correctly() {
        // Each precomposed Hangul syllable is two display columns wide.
        let model = WorkStatusModel {
            current_work: Some("일기 쓰는 중입니다 계속".to_owned()),
            ..WorkStatusModel::default()
        };
        let rows = model.rows(8);
        assert!(
            Line::from(rows[0].as_str()).width() <= 8,
            "truncated row must respect the display-width budget, got {:?}",
            rows[0]
        );
    }

    #[test]
    fn connection_safety_may_intentionally_repeat_in_the_status_line() {
        let work = WorkStatusModel {
            connection_and_save_safety: Some("Connected; Draft saved".to_owned()),
            ..WorkStatusModel::default()
        };
        let status = StatusLineModel {
            diary_day: None,
            connection_label: "Connected",
            ime: ImeState::Unknown,
            action_hint: None,
        };
        assert!(work.rows(80)[2].contains("Connected"));
        assert!(status.render(80).contains("Connected"));
    }

    /// Reads only the inner columns of a row (excluding the left/right
    /// border glyphs at x=0 and x=width-1).
    fn row_text(buffer: &Buffer, y: u16) -> String {
        (1..buffer.area.width.saturating_sub(1))
            .map(|x| buffer[(x, y)].symbol())
            .collect()
    }

    fn assert_buffer_row_contains(buffer: &Buffer, y: u16, expected: &str) {
        let text = row_text(buffer, y);
        assert!(
            text.contains(expected),
            "row {y} ({text:?}) did not contain {expected:?}"
        );
    }
}
