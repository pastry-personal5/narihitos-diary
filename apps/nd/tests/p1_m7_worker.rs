#![forbid(unsafe_code)]

//! End-to-end check that `diary_tui::worker::Worker` actually talks to a
//! real `nd serve` process on its background thread: the test thread
//! never calls the network directly, only `try_send`/`try_recv` on the
//! bounded channels, which is exactly the boundary the render loop will
//! use once autosave/IME-poll/SSE-reload are wired on top of it.

use std::fs;
use std::net::TcpListener;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use diary_control::load_runtime_attachment;
use diary_tui::client::Attachment as ClientAttachment;
use diary_tui::client::{ClientOutcome, UreqTransport};
use diary_tui::worker::{Intent, Worker, WorkerResult};

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[test]
fn the_worker_thread_completes_a_real_request_against_a_real_server() {
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

    let state_dir = scratch.path().join("state");
    let runtime_file = state_dir.join("runtime/control-api.json");
    let mut server = serve_command(&scratch, &config)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    wait_for_path(&runtime_file, Duration::from_secs(10), &mut server);

    let expected_uid = fs::metadata(scratch.path()).unwrap().uid();
    let control_attachment =
        load_runtime_attachment(&state_dir.join("runtime"), expected_uid).unwrap();
    let client_attachment = ClientAttachment::new(
        control_attachment.instance_id().to_owned(),
        control_attachment.host().to_owned(),
        control_attachment.port(),
        control_attachment.control_token().to_owned(),
        control_attachment.contract_version(),
    );

    let worker = Worker::spawn(UreqTransport::new(), client_attachment);
    worker.try_send(Intent::RefreshWorkflows).unwrap();

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut received = None;
    while Instant::now() < deadline && received.is_none() {
        if let Some(result) = worker.try_recv() {
            received = Some(result);
        } else {
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    match received {
        Some(WorkerResult::Workflows(ClientOutcome::Ok(list))) => {
            assert!(
                list.workflows.is_empty(),
                "a freshly started server should have no workflows yet"
            );
        }
        Some(_) => panic!("expected a successful Workflows result from the worker thread"),
        None => panic!("worker thread did not reply within the timeout"),
    }
    drop(worker);

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
            "nd-p1-m7-worker-{}-{}",
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
