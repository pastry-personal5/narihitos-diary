#![forbid(unsafe_code)]

use std::fs;
use std::net::TcpListener;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[test]
fn serve_process_publishes_runtime_refuses_duplicate_and_cleans_up() {
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

    let runtime_dir = scratch.path().join("state/runtime");
    fs::create_dir_all(&runtime_dir).unwrap();
    fs::set_permissions(
        scratch.path().join("state"),
        fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    fs::set_permissions(&runtime_dir, fs::Permissions::from_mode(0o700)).unwrap();
    let runtime_file = runtime_dir.join("control-api.json");
    fs::write(
        &runtime_file,
        r#"{"version":1,"instance_id":"stale","host":"127.0.0.1","port":8771,"control_token":"stale","contract_version":1}
"#,
    )
    .unwrap();
    fs::set_permissions(&runtime_file, fs::Permissions::from_mode(0o600)).unwrap();

    let mut server = command(&scratch, &config)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    wait_for_runtime_replacement(&runtime_file, Duration::from_secs(10), &mut server);
    let runtime = fs::read_to_string(&runtime_file).unwrap();
    assert!(runtime.contains("\"host\":\"127.0.0.1\""));
    assert!(runtime.contains("\"control_token\":\"token_"));
    assert!(!runtime.contains("\"instance_id\":\"stale\""));
    let first_token = json_string(&runtime, "control_token").unwrap();
    let metadata = fs::metadata(&runtime_file).unwrap();
    assert_eq!(metadata.mode() & 0o777, 0o600);
    assert_eq!(metadata.nlink(), 1);

    let duplicate = command(&scratch, &config).output().unwrap();
    assert_eq!(duplicate.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(duplicate.stderr).unwrap(),
        "nd serve: server-already-running\n"
    );

    let status = Command::new("/bin/kill")
        .args(["-TERM", &server.id().to_string()])
        .status()
        .unwrap();
    assert!(status.success());
    wait_for_exit(&mut server, Duration::from_secs(12));
    assert!(server.wait().unwrap().success());
    assert!(!runtime_file.exists());

    let mut restarted = command(&scratch, &config)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    wait_for_path(&runtime_file, Duration::from_secs(10), &mut restarted);
    let restarted_runtime = fs::read_to_string(&runtime_file).unwrap();
    assert_ne!(
        json_string(&restarted_runtime, "control_token").unwrap(),
        first_token
    );
    assert!(
        Command::new("/bin/kill")
            .args(["-TERM", &restarted.id().to_string()])
            .status()
            .unwrap()
            .success()
    );
    wait_for_exit(&mut restarted, Duration::from_secs(12));
    assert!(restarted.wait().unwrap().success());
    assert!(!runtime_file.exists());

    let log = scratch
        .path()
        .join(".local/logs/control-api/control-api.log");
    let records = fs::read_to_string(&log).unwrap();
    for event in [
        "event=startup",
        "event=ready",
        "event=request",
        "event=shutdown",
    ] {
        assert!(records.contains(event));
    }
    assert!(!records.contains("REPLACE_WITH"));
    assert!(!records.contains("control_token"));
    assert_eq!(fs::metadata(log).unwrap().mode() & 0o777, 0o600);
}

fn json_string(document: &str, key: &str) -> Option<String> {
    let prefix = format!("\"{key}\":\"");
    let value = document.split_once(&prefix)?.1;
    Some(value.split_once('"')?.0.to_owned())
}

fn command(scratch: &Scratch, config: &Path) -> Command {
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

fn wait_for_runtime_replacement(path: &Path, timeout: Duration, child: &mut Child) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if fs::read_to_string(path).is_ok_and(|value| !value.contains("\"instance_id\":\"stale\""))
        {
            return;
        }
        if let Some(status) = child.try_wait().unwrap() {
            panic!("server exited before replacing stale runtime state: {status}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let _ = child.kill();
    panic!("server did not replace stale runtime state before timeout");
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
            "nd-control-api-process-{}-{}",
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
