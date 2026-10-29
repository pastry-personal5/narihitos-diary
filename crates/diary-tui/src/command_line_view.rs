//! The Command Line's dedicated one-row presentation zone.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::widgets::Widget;

use crate::display::{truncate, width};
use crate::sanitize::sanitize_for_display;
use crate::theme::Palette;

pub struct CommandLineWidget<'a> {
    active: bool,
    raw: &'a str,
    feedback: Option<&'a str>,
    palette: Palette,
}

impl<'a> CommandLineWidget<'a> {
    #[must_use]
    pub const fn new(active: bool, raw: &'a str, feedback: Option<&'a str>) -> Self {
        Self {
            active,
            raw,
            feedback,
            palette: Palette::new(true),
        }
    }

    #[must_use]
    pub const fn palette(mut self, palette: Palette) -> Self {
        self.palette = palette;
        self
    }

    #[must_use]
    pub fn text(&self, width: u16) -> String {
        let text = if self.active {
            format!("Command> {}_", sanitize_for_display(self.raw))
        } else {
            self.feedback
                .map(|message| format!("Command: {}", sanitize_for_display(message)))
                .unwrap_or_default()
        };
        truncate(&text, width)
    }

    #[must_use]
    pub fn cursor_column(&self, area_width: u16) -> Option<u16> {
        if !self.active || area_width == 0 {
            return None;
        }
        let prefix = format!("Command> {}", sanitize_for_display(self.raw));
        Some(
            u16::try_from(width(&prefix))
                .unwrap_or(u16::MAX)
                .min(area_width - 1),
        )
    }
}

impl Widget for CommandLineWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.is_empty() {
            return;
        }
        buf.set_string(
            area.x,
            area.y,
            self.text(area.width),
            if self.active {
                self.palette.current()
            } else {
                self.palette.plain()
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;

    #[test]
    fn active_input_and_feedback_have_a_dedicated_bounded_line() {
        let active = CommandLineWidget::new(true, "stage sources", None);
        assert_eq!(active.text(80), "Command> stage sources_");

        let closed = CommandLineWidget::new(false, "", Some("Opened Sources"));
        assert_eq!(closed.text(80), "Command: Opened Sources");
        assert!(width(&closed.text(10)) <= 10);
    }

    #[test]
    fn cursor_columns_use_display_width_for_korean_input() {
        let widget = CommandLineWidget::new(true, "한", None);
        assert_eq!(widget.cursor_column(80), Some(11));
    }

    #[test]
    fn widget_renders_only_in_its_one_row_area() {
        let backend = TestBackend::new(30, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                frame.render_widget(CommandLineWidget::new(true, "help", None), frame.area());
            })
            .unwrap();
        let row: String = (0..30)
            .map(|x| terminal.backend().buffer()[(x, 0)].symbol())
            .collect();
        assert!(row.contains("Command> help_"));
    }
}
