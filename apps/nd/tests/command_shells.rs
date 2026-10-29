#![forbid(unsafe_code)]

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static SCRATCH_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[test]
fn help_describes_only_the_two_command_shells() {
    let output = run_in_empty_home(&["--help"]);

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("help output must be UTF-8");
    assert!(stdout.contains("nd           Open the TUI (not implemented yet)"));
    assert!(stdout.contains("nd serve     Start the Control API Server"));
    assert!(output.stderr.is_empty());
}

#[test]
fn default_mode_reports_that_it_is_not_implemented() {
    let output = run_in_empty_home(&[]);

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).expect("diagnostic must be UTF-8"),
        "nd: default TUI mode is not implemented yet\n"
    );
}

#[test]
fn serve_rejects_missing_configuration_before_creating_state() {
    let output = run_in_empty_home(&["serve"]);

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).expect("diagnostic must be UTF-8"),
        "nd serve: configuration-invalid\n"
    );
}

#[test]
fn unsupported_arguments_fail_without_starting_a_mode() {
    let output = run_in_empty_home(&["serve", "unexpected"]);

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).expect("diagnostic must be UTF-8");
    assert!(stderr.starts_with("nd: unsupported arguments\n\n"));
    assert!(stderr.contains("nd serve     Start the Control API Server"));
}

fn run_in_empty_home(arguments: &[&str]) -> Output {
    let scratch = ScratchDirectory::create();

    let output = Command::new(env!("CARGO_BIN_EXE_nd"))
        .args(arguments)
        .current_dir(scratch.path())
        .env("HOME", scratch.path())
        .env("ND_CONFIG_PATH", scratch.path().join("missing-config.yaml"))
        .env(
            "ND_APPLE_BRIDGE_PATH",
            scratch.path().join("missing-apple-bridge"),
        )
        .output()
        .expect("nd command shell should run");

    let remaining_entries = fs::read_dir(scratch.path())
        .expect("scratch directory should remain readable")
        .count();
    assert_eq!(
        remaining_entries, 0,
        "command shell must not create persistent or runtime state"
    );

    output
}

struct ScratchDirectory(PathBuf);

impl ScratchDirectory {
    fn create() -> Self {
        loop {
            let sequence = SCRATCH_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "nd-command-shell-{}-{sequence}",
                std::process::id(),
            ));

            match fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("scratch directory should be created: {error}"),
            }
        }
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for ScratchDirectory {
    fn drop(&mut self) {
        match fs::remove_dir_all(&self.0) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => eprintln!("failed to remove test scratch directory: {error}"),
        }
    }
}
