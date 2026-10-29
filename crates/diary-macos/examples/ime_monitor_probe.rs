//! Bounded production-shape smoke probe for the TIS notification monitor.
//!
//! This process only reads the selected input language. It does not switch,
//! enable, disable, or enumerate input sources.

use std::process::ExitCode;
use std::thread;
use std::time::{Duration, Instant};

use diary_macos::{InputLanguage, InputLanguageMonitor};

fn main() -> ExitCode {
    let mut monitor = match InputLanguageMonitor::start() {
        Ok(monitor) => monitor,
        Err(_) => {
            eprintln!("ime_monitor_probe: notification monitor unavailable");
            return ExitCode::FAILURE;
        }
    };
    let deadline = Instant::now() + Duration::from_millis(250);
    loop {
        match monitor.poll_update() {
            Ok(Some(language)) => {
                println!("ime_monitor_probe: {}", label(language));
                return ExitCode::SUCCESS;
            }
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            Ok(None) => {
                eprintln!("ime_monitor_probe: no initial state");
                return ExitCode::FAILURE;
            }
            Err(_) => {
                eprintln!("ime_monitor_probe: notification delivery unavailable");
                return ExitCode::FAILURE;
            }
        }
    }
}

const fn label(language: InputLanguage) -> &'static str {
    match language {
        InputLanguage::English => "IME: EN",
        InputLanguage::Korean => "IME: KO",
        InputLanguage::Other => "IME: ?",
    }
}
