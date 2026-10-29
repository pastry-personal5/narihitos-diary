//! One conservative 16-color palette. Text markers always carry semantics;
//! these styles only reinforce them and collapse to defaults under
//! `NO_COLOR`.

use ratatui::style::{Color, Modifier, Style};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Palette {
    colors_enabled: bool,
}

impl Palette {
    #[must_use]
    pub const fn new(no_color: bool) -> Self {
        Self {
            colors_enabled: !no_color,
        }
    }

    #[must_use]
    pub const fn plain(self) -> Style {
        Style::new()
    }

    #[must_use]
    pub fn border(self, focused: bool) -> Style {
        if !self.colors_enabled {
            return Style::new();
        }
        if focused {
            Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(Color::DarkGray)
        }
    }

    #[must_use]
    pub fn current(self) -> Style {
        if self.colors_enabled {
            Style::new().fg(Color::Yellow)
        } else {
            Style::new()
        }
    }

    #[must_use]
    pub fn safe(self) -> Style {
        if self.colors_enabled {
            Style::new().fg(Color::Green)
        } else {
            Style::new()
        }
    }
}

impl Default for Palette {
    fn default() -> Self {
        Self::new(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_color_collapses_every_semantic_style_to_the_default() {
        let palette = Palette::new(true);
        assert_eq!(palette.border(true), Style::new());
        assert_eq!(palette.current(), Style::new());
        assert_eq!(palette.safe(), Style::new());
    }

    #[test]
    fn enabled_palette_uses_only_basic_terminal_colors() {
        let palette = Palette::new(false);
        assert_eq!(palette.border(true).fg, Some(Color::Cyan));
        assert_eq!(palette.current().fg, Some(Color::Yellow));
        assert_eq!(palette.safe().fg, Some(Color::Green));
    }
}
