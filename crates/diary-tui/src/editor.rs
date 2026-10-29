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
use ratatui::widgets::{Block, Widget};
use ratatui_textarea::{CursorMove, Input, Key, TextArea};

use crate::terminal::InputEvent;
use crate::theme::Palette;

/// Bracketed paste is one edit; a payload over this limit is rejected
/// whole rather than partially inserted.
pub const MAX_ENTRY_BYTES: usize = 512 * 1024;
pub const MAX_PASTE_BYTES: usize = MAX_ENTRY_BYTES;

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
            KeyCode::Char(ch) if self.can_insert(ch.len_utf8()) => {
                self.textarea.input_without_shortcuts(Input {
                    key: Key::Char(ch),
                    ctrl: false,
                    alt: false,
                    shift: false,
                })
            }
            KeyCode::Tab if self.can_insert(1) => self.textarea.input_without_shortcuts(Input {
                key: Key::Tab,
                ctrl: false,
                alt: false,
                shift: false,
            }),
            KeyCode::Enter if self.can_insert(1) => self.textarea.input_without_shortcuts(Input {
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

    fn can_insert(&self, additional_bytes: usize) -> bool {
        let lines = self.textarea.lines();
        let current_bytes = lines
            .iter()
            .map(String::len)
            .sum::<usize>()
            .saturating_add(lines.len().saturating_sub(1));
        current_bytes.saturating_add(additional_bytes) <= MAX_ENTRY_BYTES
    }

    /// Applies the terminal-neutral event used by the live loop. Escape
    /// remains a focus-leave action for the caller and is therefore not
    /// consumed here.
    pub fn handle_input(&mut self, event: &InputEvent) -> bool {
        let code = match event {
            InputEvent::Char(ch) => KeyCode::Char(*ch),
            InputEvent::Enter => KeyCode::Enter,
            InputEvent::Backspace => KeyCode::Backspace,
            InputEvent::Tab => KeyCode::Tab,
            InputEvent::Up => KeyCode::Up,
            InputEvent::Down => KeyCode::Down,
            InputEvent::Left => KeyCode::Left,
            InputEvent::Right => KeyCode::Right,
            InputEvent::Home => KeyCode::Home,
            InputEvent::End => KeyCode::End,
            InputEvent::PageUp | InputEvent::PageDown => return false,
            InputEvent::Delete => KeyCode::Delete,
            InputEvent::Paste(data) => return self.paste(data).unwrap_or(false),
            InputEvent::Esc | InputEvent::BackTab => return false,
        };
        self.handle_key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    /// Inserts a bracketed-paste payload as one edit, after rejecting it
    /// whole if it exceeds the UTF-8 byte limit. Never partially inserts
    /// an oversized payload.
    pub fn paste(&mut self, data: &str) -> Result<bool, PasteError> {
        if data.len() > MAX_PASTE_BYTES || !self.can_insert(data.len()) {
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

pub struct DraftEditorPane<'a> {
    editor: &'a DraftEditor,
    focused: bool,
    palette: Palette,
}

impl<'a> DraftEditorPane<'a> {
    #[must_use]
    pub const fn new(editor: &'a DraftEditor, focused: bool) -> Self {
        Self {
            editor,
            focused,
            palette: Palette::new(true),
        }
    }

    #[must_use]
    pub const fn palette(mut self, palette: Palette) -> Self {
        self.palette = palette;
        self
    }
}

impl Widget for DraftEditorPane<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let title = if self.focused {
            "Draft editor [focus]"
        } else {
            "Draft editor"
        };
        let block = Block::bordered()
            .title(title)
            .border_style(self.palette.border(self.focused));
        let inner = block.inner(area);
        block.render(area, buf);
        self.editor.render(inner, buf);
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
    fn cursor_edits_treat_wide_korean_and_emoji_scalars_as_whole_units() {
        let mut editor = DraftEditor::new("한🙂b");
        editor.handle_key(press(KeyCode::Left));
        editor.handle_key(press(KeyCode::Backspace));
        assert_eq!(editor.text(), "한b");
        editor.handle_key(press(KeyCode::Home));
        editor.handle_key(press(KeyCode::Delete));
        assert_eq!(editor.text(), "b");
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

    #[test]
    fn typing_and_paste_cannot_grow_the_complete_entry_past_the_byte_limit() {
        let initial = "x".repeat(MAX_ENTRY_BYTES - 1);
        let mut editor = DraftEditor::new(&initial);
        assert!(!editor.handle_key(press(KeyCode::Char('한'))));
        assert!(editor.handle_key(press(KeyCode::Char('y'))));
        assert!(!editor.handle_key(press(KeyCode::Char('z'))));
        assert_eq!(editor.text().len(), MAX_ENTRY_BYTES);

        let mut paste_editor = DraftEditor::new(&initial);
        assert_eq!(paste_editor.paste("yz"), Err(PasteError::Oversized));
        assert_eq!(paste_editor.text(), initial);
    }

    #[test]
    fn terminal_neutral_input_edits_and_escape_remains_for_focus_leave() {
        let mut editor = DraftEditor::new("");
        assert!(editor.handle_input(&InputEvent::Char('a')));
        assert!(!editor.handle_input(&InputEvent::Esc));
        assert_eq!(editor.text(), "a");
    }
}
