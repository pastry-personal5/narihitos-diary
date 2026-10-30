//! The middle Resources Sidebar: renders the active stage's source,
//! draft, or destination resources with their explicit state, using
//! indented trees where a server resource has a hierarchy. The Start
//! stage has no resources yet; later stages (Sources, Compose, Review)
//! populate this with real server-backed entries in separate work.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, Widget};

use crate::date_display::format_diary_day;
use crate::display::{truncate, width};
use crate::sanitize::sanitize_for_display;
use crate::theme::Palette;

/// One resource row: a label and its explicit state (e.g. "included",
/// "excluded", "confirmed"). Indentation shows a server-reported
/// hierarchy; it is presentation-only and never implies inclusion.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResourceRow {
    pub label: String,
    pub state: String,
    pub indent: u8,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ResourcesSidebarModel {
    pub rows: Vec<ResourceRow>,
}

impl ResourcesSidebarModel {
    #[must_use]
    pub fn lines(&self) -> Vec<String> {
        if self.rows.is_empty() {
            return vec!["(no resources)".to_owned()];
        }
        self.rows
            .iter()
            .map(|row| {
                let indent = "  ".repeat(usize::from(row.indent));
                // `label` may be a server-supplied source-note name:
                // sanitize only the rendered copy, never the stored row.
                format!(
                    "{indent}{} ({})",
                    sanitize_for_display(&row.label),
                    sanitize_for_display(&row.state)
                )
            })
            .collect()
    }
}

pub struct ResourcesSidebarWidget<'a> {
    model: &'a ResourcesSidebarModel,
    active: Option<usize>,
    focused_row: Option<usize>,
    focused: bool,
    palette: Palette,
}

impl<'a> ResourcesSidebarWidget<'a> {
    #[must_use]
    pub const fn new(model: &'a ResourcesSidebarModel) -> Self {
        Self {
            model,
            active: None,
            focused_row: None,
            focused: false,
            palette: Palette::new(true),
        }
    }

    #[must_use]
    pub const fn active(mut self, active: Option<usize>) -> Self {
        self.active = active;
        self
    }

    #[must_use]
    pub const fn focused_row(mut self, focused_row: Option<usize>) -> Self {
        self.focused_row = focused_row;
        self
    }

    #[must_use]
    pub const fn focused(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }

    #[must_use]
    pub const fn palette(mut self, palette: Palette) -> Self {
        self.palette = palette;
        self
    }
}

impl Widget for ResourcesSidebarWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let base_block = Block::bordered();
        let inner = base_block.inner(area);
        let lines = self.model.lines();
        let capacity = usize::from(inner.height);
        let selected = self.focused_row.or(self.active).unwrap_or(0);
        let start = visible_start(lines.len(), selected, capacity);
        let end = start.saturating_add(capacity).min(lines.len());
        let arrows = match (start > 0, end < lines.len()) {
            (true, true) => " ↑↓",
            (true, false) => " ↑",
            (false, true) => " ↓",
            (false, false) => "",
        };
        let title = if self.focused {
            format!("Resources [focus]{arrows}")
        } else {
            format!("Resources{arrows}")
        };
        let block = base_block
            .title(title)
            .border_style(self.palette.border(self.focused));
        block.render(area, buf);
        for (visible_index, row_index) in (start..end).enumerate() {
            let Some(y) = inner.y.checked_add(visible_index as u16) else {
                break;
            };
            let marker = if self.active == Some(row_index) {
                "> "
            } else {
                "  "
            };
            let focus_marker = if self.focused_row == Some(row_index) {
                "*"
            } else {
                " "
            };
            let prefix = format!("{focus_marker}{marker}");
            let available = inner
                .width
                .saturating_sub(u16::try_from(width(&prefix)).unwrap_or(u16::MAX));
            let line = format_row(
                &self.model.rows.get(row_index),
                &lines[row_index],
                available,
            );
            buf.set_string(
                inner.x,
                y,
                format!("{prefix}{line}"),
                if self.focused_row == Some(row_index) || self.active == Some(row_index) {
                    self.palette.current()
                } else {
                    self.palette.plain()
                },
            );
        }
    }
}

fn visible_start(total: usize, selected: usize, capacity: usize) -> usize {
    if total <= capacity || capacity == 0 {
        0
    } else {
        selected
            .saturating_sub(capacity - 1)
            .min(total.saturating_sub(capacity))
    }
}

fn format_row(row: &Option<&ResourceRow>, fallback: &str, available: u16) -> String {
    let Some(row) = row else {
        return truncate(fallback, available);
    };
    let indent = "  ".repeat(usize::from(row.indent));
    let state = sanitize_for_display(&row.state);
    let suffix = format!(" ({state})");
    let suffix_width = u16::try_from(width(&suffix)).unwrap_or(u16::MAX);
    let indent_width = u16::try_from(width(&indent)).unwrap_or(u16::MAX);
    let label_budget = available.saturating_sub(indent_width.saturating_add(suffix_width));
    let safe_label = sanitize_for_display(&row.label);
    let label = format_diary_day(&safe_label, label_budget);
    truncate(&format!("{indent}{label}{suffix}"), available)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_model_renders_a_placeholder_rather_than_nothing() {
        let model = ResourcesSidebarModel::default();
        assert_eq!(model.lines(), vec!["(no resources)".to_owned()]);
    }

    #[test]
    fn rows_render_with_their_explicit_state_and_hierarchy_indent() {
        let model = ResourcesSidebarModel {
            rows: vec![
                ResourceRow {
                    label: "2026-10-28 morning note".to_owned(),
                    state: "included".to_owned(),
                    indent: 0,
                },
                ResourceRow {
                    label: "attachment".to_owned(),
                    state: "ignored".to_owned(),
                    indent: 1,
                },
            ],
        };
        let lines = model.lines();
        assert_eq!(lines[0], "2026-10-28 morning note (included)");
        assert_eq!(lines[1], "  attachment (ignored)");
    }

    #[test]
    fn scrolling_keeps_the_focused_resource_visible() {
        assert_eq!(visible_start(20, 18, 5), 14);
        assert_eq!(visible_start(20, 2, 5), 0);
    }

    #[test]
    fn a_workflow_day_gains_a_weekday_only_when_the_row_has_room() {
        let row = ResourceRow {
            label: "2026-10-29".to_owned(),
            state: "draft".to_owned(),
            indent: 0,
        };
        assert_eq!(format_row(&Some(&row), "", 30), "2026-10-29 (Thu) (draft)");
        assert_eq!(format_row(&Some(&row), "", 18), "2026-10-29 (draft)");
    }
}
