//! Throwaway real-terminal chord probe for terminal shortcut verification.
//!
//! Run in a real Wave Terminal window for primary qualification, or Apple
//! Terminal for the secondary regression check (not over SSH, tmux, or an IDE
//! terminal), with `cargo run -p diary-tui --example chord_probe`. It prints
//! every key event it receives, decoded as the chords used by the terminal client, so the owner can confirm each one arrives as a distinct event before
//! any editor/Command Line code relies on it. There are no function-key
//! aliases to check (see AGENTS.md "Keybindings"): F10 never arrived as a
//! distinct chord, and the owner decided to drop function-key aliases from
//! the design entirely rather than confirm the rest one at a time.
//!
//! Test text only: type ASCII and, separately, Korean 2-Set Hangul syllables.
//! Press Ctrl-C to exit the probe.

use std::io::{self, Write};

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal;

fn main() -> io::Result<()> {
    terminal::enable_raw_mode()?;
    println!(
        "Chord probe: type keys to see how this terminal delivers them.\r\n\
         Checking: Ctrl-G, Ctrl-S, Ctrl-D, Ctrl-P/Ctrl-N.\r\n\
         Press Ctrl-C to quit.\r\n"
    );
    io::stdout().flush()?;

    let result = probe_loop();
    terminal::disable_raw_mode()?;
    result
}

fn probe_loop() -> io::Result<()> {
    loop {
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let label = match (key.code, ctrl) {
            (KeyCode::Char('g'), true) => "Ctrl-G".to_string(),
            (KeyCode::Char('s'), true) => "Ctrl-S".to_string(),
            (KeyCode::Char('d'), true) => "Ctrl-D".to_string(),
            (KeyCode::Char('p'), true) => "Ctrl-P".to_string(),
            (KeyCode::Char('n'), true) => "Ctrl-N".to_string(),
            (KeyCode::F(n), _) => format!("F{n}"),
            (KeyCode::Char(c), true) => format!("Ctrl-{c} (unused chord)"),
            (KeyCode::Char(c), false) => format!("char {c:?} (U+{:04X})", c as u32),
            (other, _) => format!("{other:?}"),
        };
        println!("received: {label}\r");
        io::stdout().flush()?;
        if ctrl && matches!(key.code, KeyCode::Char('c')) {
            println!("quit chord observed, exiting\r");
            io::stdout().flush()?;
            return Ok(());
        }
    }
}
