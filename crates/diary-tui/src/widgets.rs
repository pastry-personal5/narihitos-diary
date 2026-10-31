//! The rightmost Utility Sidebar: a vertical stack of Widgets that stay
//! reachable in every stage. The read-only Diary Day Widget appears above
//! the Assistant Widget. Presentation only: selection is sent through the
//! existing assistant-selection request, never decided here.

use diary_control_contract::{AssistantBackendDto, WorkflowDto};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, Widget};

use crate::date_display::format_diary_day;
use crate::display::{truncate, width};
use crate::sanitize::sanitize_for_display;
use crate::theme::Palette;

const NO_DAY: &str = "No day chosen";
const NONE_LABEL: &str = "None (manual only)";

/// What an Assistant Widget row selects. `NoAssistant` sends a null backend ID.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AssistantTarget {
    Backend(usize),
    NoAssistant,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssistantWidgetRow {
    pub target: AssistantTarget,
    pub title: String,
    pub detail: String,
    pub selected: bool,
}

/// The Diary Day Widget's input: the open workflow's day and state word.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DiaryDayWidgetModel {
    pub day: Option<String>,
    pub state: Option<String>,
}

impl DiaryDayWidgetModel {
    #[must_use]
    pub fn from_workflow(workflow: Option<&WorkflowDto>) -> Self {
        Self {
            day: workflow.map(|value| value.diary_day.clone()),
            state: workflow.map(|value| value.state.clone()),
        }
    }

    fn lines(&self, available: u16) -> [String; 2] {
        match &self.day {
            Some(day) => [
                format_diary_day(&sanitize_for_display(day), available),
                truncate(
                    &sanitize_for_display(self.state.as_deref().unwrap_or("")),
                    available,
                ),
            ],
            None => [truncate(NO_DAY, available), String::new()],
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AssistantWidgetModel {
    pub rows: Vec<AssistantWidgetRow>,
    /// Why selection is refused right now; `None` means Enter selects.
    pub read_only_reason: Option<String>,
}

impl AssistantWidgetModel {
    /// Rows come from server capabilities, then the always-present
    /// "None (manual only)" row. Manual editing is never a backend.
    #[must_use]
    pub fn new(
        backends: &[AssistantBackendDto],
        workflow: Option<&WorkflowDto>,
        read_only_reason: Option<String>,
    ) -> Self {
        let selected_id = workflow.and_then(|value| value.assistant_backend_id.as_deref());
        let language = workflow.and_then(|value| value.language.as_deref());
        let mut rows: Vec<AssistantWidgetRow> = backends
            .iter()
            .enumerate()
            .map(|(index, backend)| {
                let selected = selected_id == Some(backend.id.as_str());
                let state = if selected && backend.available {
                    "selected"
                } else if selected {
                    "selected; unavailable"
                } else if !backend.available {
                    "unavailable"
                } else if language.is_some_and(|code| {
                    !backend
                        .supported_languages
                        .iter()
                        .any(|value| value == code)
                }) {
                    "unsupported language"
                } else {
                    "available"
                };
                AssistantWidgetRow {
                    target: AssistantTarget::Backend(index),
                    title: backend.display_label.clone(),
                    detail: format!("{} · {state}", backend.composition_path),
                    selected,
                }
            })
            .collect();
        let none_selected = workflow.is_some() && selected_id.is_none();
        rows.push(AssistantWidgetRow {
            target: AssistantTarget::NoAssistant,
            title: NONE_LABEL.to_owned(),
            detail: if none_selected {
                "selected"
            } else {
                "available"
            }
            .to_owned(),
            selected: none_selected,
        });
        Self {
            rows,
            read_only_reason,
        }
    }
}

/// Why the Assistant Widget refuses selection right now, if it does. Checked in
/// order so the most fundamental cause is the one named.
#[must_use]
pub const fn read_only_reason(
    has_workflow: bool,
    can_mutate: bool,
    job_running: bool,
    selection_pending: bool,
) -> Option<&'static str> {
    if !has_workflow {
        Some("no workflow open")
    } else if !can_mutate {
        Some("actions disabled")
    } else if job_running {
        Some("job running")
    } else if selection_pending {
        Some("saving choice")
    } else {
        None
    }
}

/// Resolves Enter on an Assistant Widget row into the backend ID to send
/// (`None` deselects) or the reason nothing is sent.
///
/// # Errors
/// Returns a short, safe message when the widget is read-only, the row is
/// already selected, or the chosen backend is unavailable.
pub fn selection_request(
    model: &AssistantWidgetModel,
    backends: &[AssistantBackendDto],
    row_index: usize,
) -> Result<Option<String>, String> {
    if let Some(reason) = &model.read_only_reason {
        return Err(format!("Assistant choice is read-only: {reason}"));
    }
    let row = model
        .rows
        .get(row_index)
        .ok_or_else(|| "No assistant row is focused".to_owned())?;
    if row.selected {
        return Err("That choice is already selected".to_owned());
    }
    match row.target {
        AssistantTarget::NoAssistant => Ok(None),
        AssistantTarget::Backend(index) => match backends.get(index) {
            Some(backend) if backend.available => Ok(Some(backend.id.clone())),
            Some(_) => Err("That backend is unavailable; nothing was sent".to_owned()),
            None => Err("That backend is no longer listed".to_owned()),
        },
    }
}

/// Both Widgets plus the number of rows that take focus (the Diary Day Widget
/// has none, so focus lands only on Assistant Widget rows).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct UtilitySidebarModel {
    pub diary_day: DiaryDayWidgetModel,
    pub assistant: AssistantWidgetModel,
}

impl UtilitySidebarModel {
    #[must_use]
    pub fn focusable_rows(&self) -> usize {
        self.assistant.rows.len()
    }
}

pub struct UtilitySidebarWidget<'a> {
    model: &'a UtilitySidebarModel,
    focused_row: Option<usize>,
    focused: bool,
    palette: Palette,
}

impl<'a> UtilitySidebarWidget<'a> {
    #[must_use]
    pub const fn new(model: &'a UtilitySidebarModel) -> Self {
        Self {
            model,
            focused_row: None,
            focused: false,
            palette: Palette::new(true),
        }
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

impl Widget for UtilitySidebarWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let title = if self.focused {
            "Utility [focus]"
        } else {
            "Utility"
        };
        let block = Block::bordered()
            .title(title)
            .border_style(self.palette.border(self.focused));
        let inner = block.inner(area);
        block.render(area, buf);
        if inner.width == 0 || inner.height == 0 {
            return;
        }
        let mut y = inner.y;
        let bottom = inner.bottom();
        let put = |buf: &mut Buffer, y: &mut u16, text: &str, style| {
            if *y < bottom {
                buf.set_string(inner.x, *y, truncate(text, inner.width), style);
                *y += 1;
            }
        };

        put(buf, &mut y, "Diary Day", self.palette.plain());
        for line in self.model.diary_day.lines(inner.width) {
            if !line.is_empty() {
                put(buf, &mut y, &line, self.palette.plain());
            }
        }
        y = y.saturating_add(1);
        put(buf, &mut y, "Assistant", self.palette.plain());
        if let Some(reason) = &self.model.assistant.read_only_reason {
            put(buf, &mut y, "(read-only)", self.palette.plain());
            put(buf, &mut y, &format!("  {reason}"), self.palette.plain());
        }

        let rows = &self.model.assistant.rows;
        let capacity = usize::from(bottom.saturating_sub(y) / 2);
        let selected = self
            .focused_row
            .or_else(|| rows.iter().position(|row| row.selected))
            .unwrap_or(0);
        let start = if rows.len() <= capacity || capacity == 0 {
            0
        } else {
            selected
                .saturating_sub(capacity - 1)
                .min(rows.len() - capacity)
        };
        for (index, row) in rows.iter().enumerate().skip(start).take(capacity) {
            let marker = if row.selected { "> " } else { "  " };
            let focus_marker = if self.focused_row == Some(index) {
                "*"
            } else {
                " "
            };
            let style = if self.focused_row == Some(index) || row.selected {
                self.palette.current()
            } else {
                self.palette.plain()
            };
            let prefix = format!("{focus_marker}{marker}");
            let available = inner
                .width
                .saturating_sub(u16::try_from(width(&prefix)).unwrap_or(u16::MAX));
            put(
                buf,
                &mut y,
                &format!(
                    "{prefix}{}",
                    truncate(&sanitize_for_display(&row.title), available)
                ),
                style,
            );
            put(
                buf,
                &mut y,
                &format!(
                    "    {}",
                    truncate(
                        &sanitize_for_display(&row.detail),
                        inner.width.saturating_sub(4)
                    )
                ),
                style,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;

    fn backend(id: &str, label: &str, available: bool) -> AssistantBackendDto {
        AssistantBackendDto {
            id: id.to_owned(),
            display_label: label.to_owned(),
            composition_path: "hosted".to_owned(),
            available,
            reason_code: (!available).then(|| "offline".to_owned()),
            supported_languages: vec!["en-US".to_owned()],
            supported_actions: vec!["draft-entry".to_owned()],
        }
    }

    fn workflow(backend_id: Option<&str>, language: Option<&str>) -> WorkflowDto {
        let mut value: WorkflowDto = serde_json::from_value(serde_json::json!({
            "id": "w", "diary_day": "2026-10-30", "state": "composing",
            "revision": 1, "buffer_version": 1, "language": language,
            "style": "plain", "assistant_backend_id": backend_id,
            "composition_path": backend_id.map(|_| "hosted"),
            "selection_digest": null, "explicit_empty_selection": false,
            "invalidation_code": null, "entry_text": "", "entry_confirmed": false,
            "local_status": null, "notion_status": null,
            "amendment": null, "candidates": []
        }))
        .expect("workflow fixture");
        value.language = language.map(str::to_owned);
        value
    }

    fn render(model: &UtilitySidebarModel, focused_row: Option<usize>) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(24, 16)).unwrap();
        terminal
            .draw(|frame| {
                frame.render_widget(
                    UtilitySidebarWidget::new(model)
                        .focused(focused_row.is_some())
                        .focused_row(focused_row),
                    frame.area(),
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
            .collect()
    }

    #[test]
    fn rows_list_backends_then_the_none_row_with_states() {
        let backends = [backend("a", "Codex", true), backend("b", "Apple", false)];
        let flow = workflow(Some("a"), Some("en-US"));
        let model = AssistantWidgetModel::new(&backends, Some(&flow), None);
        assert_eq!(model.rows.len(), 3);
        assert_eq!(model.rows[0].detail, "hosted · selected");
        assert!(model.rows[0].selected);
        assert_eq!(model.rows[1].detail, "hosted · unavailable");
        assert_eq!(model.rows[2].target, AssistantTarget::NoAssistant);
        assert!(!model.rows[2].selected);
    }

    #[test]
    fn unsupported_language_and_deselected_states_are_words() {
        let backends = [backend("a", "Codex", true)];
        let flow = workflow(None, Some("ko-KR"));
        let model = AssistantWidgetModel::new(&backends, Some(&flow), None);
        assert_eq!(model.rows[0].detail, "hosted · unsupported language");
        assert!(model.rows[1].selected);
        assert_eq!(model.rows[1].title, NONE_LABEL);
    }

    #[test]
    fn with_no_workflow_nothing_is_selected_and_the_none_row_remains() {
        let backends = [backend("a", "Codex", true)];
        let model = AssistantWidgetModel::new(&backends, None, Some("no workflow open".into()));
        assert!(model.rows.iter().all(|row| !row.selected));
        assert_eq!(
            model.rows.last().unwrap().target,
            AssistantTarget::NoAssistant
        );
    }

    #[test]
    fn read_only_reason_names_the_most_fundamental_cause_first() {
        assert_eq!(
            read_only_reason(false, false, true, true),
            Some("no workflow open")
        );
        assert_eq!(
            read_only_reason(true, false, true, true),
            Some("actions disabled")
        );
        assert_eq!(
            read_only_reason(true, true, true, true),
            Some("job running")
        );
        assert_eq!(
            read_only_reason(true, true, false, true),
            Some("saving choice")
        );
        assert_eq!(read_only_reason(true, true, false, false), None);
    }

    #[test]
    fn enter_resolves_to_a_backend_id_none_or_a_refusal() {
        let backends = [backend("a", "Codex", true), backend("b", "Apple", false)];
        let flow = workflow(Some("a"), Some("en-US"));
        let open = AssistantWidgetModel::new(&backends, Some(&flow), None);
        assert!(
            selection_request(&open, &backends, 0).is_err(),
            "already selected"
        );
        assert!(
            selection_request(&open, &backends, 1)
                .unwrap_err()
                .contains("unavailable")
        );
        assert_eq!(selection_request(&open, &backends, 2), Ok(None));
        assert!(selection_request(&open, &backends, 9).is_err());
        let other = workflow(None, Some("en-US"));
        let open = AssistantWidgetModel::new(&backends, Some(&other), None);
        assert_eq!(
            selection_request(&open, &backends, 0),
            Ok(Some("a".to_owned()))
        );
        let locked =
            AssistantWidgetModel::new(&backends, Some(&other), Some("job running".to_owned()));
        assert!(
            selection_request(&locked, &backends, 0)
                .unwrap_err()
                .contains("job running")
        );
    }

    #[test]
    fn diary_day_widget_shows_the_date_weekday_and_state() {
        let flow = workflow(None, None);
        let model = UtilitySidebarModel {
            diary_day: DiaryDayWidgetModel::from_workflow(Some(&flow)),
            assistant: AssistantWidgetModel::default(),
        };
        let screen = render(&model, None).join("\n");
        assert!(screen.contains("Diary Day"));
        assert!(screen.contains("2026-10-30 (Fri)"));
        assert!(screen.contains("composing"));
    }

    #[test]
    fn diary_day_widget_says_no_day_chosen_without_a_workflow() {
        let model = UtilitySidebarModel {
            diary_day: DiaryDayWidgetModel::from_workflow(None),
            assistant: AssistantWidgetModel::default(),
        };
        assert!(render(&model, None).join("\n").contains(NO_DAY));
    }

    #[test]
    fn diary_day_widget_is_never_focusable() {
        let flow = workflow(None, None);
        let model = UtilitySidebarModel {
            diary_day: DiaryDayWidgetModel::from_workflow(Some(&flow)),
            assistant: AssistantWidgetModel::new(&[backend("a", "Codex", true)], Some(&flow), None),
        };
        assert_eq!(model.focusable_rows(), 2);
    }

    #[test]
    fn assistant_widget_renders_rows_markers_and_read_only_reason() {
        let backends = [backend("a", "Codex", true)];
        let flow = workflow(Some("a"), Some("en-US"));
        let model = UtilitySidebarModel {
            diary_day: DiaryDayWidgetModel::from_workflow(Some(&flow)),
            assistant: AssistantWidgetModel::new(
                &backends,
                Some(&flow),
                Some("job running".to_owned()),
            ),
        };
        let lines = render(&model, Some(1));
        let screen = lines.join("\n");
        assert!(screen.contains("Utility [focus]"));
        assert!(screen.contains("(read-only)"));
        assert!(screen.contains("job running"));
        assert!(
            lines
                .iter()
                .any(|line| line.contains(">  Codex") || line.contains("> Codex"))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("* ") && line.contains(NONE_LABEL))
        );
    }
}
