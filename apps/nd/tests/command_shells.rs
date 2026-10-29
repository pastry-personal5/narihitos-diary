#![forbid(unsafe_code)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static SCRATCH_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[test]
fn help_describes_only_the_two_command_shells() {
    let output = run_in_empty_home(&["--help"]);

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("help output must be UTF-8");
    assert!(stdout.contains("nd           Open the TUI (manual workflow slice)"));
    assert!(stdout.contains("nd serve     Start the Control API Server"));
    assert!(output.stderr.is_empty());
}

#[test]
fn default_mode_reports_the_server_is_not_running_without_starting_it() {
    let output = run_in_empty_home(&[]);

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).expect("diagnostic must be UTF-8"),
        "nd: the Control API Server is not running.\nStart it in another terminal with: nd serve\n"
    );
}

#[test]
fn serve_rejects_missing_configuration_before_creating_state() {
    let output = run_in_empty_home(&["serve"]);

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).expect("diagnostic must be UTF-8"),
        "nd serve: configuration file is missing\n  Create the file selected by ND_CONFIG_PATH (default: config/global_config.yaml) from config/global_config.example.yaml.\n  Then restrict it to the current user (default file: chmod 600 config/global_config.yaml).\n"
    );
}

#[test]
fn serve_explains_unsafe_configuration_permissions_without_creating_state() {
    let (output, created_state) =
        run_with_config("sources:\n  ayumi_directory_name: \"Ayumi\"\n", 0o644);

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).expect("diagnostic must be UTF-8"),
        "nd serve: configuration file is unsafe\n  It must be a UTF-8 regular file owned by the current user, have exactly one hard link, be no larger than 256 KiB, and grant no group or world permissions.\n  For the default file, run: chmod 600 config/global_config.yaml\n"
    );
    assert!(!created_state);
}

#[test]
fn serve_explains_invalid_configuration_contents() {
    let (output, _) = run_with_config("unknown:\n  value: 1\n", 0o600);

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).expect("diagnostic must be UTF-8"),
        "nd serve: configuration file contents are invalid\n  Compare the file with config/global_config.example.yaml; unknown, duplicate, missing, and malformed fields are rejected.\n  sources.ayumi_directory_name must be one directory name, and server.port_fallback_count must be an integer from 0 through 16.\n"
    );
}

#[test]
fn serve_explains_invalid_source_directory_name() {
    let (output, _) = run_with_config(
        "sources:\n  ayumi_directory_name: \"/nested/Ayumi\"\n",
        0o600,
    );

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).expect("diagnostic must be UTF-8"),
        "nd serve: source configuration is invalid\n  Set sources.ayumi_directory_name to one directory name directly beneath iCloud Drive (for example, \"Ayumi\"), not an absolute or nested path.\n"
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

fn run_with_config(contents: &str, mode: u32) -> (Output, bool) {
    let scratch = ScratchDirectory::create();
    let config = scratch.path().join("global_config.yaml");
    fs::write(&config, contents).expect("configuration fixture should be written");
    fs::set_permissions(&config, fs::Permissions::from_mode(mode))
        .expect("configuration fixture permissions should be set");

    let output = Command::new(env!("CARGO_BIN_EXE_nd"))
        .arg("serve")
        .current_dir(scratch.path())
        .env("HOME", scratch.path())
        .env("ND_CONFIG_PATH", &config)
        .env("ND_STATE_DIR", scratch.path().join("state"))
        .env(
            "ND_APPLE_BRIDGE_PATH",
            scratch.path().join("missing-apple-bridge"),
        )
        .output()
        .expect("nd serve command shell should run");
    let created_state =
        scratch.path().join("state").exists() || scratch.path().join(".local").exists();
    (output, created_state)
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
