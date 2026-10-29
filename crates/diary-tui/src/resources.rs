//! The middle Resources Sidebar: renders the active stage's source,
//! draft, or destination resources with their explicit state, using
//! indented trees where a server resource has a hierarchy. P1-M7's Start
//! stage has no resources yet; later stages (Sources, Compose, Review)
//! populate this with real server-backed entries in separate work.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, Widget};

use crate::sanitize::sanitize_for_display;

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
                    row.state
                )
            })
            .collect()
    }
}

pub struct ResourcesSidebarWidget<'a> {
    model: &'a ResourcesSidebarModel,
}

impl<'a> ResourcesSidebarWidget<'a> {
    #[must_use]
    pub const fn new(model: &'a ResourcesSidebarModel) -> Self {
        Self { model }
    }
}

impl Widget for ResourcesSidebarWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let block = Block::bordered().title("Resources");
        let inner = block.inner(area);
        block.render(area, buf);
        for (row_index, line) in self.model.lines().into_iter().enumerate() {
            let Some(y) = inner.y.checked_add(row_index as u16) else {
                break;
            };
            if y >= inner.y + inner.height {
                break;
            }
            buf.set_string(inner.x, y, line, ratatui::style::Style::default());
        }
    }
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
}
