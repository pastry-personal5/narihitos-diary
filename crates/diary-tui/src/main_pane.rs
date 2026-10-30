//! The right Main Pane: the active resource's content for the current
//! stage. The Start stage shows a fixed instructional message; later
//! stages render prepared source bodies, the Draft editor, and the entry
//! review in separate work.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, Paragraph, Widget, Wrap};

use crate::sanitize::sanitize_for_display;
use crate::theme::Palette;

pub struct MainPaneWidget<'a> {
    title: &'a str,
    body: &'a str,
    focused: bool,
    scroll: u16,
    palette: Palette,
}

impl<'a> MainPaneWidget<'a> {
    #[must_use]
    pub const fn new(title: &'a str, body: &'a str) -> Self {
        Self {
            title,
            body,
            focused: false,
            scroll: 0,
            palette: Palette::new(true),
        }
    }

    #[must_use]
    pub const fn focused(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }

    #[must_use]
    pub const fn scroll(mut self, scroll: u16) -> Self {
        self.scroll = scroll;
        self
    }

    #[must_use]
    pub const fn palette(mut self, palette: Palette) -> Self {
        self.palette = palette;
        self
    }
}

impl Widget for MainPaneWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // `body` may be untrusted source/entry text; sanitize only the
        // rendered copy, never the caller's own stored value.
        let title = if self.focused {
            format!("{} [focus]", self.title)
        } else {
            self.title.to_owned()
        };
        Paragraph::new(sanitize_for_display(self.body))
            .block(
                Block::bordered()
                    .title(title)
                    .border_style(self.palette.border(self.focused)),
            )
            .wrap(Wrap { trim: false })
            .scroll((self.scroll, 0))
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
