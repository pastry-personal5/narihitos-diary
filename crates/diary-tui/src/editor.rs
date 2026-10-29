//! The multiline Draft editor. Global chords (Ctrl-G, Ctrl-S, Ctrl-D,
//! Ctrl-P/Ctrl-N) are intercepted by the caller before reaching this
//! module, so `ratatui-textarea`'s own Emacs-like Ctrl bindings (which
//! would otherwise consume them, e.g. its default Ctrl-D delete-next-char)
//! can never fire. This module supplies its own key dispatch: content
//! edits go through `input_without_shortcuts`, and cursor navigation is
//! driven directly through `move_cursor` rather than through any default
//! binding.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::widgets::Widget;
use ratatui_textarea::{CursorMove, Input, Key, TextArea};

/// Bracketed paste is one edit; a payload over this limit is rejected
/// whole rather than partially inserted.
pub const MAX_PASTE_BYTES: usize = 512 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PasteError {
    Oversized,
}

pub struct DraftEditor {
    textarea: TextArea<'static>,
}

impl DraftEditor {
    #[must_use]
    pub fn new(initial_text: &str) -> Self {
        let mut textarea = TextArea::default();
        textarea.insert_str(initial_text);
        Self { textarea }
    }

    #[must_use]
    pub fn text(&self) -> String {
        self.textarea.lines().join("\n")
    }

    #[must_use]
    pub const fn textarea(&self) -> &TextArea<'static> {
        &self.textarea
    }

    /// Feeds one key-press event through the editor's own dispatch.
    /// Returns whether the text changed. A key this editor does not
    /// recognize (including any Ctrl chord, which the caller should
    /// already have handled as a global chord before this is called) is
    /// ignored rather than falling through to a default binding.
    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        if key.kind != KeyEventKind::Press {
            return false;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            return false;
        }
        match key.code {
            KeyCode::Left => {
                self.textarea.move_cursor(CursorMove::Back);
                false
            }
            KeyCode::Right => {
                self.textarea.move_cursor(CursorMove::Forward);
                false
            }
            KeyCode::Up => {
                self.textarea.move_cursor(CursorMove::Up);
                false
            }
            KeyCode::Down => {
                self.textarea.move_cursor(CursorMove::Down);
                false
            }
            KeyCode::Home => {
                self.textarea.move_cursor(CursorMove::Head);
                false
            }
            KeyCode::End => {
                self.textarea.move_cursor(CursorMove::End);
                false
            }
            KeyCode::Char(ch) => self.textarea.input_without_shortcuts(Input {
                key: Key::Char(ch),
                ctrl: false,
                alt: false,
                shift: false,
            }),
            KeyCode::Tab => self.textarea.input_without_shortcuts(Input {
                key: Key::Tab,
                ctrl: false,
                alt: false,
                shift: false,
            }),
            KeyCode::Enter => self.textarea.input_without_shortcuts(Input {
                key: Key::Enter,
                ctrl: false,
                alt: false,
                shift: false,
            }),
            KeyCode::Backspace => self.textarea.input_without_shortcuts(Input {
                key: Key::Backspace,
                ctrl: false,
                alt: false,
                shift: false,
            }),
            KeyCode::Delete => self.textarea.input_without_shortcuts(Input {
                key: Key::Delete,
                ctrl: false,
                alt: false,
                shift: false,
            }),
            _ => false,
        }
    }

    /// Inserts a bracketed-paste payload as one edit, after rejecting it
    /// whole if it exceeds the UTF-8 byte limit. Never partially inserts
    /// an oversized payload.
    pub fn paste(&mut self, data: &str) -> Result<bool, PasteError> {
        if data.len() > MAX_PASTE_BYTES {
            return Err(PasteError::Oversized);
        }
        Ok(self.textarea.insert_str(data))
    }
}

impl Widget for &DraftEditor {
    fn render(self, area: Rect, buf: &mut Buffer) {
        self.textarea.render(area, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn new_seeds_the_buffer_with_initial_text() {
        let editor = DraftEditor::new("hello\nworld");
        assert_eq!(editor.text(), "hello\nworld");
    }

    #[test]
    fn typing_a_character_inserts_it_at_the_cursor() {
        let mut editor = DraftEditor::new("");
        assert!(editor.handle_key(press(KeyCode::Char('h'))));
        assert!(editor.handle_key(press(KeyCode::Char('i'))));
        assert_eq!(editor.text(), "hi");
    }

    #[test]
    fn enter_inserts_a_newline() {
        let mut editor = DraftEditor::new("");
        editor.handle_key(press(KeyCode::Char('a')));
        editor.handle_key(press(KeyCode::Enter));
        editor.handle_key(press(KeyCode::Char('b')));
        assert_eq!(editor.text(), "a\nb");
    }

    #[test]
    fn backspace_deletes_the_previous_character() {
        let mut editor = DraftEditor::new("ab");
        editor.handle_key(press(KeyCode::Backspace));
        assert_eq!(editor.text(), "a");
    }

    #[test]
    fn a_ctrl_chord_is_never_consumed_by_the_editor() {
        let mut editor = DraftEditor::new("text");
        let ctrl_d = KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL);
        let changed = editor.handle_key(ctrl_d);
        assert!(
            !changed,
            "a Ctrl chord must never fall through to a default ratatui-textarea binding"
        );
        assert_eq!(
            editor.text(),
            "text",
            "ratatui-textarea's default Ctrl-D (delete-next-char) must not fire"
        );
    }

    #[test]
    fn arrow_keys_move_the_cursor_without_editing_text() {
        let mut editor = DraftEditor::new("ab");
        editor.handle_key(press(KeyCode::Left));
        assert_eq!(editor.text(), "ab", "cursor movement must never edit text");
    }

    #[test]
    fn a_paste_within_the_limit_inserts_as_one_edit() {
        let mut editor = DraftEditor::new("");
        assert_eq!(editor.paste("pasted text"), Ok(true));
        assert_eq!(editor.text(), "pasted text");
    }

    #[test]
    fn an_oversized_paste_is_rejected_and_inserts_nothing() {
        let mut editor = DraftEditor::new("kept");
        let oversized = "x".repeat(MAX_PASTE_BYTES + 1);
        assert_eq!(editor.paste(&oversized), Err(PasteError::Oversized));
        assert_eq!(
            editor.text(),
            "kept",
            "a rejected paste must never partially insert"
        );
    }
}
