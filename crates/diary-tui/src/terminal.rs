//! Terminal ownership: enter/restore raw mode and the alternate screen, and
//! guarantee restoration on normal return, a handled error, or panic unwind.
//!
//! This is the one place that touches the real terminal's global state. It
//! has no diary domain logic.

use std::io;
use std::panic;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crossterm::event::{
    DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEventKind, KeyModifiers,
};
use crossterm::execute;
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

static RESTORED: AtomicBool = AtomicBool::new(true);

/// Owns the terminal for the lifetime of the TUI session. Dropping it (via
/// normal return or panic unwind) restores the terminal; a panic hook also
/// restores it before the default panic message prints, so a crash never
/// leaves the terminal in raw/alternate-screen state.
pub struct TerminalGuard {
    terminal: Terminal<CrosstermBackend<io::Stdout>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalError {
    RawModeFailed,
    AlternateScreenFailed,
    SizeUnavailable,
}

impl TerminalGuard {
    pub fn acquire() -> Result<Self, TerminalError> {
        crossterm::terminal::enable_raw_mode().map_err(|_| TerminalError::RawModeFailed)?;
        if let Err(error) = execute!(io::stdout(), EnterAlternateScreen, EnableBracketedPaste) {
            let _ = crossterm::terminal::disable_raw_mode();
            return Err(classify_alt_screen_error(error));
        }
        RESTORED.store(false, Ordering::SeqCst);
        install_panic_hook_once();

        let backend = CrosstermBackend::new(io::stdout());
        let terminal = match Terminal::new(backend) {
            Ok(terminal) => terminal,
            Err(_) => {
                restore();
                return Err(TerminalError::SizeUnavailable);
            }
        };
        Ok(Self { terminal })
    }

    pub fn terminal_mut(&mut self) -> &mut Terminal<CrosstermBackend<io::Stdout>> {
        &mut self.terminal
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        restore();
    }
}

/// Blocks for the next terminal input event and reports whether it was the
/// quit chord (Ctrl-D; see AGENTS.md "Keybindings" — it has no function-key
/// alias, since F10 does not arrive in Apple Terminal). A read error is
/// treated as a quit request rather than looping forever on a broken
/// terminal.
///
/// This is a minimal bootstrap for the default TUI entry point; the full
/// key-dispatch engine (global chords intercepted before the editor,
/// Command Line, focus navigation) is implemented separately and will
/// replace single-purpose calls like this one.
#[must_use]
pub fn next_event_is_quit() -> bool {
    match crossterm::event::read() {
        Ok(Event::Key(key)) => {
            key.kind == KeyEventKind::Press
                && key.modifiers.contains(KeyModifiers::CONTROL)
                && matches!(key.code, KeyCode::Char('d'))
        }
        Ok(_) => false,
        Err(_) => true,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PollOutcome {
    Quit,
    Other,
    TimedOut,
}

/// Waits up to `timeout` for a terminal input event, without blocking
/// forever. Used by the attach/retry loop so it can redraw and back off
/// on its own schedule instead of being stuck inside a blocking read —
/// the same reason no autosave, IME poll, or SSE reload can be wired into
/// a loop built on [`next_event_is_quit`]'s blocking read.
#[must_use]
pub fn poll_event(timeout: Duration) -> PollOutcome {
    match crossterm::event::poll(timeout) {
        Ok(true) => match crossterm::event::read() {
            Ok(Event::Key(key))
                if key.kind == KeyEventKind::Press
                    && key.modifiers.contains(KeyModifiers::CONTROL)
                    && matches!(key.code, KeyCode::Char('d')) =>
            {
                PollOutcome::Quit
            }
            Ok(_) => PollOutcome::Other,
            Err(_) => PollOutcome::Quit,
        },
        Ok(false) => PollOutcome::TimedOut,
        Err(_) => PollOutcome::Quit,
    }
}

/// A non-Ctrl key event the Command Line or editor cares about.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InputEvent {
    Char(char),
    Enter,
    Backspace,
    Esc,
    Tab,
    BackTab,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Delete,
    /// A bracketed-paste payload, delivered as one event. The editor's
    /// own `paste` enforces the 512 KiB limit; this does not pre-filter
    /// so the caller can always see the real payload size.
    Paste(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InputPoll {
    /// The quit chord (Ctrl-D) or a read error.
    Quit,
    /// A Ctrl chord other than Ctrl-D, e.g. Ctrl-G for Command Line.
    Ctrl(char),
    Event(InputEvent),
    /// A key this function does not classify (e.g. arrow keys, which the
    /// editor handles through its own dispatch, not through this type).
    Ignored,
    TimedOut,
}

/// Waits up to `timeout` for one terminal input event and classifies it
/// for the Command Line / connected-session loop, which needs more detail
/// than [`poll_event`]'s simple quit/other/timeout. Ctrl-D always reports
/// `Quit` regardless of mode, since it is the one global, unconditional
/// chord.
#[must_use]
pub fn poll_input_event(timeout: Duration) -> InputPoll {
    match crossterm::event::poll(timeout) {
        Ok(true) => match crossterm::event::read() {
            Ok(Event::Key(key)) if key.kind == KeyEventKind::Press => {
                let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
                match key.code {
                    KeyCode::Char('d') if ctrl => InputPoll::Quit,
                    KeyCode::Char(c) if ctrl => InputPoll::Ctrl(c),
                    KeyCode::Char(c) => InputPoll::Event(InputEvent::Char(c)),
                    KeyCode::Enter => InputPoll::Event(InputEvent::Enter),
                    KeyCode::Backspace => InputPoll::Event(InputEvent::Backspace),
                    KeyCode::Esc => InputPoll::Event(InputEvent::Esc),
                    KeyCode::Tab => InputPoll::Event(InputEvent::Tab),
                    KeyCode::BackTab => InputPoll::Event(InputEvent::BackTab),
                    KeyCode::Up => InputPoll::Event(InputEvent::Up),
                    KeyCode::Down => InputPoll::Event(InputEvent::Down),
                    KeyCode::Left => InputPoll::Event(InputEvent::Left),
                    KeyCode::Right => InputPoll::Event(InputEvent::Right),
                    KeyCode::Home => InputPoll::Event(InputEvent::Home),
                    KeyCode::End => InputPoll::Event(InputEvent::End),
                    KeyCode::PageUp => InputPoll::Event(InputEvent::PageUp),
                    KeyCode::PageDown => InputPoll::Event(InputEvent::PageDown),
                    KeyCode::Delete => InputPoll::Event(InputEvent::Delete),
                    _ => InputPoll::Ignored,
                }
            }
            Ok(Event::Paste(data)) => InputPoll::Event(InputEvent::Paste(data)),
            Ok(_) => InputPoll::Ignored,
            Err(_) => InputPoll::Quit,
        },
        Ok(false) => InputPoll::TimedOut,
        Err(_) => InputPoll::Quit,
    }
}

fn classify_alt_screen_error(_error: io::Error) -> TerminalError {
    TerminalError::AlternateScreenFailed
}

/// Restores raw mode and the primary screen. Safe to call more than once
/// (from a panic hook and then again from `Drop`); only the first call after
/// acquisition does anything.
fn restore() {
    if RESTORED.swap(true, Ordering::SeqCst) {
        return;
    }
    let _ = execute!(io::stdout(), DisableBracketedPaste, LeaveAlternateScreen);
    let _ = crossterm::terminal::disable_raw_mode();
}

fn install_panic_hook_once() {
    use std::sync::Once;
    static HOOK_INSTALLED: Once = Once::new();
    HOOK_INSTALLED.call_once(|| {
        let default_hook = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            restore();
            default_hook(info);
        }));
    });
}
