//! The left Workflow Sidebar: the six canonical stages in their fixed
//! order, each showing server-derived progress, plus one presentation-only
//! current marker. Navigation can change the current stage but never
//! synthesizes or mutates server workflow state (see `stage.rs`).

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, Widget};

use crate::stage::{ALL_STAGES, Progress, Stage, StageModel};
use crate::theme::Palette;

/// Two border columns plus the longest focused stage/progress row.
pub const MINIMUM_WORKFLOW_SIDEBAR_WIDTH: u16 = 31;

#[must_use]
pub const fn stage_label(stage: Stage) -> &'static str {
    match stage {
        Stage::Start => "Start",
        Stage::Sources => "Sources",
        Stage::Compose => "Compose",
        Stage::Review => "Review",
        Stage::Preview => "Preview",
        Stage::PublishLocal => "Publish Local",
        Stage::PublishRemote => "Publish Remote",
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
    focused: bool,
    focused_stage: Option<Stage>,
    palette: Palette,
}

impl<'a> WorkflowSidebarWidget<'a> {
    #[must_use]
    pub const fn new(model: &'a StageModel, current: Stage) -> Self {
        Self {
            model,
            current,
            focused: false,
            focused_stage: None,
            palette: Palette::new(true),
        }
    }

    #[must_use]
    pub const fn focused(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }

    #[must_use]
    pub const fn focused_stage(mut self, stage: Option<Stage>) -> Self {
        self.focused_stage = stage;
        self
    }

    #[must_use]
    pub const fn palette(mut self, palette: Palette) -> Self {
        self.palette = palette;
        self
    }

    /// One line per canonical stage, in fixed order: `"> Sources (available)"`
    /// for the current stage, `"  Sources (available)"` otherwise. The
    /// marker is presentation-only and never affects `progress_label`.
    #[must_use]
    pub fn lines(&self) -> [String; 7] {
        let mut lines: [String; 7] = Default::default();
        for (index, stage) in ALL_STAGES.into_iter().enumerate() {
            let marker = if stage == self.current { ">" } else { " " };
            let line = format!(
                "{marker} {} ({})",
                stage_label(stage),
                progress_label(self.model.progress_of(stage))
            );
            lines[index] = if self.focused_stage == Some(stage) {
                format!("*{line}")
            } else {
                line
            };
        }
        lines
    }
}

impl Widget for WorkflowSidebarWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let title = if self.focused {
            "Workflow [focus]"
        } else {
            "Workflow"
        };
        let block = Block::bordered()
            .title(title)
            .border_style(self.palette.border(self.focused));
        let inner = block.inner(area);
        block.render(area, buf);
        for (row_index, line) in self.lines().into_iter().enumerate() {
            let Some(y) = inner.y.checked_add(row_index as u16) else {
                break;
            };
            if y >= inner.y + inner.height {
                break;
            }
            let style = if self.focused_stage == Some(ALL_STAGES[row_index])
                || self.current == ALL_STAGES[row_index]
            {
                self.palette.current()
            } else {
                self.palette.plain()
            };
            buf.set_string(inner.x, y, line, style);
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
        assert_eq!(lines[5], "  Publish Local (locked)");
        assert_eq!(lines[6], "  Publish Remote (locked)");
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

    #[test]
    fn the_minimum_width_fits_the_longest_focused_row_exactly() {
        let model = StageModel {
            progress: [Progress::Available; 7],
            default_current: Stage::Sources,
        };
        let widget = WorkflowSidebarWidget::new(&model, Stage::PublishRemote)
            .focused(true)
            .focused_stage(Some(Stage::PublishRemote));
        let longest_row = widget
            .lines()
            .into_iter()
            .map(|line| line.len())
            .max()
            .unwrap();

        assert_eq!(longest_row + 2, usize::from(MINIMUM_WORKFLOW_SIDEBAR_WIDTH));

        let backend = TestBackend::new(MINIMUM_WORKFLOW_SIDEBAR_WIDTH, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| frame.render_widget(widget, frame.area()))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let row: String = (1..buffer.area.width - 1)
            .map(|x| buffer[(x, 7)].symbol())
            .collect();
        assert_eq!(row, "*> Publish Remote (available)");
    }
}
