#![forbid(unsafe_code)]

use std::env;
use std::ffi::OsString;
use std::process::ExitCode;

const HELP: &str = "\
nd — Narihito's Diary

Usage:
  nd           Open the TUI (not implemented yet)
  nd serve     Start the Control API Server (not implemented yet)
  nd --help    Show this help
";

fn main() -> ExitCode {
    run(env::args_os().skip(1))
}

fn run(mut arguments: impl Iterator<Item = OsString>) -> ExitCode {
    match (arguments.next(), arguments.next()) {
        (None, None) => not_implemented("default TUI mode"),
        (Some(argument), None) if argument == "serve" => not_implemented("Control API Server mode"),
        (Some(argument), None) if argument == "--help" || argument == "-h" => {
            print!("{HELP}");
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("nd: unsupported arguments\n\n{HELP}");
            ExitCode::from(2)
        }
    }
}

fn not_implemented(mode: &str) -> ExitCode {
    eprintln!("nd: {mode} is not implemented yet");
    ExitCode::FAILURE
}
