//! Interactive real-terminal probe for the Command Line's raw-jamo-to-ASCII
//! mapping (`crate::command_line`). Run with Apple Terminal's 2-Set Korean
//! IME active and type one of the grammar tokens (`help`, `retry`,
//! `day 2026-10-29`, `stage sources`, `save`, `quit`) using the Korean
//! keyboard layout, then press Enter to see exactly what the real terminal
//! delivered, how it normalized, and what command it parsed to.
//!
//! Run with `cargo run -p diary-tui --example command_line_probe`.
//! Press Ctrl-D to exit. Test text only — type grammar tokens, not private
//! content.

use std::io::{self, Write};

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal;
use diary_tui::command_line::{normalize, parse};

fn main() -> io::Result<()> {
    terminal::enable_raw_mode()?;
    println!(
        "Command Line probe: switch to the 2-Set Korean IME and type a grammar\r\n\
         token (help, retry, day 2026-10-29, stage sources, save, quit), then\r\n\
         press Enter. Press Ctrl-D to quit.\r\n"
    );
    io::stdout().flush()?;

    let result = probe_loop();
    terminal::disable_raw_mode()?;
    result
}

fn probe_loop() -> io::Result<()> {
    let mut line = String::new();
    loop {
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('d')) {
            println!("quit chord observed, exiting\r");
            io::stdout().flush()?;
            return Ok(());
        }
        match key.code {
            KeyCode::Enter => {
                report(&line);
                line.clear();
            }
            KeyCode::Backspace => {
                line.pop();
            }
            KeyCode::Char(ch) => {
                print!("received: {ch:?} (U+{:04X})\r\n", ch as u32);
                io::stdout().flush()?;
                line.push(ch);
            }
            _ => {}
        }
    }
}

fn report(raw: &str) {
    print!("raw line: {raw:?}\r\n");
    match normalize(raw) {
        Ok(normalized) => {
            print!("normalized: {normalized:?}\r\n");
            match parse(&normalized) {
                Ok(command) => print!("parsed: {command:?}\r\n"),
                Err(error) => print!("parse error: {error:?}\r\n"),
            }
        }
        Err(error) => print!("normalize error: {error:?}\r\n"),
    }
    print!("\r\n");
    let _ = io::stdout().flush();
}
