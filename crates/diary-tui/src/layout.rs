//! The Standard Layout shell: a full-width Work Status pane above a left
//! Workflow Sidebar, middle Resources Sidebar, and right Main Pane, plus
//! the compact bottom Status Line. See the architecture doc's "Approach
//! and sequence" step 4.

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::widgets::{Paragraph, Widget};

pub const MIN_COLUMNS: u16 = 80;
pub const MIN_ROWS: u16 = 30;

/// Shown instead of the Standard Layout when the terminal is smaller than
/// 80x30; the client never tries to cram the real layout into less space.
pub struct TerminalTooSmallWidget;

impl Widget for TerminalTooSmallWidget {
    fn render(self, area: Rect, buf: &mut Buffer) {
        Paragraph::new(format!(
            "Terminal too small. Needs at least {MIN_COLUMNS}x{MIN_ROWS}."
        ))
        .render(area, buf);
    }
}

/// The Work Status pane is always exactly four content rows plus its
/// border and title, i.e. 6 terminal rows total.
const WORK_STATUS_HEIGHT: u16 = 6;
const STATUS_LINE_HEIGHT: u16 = 1;

#[must_use]
pub const fn is_supported_size(width: u16, height: u16) -> bool {
    width >= MIN_COLUMNS && height >= MIN_ROWS
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StandardAreas {
    pub work_status: Rect,
    pub workflow_sidebar: Rect,
    pub resources_sidebar: Rect,
    pub main_pane: Rect,
    pub status_line: Rect,
}

/// Splits `area` into the standard layout. Callers must check
/// [`is_supported_size`] first; this function does not itself fall back to
/// a "terminal too small" notice.
#[must_use]
pub fn standard_areas(area: Rect) -> StandardAreas {
    let [work_status, body, status_line] = Layout::vertical([
        Constraint::Length(WORK_STATUS_HEIGHT),
        Constraint::Min(0),
        Constraint::Length(STATUS_LINE_HEIGHT),
    ])
    .areas(area);

    let [workflow_sidebar, resources_sidebar, main_pane] = Layout::horizontal([
        Constraint::Percentage(20),
        Constraint::Percentage(25),
        Constraint::Min(0),
    ])
    .direction(Direction::Horizontal)
    .areas(body);

    StandardAreas {
        work_status,
        workflow_sidebar,
        resources_sidebar,
        main_pane,
        status_line,
    }
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;

    #[test]
    fn the_minimum_supported_size_is_80_by_30() {
        assert!(is_supported_size(80, 30));
        assert!(!is_supported_size(79, 30));
        assert!(!is_supported_size(80, 29));
    }

    #[test]
    fn the_too_small_widget_renders_without_panicking_on_a_tiny_terminal() {
        let backend = TestBackend::new(20, 5);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                frame.render_widget(TerminalTooSmallWidget, frame.area());
            })
            .unwrap();
    }

    #[test]
    fn work_status_is_always_exactly_six_rows_tall() {
        let areas = standard_areas(Rect::new(0, 0, 80, 30));
        assert_eq!(areas.work_status.height, WORK_STATUS_HEIGHT);
        assert_eq!(areas.work_status.y, 0);
    }

    #[test]
    fn the_status_line_is_always_exactly_one_row_at_the_bottom() {
        let areas = standard_areas(Rect::new(0, 0, 80, 30));
        assert_eq!(areas.status_line.height, STATUS_LINE_HEIGHT);
        assert_eq!(areas.status_line.y, 29);
    }

    #[test]
    fn the_three_columns_tile_the_body_with_no_gap_or_overlap() {
        let areas = standard_areas(Rect::new(0, 0, 100, 30));
        assert_eq!(areas.workflow_sidebar.x, 0);
        assert_eq!(
            areas.resources_sidebar.x,
            areas.workflow_sidebar.x + areas.workflow_sidebar.width
        );
        assert_eq!(
            areas.main_pane.x,
            areas.resources_sidebar.x + areas.resources_sidebar.width
        );
        assert_eq!(
            areas.workflow_sidebar.width + areas.resources_sidebar.width + areas.main_pane.width,
            100
        );
        for area in [
            areas.workflow_sidebar,
            areas.resources_sidebar,
            areas.main_pane,
        ] {
            assert_eq!(area.y, areas.work_status.y + areas.work_status.height);
            assert_eq!(area.height, 30 - WORK_STATUS_HEIGHT - STATUS_LINE_HEIGHT);
        }
    }

    #[test]
    fn a_minimum_sized_terminal_still_produces_nonzero_columns() {
        let areas = standard_areas(Rect::new(0, 0, MIN_COLUMNS, MIN_ROWS));
        assert!(areas.workflow_sidebar.width > 0);
        assert!(areas.resources_sidebar.width > 0);
        assert!(areas.main_pane.width > 0);
    }
}
