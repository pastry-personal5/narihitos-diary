//! The left Workflow Sidebar: the six canonical stages in their fixed
//! order, each showing server-derived progress, plus one presentation-only
//! current marker. Navigation can change the current stage but never
//! synthesizes or mutates server workflow state (see `stage.rs`).

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, Widget};

use crate::stage::{ALL_STAGES, Progress, Stage, StageModel};

#[must_use]
pub const fn stage_label(stage: Stage) -> &'static str {
    match stage {
        Stage::Start => "Start",
        Stage::Sources => "Sources",
        Stage::Compose => "Compose",
        Stage::Review => "Review",
        Stage::Preview => "Preview",
        Stage::Publish => "Publish",
    }
}

#[must_use]
pub const fn progress_label(progress: Progress) -> &'static str {
    match progress {
        Progress::Locked => "locked",
        Progress::Available => "available",
        Progress::Stale => "stale",
        Progress::Done => "done",
    }
}

pub struct WorkflowSidebarWidget<'a> {
    model: &'a StageModel,
    current: Stage,
}

impl<'a> WorkflowSidebarWidget<'a> {
    #[must_use]
    pub const fn new(model: &'a StageModel, current: Stage) -> Self {
        Self { model, current }
    }

    /// One line per canonical stage, in fixed order: `"> Sources (available)"`
    /// for the current stage, `"  Sources (available)"` otherwise. The
    /// marker is presentation-only and never affects `progress_label`.
    #[must_use]
    pub fn lines(&self) -> [String; 6] {
        let mut lines: [String; 6] = Default::default();
        for (index, stage) in ALL_STAGES.into_iter().enumerate() {
            let marker = if stage == self.current { ">" } else { " " };
            lines[index] = format!(
                "{marker} {} ({})",
                stage_label(stage),
                progress_label(self.model.progress_of(stage))
            );
        }
        lines
    }
}

impl Widget for WorkflowSidebarWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let block = Block::bordered().title("Workflow");
        let inner = block.inner(area);
        block.render(area, buf);
        for (row_index, line) in self.lines().into_iter().enumerate() {
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
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;

    use super::*;
    use crate::stage::map_workflow;

    #[test]
    fn stages_render_in_canonical_order_with_progress_and_marker() {
        let model = map_workflow(None, false);
        let widget = WorkflowSidebarWidget::new(&model, Stage::Start);
        let lines = widget.lines();
        assert_eq!(lines[0], "> Start (available)");
        assert_eq!(lines[1], "  Sources (locked)");
        assert_eq!(lines[5], "  Publish (locked)");
    }

    #[test]
    fn the_current_marker_moves_without_changing_progress() {
        let model = map_workflow(None, false);
        let widget = WorkflowSidebarWidget::new(&model, Stage::Sources);
        let lines = widget.lines();
        assert_eq!(lines[0], "  Start (available)");
        assert_eq!(lines[1], "> Sources (locked)");
    }

    #[test]
    fn it_renders_inside_the_bordered_pane_without_panicking() {
        let model = map_workflow(None, false);
        let backend = TestBackend::new(24, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                frame.render_widget(
                    WorkflowSidebarWidget::new(&model, Stage::Start),
                    frame.area(),
                );
            })
            .unwrap();
        let buffer: &Buffer = terminal.backend().buffer();
        let row: String = (1..buffer.area.width - 1)
            .map(|x| buffer[(x, 1)].symbol())
            .collect();
        assert!(row.contains("Start"));
    }
}
