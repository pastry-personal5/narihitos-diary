#![forbid(unsafe_code)]

//! End-to-end check that the default `nd` TUI entry point attaches to a
//! real `nd serve` process: reads the runtime file, matches unauthenticated
//! health, and only then proceeds — reaching "terminal unavailable" (there
//! is no real tty in this test harness) rather than "server not running"
//! or any unsafe-attachment state. See
//! `docs/ux-tui.md`
//! "Attachment and HTTP safety".

use std::fs;
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[test]
fn tui_mode_attaches_to_a_real_server_and_fails_only_on_the_missing_tty() {
    match TcpListener::bind(("127.0.0.1", 0)) {
        Ok(listener) => drop(listener),
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => return,
        Err(error) => panic!("test listener bind failed: {error}"),
    }
    let scratch = Scratch::new();
    let config = scratch.path().join("global_config.yaml");
    fs::write(
        &config,
        "sources:\n  ayumi_directory_name: \"SyntheticAyumi\"\npaths:\n  entries_root: \"ignored\"\nnotion:\n  token: \"REPLACE_WITH_NOTION_TOKEN\"\n  root_page_id: \"REPLACE_WITH_NOTION_ROOT_PAGE_ID\"\nserver:\n  port_fallback_count: 8\n",
    )
    .unwrap();
    fs::set_permissions(&config, fs::Permissions::from_mode(0o600)).unwrap();

    let runtime_file = scratch.path().join("state/runtime/control-api.json");
    let mut server = serve_command(&scratch, &config)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    wait_for_path(&runtime_file, Duration::from_secs(10), &mut server);

    let tui = Command::new(env!("CARGO_BIN_EXE_nd"))
        .current_dir(scratch.path())
        .env("HOME", scratch.path())
        .env("ND_STATE_DIR", scratch.path().join("state"))
        .stdin(Stdio::null())
        .output()
        .unwrap();

    let stderr = String::from_utf8(tui.stderr).unwrap();
    assert_eq!(
        tui.status.code(),
        Some(1),
        "unexpected tui exit, stderr: {stderr}"
    );
    assert_eq!(
        stderr, "nd: terminal-unavailable\n",
        "the client must get past attachment and health verification before \
         failing only on the missing tty in this harness"
    );
    assert!(tui.stdout.is_empty());

    let status = Command::new("/bin/kill")
        .args(["-TERM", &server.id().to_string()])
        .status()
        .unwrap();
    assert!(status.success());
    wait_for_exit(&mut server, Duration::from_secs(12));
}

fn serve_command(scratch: &Scratch, config: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_nd"));
    command
        .arg("serve")
        .current_dir(scratch.path())
        .env("HOME", scratch.path())
        .env("ND_CONFIG_PATH", config)
        .env("ND_STATE_DIR", scratch.path().join("state"))
        .env(
            "ND_APPLE_BRIDGE_PATH",
            scratch.path().join("missing-apple-bridge"),
        );
    command
}

fn wait_for_path(path: &Path, timeout: Duration, child: &mut Child) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if path.is_file() {
            return;
        }
        if let Some(status) = child.try_wait().unwrap() {
            panic!("server exited before publishing runtime state: {status}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let _ = child.kill();
    panic!("server did not publish runtime state before timeout");
}

fn wait_for_exit(child: &mut Child, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if child.try_wait().unwrap().is_some() {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let _ = child.kill();
    panic!("server did not stop before timeout");
}

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "nd-tui-tui-attach-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        Self(path)
    }
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
