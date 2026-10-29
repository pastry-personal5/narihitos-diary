//! The compact bottom Status Line: one row summarizing diary day, workflow
//! state, connection, IME, progress, short error state, and action hints.
//! It remains even though Work Status may intentionally repeat some of the
//! same information (see the architecture doc's "Work Status and compact
//! status" section).

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::Widget;

use crate::date_display::format_diary_day;
use crate::ime::ImeState;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StatusLineModel {
    /// Raw `YYYY-MM-DD`, never pre-formatted with a weekday: `render`
    /// decides whether `(Www)` fits using the actual area width.
    pub diary_day: Option<String>,
    pub connection_label: &'static str,
    pub ime: ImeState,
    pub action_hint: Option<String>,
}

impl StatusLineModel {
    /// Joins the fixed segments with " | ", skipping any that are absent,
    /// then truncates the whole line to `width` display columns. The
    /// diary day renders with its abbreviated weekday only where that
    /// still leaves room for the rest of the line.
    #[must_use]
    pub fn render(&self, width: u16) -> String {
        let mut segments = Vec::new();
        if let Some(day) = &self.diary_day {
            let plain_rest: Vec<&str> = std::iter::once(self.connection_label)
                .chain(std::iter::once(self.ime.status_label()))
                .chain(self.action_hint.as_deref())
                .collect();
            let rest_width = Line::from(plain_rest.join(" | ").as_str()).width();
            // Separators: one after the day, one between each remaining
            // segment.
            let separator_width = 3 * plain_rest.len().max(1);
            let budget = usize::from(width).saturating_sub(rest_width + separator_width);
            segments.push(format_diary_day(day, budget as u16));
        }
        segments.push(self.connection_label.to_owned());
        segments.push(self.ime.status_label().to_owned());
        if let Some(hint) = &self.action_hint {
            segments.push(hint.clone());
        }
        truncate_to_width(&segments.join(" | "), width)
    }
}

fn truncate_to_width(text: &str, max_width: u16) -> String {
    let max_width = usize::from(max_width);
    let mut result = String::new();
    for ch in text.chars() {
        let mut candidate = result.clone();
        candidate.push(ch);
        if Line::from(candidate.as_str()).width() > max_width {
            break;
        }
        result = candidate;
    }
    result
}

pub struct StatusLineWidget<'a> {
    model: &'a StatusLineModel,
}

impl<'a> StatusLineWidget<'a> {
    #[must_use]
    pub const fn new(model: &'a StatusLineModel) -> Self {
        Self { model }
    }
}

impl Widget for StatusLineWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        buf.set_string(
            area.x,
            area.y,
            self.model.render(area.width),
            ratatui::style::Style::default(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segments_join_in_fixed_order_with_a_separator() {
        let model = StatusLineModel {
            diary_day: Some("2026-10-29".to_owned()),
            connection_label: "Connected",
            ime: ImeState::Korean,
            action_hint: Some("Ctrl-G: Command Line".to_owned()),
        };
        assert_eq!(
            model.render(80),
            "2026-10-29 (Thu) | Connected | IME: KO | Ctrl-G: Command Line"
        );
    }

    #[test]
    fn the_diary_day_falls_back_to_its_bare_form_when_the_weekday_would_not_fit() {
        let model = StatusLineModel {
            diary_day: Some("2026-10-29".to_owned()),
            connection_label: "Connected",
            ime: ImeState::Korean,
            action_hint: Some("Ctrl-G: Command Line".to_owned()),
        };
        // Wide enough for "2026-10-29 | Connected | IME: KO | Ctrl-G: Command Line"
        // (57 columns) but not for the 5 extra columns " (Thu)" would add.
        assert_eq!(
            model.render(57),
            "2026-10-29 | Connected | IME: KO | Ctrl-G: Command Line"
        );
    }

    #[test]
    fn a_missing_diary_day_and_hint_are_skipped_not_left_blank() {
        let model = StatusLineModel {
            diary_day: None,
            connection_label: "Not running",
            ime: ImeState::Unknown,
            action_hint: None,
        };
        assert_eq!(model.render(80), "Not running | IME: ?");
    }

    #[test]
    fn the_line_truncates_to_the_available_width() {
        let model = StatusLineModel {
            diary_day: Some("2026-10-29".to_owned()),
            connection_label: "Connected",
            ime: ImeState::English,
            action_hint: Some("Ctrl-G: Command Line, Ctrl-S: Save".to_owned()),
        };
        let rendered = model.render(10);
        assert_eq!(Line::from(rendered.as_str()).width(), 10);
    }
}
