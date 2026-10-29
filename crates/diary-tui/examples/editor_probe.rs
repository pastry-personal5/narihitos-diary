//! Interactive real-terminal probe for the Draft editor (`crate::editor`).
//!
//! Run with `cargo run -p diary-tui --example editor_probe`. Type freely,
//! including Korean 2-Set IME input — Enter inserts a newline (it does
//! not submit anything), arrow keys move the cursor, and Backspace
//! deletes. Press Ctrl-D and confirm nothing is deleted (`ratatui-textarea`'s
//! own default Ctrl-D binding is delete-next-char), which is exactly the
//! global-chord leak this probe exists to catch in a real terminal, not
//! just in unit tests. Press Ctrl-C to quit and print the final buffer.
//!
//! Test text only.

use std::io;

use crossterm::event::{self, Event, KeyEventKind, KeyModifiers};
use diary_tui::editor::DraftEditor;
use diary_tui::terminal::TerminalGuard;

fn main() -> io::Result<()> {
    let mut guard = TerminalGuard::acquire().expect("acquire terminal");
    let mut editor = DraftEditor::new("");

    loop {
        let drawn = guard.terminal_mut().draw(|frame| {
            frame.render_widget(&editor, frame.area());
        });
        if drawn.is_err() {
            break;
        }
        if !event::poll(std::time::Duration::from_millis(200))? {
            continue;
        }
        match event::read()? {
            Event::Key(key) => {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                if key.modifiers.contains(KeyModifiers::CONTROL)
                    && matches!(key.code, crossterm::event::KeyCode::Char('c'))
                {
                    break;
                }
                editor.handle_key(key);
            }
            Event::Paste(data) => match editor.paste(&data) {
                Ok(_) => {}
                Err(diary_tui::editor::PasteError::Oversized) => {
                    // Rejected whole, per the 512 KiB bracketed-paste
                    // limit; nothing to undo since nothing was inserted.
                }
            },
            _ => {}
        }
    }
    let text = editor.text();
    drop(guard);
    println!("final buffer:\n{text:?}");
    Ok(())
}
