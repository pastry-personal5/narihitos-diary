//! Adaptive Standard Layout geometry. Work Status is always full-width and
//! exactly four content rows; the Command Line and Status Line each own a
//! separate row below the body.

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::widgets::{Clear, Paragraph, Widget};

use crate::focus::{FocusZone, SidebarKind};
use crate::sidebar::MINIMUM_WORKFLOW_SIDEBAR_WIDTH;

pub const MIN_COLUMNS: u16 = 80;
pub const MIN_ROWS: u16 = 30;
pub const WIDE_COLUMNS: u16 = 120;
pub const MEDIUM_COLUMNS: u16 = 100;
/// Fixed Utility Sidebar width, border included.
pub const UTILITY_SIDEBAR_WIDTH: u16 = 24;
/// Smallest Main Pane width the shipped Wide band already accepts (120
/// columns leave it 50), so the four-column band never goes below it.
pub const MIN_MAIN_PANE_WIDTH: u16 = 50;
/// Narrowest terminal that keeps all four columns inline without the Main
/// Pane dropping below [`MIN_MAIN_PANE_WIDTH`].
pub const FULL_COLUMNS: u16 = 161;
/// Extra Resources Sidebar columns so a `YYYY-MM-DD HH:MM:SS` source label fits.
pub const RESOURCES_EXTRA_WIDTH: u16 = 16;

pub struct TerminalTooSmallWidget;

impl Widget for TerminalTooSmallWidget {
    fn render(self, area: Rect, buf: &mut Buffer) {
        Paragraph::new(format!(
            "Terminal too small. Needs at least {MIN_COLUMNS}x{MIN_ROWS}."
        ))
        .render(area, buf);
    }
}

/// Clears the body beneath a focused sidebar overlay so text from the Main
/// Pane cannot bleed through its blank rows.
pub struct OverlayClearWidget;

impl Widget for OverlayClearWidget {
    fn render(self, area: Rect, buf: &mut Buffer) {
        Clear.render(area, buf);
    }
}

const WORK_STATUS_HEIGHT: u16 = 6;
const COMMAND_LINE_HEIGHT: u16 = 1;
const STATUS_LINE_HEIGHT: u16 = 1;

#[must_use]
pub const fn is_supported_size(width: u16, height: u16) -> bool {
    width >= MIN_COLUMNS && height >= MIN_ROWS
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WidthBand {
    /// Workflow, Resources, Main Pane, and Utility Sidebar all inline.
    Full,
    /// Workflow, Resources, and Main Pane inline; Utility overlays.
    Wide,
    Medium,
    Narrow,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SidebarOverlay {
    pub kind: SidebarKind,
    pub area: Rect,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AdaptiveAreas {
    pub band: WidthBand,
    pub work_status: Rect,
    pub workflow_sidebar: Option<Rect>,
    pub resources_sidebar: Option<Rect>,
    pub utility_sidebar: Option<Rect>,
    pub main_pane: Rect,
    pub overlay: Option<SidebarOverlay>,
    pub command_line: Rect,
    pub status_line: Rect,
}

/// Produces the four width bands. A collapsed sidebar becomes an
/// overlay only while its focus zone is active, so Tab/Shift-Tab never makes
/// its controls unreachable.
#[must_use]
pub fn adaptive_areas(area: Rect, focus: FocusZone) -> AdaptiveAreas {
    let [work_status, body, command_line, status_line] = Layout::vertical([
        Constraint::Length(WORK_STATUS_HEIGHT),
        Constraint::Min(0),
        Constraint::Length(COMMAND_LINE_HEIGHT),
        Constraint::Length(STATUS_LINE_HEIGHT),
    ])
    .areas(area);

    let utility_overlay = |container: Rect| {
        (focus == FocusZone::Utility).then_some(SidebarOverlay {
            kind: SidebarKind::Utility,
            area: right_overlay(container, UTILITY_SIDEBAR_WIDTH),
        })
    };
    let (band, workflow_sidebar, resources_sidebar, utility_sidebar, main_pane, overlay) =
        if area.width >= FULL_COLUMNS {
            let resources_width = body.width / 4 + RESOURCES_EXTRA_WIDTH;
            let [workflow, resources, main, utility] = Layout::horizontal([
                Constraint::Length(MINIMUM_WORKFLOW_SIDEBAR_WIDTH),
                Constraint::Length(resources_width),
                Constraint::Min(0),
                Constraint::Length(UTILITY_SIDEBAR_WIDTH),
            ])
            .areas(body);
            (
                WidthBand::Full,
                Some(workflow),
                Some(resources),
                Some(utility),
                main,
                None,
            )
        } else if area.width >= WIDE_COLUMNS {
            let resources_width = body.width / 4 + RESOURCES_EXTRA_WIDTH;
            let [workflow, resources, main] = Layout::horizontal([
                Constraint::Length(MINIMUM_WORKFLOW_SIDEBAR_WIDTH),
                Constraint::Length(resources_width),
                Constraint::Min(0),
            ])
            .areas(body);
            (
                WidthBand::Wide,
                Some(workflow),
                Some(resources),
                None,
                main,
                utility_overlay(main),
            )
        } else if area.width >= MEDIUM_COLUMNS {
            let [workflow, main] = Layout::horizontal([
                Constraint::Length(MINIMUM_WORKFLOW_SIDEBAR_WIDTH),
                Constraint::Min(0),
            ])
            .areas(body);
            let overlay = if focus == FocusZone::Resources {
                Some(SidebarOverlay {
                    kind: SidebarKind::Resources,
                    area: right_overlay(main, 44 + RESOURCES_EXTRA_WIDTH),
                })
            } else {
                utility_overlay(main)
            };
            (WidthBand::Medium, Some(workflow), None, None, main, overlay)
        } else {
            let overlay = match focus {
                FocusZone::Workflow => Some(SidebarOverlay {
                    kind: SidebarKind::Workflow,
                    area: left_overlay(body, MINIMUM_WORKFLOW_SIDEBAR_WIDTH),
                }),
                FocusZone::Resources => Some(SidebarOverlay {
                    kind: SidebarKind::Resources,
                    area: right_overlay(body, 44 + RESOURCES_EXTRA_WIDTH),
                }),
                FocusZone::Utility => utility_overlay(body),
                FocusZone::Main => None,
            };
            (WidthBand::Narrow, None, None, None, body, overlay)
        };

    AdaptiveAreas {
        band,
        work_status,
        workflow_sidebar,
        resources_sidebar,
        utility_sidebar,
        main_pane,
        overlay,
        command_line,
        status_line,
    }
}

fn left_overlay(body: Rect, preferred_width: u16) -> Rect {
    Rect::new(body.x, body.y, preferred_width.min(body.width), body.height)
}

fn right_overlay(body: Rect, preferred_width: u16) -> Rect {
    let width = preferred_width.min(body.width);
    Rect::new(
        body.right().saturating_sub(width),
        body.y,
        width,
        body.height,
    )
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::widgets::{Block, Paragraph};

    use super::*;

    #[test]
    fn the_minimum_supported_size_is_80_by_30() {
        assert!(is_supported_size(80, 30));
        assert!(!is_supported_size(79, 30));
        assert!(!is_supported_size(80, 29));
    }

    #[test]
    fn the_too_small_widget_renders_on_a_tiny_terminal() {
        let backend = TestBackend::new(20, 5);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| frame.render_widget(TerminalTooSmallWidget, frame.area()))
            .unwrap();
    }

    #[test]
    fn work_status_command_and_status_keep_their_fixed_rows() {
        let areas = adaptive_areas(Rect::new(0, 0, 80, 30), FocusZone::Main);
        assert_eq!(areas.work_status.height, 6);
        assert_eq!(areas.command_line.height, 1);
        assert_eq!(areas.command_line.y, 28);
        assert_eq!(areas.status_line.height, 1);
        assert_eq!(areas.status_line.y, 29);
    }

    #[test]
    fn full_layout_keeps_all_four_columns_inline() {
        let areas = adaptive_areas(Rect::new(0, 0, FULL_COLUMNS, 30), FocusZone::Main);
        assert_eq!(areas.band, WidthBand::Full);
        assert_eq!(
            areas.workflow_sidebar.map(|area| area.width),
            Some(MINIMUM_WORKFLOW_SIDEBAR_WIDTH)
        );
        assert!(areas.resources_sidebar.is_some());
        assert_eq!(
            areas.utility_sidebar.map(|area| area.width),
            Some(UTILITY_SIDEBAR_WIDTH)
        );
        assert!(areas.overlay.is_none());
        let utility = areas.utility_sidebar.expect("Utility Sidebar");
        assert_eq!(
            utility.right(),
            areas.main_pane.right() + UTILITY_SIDEBAR_WIDTH
        );
        assert!(areas.main_pane.right() <= utility.x);
    }

    #[test]
    fn the_full_threshold_is_the_narrowest_width_that_keeps_main_usable() {
        let main_width = |width: u16| {
            adaptive_areas(Rect::new(0, 0, width, 30), FocusZone::Main)
                .main_pane
                .width
        };
        assert!(main_width(FULL_COLUMNS) >= MIN_MAIN_PANE_WIDTH);
        let at = adaptive_areas(Rect::new(0, 0, FULL_COLUMNS, 30), FocusZone::Main);
        assert_eq!(at.band, WidthBand::Full);
        let below = adaptive_areas(Rect::new(0, 0, FULL_COLUMNS - 1, 30), FocusZone::Main);
        assert_eq!(below.band, WidthBand::Wide);
        // Without the Utility column the Main Pane would also be at the floor
        // here, so the threshold is exactly where a fourth column still fits.
        let no_utility = main_width(FULL_COLUMNS - 1);
        assert!(no_utility - UTILITY_SIDEBAR_WIDTH < MIN_MAIN_PANE_WIDTH);
    }

    #[test]
    fn wide_layout_keeps_three_columns_and_overlays_utility_on_focus() {
        let hidden = adaptive_areas(Rect::new(0, 0, 120, 30), FocusZone::Main);
        assert_eq!(hidden.band, WidthBand::Wide);
        assert!(hidden.resources_sidebar.is_some());
        assert!(hidden.utility_sidebar.is_none());
        assert!(hidden.overlay.is_none());
        let focused = adaptive_areas(Rect::new(0, 0, 120, 30), FocusZone::Utility);
        let overlay = focused.overlay.expect("Utility overlay");
        assert_eq!(overlay.kind, SidebarKind::Utility);
        assert_eq!(overlay.area.width, UTILITY_SIDEBAR_WIDTH);
        assert_eq!(overlay.area.right(), focused.main_pane.right());
    }

    #[test]
    fn medium_layout_overlays_resources_only_while_it_has_focus() {
        let hidden = adaptive_areas(Rect::new(0, 0, 110, 30), FocusZone::Main);
        assert_eq!(hidden.band, WidthBand::Medium);
        assert_eq!(
            hidden.workflow_sidebar.map(|area| area.width),
            Some(MINIMUM_WORKFLOW_SIDEBAR_WIDTH)
        );
        assert!(hidden.resources_sidebar.is_none());
        assert!(hidden.overlay.is_none());

        let focused = adaptive_areas(Rect::new(0, 0, 110, 30), FocusZone::Resources);
        assert_eq!(
            focused.overlay.map(|overlay| overlay.kind),
            Some(SidebarKind::Resources)
        );
        let utility = adaptive_areas(Rect::new(0, 0, 119, 30), FocusZone::Utility);
        assert_eq!(utility.band, WidthBand::Medium);
        assert_eq!(
            utility.overlay.map(|overlay| overlay.kind),
            Some(SidebarKind::Utility)
        );
    }

    #[test]
    fn narrow_layout_overlays_either_focused_sidebar() {
        for (focus, expected) in [
            (FocusZone::Workflow, Some(SidebarKind::Workflow)),
            (FocusZone::Resources, Some(SidebarKind::Resources)),
            (FocusZone::Utility, Some(SidebarKind::Utility)),
            (FocusZone::Main, None),
        ] {
            let areas = adaptive_areas(Rect::new(0, 0, 80, 30), focus);
            assert_eq!(areas.band, WidthBand::Narrow);
            assert!(areas.workflow_sidebar.is_none());
            assert!(areas.resources_sidebar.is_none());
            assert!(areas.utility_sidebar.is_none());
            assert_eq!(areas.overlay.map(|overlay| overlay.kind), expected);
            if focus == FocusZone::Workflow {
                assert_eq!(
                    areas.overlay.map(|overlay| overlay.area.width),
                    Some(MINIMUM_WORKFLOW_SIDEBAR_WIDTH)
                );
            }
        }
    }

    #[test]
    fn overlay_clear_removes_main_pane_text_beneath_blank_sidebar_rows() {
        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                let areas = adaptive_areas(frame.area(), FocusZone::Resources);
                frame.render_widget(
                    Paragraph::new("main text that must not bleed through")
                        .block(Block::bordered()),
                    areas.main_pane,
                );
                let overlay = areas.overlay.expect("Resources overlay");
                frame.render_widget(OverlayClearWidget, overlay.area);
                frame.render_widget(Block::bordered().title("Resources [focus]"), overlay.area);
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let overlay = adaptive_areas(buffer.area, FocusZone::Resources)
            .overlay
            .expect("Resources overlay")
            .area;
        let interior: String = ((overlay.x + 1)..overlay.right().saturating_sub(1))
            .map(|x| buffer[(x, overlay.y + 2)].symbol())
            .collect();
        assert!(!interior.contains("main text"));
    }
}
