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
use diary_tui::events::{EventWorker, EventWorkerResult, UreqEventSource};
use diary_tui::worker::{Intent, Worker, WorkerResult};

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[test]
fn the_workers_complete_real_rest_and_event_requests_against_a_real_server() {
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
    let first_instance_id = control_attachment.instance_id().to_owned();
    let client_attachment = ClientAttachment::new(
        control_attachment.instance_id().to_owned(),
        control_attachment.host().to_owned(),
        control_attachment.port(),
        control_attachment.control_token().to_owned(),
        control_attachment.contract_version(),
    );

    let event_worker = EventWorker::spawn(UreqEventSource::new(), client_attachment.clone(), None);
    let event_deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match event_worker.try_recv() {
            Some(EventWorkerResult::Connected) => break,
            Some(EventWorkerResult::Disconnected(error)) => {
                panic!("event worker did not attach: {error:?}")
            }
            Some(_) | None if Instant::now() < event_deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            _ => panic!("event worker did not connect within the timeout"),
        }
    }

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

    worker
        .try_send(Intent::StartOrResumeWorkflow {
            diary_day: Some("2026-10-29".to_owned()),
        })
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let started_workflow = loop {
        match worker.try_recv() {
            Some(WorkerResult::StartedWorkflow(outcome)) => match *outcome {
                ClientOutcome::Ok(workflow) => break workflow,
                _ => panic!("start request did not return a workflow"),
            },
            Some(_) | None if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            _ => panic!("ordinary worker did not start a workflow within the timeout"),
        }
    };
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match event_worker.try_recv() {
            Some(EventWorkerResult::Event(event)) => {
                assert_eq!(event.event_type, "workflow.changed");
                assert_eq!(
                    event.workflow_id.as_deref(),
                    Some(started_workflow.id.as_str())
                );
                break;
            }
            Some(EventWorkerResult::Disconnected(error)) => {
                panic!("event worker disconnected before the mutation event: {error:?}")
            }
            Some(_) | None if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            _ => panic!("event worker did not receive the mutation within the timeout"),
        }
    }

    worker
        .try_send(Intent::ConfirmEmptySelection {
            workflow_id: started_workflow.id.clone(),
            revision: started_workflow.revision,
        })
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let empty_workflow = loop {
        match worker.try_recv() {
            Some(WorkerResult::EmptySelectionConfirmed(outcome)) => match *outcome {
                ClientOutcome::Ok(workflow) => {
                    assert_eq!(workflow.state, "sources-confirmed");
                    assert!(workflow.explicit_empty_selection);
                    break workflow;
                }
                _ => panic!("explicit empty selection did not succeed"),
            },
            Some(_) | None if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            _ => panic!("empty-selection confirmation did not finish within the timeout"),
        }
    };

    worker
        .try_send(Intent::SetWritingSettings {
            workflow_id: empty_workflow.id.clone(),
            revision: empty_workflow.revision,
            language: "en-US".to_owned(),
            style: "concise".to_owned(),
        })
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let settings_workflow = loop {
        match worker.try_recv() {
            Some(WorkerResult::WritingSettingsSet(outcome)) => match *outcome {
                ClientOutcome::Ok(workflow) => break workflow,
                _ => panic!("writing settings did not succeed"),
            },
            Some(_) | None if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            _ => panic!("writing settings did not finish within the timeout"),
        }
    };
    worker
        .try_send(Intent::BeginManualComposition {
            workflow_id: settings_workflow.id.clone(),
            revision: settings_workflow.revision,
        })
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let composing_workflow = loop {
        match worker.try_recv() {
            Some(WorkerResult::ManualCompositionBegun(outcome)) => match *outcome {
                ClientOutcome::Ok(workflow) => {
                    assert_eq!(workflow.state, "composing");
                    break workflow;
                }
                _ => panic!("manual composition did not succeed"),
            },
            Some(_) | None if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            _ => panic!("manual composition did not finish within the timeout"),
        }
    };
    worker
        .try_send(Intent::SaveEntry {
            workflow_id: composing_workflow.id.clone(),
            buffer_version: composing_workflow.buffer_version,
            text: "A synthetic, non-private test entry.".to_owned(),
        })
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let saved_workflow = loop {
        match worker.try_recv() {
            Some(WorkerResult::EntrySaved(outcome)) => match *outcome {
                ClientOutcome::Ok(workflow) => {
                    assert_eq!(workflow.entry_text, "A synthetic, non-private test entry.");
                    break workflow;
                }
                _ => panic!("entry save did not succeed"),
            },
            Some(_) | None if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            _ => panic!("entry save did not finish within the timeout"),
        }
    };
    worker
        .try_send(Intent::ConfirmEntry {
            workflow_id: saved_workflow.id.clone(),
            revision: saved_workflow.revision,
            buffer_version: saved_workflow.buffer_version,
        })
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let confirmed_workflow = loop {
        match worker.try_recv() {
            Some(WorkerResult::EntryConfirmed(outcome)) => match *outcome {
                ClientOutcome::Ok(workflow) => {
                    assert_eq!(workflow.state, "entry-confirmed");
                    assert!(workflow.entry_confirmed);
                    break workflow;
                }
                _ => panic!("entry confirmation did not succeed"),
            },
            Some(_) | None if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            _ => panic!("entry confirmation did not finish within the timeout"),
        }
    };
    drop(worker);
    drop(event_worker);

    let status = Command::new("/bin/kill")
        .args(["-TERM", &server.id().to_string()])
        .status()
        .unwrap();
    assert!(status.success());
    wait_for_exit(&mut server, Duration::from_secs(12));
    wait_for_absence(&runtime_file, Duration::from_secs(2));

    let mut restarted_server = serve_command(&scratch, &config)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    wait_for_path(
        &runtime_file,
        Duration::from_secs(10),
        &mut restarted_server,
    );
    let restarted_control_attachment =
        load_runtime_attachment(&state_dir.join("runtime"), expected_uid).unwrap();
    assert_ne!(
        restarted_control_attachment.instance_id(),
        first_instance_id,
        "a restarted server must publish a new attachment instance"
    );
    let restarted_client_attachment = ClientAttachment::new(
        restarted_control_attachment.instance_id().to_owned(),
        restarted_control_attachment.host().to_owned(),
        restarted_control_attachment.port(),
        restarted_control_attachment.control_token().to_owned(),
        restarted_control_attachment.contract_version(),
    );
    let recovery_worker = Worker::spawn(UreqTransport::new(), restarted_client_attachment);
    recovery_worker
        .try_send(Intent::GetWorkflow {
            workflow_id: confirmed_workflow.id.clone(),
        })
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match recovery_worker.try_recv() {
            Some(WorkerResult::WorkflowFetched(outcome)) => match *outcome {
                ClientOutcome::Ok(workflow) => {
                    assert_eq!(workflow.id, confirmed_workflow.id);
                    assert_eq!(workflow.state, "entry-confirmed");
                    assert!(workflow.entry_confirmed);
                    assert_eq!(workflow.entry_text, "A synthetic, non-private test entry.");
                    break;
                }
                _ => panic!("restarted server did not recover the confirmed workflow"),
            },
            Some(_) | None if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            _ => panic!("workflow recovery did not finish within the timeout"),
        }
    }
    drop(recovery_worker);

    let status = Command::new("/bin/kill")
        .args(["-TERM", &restarted_server.id().to_string()])
        .status()
        .unwrap();
    assert!(status.success());
    wait_for_exit(&mut restarted_server, Duration::from_secs(12));
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

fn wait_for_absence(path: &Path, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if !path.exists() {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("server did not remove runtime state before timeout");
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
