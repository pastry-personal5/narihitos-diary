//! Real-terminal restoration probe for P1-M7's `TerminalGuard`.
//!
//! Run each mode in Wave Terminal for primary qualification and Apple Terminal
//! for the secondary regression check, and
//! confirm the shell prompt looks sane afterward (no leftover raw mode,
//! alternate screen, or bracketed-paste state):
//!
//!   cargo run -p diary-tui --example restoration_probe -- normal
//!   cargo run -p diary-tui --example restoration_probe -- error
//!   cargo run -p diary-tui --example restoration_probe -- panic
//!
//! Each mode briefly enters the alternate screen (you will see a cleared
//! screen with a short message), then exits the requested way. After it
//! returns, type a few characters and press Enter — if the terminal
//! echoes normally and Enter produces a new line, restoration worked.

use std::time::Duration;

use diary_tui::terminal::TerminalGuard;

fn main() {
    let mode = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "normal".to_owned());
    match mode.as_str() {
        "normal" => run_normal(),
        "error" => run_error(),
        "panic" => run_panic(),
        other => {
            eprintln!("unknown mode {other:?}; use normal, error, or panic");
            std::process::exit(2);
        }
    }
}

fn run_normal() {
    let mut guard = TerminalGuard::acquire().expect("acquire terminal");
    let _ = guard.terminal_mut().draw(|frame| {
        frame.render_widget("restoration_probe: normal exit in 2s\r", frame.area());
    });
    std::thread::sleep(Duration::from_secs(2));
    drop(guard);
    println!("restoration_probe: returned normally");
}

fn run_error() {
    let mut guard = TerminalGuard::acquire().expect("acquire terminal");
    let _ = guard.terminal_mut().draw(|frame| {
        frame.render_widget(
            "restoration_probe: handled error path in 2s\r",
            frame.area(),
        );
    });
    std::thread::sleep(Duration::from_secs(2));
    drop(guard); // explicit early drop, as a real error path would do
    eprintln!("restoration_probe: returning after a handled error");
    std::process::exit(1);
}

fn run_panic() {
    let mut guard = TerminalGuard::acquire().expect("acquire terminal");
    let _ = guard.terminal_mut().draw(|frame| {
        frame.render_widget("restoration_probe: panicking in 2s\r", frame.area());
    });
    std::thread::sleep(Duration::from_secs(2));
    panic!("restoration_probe: deliberate panic to test unwind restoration");
}
