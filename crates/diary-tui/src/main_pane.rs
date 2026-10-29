//! The right Main Pane: the active resource's content for the current
//! stage. P1-M7's Start stage shows a fixed instructional message; later
//! stages render prepared source bodies, the Draft editor, and the entry
//! review in separate work.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, Paragraph, Widget};

use crate::sanitize::sanitize_for_display;

pub struct MainPaneWidget<'a> {
    title: &'a str,
    body: &'a str,
}

impl<'a> MainPaneWidget<'a> {
    #[must_use]
    pub const fn new(title: &'a str, body: &'a str) -> Self {
        Self { title, body }
    }
}

impl Widget for MainPaneWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // `body` may be untrusted source/entry text; sanitize only the
        // rendered copy, never the caller's own stored value.
        Paragraph::new(sanitize_for_display(self.body))
            .block(Block::bordered().title(self.title))
            .render(area, buf);
    }
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;

    #[test]
    fn it_renders_the_title_and_body_without_panicking() {
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                frame.render_widget(
                    MainPaneWidget::new("Main", "Choose a workflow to continue."),
                    frame.area(),
                );
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let row: String = (1..buffer.area.width - 1)
            .map(|x| buffer[(x, 1)].symbol())
            .collect();
        assert!(row.contains("Choose a workflow"));
    }
}
