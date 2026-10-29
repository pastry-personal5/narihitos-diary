#![forbid(unsafe_code)]

use std::env;
use std::ffi::OsString;
use std::io::{IsTerminal, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::process::{Command, ExitCode};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use diary_connectors::{AppleBridgeClient, AyumiCatalog, AyumiSource, SqliteStore};
use diary_control::{
    CommandService, DiagnosticEvent, DiagnosticLevel, DiagnosticLog, LifecycleConfig,
    LifecycleError, LogClock, RuntimeAdvertisement, ServerConfig, ServerLock, bind_loopback,
    load_runtime_attachment, random_id, serve,
};
use diary_control_contract::CONTRACT_VERSION;
use diary_core::{Clock, DiaryDay, OperationLog, Timestamp, WorkflowStore};
use diary_tui::client::{
    ApiClient, Attachment as ClientAttachment, AttachmentState, UreqTransport, verify_health,
};
use diary_tui::ime::{ImeState, MacInputLanguageQuery, PollingImeTracker};
use diary_tui::layout::standard_areas;
use diary_tui::main_pane::MainPaneWidget;
use diary_tui::resources::ResourcesSidebarWidget;
use diary_tui::sidebar::WorkflowSidebarWidget;
use diary_tui::stage::{Stage, map_workflow};
use diary_tui::start_view::StartViewModel;
use diary_tui::status_line::{StatusLineModel, StatusLineWidget};
use diary_tui::terminal::TerminalGuard;
use diary_tui::work_status::{WorkStatusModel, WorkStatusWidget};
use diary_tui::worker::{Intent, Worker, WorkerResult};
use tokio::sync::{mpsc, oneshot};

const HELP: &str = "\
nd — Narihito's Diary

Usage:
  nd           Open the TUI (manual workflow slice)
  nd serve     Start the Control API Server
  nd --help    Show this help
";

fn main() -> ExitCode {
    run(env::args_os().skip(1))
}

fn run(mut arguments: impl Iterator<Item = OsString>) -> ExitCode {
    match (arguments.next(), arguments.next()) {
        (None, None) => tui_mode(),
        (Some(argument), None) if argument == "serve" => serve_mode(),
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

fn serve_mode() -> ExitCode {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(_) => return fail("runtime-init-failed"),
    };
    match runtime.block_on(run_server()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(code) => fail(code),
    }
}

async fn run_server() -> Result<(), &'static str> {
    let lifecycle = LifecycleConfig::load().map_err(|_| "configuration-invalid")?;
    lifecycle
        .prepare_private_state()
        .map_err(|_| "private-state-invalid")?;
    let server_lock = ServerLock::acquire(&lifecycle.runtime_dir, lifecycle.expected_uid)
        .map_err(|_| "server-already-running")?;
    lifecycle
        .prepare_log_directory()
        .map_err(|_| "diagnostic-path-invalid")?;
    let clock = Arc::new(SystemClock);
    let log = Arc::new(
        DiagnosticLog::open(&lifecycle.log_dir, lifecycle.expected_uid, clock.clone())
            .map_err(|_| "diagnostic-init-failed")?,
    );
    log.write(
        DiagnosticLevel::Info,
        DiagnosticEvent::Lifecycle {
            component: "server",
            event: "startup",
        },
    )
    .map_err(|_| "diagnostic-write-failed")?;

    lifecycle
        .prepare_database()
        .map_err(|_| "database-permission-failed")?;
    lifecycle
        .validate_database_files()
        .map_err(|_| "database-permission-failed")?;
    let store =
        Arc::new(SqliteStore::open(&lifecycle.database_path).map_err(|_| "database-open-failed")?);
    lifecycle
        .validate_database_files()
        .map_err(|_| "database-permission-failed")?;
    store
        .purge_expired(clock.now())
        .map_err(|_| "database-purge-failed")?;
    let _unresolved = store
        .unresolved_intents(None)
        .map_err(|_| "reconciliation-load-failed")?;

    let executable = env::current_exe().map_err(|_| "helper-lookup-failed")?;
    let helper = AppleBridgeClient::locate_executable(&executable, &lifecycle.launch_root);
    let helper_available = helper.is_ok();
    let helper_path =
        helper.unwrap_or_else(|_| lifecycle.runtime_dir.join("apple-bridge-unavailable"));
    let catalog = AyumiCatalog::new(icloud_root()?, lifecycle.ayumi_directory_name.clone())
        .map_err(|_| "source-configuration-invalid")?;
    let bridge = AppleBridgeClient::new(
        helper_path,
        lifecycle.ayumi_directory_name.clone(),
        lifecycle.runtime_dir.join("apple-bridge-handoff"),
        Duration::from_secs(120),
        4,
    )
    .map_err(|_| "helper-init-failed")?;
    let source = Arc::new(AyumiSource::new(catalog, bridge));
    let commands = CommandService::new(store.clone(), source.clone(), source, clock.clone());

    let (listener, port) = bind_loopback(lifecycle.port_fallback_count).await?;
    let instance_id = random_id("instance_").map_err(|_| "random-failed")?;
    let control_token = random_id("token_").map_err(|_| "random-failed")?;
    let config = ServerConfig {
        instance_id: instance_id.clone(),
        control_token: control_token.clone(),
        port,
        commands,
        apple_helper_available: helper_available,
        notion_configured: lifecycle.notion_configured,
        diagnostics: Some(Arc::clone(&log)),
        diagnostic_failure: None,
    };
    let (diagnostic_failure_tx, mut diagnostic_failure_rx) = mpsc::unbounded_channel();
    let config = ServerConfig {
        diagnostic_failure: Some(diagnostic_failure_tx.clone()),
        ..config
    };
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let mut server = tokio::spawn(serve(listener, config, async move {
        let _ = shutdown_rx.await;
    }));
    health_probe(port, &instance_id).await?;
    if diagnostic_failure_rx.try_recv().is_ok() {
        return Err("diagnostic-runtime-failed");
    }
    if server.is_finished() {
        return Err("server-stopped-before-publication");
    }
    let advertisement = RuntimeAdvertisement {
        version: 1,
        instance_id: instance_id.clone(),
        host: "127.0.0.1".to_owned(),
        port,
        control_token,
        contract_version: CONTRACT_VERSION,
    };
    log.write(
        DiagnosticLevel::Info,
        DiagnosticEvent::Lifecycle {
            component: "server",
            event: "ready",
        },
    )
    .map_err(|_| "diagnostic-write-failed")?;
    let runtime_file = advertisement
        .publish(&lifecycle.runtime_dir, lifecycle.expected_uid)
        .map_err(|_| "runtime-publish-failed")?;
    println!("nd serve: ready on 127.0.0.1:{port}");

    let maintenance_log = Arc::clone(&log);
    let maintenance_failure = diagnostic_failure_tx;
    let maintenance = tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(60 * 60)).await;
            let log = Arc::clone(&maintenance_log);
            if !matches!(
                tokio::task::spawn_blocking(move || log.maintain()).await,
                Ok(Ok(()))
            ) {
                let _ = maintenance_failure.send(());
                break;
            }
        }
    });
    let mut failure = tokio::select! {
        _ = wait_for_signal() => None,
        _ = diagnostic_failure_rx.recv() => Some("diagnostic-runtime-failed"),
        result = &mut server => Some(match result {
            Ok(Ok(())) => "server-stopped-unexpectedly",
            Ok(Err(_)) => "server-runtime-failed",
            Err(_) => "server-task-failed",
        }),
    };
    if failure.is_none()
        && log
            .write(
                DiagnosticLevel::Info,
                DiagnosticEvent::Lifecycle {
                    component: "server",
                    event: "shutdown",
                },
            )
            .is_err()
    {
        failure = Some("diagnostic-write-failed");
    }
    let _ = shutdown_tx.send(());
    if !server.is_finished() {
        match tokio::time::timeout(Duration::from_secs(10), &mut server).await {
            Ok(Ok(Ok(()))) => {}
            Ok(Ok(Err(_))) => failure = Some("server-runtime-failed"),
            Ok(Err(_)) => failure = Some("server-task-failed"),
            Err(_) => {
                server.abort();
                let _ = server.await;
                failure = Some("shutdown-timeout");
            }
        }
    }
    maintenance.abort();
    let _ = maintenance.await;
    runtime_file
        .remove_if_owned()
        .map_err(|_| "runtime-cleanup-failed")?;
    drop(server_lock);
    match failure {
        Some(code) => Err(code),
        None => Ok(()),
    }
}

async fn health_probe(port: u16, instance_id: &str) -> Result<(), &'static str> {
    let expected = instance_id.to_owned();
    tokio::task::spawn_blocking(move || {
        let address = SocketAddr::from(([127, 0, 0, 1], port));
        let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(2))
            .map_err(|_| "health-probe-failed")?;
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .map_err(|_| "health-probe-failed")?;
        write!(
            stream,
            "GET /v1/health HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
        )
        .map_err(|_| "health-probe-failed")?;
        let mut response = String::new();
        stream
            .read_to_string(&mut response)
            .map_err(|_| "health-probe-failed")?;
        if response.starts_with("HTTP/1.1 200") && response.contains(&expected) {
            Ok(())
        } else {
            Err("health-probe-failed")
        }
    })
    .await
    .map_err(|_| "health-probe-failed")?
}

async fn wait_for_signal() {
    #[cfg(unix)]
    {
        if let Ok(mut terminate) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = terminate.recv() => {} }
            return;
        }
    }
    let _ = tokio::signal::ctrl_c().await;
}

fn icloud_root() -> Result<PathBuf, &'static str> {
    let home = env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or("home-unavailable")?;
    Ok(home.join("Library/Mobile Documents/com~apple~CloudDocs"))
}

#[derive(Debug)]
struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        Timestamp(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |duration| {
                    i64::try_from(duration.as_secs()).unwrap_or(i64::MAX)
                }),
        )
    }

    fn local_day(&self) -> DiaryDay {
        let value = Command::new("/bin/date")
            .arg("+%Y-%m-%d")
            .output()
            .ok()
            .and_then(|output| output.status.success().then_some(output.stdout))
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .and_then(|value| DiaryDay::parse(value.trim()).ok());
        value.unwrap_or_else(|| DiaryDay::parse("1970-01-01").expect("fixed fallback day"))
    }
}

impl LogClock for SystemClock {
    fn now_unix_ms(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| {
                u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
            })
    }
}

/// Resolves the Application Support runtime directory and the owning uid,
/// without loading product configuration or creating any state — the same
/// boundary `LifecycleConfig::load` enforces for the server, but the TUI
/// client must reach it without reading `config/global_config.yaml`.
fn resolve_runtime_location() -> Result<(PathBuf, u32), &'static str> {
    use std::os::unix::fs::MetadataExt;
    let launch_root = env::current_dir().map_err(|_| "runtime-location-invalid")?;
    let expected_uid = std::fs::metadata(&launch_root)
        .map_err(|_| "runtime-location-invalid")?
        .uid();
    let home = env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or("runtime-location-invalid")?;
    let application_support = env::var_os("ND_STATE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join("Library/Application Support/narihitos-diary"));
    Ok((application_support.join("runtime"), expected_uid))
}

/// Dispatches to the real interactive TUI when stdout is a tty, or to a
/// one-shot headless path otherwise. The headless path exists because an
/// automated harness (the `command_shells`/`p1_m7_tui_attach` integration
/// tests) has no terminal to render a persistent retry screen into; a
/// real interactive session must never exit just because the server
/// is not running yet (see AGENTS.md "Server lifecycle").
fn tui_mode() -> ExitCode {
    if std::io::stdout().is_terminal() {
        tui_mode_interactive()
    } else {
        tui_mode_headless()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AttachPhase {
    NotRunning,
    Unsafe,
    VersionMismatch,
    InstanceMismatch,
}

/// Reads the runtime file and verifies health once. Returns the connected
/// client attachment on success, or the reason it could not connect.
fn attempt_attach(
    runtime_dir: &std::path::Path,
    expected_uid: u32,
    client: &ApiClient<UreqTransport>,
) -> Result<ClientAttachment, AttachPhase> {
    let control_attachment = match load_runtime_attachment(runtime_dir, expected_uid) {
        Ok(attachment) => attachment,
        Err(LifecycleError::MissingRuntime) => return Err(AttachPhase::NotRunning),
        Err(LifecycleError::IncompatibleContract) => return Err(AttachPhase::VersionMismatch),
        Err(_) => return Err(AttachPhase::Unsafe),
    };
    let client_attachment = ClientAttachment::new(
        control_attachment.instance_id().to_owned(),
        control_attachment.host().to_owned(),
        control_attachment.port(),
        control_attachment.control_token().to_owned(),
        control_attachment.contract_version(),
    );
    match verify_health(client, &client_attachment) {
        AttachmentState::Connected => Ok(client_attachment),
        AttachmentState::NotRunning => Err(AttachPhase::NotRunning),
        AttachmentState::InstanceMismatch => Err(AttachPhase::InstanceMismatch),
        AttachmentState::VersionMismatch => Err(AttachPhase::VersionMismatch),
        AttachmentState::UnsafeAttachment => Err(AttachPhase::Unsafe),
    }
}

fn attach_phase_message(phase: AttachPhase) -> &'static str {
    match phase {
        AttachPhase::NotRunning => {
            "The Control API Server is not running. Start it in another terminal with: nd serve"
        }
        AttachPhase::Unsafe => "The server responded unsafely; refusing to attach.",
        AttachPhase::VersionMismatch => {
            "This client does not support the running server's contract version."
        }
        AttachPhase::InstanceMismatch => "A different server instance is now running; reattaching.",
    }
}

const RETRY_INITIAL_BACKOFF: Duration = Duration::from_millis(250);
const RETRY_BACKOFF_CEILING: Duration = Duration::from_secs(4);

/// The real interactive entry point. Acquires the terminal once and keeps
/// it until the person quits: a not-running, unsafe, or version-mismatch
/// server is a screen with a retry backoff, never an exit. Ctrl-D quits
/// from any screen.
fn tui_mode_interactive() -> ExitCode {
    let mut guard = match TerminalGuard::acquire() {
        Ok(guard) => guard,
        Err(_) => return fail_tui("terminal-unavailable"),
    };
    let client = ApiClient::new(UreqTransport::new());
    let mut backoff = RETRY_INITIAL_BACKOFF;

    loop {
        let (runtime_dir, expected_uid) = match resolve_runtime_location() {
            Ok(value) => value,
            Err(_) => {
                draw_phase_screen(&mut guard, "Cannot resolve the runtime location.");
                match diary_tui::terminal::poll_event(backoff) {
                    diary_tui::terminal::PollOutcome::Quit => {
                        drop(guard);
                        return ExitCode::SUCCESS;
                    }
                    diary_tui::terminal::PollOutcome::Other => {}
                    diary_tui::terminal::PollOutcome::TimedOut => {
                        backoff = (backoff * 2).min(RETRY_BACKOFF_CEILING);
                    }
                }
                continue;
            }
        };
        match attempt_attach(&runtime_dir, expected_uid, &client) {
            Ok(client_attachment) => {
                let quit = run_connected_session(&mut guard, &client_attachment);
                drop(guard);
                return if quit {
                    ExitCode::SUCCESS
                } else {
                    fail_tui("render-failed")
                };
            }
            Err(phase) => {
                draw_phase_screen(&mut guard, attach_phase_message(phase));
                match diary_tui::terminal::poll_event(backoff) {
                    diary_tui::terminal::PollOutcome::Quit => {
                        drop(guard);
                        return ExitCode::SUCCESS;
                    }
                    diary_tui::terminal::PollOutcome::Other => {}
                    diary_tui::terminal::PollOutcome::TimedOut => {
                        backoff = (backoff * 2).min(RETRY_BACKOFF_CEILING);
                    }
                }
            }
        }
    }
}

fn draw_phase_screen(guard: &mut TerminalGuard, message: &str) {
    let work_status = WorkStatusModel {
        current_work: Some(message.to_owned()),
        pending_or_blocked: None,
        connection_and_save_safety: Some("Not connected".to_owned()),
        notice: Some("Retrying automatically. Press Ctrl-D to quit.".to_owned()),
    };
    let stage_model = map_workflow(None, false);
    let _ = guard.terminal_mut().draw(|frame| {
        let area = frame.area();
        if !diary_tui::layout::is_supported_size(area.width, area.height) {
            frame.render_widget(diary_tui::layout::TerminalTooSmallWidget, area);
            return;
        }
        let areas = standard_areas(area);
        WorkStatusWidget::new(&work_status).render_in(frame, areas.work_status);
        frame.render_widget(
            WorkflowSidebarWidget::new(&stage_model, Stage::Start),
            areas.workflow_sidebar,
        );
        frame.render_widget(MainPaneWidget::new("Main", message), areas.main_pane);
    });
}

/// Runs the connected shell until the person quits or a render fails.
/// Returns `true` for a clean quit, `false` for a render failure.
///
/// Per the architecture doc ("the event/render loop never performs HTTP...
/// work"), every network operation goes through the bounded [`Worker`]:
/// this function only sends `Intent`s with `try_send` and drains
/// `WorkerResult`s with `try_recv`, never calling `ApiClient` directly.
fn run_connected_session(guard: &mut TerminalGuard, client_attachment: &ClientAttachment) -> bool {
    // P1-M7 minimal vertical slice: no workflow is attached yet, so the
    // Workflow Sidebar shows Start only. Sources/Compose/Review ViewModels
    // and the editor are implemented separately.
    let stage_model = map_workflow(None, false);
    let work_status = WorkStatusModel {
        current_work: Some("Connected to nd serve".to_owned()),
        pending_or_blocked: None,
        connection_and_save_safety: Some("Connected".to_owned()),
        notice: Some("Press Ctrl-G for Command Line, Ctrl-D to quit".to_owned()),
    };
    let mut status_line = StatusLineModel {
        diary_day: None,
        connection_label: "Connected",
        ime: ImeState::Unknown,
        action_hint: Some("Ctrl-D: quit".to_owned()),
    };

    let worker = Worker::spawn(UreqTransport::new(), client_attachment.clone());
    let mut start_view = StartViewModel::default();
    let mut refresh_pending = worker.try_send(Intent::RefreshWorkflows).is_ok();

    let mut ime_tracker = PollingImeTracker::new(MacInputLanguageQuery);
    const TICK_INTERVAL: Duration = Duration::from_millis(500);
    const DEFAULT_NOTICE: &str = "Press Ctrl-G for Command Line, Ctrl-D to quit";

    let mut command_line_active = false;
    let mut command_line = diary_tui::command_line::CommandLineState::default();
    let mut feedback: Option<String> = None;
    let mut selected_day: Option<String> = None;

    loop {
        while let Some(result) = worker.try_recv() {
            if let WorkerResult::Workflows(outcome) = result {
                refresh_pending = false;
                start_view.apply_refresh(outcome);
                if let Some(error) = &start_view.last_error {
                    feedback = Some(error.safe_label());
                }
            }
        }

        status_line.ime = ime_tracker.tick(std::time::Instant::now());
        status_line.diary_day = selected_day.clone();

        let mut displayed_status = work_status.clone();
        displayed_status.notice = Some(if command_line_active {
            format!(":{}", command_line.raw())
        } else {
            feedback
                .clone()
                .unwrap_or_else(|| DEFAULT_NOTICE.to_owned())
        });

        let drawn = guard.terminal_mut().draw(|frame| {
            let area = frame.area();
            if !diary_tui::layout::is_supported_size(area.width, area.height) {
                frame.render_widget(diary_tui::layout::TerminalTooSmallWidget, area);
                return;
            }
            let areas = standard_areas(area);
            WorkStatusWidget::new(&displayed_status).render_in(frame, areas.work_status);
            frame.render_widget(
                WorkflowSidebarWidget::new(&stage_model, Stage::Start),
                areas.workflow_sidebar,
            );
            frame.render_widget(
                ResourcesSidebarWidget::new(&start_view.resources()),
                areas.resources_sidebar,
            );
            frame.render_widget(
                MainPaneWidget::new(
                    "Main",
                    "Choose, resume, or discard a workflow from the Start stage.",
                ),
                areas.main_pane,
            );
            frame.render_widget(StatusLineWidget::new(&status_line), areas.status_line);
        });
        if drawn.is_err() {
            return false;
        }

        use diary_tui::command_line::Command;
        use diary_tui::terminal::{InputEvent, InputPoll};
        match diary_tui::terminal::poll_input_event(TICK_INTERVAL) {
            InputPoll::Quit => return true,
            InputPoll::TimedOut | InputPoll::Ignored => {}
            InputPoll::Ctrl('g') if !command_line_active => {
                command_line_active = true;
                command_line.clear();
                feedback = None;
            }
            InputPoll::Ctrl(_) => {}
            InputPoll::Event(event) if command_line_active => match event {
                InputEvent::Char(ch) => command_line.push_char(ch),
                InputEvent::Backspace => command_line.backspace(),
                InputEvent::Esc => {
                    command_line_active = false;
                    command_line.clear();
                }
                InputEvent::Paste(data) => {
                    for ch in data.chars() {
                        command_line.push_char(ch);
                    }
                }
                InputEvent::Enter => {
                    command_line_active = false;
                    feedback = Some(match command_line.submit() {
                        Ok(Command::Quit) => return true,
                        Ok(Command::Help) => {
                            "help, retry, day YYYY-MM-DD, stage <name>, save, quit".to_owned()
                        }
                        Ok(Command::Retry) => {
                            if refresh_pending {
                                "A refresh is already in progress".to_owned()
                            } else {
                                refresh_pending = worker.try_send(Intent::RefreshWorkflows).is_ok();
                                "Refreshing...".to_owned()
                            }
                        }
                        Ok(Command::Save) => "Nothing to save yet".to_owned(),
                        Ok(Command::Day(day)) => {
                            let message = format!("Day set to {day}");
                            selected_day = Some(day);
                            message
                        }
                        Ok(Command::Stage(stage)) => {
                            if stage == Stage::Start {
                                "Already at Start".to_owned()
                            } else {
                                "That stage is not available yet".to_owned()
                            }
                        }
                        Err(_) => "Unknown or malformed command".to_owned(),
                    });
                }
            },
            InputPoll::Event(_) => {}
        }
    }
}

/// Headless, one-shot path used only when stdout is not a tty (automated
/// tests). Mirrors the old always-exit behavior: attach once, then exit
/// with a content-free diagnostic rather than opening a persistent
/// terminal screen that nothing will ever see.
fn tui_mode_headless() -> ExitCode {
    let (runtime_dir, expected_uid) = match resolve_runtime_location() {
        Ok(value) => value,
        Err(code) => return fail_tui(code),
    };
    let client = ApiClient::new(UreqTransport::new());
    let client_attachment = match attempt_attach(&runtime_dir, expected_uid, &client) {
        Ok(attachment) => attachment,
        Err(AttachPhase::NotRunning) => return not_running(),
        Err(phase) => {
            eprintln!("nd: {}", attach_phase_message(phase));
            return ExitCode::FAILURE;
        }
    };
    let _ = &client_attachment;

    match TerminalGuard::acquire() {
        Ok(guard) => {
            drop(guard);
            ExitCode::SUCCESS
        }
        Err(_) => fail_tui("terminal-unavailable"),
    }
}

fn not_running() -> ExitCode {
    eprintln!(
        "nd: the Control API Server is not running.\nStart it in another terminal with: nd serve"
    );
    ExitCode::FAILURE
}

fn fail_tui(code: &str) -> ExitCode {
    eprintln!("nd: {code}");
    ExitCode::FAILURE
}

fn fail(code: &str) -> ExitCode {
    eprintln!("nd serve: {code}");
    ExitCode::FAILURE
}
