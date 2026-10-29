#![forbid(unsafe_code)]

use std::env;
use std::ffi::OsString;
use std::io::{IsTerminal, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::process::{Command, ExitCode};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use diary_connectors::{AppleBridgeClient, AyumiCatalog, AyumiSource, SqliteStore};
use diary_control::{
    CommandService, DiagnosticEvent, DiagnosticLevel, DiagnosticLog, LifecycleConfig,
    LifecycleError, LogClock, RuntimeAdvertisement, ServerConfig, ServerLock, bind_loopback,
    load_runtime_attachment, random_id, serve,
};
use diary_control_contract::{
    CONTRACT_VERSION, CapabilitiesDto, PreferencesDto, SetupCheckDto, SourceDetailDto, WorkflowDto,
};
use diary_core::{Clock, DiaryDay, OperationLog, Timestamp, WorkflowStore};
use diary_tui::client::{
    ApiClient, Attachment as ClientAttachment, AttachmentState, ClientOutcome, UreqTransport,
    verify_health,
};
use diary_tui::command_line_view::CommandLineWidget;
use diary_tui::compose_view::ComposeViewModel;
use diary_tui::date_display::format_diary_day;
use diary_tui::editor::{DraftEditor, DraftEditorPane};
use diary_tui::events::{EventError, EventWorker, EventWorkerResult, UreqEventSource};
use diary_tui::focus::{FocusModel, FocusZone, SidebarKind};
use diary_tui::ime::{ImeState, ImeTracker, MacInputLanguageNotifications, MacInputLanguageQuery};
use diary_tui::layout::{OverlayClearWidget, adaptive_areas};
use diary_tui::main_pane::MainPaneWidget;
use diary_tui::resources::{ResourceRow, ResourcesSidebarModel, ResourcesSidebarWidget};
use diary_tui::review_view::{ReviewViewModel, can_confirm, first_unresolved_delimiter};
use diary_tui::session::{RecoveryCause, SessionAction, SessionAuthority};
use diary_tui::sidebar::WorkflowSidebarWidget;
use diary_tui::sources_view::{JobPhase, SourcesViewModel};
use diary_tui::stage::{ALL_STAGES, Progress, Stage, adjacent_accessible_stage, map_workflow};
use diary_tui::start_view::StartViewModel;
use diary_tui::status_line::{StatusLineModel, StatusLineWidget};
use diary_tui::terminal::TerminalGuard;
use diary_tui::theme::Palette;
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

const SOURCE_CONFIGURATION_ERROR: &str = "source configuration is invalid
  Set sources.ayumi_directory_name to one directory name directly beneath iCloud Drive (for example, \"Ayumi\"), not an absolute or nested path.";

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
    let lifecycle = LifecycleConfig::load().map_err(configuration_error)?;
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
        .map_err(|_| SOURCE_CONFIGURATION_ERROR)?;
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

fn configuration_error(error: LifecycleError) -> &'static str {
    match error {
        LifecycleError::MissingConfig => {
            "configuration file is missing
  Create the file selected by ND_CONFIG_PATH (default: config/global_config.yaml) from config/global_config.example.yaml.
  Then restrict it to the current user (default file: chmod 600 config/global_config.yaml)."
        }
        LifecycleError::UnsafeConfig => {
            "configuration file is unsafe
  It must be a UTF-8 regular file owned by the current user, have exactly one hard link, be no larger than 256 KiB, and grant no group or world permissions.
  For the default file, run: chmod 600 config/global_config.yaml"
        }
        LifecycleError::InvalidConfig => {
            "configuration file contents are invalid
  Compare the file with config/global_config.example.yaml; unknown, duplicate, missing, and malformed fields are rejected.
  sources.ayumi_directory_name must be one directory name, and server.port_fallback_count must be an integer from 0 through 16."
        }
        LifecycleError::MissingHome => {
            "configuration environment is invalid
  HOME is not set; set it to the current user's home directory and retry."
        }
        LifecycleError::Io => {
            "configuration could not be loaded
  Verify that the working directory and configuration file are accessible, then retry."
        }
        LifecycleError::MissingRuntime
        | LifecycleError::UnsafePath
        | LifecycleError::AlreadyRunning
        | LifecycleError::InvalidRuntime
        | LifecycleError::IncompatibleContract
        | LifecycleError::Random => {
            "configuration validation failed
  Verify config/global_config.yaml against config/global_config.example.yaml and retry."
        }
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
            Ok(client_attachment) => match run_connected_session(&mut guard, &client_attachment) {
                SessionEnd::Quit => {
                    drop(guard);
                    return ExitCode::SUCCESS;
                }
                SessionEnd::Reattach => {
                    backoff = RETRY_INITIAL_BACKOFF;
                }
                SessionEnd::RenderFailed => {
                    drop(guard);
                    return fail_tui("render-failed");
                }
            },
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
    let status_line = StatusLineModel {
        diary_day: None,
        connection_label: "Not connected",
        ime: ImeState::Unknown,
        action_hint: Some("Ctrl-D: quit".to_owned()),
    };
    let palette = Palette::new(env::var_os("NO_COLOR").is_some());
    let _ = guard.terminal_mut().draw(|frame| {
        let area = frame.area();
        if !diary_tui::layout::is_supported_size(area.width, area.height) {
            frame.render_widget(diary_tui::layout::TerminalTooSmallWidget, area);
            return;
        }
        let areas = adaptive_areas(area, FocusZone::Main);
        WorkStatusWidget::new(&work_status)
            .palette(palette)
            .render_in(frame, areas.work_status);
        frame.render_widget(
            MainPaneWidget::new("Main", message).palette(palette),
            areas.main_pane,
        );
        if let Some(workflow_area) = areas.workflow_sidebar {
            frame.render_widget(
                WorkflowSidebarWidget::new(&stage_model, Stage::Start).palette(palette),
                workflow_area,
            );
        }
        frame.render_widget(
            CommandLineWidget::new(false, "", None).palette(palette),
            areas.command_line,
        );
        frame.render_widget(
            StatusLineWidget::new(&status_line).palette(palette),
            areas.status_line,
        );
    });
}

/// Runs the connected shell until the person quits or a render fails.
/// Returns `true` for a clean quit, `false` for a render failure.
///
/// Per the architecture doc ("the event/render loop never performs HTTP...
/// work"), every network operation goes through the bounded [`Worker`]:
/// this function only sends `Intent`s with `try_send` and drains
/// `WorkerResult`s with `try_recv`, never calling `ApiClient` directly.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SessionEnd {
    Quit,
    Reattach,
    RenderFailed,
}

fn run_connected_session(
    guard: &mut TerminalGuard,
    client_attachment: &ClientAttachment,
) -> SessionEnd {
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
    let event_worker = EventWorker::spawn(UreqEventSource::new(), client_attachment.clone(), None);
    let mut authority = SessionAuthority::connected(client_attachment.instance_id().to_owned());
    let mut start_view = StartViewModel::default();
    let mut refresh_pending = worker.try_send(Intent::RefreshWorkflows).is_ok();
    let _ = worker.try_send(Intent::GetCapabilities);
    let _ = worker.try_send(Intent::GetSetupCheck);
    let _ = worker.try_send(Intent::GetPreferences);
    let mut recovery_list_required = false;
    let mut recovery_workflow: Option<(String, bool)> = None;
    let mut recovery_job: Option<(String, bool)> = None;
    let mut capabilities: Option<CapabilitiesDto> = None;
    let mut setup_check: Option<SetupCheckDto> = None;
    let mut preferences: Option<PreferencesDto> = None;
    let mut current_workflow: Option<WorkflowDto> = None;
    let mut current_stage = Stage::Start;
    let mut focus = FocusModel::default();
    let mut start_pending = false;
    let mut more_pending = false;
    let mut discard_pending: Option<usize> = None;
    let mut discard_confirmation: Option<usize> = None;
    let mut help_open = false;
    let mut sources_view = SourcesViewModel::default();
    let mut pending_source_job_kind: Option<bool> = None;
    let mut source_job_poll_in_flight = false;
    let mut source_page_pending = false;
    let mut source_page_append = false;
    let mut source_detail_pending_index: Option<usize> = None;
    let mut active_source_index: Option<usize> = None;
    let mut source_confirmation_pending = false;
    let mut empty_selection_confirmation = false;
    let mut source_date_editing = false;
    let mut source_date_draft = String::new();
    let mut explicit_source_day: Option<String> = None;
    let mut compose_view = ComposeViewModel::default();
    let mut draft_editor = DraftEditor::new("");
    let mut editor_active = false;
    let mut compose_active_resource = 0usize;
    let mut confirmed_source_detail: Option<SourceDetailDto> = None;
    let mut confirmed_source_pending = false;
    let mut settings_pending = false;
    let mut manual_composition_pending = false;
    let mut save_in_flight: Option<u64> = None;
    let mut save_requested = false;
    let mut save_blocked_for_resolution = false;
    let mut last_edit_at: Option<Instant> = None;
    let mut review_view = ReviewViewModel::default();
    let mut entry_confirmation_pending = false;
    let mut quit_confirmation = false;

    let mut ime_tracker = ImeTracker::new(
        MacInputLanguageNotifications::start().ok(),
        MacInputLanguageQuery,
    );
    const TICK_INTERVAL: Duration = Duration::from_millis(100);
    const DEFAULT_NOTICE: &str = "Press Ctrl-G for Command Line, Ctrl-D to quit";

    let mut command_line_active = false;
    let mut command_line = diary_tui::command_line::CommandLineState::default();
    let mut command_line_feedback: Option<String> = None;
    let mut feedback: Option<String> = None;
    let mut selected_day: Option<String> = None;
    let mut day_draft = String::new();
    let mut day_editing = false;
    let mut main_scroll = 0u16;
    let palette = Palette::new(env::var_os("NO_COLOR").is_some());

    loop {
        while let Some(result) = event_worker.try_recv() {
            let action = match result {
                EventWorkerResult::Connected => {
                    feedback = Some("Event stream connected".to_owned());
                    SessionAction::None
                }
                EventWorkerResult::Event(event) => authority.observe_event(&event),
                EventWorkerResult::Disconnected(error) => {
                    if matches!(error, EventError::Rejected | EventError::WrongContentType) {
                        return SessionEnd::Reattach;
                    }
                    feedback = Some(match error {
                        EventError::DecodeFailed => {
                            "Event stream reset; reloading canonical state".to_owned()
                        }
                        _ => "Event stream reconnecting; actions are paused".to_owned(),
                    });
                    authority.recover(
                        None,
                        None,
                        if error == EventError::DecodeFailed {
                            RecoveryCause::DecodeFailure
                        } else {
                            RecoveryCause::TransportLoss
                        },
                    )
                }
                EventWorkerResult::ResultOverflow => {
                    feedback = Some("Event queue overflow; reloading canonical state".to_owned());
                    authority.recover(None, None, RecoveryCause::ResultOverflow)
                }
            };
            match action {
                SessionAction::None => {}
                SessionAction::Reattach => return SessionEnd::Reattach,
                SessionAction::Reload {
                    workflow_id,
                    job_id,
                    ..
                } => {
                    recovery_list_required = true;
                    if let Some(workflow_id) = workflow_id {
                        recovery_workflow = Some((workflow_id, false));
                    }
                    if let Some(job_id) = job_id {
                        recovery_job = Some((job_id, false));
                    }
                }
            }
        }

        while let Some(result) = worker.try_recv() {
            match result {
                WorkerResult::Capabilities(outcome) => match outcome {
                    ClientOutcome::Ok(value) => capabilities = Some(value),
                    ClientOutcome::Problem(problem) => feedback = Some(problem.safe_label()),
                    ClientOutcome::Transport(error) => {
                        feedback = Some(error.safe_label().to_owned())
                    }
                },
                WorkerResult::SetupCheck(outcome) => match outcome {
                    ClientOutcome::Ok(value) => setup_check = Some(value),
                    ClientOutcome::Problem(problem) => feedback = Some(problem.safe_label()),
                    ClientOutcome::Transport(error) => {
                        feedback = Some(error.safe_label().to_owned())
                    }
                },
                WorkerResult::Preferences(outcome) => match outcome {
                    ClientOutcome::Ok(value) => preferences = Some(value),
                    ClientOutcome::Problem(problem) => feedback = Some(problem.safe_label()),
                    ClientOutcome::Transport(error) => {
                        feedback = Some(error.safe_label().to_owned())
                    }
                },
                WorkerResult::Workflows(outcome) => {
                    refresh_pending = false;
                    let loaded = matches!(&outcome, ClientOutcome::Ok(_));
                    start_view.apply_refresh(outcome);
                    if loaded && recovery_list_required {
                        recovery_list_required = false;
                        authority.canonical_workflow_list_loaded();
                    }
                    if let Some(error) = &start_view.last_error {
                        feedback = Some(error.safe_label());
                    }
                    focus.clamp(
                        ALL_STAGES.len(),
                        start_view.workflows.len(),
                        START_CONTROL_COUNT,
                    );
                }
                WorkerResult::MoreWorkflows(outcome) => {
                    more_pending = false;
                    start_view.apply_more(outcome);
                    focus.clamp(
                        ALL_STAGES.len(),
                        start_view.workflows.len(),
                        START_CONTROL_COUNT,
                    );
                    if let Some(error) = &start_view.last_error {
                        feedback = Some(error.safe_label());
                    }
                }
                WorkerResult::StartedWorkflow(outcome) => {
                    start_pending = false;
                    match *outcome {
                        ClientOutcome::Ok(workflow) => {
                            selected_day = Some(workflow.diary_day.clone());
                            let model = map_workflow(Some(&workflow), false);
                            current_stage = model.default_current;
                            sources_view.load_candidates_from(&workflow);
                            compose_view.load_from_canonical(&workflow);
                            draft_editor = DraftEditor::new(&compose_view.text);
                            source_page_append = false;
                            source_page_pending = worker
                                .try_send(Intent::ListSources {
                                    workflow_id: workflow.id.clone(),
                                    cursor: None,
                                })
                                .is_ok();
                            current_workflow = Some(workflow);
                            feedback = Some("Workflow ready".to_owned());
                            if !refresh_pending {
                                refresh_pending = worker.try_send(Intent::RefreshWorkflows).is_ok();
                            }
                        }
                        ClientOutcome::Problem(problem) => feedback = Some(problem.safe_label()),
                        ClientOutcome::Transport(error) => {
                            feedback = Some(error.safe_label().to_owned())
                        }
                    }
                }
                WorkerResult::Discarded(outcome) => {
                    if let Some(index) = discard_pending.take() {
                        let discarded_id = start_view
                            .workflows
                            .get(index)
                            .map(|workflow| workflow.id.clone());
                        if start_view.apply_discard(index, outcome) {
                            if current_workflow.as_ref().map(|workflow| &workflow.id)
                                == discarded_id.as_ref()
                            {
                                current_workflow = None;
                                current_stage = Stage::Start;
                            }
                            feedback = Some("Workflow discarded".to_owned());
                        } else if let Some(error) = &start_view.last_error {
                            feedback = Some(error.safe_label());
                        }
                        focus.clamp(
                            ALL_STAGES.len(),
                            start_view.workflows.len(),
                            START_CONTROL_COUNT,
                        );
                    }
                }
                WorkerResult::WorkflowFetched(outcome) => {
                    let loaded = matches!(&*outcome, ClientOutcome::Ok(_));
                    if loaded {
                        if let ClientOutcome::Ok(workflow) = &*outcome
                            && current_workflow
                                .as_ref()
                                .is_some_and(|current| current.id == workflow.id)
                        {
                            sources_view.load_candidates_from(workflow);
                            compose_view.load_from_canonical(workflow);
                            if !compose_view.is_dirty() {
                                draft_editor = DraftEditor::new(&compose_view.text);
                                save_blocked_for_resolution = false;
                            }
                            current_workflow = Some(workflow.clone());
                        }
                        recovery_workflow = None;
                        authority.canonical_workflow_loaded();
                    } else if let Some((_, in_flight)) = &mut recovery_workflow {
                        *in_flight = false;
                    }
                }
                WorkerResult::JobStatus(outcome) => {
                    let loaded = matches!(&outcome, ClientOutcome::Ok(_));
                    if source_job_poll_in_flight {
                        source_job_poll_in_flight = false;
                        let job_kind = match &outcome {
                            ClientOutcome::Ok(job) => Some(job.kind.clone()),
                            _ => None,
                        };
                        let phase = sources_view.apply_job_status(outcome);
                        if let Some(error) = &sources_view.last_error {
                            feedback = Some(error.safe_label());
                        }
                        match (job_kind.as_deref(), phase) {
                            (Some("discovery"), Some(JobPhase::Succeeded)) => {
                                sources_view.accept_fresh_discovery();
                                active_source_index = None;
                                if let Some(workflow) = &current_workflow {
                                    source_page_append = false;
                                    source_page_pending = worker
                                        .try_send(Intent::ListSources {
                                            workflow_id: workflow.id.clone(),
                                            cursor: None,
                                        })
                                        .is_ok();
                                    recovery_workflow = Some((workflow.id.clone(), false));
                                }
                                feedback = Some("Discovery complete".to_owned());
                            }
                            (Some("source-preparation"), Some(JobPhase::Succeeded)) => {
                                feedback = Some(
                                    "Preparation complete; inspect every included source"
                                        .to_owned(),
                                );
                            }
                            (_, Some(JobPhase::Failed)) => {
                                feedback = Some("Source job failed; review and retry".to_owned());
                            }
                            _ => {}
                        }
                    } else {
                        // A recovery-only job read has no Sources-stage state to apply.
                    }
                    if loaded {
                        recovery_job = None;
                        authority.canonical_job_loaded();
                    } else if let Some((_, in_flight)) = &mut recovery_job {
                        *in_flight = false;
                    }
                }
                WorkerResult::JobAccepted(outcome) => {
                    if let Some(preparation) = pending_source_job_kind.take() {
                        sources_view.apply_job_accepted(preparation, outcome);
                        if let Some(error) = &sources_view.last_error {
                            feedback = Some(error.safe_label());
                        }
                    }
                }
                WorkerResult::Sources(outcome) => {
                    source_page_pending = false;
                    sources_view.apply_source_page(source_page_append, outcome);
                    if let Some(error) = &sources_view.last_error {
                        feedback = Some(error.safe_label());
                    }
                }
                WorkerResult::SourceDetail(outcome) => {
                    if confirmed_source_pending {
                        confirmed_source_pending = false;
                        match *outcome {
                            ClientOutcome::Ok(response) => {
                                confirmed_source_detail = Some(response.detail);
                            }
                            ClientOutcome::Problem(problem) => {
                                feedback = Some(problem.safe_label())
                            }
                            ClientOutcome::Transport(error) => {
                                feedback = Some(error.safe_label().to_owned())
                            }
                        }
                    } else {
                        let pending_index = source_detail_pending_index.take();
                        sources_view.apply_source_detail(*outcome);
                        if sources_view.last_error.is_none() {
                            active_source_index = pending_index;
                        }
                        if let Some(error) = &sources_view.last_error {
                            feedback = Some(error.safe_label());
                        }
                    }
                }
                WorkerResult::SelectionConfirmed(outcome) => {
                    source_confirmation_pending = false;
                    match *outcome {
                        ClientOutcome::Ok(workflow) => {
                            sources_view.load_candidates_from(&workflow);
                            compose_view.load_from_canonical(&workflow);
                            draft_editor = DraftEditor::new(&compose_view.text);
                            current_stage = Stage::Compose;
                            current_workflow = Some(workflow);
                            feedback = Some("Source set confirmed".to_owned());
                        }
                        ClientOutcome::Problem(problem) => feedback = Some(problem.safe_label()),
                        ClientOutcome::Transport(error) => {
                            feedback = Some(error.safe_label().to_owned())
                        }
                    }
                }
                WorkerResult::EmptySelectionConfirmed(outcome) => {
                    source_confirmation_pending = false;
                    match *outcome {
                        ClientOutcome::Ok(workflow) => {
                            sources_view.load_candidates_from(&workflow);
                            compose_view.load_from_canonical(&workflow);
                            draft_editor = DraftEditor::new(&compose_view.text);
                            current_stage = Stage::Compose;
                            manual_composition_pending = worker
                                .try_send(Intent::BeginManualComposition {
                                    workflow_id: workflow.id.clone(),
                                    revision: workflow.revision,
                                })
                                .is_ok();
                            current_workflow = Some(workflow);
                            feedback =
                                Some("Empty source set confirmed; opening manual draft".to_owned());
                        }
                        ClientOutcome::Problem(problem) => feedback = Some(problem.safe_label()),
                        ClientOutcome::Transport(error) => {
                            feedback = Some(error.safe_label().to_owned())
                        }
                    }
                }
                WorkerResult::PreferencesSaved(outcome) => match outcome {
                    ClientOutcome::Ok(value) => preferences = Some(value),
                    ClientOutcome::Problem(problem) => feedback = Some(problem.safe_label()),
                    ClientOutcome::Transport(error) => {
                        feedback = Some(error.safe_label().to_owned())
                    }
                },
                WorkerResult::WritingSettingsSet(outcome) => {
                    settings_pending = false;
                    match *outcome {
                        ClientOutcome::Ok(workflow) => {
                            let language = workflow.language.clone();
                            compose_view.load_from_canonical(&workflow);
                            current_workflow = Some(workflow);
                            let _ = worker.try_send(Intent::SavePreferences {
                                last_language: language,
                                last_assistant_backend_id: preferences
                                    .as_ref()
                                    .and_then(|value| value.last_assistant_backend_id.clone()),
                            });
                            feedback = Some("Writing settings saved".to_owned());
                        }
                        ClientOutcome::Problem(problem) => feedback = Some(problem.safe_label()),
                        ClientOutcome::Transport(error) => {
                            feedback = Some(error.safe_label().to_owned())
                        }
                    }
                }
                WorkerResult::ManualCompositionBegun(outcome) => {
                    manual_composition_pending = false;
                    match *outcome {
                        ClientOutcome::Ok(workflow) => {
                            compose_view.load_from_canonical(&workflow);
                            draft_editor = DraftEditor::new(&compose_view.text);
                            current_workflow = Some(workflow);
                            editor_active = true;
                            compose_active_resource = 0;
                            focus.zone = FocusZone::Main;
                            feedback =
                                Some("Manual Draft editor ready; Esc leaves the editor".to_owned());
                        }
                        ClientOutcome::Problem(problem) => feedback = Some(problem.safe_label()),
                        ClientOutcome::Transport(error) => {
                            feedback = Some(error.safe_label().to_owned())
                        }
                    }
                }
                WorkerResult::EntrySaved(outcome) => {
                    let generation = save_in_flight.take();
                    if let Some(generation) = generation {
                        let failed = !matches!(&*outcome, ClientOutcome::Ok(_));
                        if let Some(workflow) =
                            compose_view.apply_save_outcome(generation, *outcome)
                        {
                            save_blocked_for_resolution = false;
                            current_workflow = Some(workflow);
                            feedback = Some(if compose_view.is_dirty() {
                                "Draft saved; newer edits remain pending".to_owned()
                            } else {
                                "Draft saved".to_owned()
                            });
                        } else {
                            save_blocked_for_resolution = true;
                            feedback = compose_view
                                .last_error
                                .as_ref()
                                .map(|error| error.safe_label());
                            if failed && let Some(workflow) = &current_workflow {
                                recovery_workflow = Some((workflow.id.clone(), false));
                            }
                        }
                    }
                }
                WorkerResult::EntryConfirmed(outcome) => {
                    entry_confirmation_pending = false;
                    match *outcome {
                        ClientOutcome::Ok(workflow) => {
                            review_view.last_error = None;
                            compose_view.load_from_canonical(&workflow);
                            current_workflow = Some(workflow);
                            feedback = Some("Complete entry confirmed".to_owned());
                        }
                        ClientOutcome::Problem(problem) => {
                            feedback = Some(problem.safe_label());
                            review_view.last_error =
                                Some(diary_tui::review_view::ReviewError::Problem(problem));
                        }
                        ClientOutcome::Transport(error) => {
                            feedback = Some(error.safe_label().to_owned());
                            review_view.last_error =
                                Some(diary_tui::review_view::ReviewError::TransportFailure(error));
                        }
                    }
                }
            }
        }

        if recovery_list_required && !refresh_pending {
            refresh_pending = worker.try_send(Intent::RefreshWorkflows).is_ok();
        }
        if let Some((workflow_id, in_flight)) = &mut recovery_workflow
            && !*in_flight
            && worker
                .try_send(Intent::GetWorkflow {
                    workflow_id: workflow_id.clone(),
                })
                .is_ok()
        {
            *in_flight = true;
        }
        if let Some((job_id, in_flight)) = &mut recovery_job
            && !*in_flight
            && !source_job_poll_in_flight
            && worker
                .try_send(Intent::PollJob {
                    job_id: job_id.clone(),
                })
                .is_ok()
        {
            *in_flight = true;
        }
        if !source_job_poll_in_flight
            && !recovery_job
                .as_ref()
                .is_some_and(|(_, in_flight)| *in_flight)
            && let Some(job_id) = sources_view
                .pending_discovery_job
                .as_ref()
                .or(sources_view.pending_preparation_job.as_ref())
            && worker
                .try_send(Intent::PollJob {
                    job_id: job_id.clone(),
                })
                .is_ok()
        {
            source_job_poll_in_flight = true;
        }

        status_line.ime = ime_tracker.tick(std::time::Instant::now());
        status_line.diary_day = current_workflow
            .as_ref()
            .map(|workflow| workflow.diary_day.clone())
            .or_else(|| selected_day.clone());

        let stage_model = map_workflow(current_workflow.as_ref(), false);
        let can_mutate = authority.mutations_enabled()
            && capabilities.as_ref().is_some_and(|value| {
                value
                    .capabilities
                    .iter()
                    .any(|capability| capability == "workflow:write")
            });
        let can_start = can_mutate
            && capabilities
                .as_ref()
                .is_some_and(|value| value.manual_workflow_available);
        let can_read_sources = capabilities.as_ref().is_some_and(|value| {
            value
                .capabilities
                .iter()
                .any(|capability| capability == "source:read")
        });
        let source_job_busy = pending_source_job_kind.is_some()
            || sources_view.pending_discovery_job.is_some()
            || sources_view.pending_preparation_job.is_some();
        let autosave_due =
            last_edit_at.is_some_and(|edited| edited.elapsed() >= Duration::from_secs(1));
        if can_mutate
            && save_in_flight.is_none()
            && !save_blocked_for_resolution
            && (save_requested || autosave_due)
            && let (Some(workflow), Some(snapshot)) =
                (current_workflow.as_ref(), compose_view.save_snapshot())
            && worker
                .try_send(Intent::SaveEntry {
                    workflow_id: workflow.id.clone(),
                    buffer_version: snapshot.buffer_version,
                    text: snapshot.text,
                })
                .is_ok()
        {
            save_in_flight = Some(snapshot.generation);
            save_requested = false;
            last_edit_at = None;
        }
        if stage_model.progress_of(current_stage) == Progress::Locked {
            current_stage = stage_model.default_current;
        }
        let compose_resources =
            compose_resources(current_workflow.as_ref(), compose_view.is_dirty());
        let visible_resource_count = match current_stage {
            Stage::Start => start_view.workflows.len(),
            Stage::Sources => sources_view.candidates.len(),
            Stage::Compose => compose_resources.rows.len(),
            Stage::Review => 1,
            _ => 0,
        };
        let visible_control_count = match current_stage {
            Stage::Sources => SOURCES_CONTROL_COUNT,
            Stage::Compose => COMPOSE_CONTROL_COUNT,
            Stage::Review => REVIEW_CONTROL_COUNT,
            _ => START_CONTROL_COUNT,
        };
        focus.clamp(
            ALL_STAGES.len(),
            visible_resource_count,
            visible_control_count,
        );

        let mut displayed_status = work_status.clone();
        displayed_status.connection_and_save_safety = Some(if save_in_flight.is_some() {
            "Saving Draft...".to_owned()
        } else if compose_view.is_dirty() {
            "Draft has unsaved local edits".to_owned()
        } else if authority.mutations_enabled() {
            "Connected; Draft saved".to_owned()
        } else {
            "Reloading canonical state; actions disabled".to_owned()
        });
        displayed_status.notice = Some(if command_line_active {
            DEFAULT_NOTICE.to_owned()
        } else if day_editing {
            format!("Day: {day_draft}")
        } else if source_date_editing {
            format!("Extra source date: {source_date_draft}")
        } else if empty_selection_confirmation {
            "Continue without notes? Enter confirms; Esc cancels".to_owned()
        } else if quit_confirmation {
            "Draft is not safely saved. Ctrl-D again to quit; Esc cancels".to_owned()
        } else if let Some(index) = discard_confirmation {
            format!(
                "Discard {}? Enter confirms; Esc cancels",
                start_view
                    .workflows
                    .get(index)
                    .map_or("workflow", |workflow| workflow.diary_day.as_str())
            )
        } else {
            feedback
                .clone()
                .unwrap_or_else(|| DEFAULT_NOTICE.to_owned())
        });

        let visible_resources = match current_stage {
            Stage::Start => start_view.resources(),
            Stage::Sources => sources_view.resources(),
            Stage::Compose => compose_resources,
            Stage::Review => ResourcesSidebarModel {
                rows: vec![ResourceRow {
                    label: "Complete entry".to_owned(),
                    state: if current_workflow
                        .as_ref()
                        .is_some_and(|value| value.entry_confirmed)
                    {
                        "confirmed".to_owned()
                    } else {
                        "review".to_owned()
                    },
                    indent: 0,
                }],
            },
            _ => ResourcesSidebarModel::default(),
        };
        let main_body = match current_stage {
            Stage::Start => start_screen_body(
                &focus,
                selected_day.as_deref(),
                day_editing,
                &day_draft,
                start_view.workflows.get(focus.resource_index),
                start_view.next_cursor.is_some(),
                capabilities.as_ref(),
                setup_check.as_ref(),
                authority.mutations_enabled(),
                start_pending,
                more_pending,
                discard_pending.is_some(),
                help_open,
            ),
            Stage::Sources => sources_screen_body(
                &focus,
                &sources_view,
                explicit_source_day.as_deref(),
                source_date_editing,
                &source_date_draft,
                source_page_pending,
                source_detail_pending_index.is_some(),
                source_confirmation_pending,
                source_job_busy,
            ),
            Stage::Compose => compose_screen_body(
                &focus,
                current_workflow.as_ref(),
                &compose_view,
                confirmed_source_detail.as_ref(),
                settings_pending,
                manual_composition_pending,
                save_in_flight.is_some(),
            ),
            Stage::Review => review_screen_body(
                &focus,
                current_workflow.as_ref(),
                &compose_view,
                entry_confirmation_pending,
            ),
            _ => format!(
                "{} is not part of this increment.",
                diary_tui::sidebar::stage_label(current_stage)
            ),
        };
        let drawn = guard.terminal_mut().draw(|frame| {
            let area = frame.area();
            if !diary_tui::layout::is_supported_size(area.width, area.height) {
                frame.render_widget(diary_tui::layout::TerminalTooSmallWidget, area);
                return;
            }
            let areas = adaptive_areas(area, focus.zone);
            WorkStatusWidget::new(&displayed_status)
                .palette(palette)
                .render_in(frame, areas.work_status);
            let workflow_widget = || {
                WorkflowSidebarWidget::new(&stage_model, current_stage)
                    .focused(focus.zone == FocusZone::Workflow)
                    .focused_stage(
                        (focus.zone == FocusZone::Workflow)
                            .then_some(ALL_STAGES[focus.workflow_index]),
                    )
                    .palette(palette)
            };
            let resources_widget = || {
                ResourcesSidebarWidget::new(&visible_resources)
                    .active(match current_stage {
                        Stage::Start => Some(focus.resource_index),
                        Stage::Sources => active_source_index,
                        Stage::Compose => Some(compose_active_resource),
                        Stage::Review => Some(0),
                        _ => None,
                    })
                    .focused_row(
                        (focus.zone == FocusZone::Resources).then_some(focus.resource_index),
                    )
                    .focused(focus.zone == FocusZone::Resources)
                    .palette(palette)
            };
            let main_height = areas.main_pane.height.saturating_sub(2);
            main_scroll = ensure_selected_line_visible(&main_body, main_scroll, main_height);
            if current_stage == Stage::Compose && editor_active && compose_active_resource == 0 {
                frame.render_widget(
                    DraftEditorPane::new(&draft_editor, focus.zone == FocusZone::Main)
                        .palette(palette),
                    areas.main_pane,
                );
            } else {
                frame.render_widget(
                    MainPaneWidget::new("Main", &main_body)
                        .focused(focus.zone == FocusZone::Main)
                        .scroll(main_scroll)
                        .palette(palette),
                    areas.main_pane,
                );
            }
            if let Some(workflow_area) = areas.workflow_sidebar {
                frame.render_widget(workflow_widget(), workflow_area);
            }
            if let Some(resources_area) = areas.resources_sidebar {
                frame.render_widget(resources_widget(), resources_area);
            }
            if let Some(overlay) = areas.overlay {
                frame.render_widget(OverlayClearWidget, overlay.area);
                match overlay.kind {
                    SidebarKind::Workflow => {
                        frame.render_widget(workflow_widget(), overlay.area);
                    }
                    SidebarKind::Resources => {
                        frame.render_widget(resources_widget(), overlay.area);
                    }
                }
            }
            frame.render_widget(
                CommandLineWidget::new(
                    command_line_active,
                    command_line.raw(),
                    command_line_feedback.as_deref(),
                )
                .palette(palette),
                areas.command_line,
            );
            frame.render_widget(
                StatusLineWidget::new(&status_line).palette(palette),
                areas.status_line,
            );
        });
        if drawn.is_err() {
            return SessionEnd::RenderFailed;
        }

        use diary_tui::command_line::Command;
        use diary_tui::terminal::{InputEvent, InputPoll};
        match diary_tui::terminal::poll_input_event(TICK_INTERVAL) {
            InputPoll::Quit => {
                if !compose_view.is_dirty() && save_in_flight.is_none() || quit_confirmation {
                    return SessionEnd::Quit;
                }
                save_requested = true;
                quit_confirmation = true;
                feedback =
                    Some("Saving before quit; Ctrl-D again abandons unsaved edits".to_owned());
            }
            InputPoll::TimedOut | InputPoll::Ignored => {}
            InputPoll::Ctrl('g') if !command_line_active => {
                if editor_active && compose_view.is_dirty() {
                    save_requested = true;
                }
                command_line_active = true;
                command_line.clear();
                command_line_feedback = None;
                feedback = None;
            }
            InputPoll::Ctrl('p') if !command_line_active && !day_editing => {
                if current_stage == Stage::Compose && compose_view.is_dirty() {
                    save_requested = true;
                }
                editor_active = false;
                current_stage = adjacent_accessible_stage(&stage_model, current_stage, false);
            }
            InputPoll::Ctrl('n') if !command_line_active && !day_editing => {
                if current_stage == Stage::Compose && compose_view.is_dirty() {
                    save_requested = true;
                }
                editor_active = false;
                current_stage = adjacent_accessible_stage(&stage_model, current_stage, true);
            }
            InputPoll::Ctrl('s') if !command_line_active => {
                if current_stage == Stage::Compose && compose_view.is_dirty() {
                    save_requested = true;
                    feedback = Some("Draft save requested".to_owned());
                } else {
                    feedback = Some("Draft is already saved".to_owned());
                }
            }
            InputPoll::Ctrl(_) => {}
            InputPoll::Event(event) if command_line_active => match event {
                InputEvent::Char(ch) => command_line.push_char(ch),
                InputEvent::Backspace => command_line.backspace(),
                InputEvent::Esc => {
                    command_line_active = false;
                    command_line.clear();
                    command_line_feedback = Some("Cancelled".to_owned());
                }
                InputEvent::Paste(data) => {
                    for ch in data.chars() {
                        command_line.push_char(ch);
                    }
                }
                InputEvent::Enter => {
                    command_line_active = false;
                    let message = match command_line.submit() {
                        Ok(Command::Quit) => {
                            if (compose_view.is_dirty() || save_in_flight.is_some())
                                && !quit_confirmation
                            {
                                save_requested = true;
                                quit_confirmation = true;
                                "Draft is not safely saved; use quit again to abandon it".to_owned()
                            } else {
                                return SessionEnd::Quit;
                            }
                        }
                        Ok(Command::Help) => {
                            help_open = true;
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
                        Ok(Command::Save) => {
                            save_requested = true;
                            "Draft save requested".to_owned()
                        }
                        Ok(Command::Day(day)) => {
                            let message = format!("Day set to {day}");
                            selected_day = Some(day);
                            message
                        }
                        Ok(Command::Stage(stage)) => {
                            if stage_model.progress_of(stage) != Progress::Locked {
                                if current_stage == Stage::Compose
                                    && stage != Stage::Compose
                                    && compose_view.is_dirty()
                                {
                                    save_requested = true;
                                    editor_active = false;
                                }
                                current_stage = stage;
                                format!("Opened {}", diary_tui::sidebar::stage_label(stage))
                            } else {
                                "That stage is locked".to_owned()
                            }
                        }
                        Err(_) => "Unknown or malformed command".to_owned(),
                    };
                    command_line_feedback = Some(message.clone());
                    // Command input and parser/action feedback belong to the
                    // dedicated Command Line row, never a Work Status row.
                    feedback = None;
                }
                InputEvent::Tab
                | InputEvent::BackTab
                | InputEvent::Up
                | InputEvent::Down
                | InputEvent::Left
                | InputEvent::Right
                | InputEvent::Home
                | InputEvent::End
                | InputEvent::PageUp
                | InputEvent::PageDown
                | InputEvent::Delete => {}
            },
            InputPoll::Event(event) if day_editing => match event {
                InputEvent::Char(ch)
                    if (ch.is_ascii_digit() || ch == '-') && day_draft.len() < 10 =>
                {
                    day_draft.push(ch);
                }
                InputEvent::Backspace => {
                    day_draft.pop();
                }
                InputEvent::Enter => {
                    if day_draft.is_empty() {
                        selected_day = None;
                        day_editing = false;
                        feedback = Some("Day uses Mac-local today".to_owned());
                    } else if diary_tui::command_line::is_valid_day(&day_draft) {
                        selected_day = Some(day_draft.clone());
                        day_editing = false;
                        feedback = Some(format!("Day set to {day_draft}"));
                    } else {
                        feedback = Some("Day must be YYYY-MM-DD".to_owned());
                    }
                }
                InputEvent::Esc => {
                    day_editing = false;
                    day_draft = selected_day.clone().unwrap_or_default();
                }
                _ => {}
            },
            InputPoll::Event(event) if source_date_editing => match event {
                InputEvent::Char(ch)
                    if (ch.is_ascii_digit() || ch == '-') && source_date_draft.len() < 10 =>
                {
                    source_date_draft.push(ch);
                }
                InputEvent::Backspace => {
                    source_date_draft.pop();
                }
                InputEvent::Paste(data) => {
                    let candidate = format!("{source_date_draft}{data}");
                    if candidate.len() <= 10
                        && candidate.chars().all(|ch| ch.is_ascii_digit() || ch == '-')
                    {
                        source_date_draft = candidate;
                    }
                }
                InputEvent::Enter => {
                    if diary_tui::command_line::is_valid_day(&source_date_draft) {
                        explicit_source_day = Some(source_date_draft.clone());
                        source_date_editing = false;
                        feedback = Some(format!("Extra source date set to {source_date_draft}"));
                    } else {
                        feedback = Some("Extra source date must be YYYY-MM-DD".to_owned());
                    }
                }
                InputEvent::Esc => {
                    source_date_editing = false;
                    source_date_draft = explicit_source_day.clone().unwrap_or_default();
                }
                _ => {}
            },
            InputPoll::Event(InputEvent::Enter) if empty_selection_confirmation => {
                empty_selection_confirmation = false;
                if can_mutate
                    && !source_confirmation_pending
                    && let Some(workflow) = &current_workflow
                {
                    source_confirmation_pending = worker
                        .try_send(Intent::ConfirmEmptySelection {
                            workflow_id: workflow.id.clone(),
                            revision: workflow.revision,
                        })
                        .is_ok();
                }
            }
            InputPoll::Event(InputEvent::Esc) if empty_selection_confirmation => {
                empty_selection_confirmation = false;
                feedback = Some("Continue without notes cancelled".to_owned());
            }
            InputPoll::Event(InputEvent::Enter) if discard_confirmation.is_some() => {
                let index = discard_confirmation.take().unwrap_or_default();
                if can_mutate
                    && discard_pending.is_none()
                    && let Some(workflow) = start_view.workflows.get(index)
                    && worker
                        .try_send(Intent::DiscardWorkflow {
                            id: workflow.id.clone(),
                            revision: workflow.revision,
                        })
                        .is_ok()
                {
                    discard_pending = Some(index);
                    feedback = Some("Discarding workflow...".to_owned());
                }
            }
            InputPoll::Event(InputEvent::Esc) if discard_confirmation.is_some() => {
                discard_confirmation = None;
                feedback = Some("Discard cancelled".to_owned());
            }
            InputPoll::Event(InputEvent::Esc) if quit_confirmation => {
                quit_confirmation = false;
                feedback = Some("Quit cancelled".to_owned());
            }
            InputPoll::Event(InputEvent::Esc)
                if editor_active && current_stage == Stage::Compose =>
            {
                editor_active = false;
                save_requested = compose_view.is_dirty();
                feedback = Some("Left Draft editor; pending edits will be flushed".to_owned());
            }
            InputPoll::Event(event) if editor_active && current_stage == Stage::Compose => {
                let changed = draft_editor.handle_input(&event);
                if changed {
                    compose_view.edit(draft_editor.text());
                    last_edit_at = Some(Instant::now());
                    quit_confirmation = false;
                }
            }
            InputPoll::Event(InputEvent::PageUp) if focus.zone == FocusZone::Main => {
                main_scroll = main_scroll.saturating_sub(10);
            }
            InputPoll::Event(InputEvent::PageDown) if focus.zone == FocusZone::Main => {
                main_scroll = main_scroll.saturating_add(10);
            }
            InputPoll::Event(InputEvent::Tab) => focus.next_zone(),
            InputPoll::Event(InputEvent::BackTab) => focus.previous_zone(),
            InputPoll::Event(InputEvent::Left) => focus.previous_zone(),
            InputPoll::Event(InputEvent::Right) => focus.next_zone(),
            InputPoll::Event(InputEvent::Up) => {
                focus.move_previous(focus_item_count(
                    &focus,
                    visible_resource_count,
                    visible_control_count,
                ));
            }
            InputPoll::Event(InputEvent::Down) => {
                focus.move_next(focus_item_count(
                    &focus,
                    visible_resource_count,
                    visible_control_count,
                ));
            }
            InputPoll::Event(InputEvent::Enter) => match focus.zone {
                FocusZone::Workflow => {
                    let stage = ALL_STAGES[focus.workflow_index];
                    if stage_model.progress_of(stage) != Progress::Locked {
                        if current_stage == Stage::Compose
                            && stage != Stage::Compose
                            && compose_view.is_dirty()
                        {
                            save_requested = true;
                            editor_active = false;
                        }
                        current_stage = stage;
                    } else {
                        feedback = Some("That stage is locked".to_owned());
                    }
                }
                FocusZone::Resources if current_stage == Stage::Start => {
                    if can_start
                        && !start_pending
                        && let Some(workflow) = start_view.workflows.get(focus.resource_index)
                    {
                        start_pending = worker
                            .try_send(Intent::StartOrResumeWorkflow {
                                diary_day: Some(workflow.diary_day.clone()),
                            })
                            .is_ok();
                    }
                }
                FocusZone::Resources if current_stage == Stage::Sources => {
                    if !source_job_busy
                        && source_detail_pending_index.is_none()
                        && let (Some(workflow), Some(candidate), Some(handle)) = (
                            current_workflow.as_ref(),
                            sources_view.candidates.get(focus.resource_index),
                            sources_view.latest_preparation_handle.as_ref(),
                        )
                        && worker
                            .try_send(Intent::SourceDetail {
                                workflow_id: workflow.id.clone(),
                                source_id: candidate.id.clone(),
                                preparation_handle: handle.clone(),
                            })
                            .is_ok()
                    {
                        source_detail_pending_index = Some(focus.resource_index);
                    } else if sources_view.latest_preparation_handle.is_none() {
                        feedback =
                            Some("Prepare included sources before inspecting text".to_owned());
                    }
                }
                FocusZone::Resources if current_stage == Stage::Compose => {
                    compose_active_resource = focus.resource_index;
                    if focus.resource_index == 0 {
                        confirmed_source_detail = None;
                        if current_workflow
                            .as_ref()
                            .is_some_and(|workflow| workflow.state == "composing")
                        {
                            editor_active = true;
                            focus.zone = FocusZone::Main;
                        } else if can_mutate
                            && !manual_composition_pending
                            && let Some(workflow) = &current_workflow
                        {
                            manual_composition_pending = worker
                                .try_send(Intent::BeginManualComposition {
                                    workflow_id: workflow.id.clone(),
                                    revision: workflow.revision,
                                })
                                .is_ok();
                        }
                    } else if !confirmed_source_pending
                        && let (Some(workflow), Some(candidate)) = (
                            current_workflow.as_ref(),
                            confirmed_candidate_at(current_workflow.as_ref(), focus.resource_index),
                        )
                        && worker
                            .try_send(Intent::ConfirmedSourceDetail {
                                workflow_id: workflow.id.clone(),
                                source_id: candidate.id.clone(),
                            })
                            .is_ok()
                    {
                        confirmed_source_pending = true;
                        editor_active = false;
                        save_requested = compose_view.is_dirty();
                    }
                }
                FocusZone::Main if current_stage == Stage::Start => {
                    match focus.main_control_index {
                        0 => {
                            day_draft = selected_day.clone().unwrap_or_default();
                            day_editing = true;
                        }
                        1 if can_start && !start_pending => {
                            start_pending = worker
                                .try_send(Intent::StartOrResumeWorkflow {
                                    diary_day: selected_day.clone(),
                                })
                                .is_ok();
                        }
                        2 if can_start && !start_pending => {
                            if let Some(workflow) = start_view.workflows.get(focus.resource_index) {
                                start_pending = worker
                                    .try_send(Intent::StartOrResumeWorkflow {
                                        diary_day: Some(workflow.diary_day.clone()),
                                    })
                                    .is_ok();
                            }
                        }
                        3 if can_mutate && discard_pending.is_none() => {
                            if start_view.workflows.get(focus.resource_index).is_some() {
                                discard_confirmation = Some(focus.resource_index);
                            }
                        }
                        4 if !more_pending => {
                            if let Some(cursor) = start_view.next_cursor.clone() {
                                more_pending = worker
                                    .try_send(Intent::LoadMoreWorkflows { cursor })
                                    .is_ok();
                            }
                        }
                        5 if !refresh_pending => {
                            refresh_pending = worker.try_send(Intent::RefreshWorkflows).is_ok();
                        }
                        6 => help_open = !help_open,
                        7 => feedback = Some("Nothing to save at Start".to_owned()),
                        8 => {
                            if (compose_view.is_dirty() || save_in_flight.is_some())
                                && !quit_confirmation
                            {
                                save_requested = true;
                                quit_confirmation = true;
                                feedback = Some(
                                    "Saving before quit; activate Quit again to abandon unsaved edits"
                                        .to_owned(),
                                );
                            } else {
                                return SessionEnd::Quit;
                            }
                        }
                        _ => feedback = Some("That action is currently unavailable".to_owned()),
                    }
                }
                FocusZone::Main if current_stage == Stage::Sources => {
                    let no_source_job = !source_job_busy;
                    match focus.main_control_index {
                        0 if can_mutate && can_read_sources && no_source_job => {
                            if let Some(workflow) = &current_workflow {
                                pending_source_job_kind = Some(false);
                                if worker
                                    .try_send(Intent::Discover {
                                        workflow_id: workflow.id.clone(),
                                        revision: workflow.revision,
                                        explicit_date: None,
                                    })
                                    .is_err()
                                {
                                    pending_source_job_kind = None;
                                }
                            }
                        }
                        1 => {
                            source_date_draft = explicit_source_day.clone().unwrap_or_default();
                            source_date_editing = true;
                        }
                        2 if can_mutate
                            && can_read_sources
                            && explicit_source_day.is_some()
                            && no_source_job =>
                        {
                            if let Some(workflow) = &current_workflow {
                                pending_source_job_kind = Some(false);
                                if worker
                                    .try_send(Intent::Discover {
                                        workflow_id: workflow.id.clone(),
                                        revision: workflow.revision,
                                        explicit_date: explicit_source_day.clone(),
                                    })
                                    .is_err()
                                {
                                    pending_source_job_kind = None;
                                }
                            }
                        }
                        3 if can_mutate && no_source_job => {
                            if let Some(candidate) =
                                sources_view.candidates.get(focus.resource_index)
                            {
                                let source_id = candidate.id.clone();
                                let _ = sources_view.toggle_included(&source_id);
                                active_source_index = None;
                            }
                        }
                        4 if can_mutate
                            && can_read_sources
                            && !sources_view.included.is_empty()
                            && no_source_job =>
                        {
                            if let Some(workflow) = &current_workflow {
                                sources_view.invalidate_preparation();
                                active_source_index = None;
                                pending_source_job_kind = Some(true);
                                if worker
                                    .try_send(Intent::PrepareSources {
                                        workflow_id: workflow.id.clone(),
                                        revision: workflow.revision,
                                        source_ids: sources_view.included.iter().cloned().collect(),
                                    })
                                    .is_err()
                                {
                                    pending_source_job_kind = None;
                                }
                            }
                        }
                        5 if no_source_job && source_detail_pending_index.is_none() => {
                            if let (Some(workflow), Some(candidate), Some(handle)) = (
                                current_workflow.as_ref(),
                                sources_view.candidates.get(focus.resource_index),
                                sources_view.latest_preparation_handle.as_ref(),
                            ) && worker
                                .try_send(Intent::SourceDetail {
                                    workflow_id: workflow.id.clone(),
                                    source_id: candidate.id.clone(),
                                    preparation_handle: handle.clone(),
                                })
                                .is_ok()
                            {
                                source_detail_pending_index = Some(focus.resource_index);
                            }
                        }
                        6 if can_mutate
                            && no_source_job
                            && sources_view.all_included_reviewed()
                            && !source_confirmation_pending =>
                        {
                            if let (Some(workflow), Some(handle)) = (
                                current_workflow.as_ref(),
                                sources_view.latest_preparation_handle.as_ref(),
                            ) {
                                source_confirmation_pending = worker
                                    .try_send(Intent::ConfirmSelection {
                                        workflow_id: workflow.id.clone(),
                                        revision: workflow.revision,
                                        preparation_handle: handle.clone(),
                                        source_ids: sources_view.included.iter().cloned().collect(),
                                    })
                                    .is_ok();
                            }
                        }
                        7 if can_mutate
                            && no_source_job
                            && sources_view.included.is_empty()
                            && !source_confirmation_pending =>
                        {
                            empty_selection_confirmation = true;
                        }
                        8 if !source_page_pending => {
                            if let (Some(workflow), Some(cursor)) =
                                (current_workflow.as_ref(), sources_view.next_cursor.clone())
                            {
                                source_page_append = true;
                                source_page_pending = worker
                                    .try_send(Intent::ListSources {
                                        workflow_id: workflow.id.clone(),
                                        cursor: Some(cursor),
                                    })
                                    .is_ok();
                            }
                        }
                        9 => current_stage = Stage::Start,
                        _ => {
                            feedback =
                                Some("That source action is currently unavailable".to_owned());
                        }
                    }
                }
                FocusZone::Main if current_stage == Stage::Compose => {
                    match focus.main_control_index {
                        0..=4 if can_mutate && !settings_pending => {
                            if let Some(workflow) = &current_workflow {
                                let language = match focus.main_control_index {
                                    0 => "en-US",
                                    1 => "ko-KR",
                                    _ => workflow.language.as_deref().unwrap_or("en-US"),
                                };
                                let style = match focus.main_control_index {
                                    2 => "concise",
                                    3 => "standard",
                                    4 => "reflective",
                                    _ => workflow.style.as_str(),
                                };
                                settings_pending = worker
                                    .try_send(Intent::SetWritingSettings {
                                        workflow_id: workflow.id.clone(),
                                        revision: workflow.revision,
                                        language: language.to_owned(),
                                        style: style.to_owned(),
                                    })
                                    .is_ok();
                            }
                        }
                        5 if can_mutate && !manual_composition_pending => {
                            if let Some(workflow) = &current_workflow {
                                if workflow.state == "composing" {
                                    editor_active = true;
                                    compose_active_resource = 0;
                                } else {
                                    manual_composition_pending = worker
                                        .try_send(Intent::BeginManualComposition {
                                            workflow_id: workflow.id.clone(),
                                            revision: workflow.revision,
                                        })
                                        .is_ok();
                                }
                            }
                        }
                        6 => save_requested = true,
                        7 if compose_view.conflicting_server_text.is_some() => {
                            compose_view.keep_local_after_conflict();
                            save_blocked_for_resolution = false;
                            save_requested = true;
                            feedback =
                                Some("Keeping local Draft; conditional save requested".to_owned());
                        }
                        8 if compose_view.conflicting_server_text.is_some() => {
                            compose_view.use_server_after_conflict();
                            save_blocked_for_resolution = false;
                            draft_editor = DraftEditor::new(&compose_view.text);
                            feedback = Some("Using canonical server Draft".to_owned());
                        }
                        9 => {
                            save_requested = compose_view.is_dirty();
                            editor_active = false;
                            current_stage = Stage::Review;
                        }
                        _ => feedback = Some("That Compose action is unavailable".to_owned()),
                    }
                }
                FocusZone::Main if current_stage == Stage::Review => {
                    match focus.main_control_index {
                        0 if can_mutate
                            && can_confirm(&compose_view.text)
                            && !compose_view.is_dirty()
                            && save_in_flight.is_none()
                            && !entry_confirmation_pending =>
                        {
                            if let Some(workflow) = &current_workflow {
                                entry_confirmation_pending = worker
                                    .try_send(Intent::ConfirmEntry {
                                        workflow_id: workflow.id.clone(),
                                        revision: workflow.revision,
                                        buffer_version: compose_view.buffer_version,
                                    })
                                    .is_ok();
                            }
                        }
                        1 => {
                            current_stage = Stage::Compose;
                            compose_active_resource = 0;
                            editor_active = true;
                            focus.zone = FocusZone::Main;
                        }
                        2 => {
                            feedback =
                                Some(first_unresolved_delimiter(&compose_view.text).map_or_else(
                                    || "No unresolved reflection delimiter".to_owned(),
                                    |delimiter| {
                                        format!("Resolve the first {delimiter} marker in Compose")
                                    },
                                ));
                        }
                        3 => save_requested = true,
                        _ => feedback = Some("Entry cannot be confirmed yet".to_owned()),
                    }
                }
                _ => {}
            },
            InputPoll::Event(_) => {}
        }
    }
}

const START_CONTROL_COUNT: usize = 9;
const SOURCES_CONTROL_COUNT: usize = 10;
const COMPOSE_CONTROL_COUNT: usize = 10;
const REVIEW_CONTROL_COUNT: usize = 4;

fn compose_resources(workflow: Option<&WorkflowDto>, dirty: bool) -> ResourcesSidebarModel {
    let mut rows = vec![ResourceRow {
        label: "Draft".to_owned(),
        state: if dirty { "unsaved" } else { "saved" }.to_owned(),
        indent: 0,
    }];
    if let Some(workflow) = workflow {
        rows.extend(
            workflow
                .candidates
                .iter()
                .filter(|candidate| candidate.selected)
                .map(|candidate| ResourceRow {
                    label: candidate.source_note_name.clone(),
                    state: "confirmed source".to_owned(),
                    indent: 0,
                }),
        );
    }
    ResourcesSidebarModel { rows }
}

fn confirmed_candidate_at(
    workflow: Option<&WorkflowDto>,
    resource_index: usize,
) -> Option<&diary_control_contract::SourceCandidateDto> {
    resource_index.checked_sub(1).and_then(|source_index| {
        workflow?
            .candidates
            .iter()
            .filter(|candidate| candidate.selected)
            .nth(source_index)
    })
}

#[allow(clippy::too_many_arguments)]
fn compose_screen_body(
    focus: &FocusModel,
    workflow: Option<&WorkflowDto>,
    compose: &ComposeViewModel,
    source: Option<&SourceDetailDto>,
    settings_pending: bool,
    manual_pending: bool,
    save_pending: bool,
) -> String {
    if let Some(source) = source {
        return format!(
            "Read-only confirmed source: {}\n\n{}\n\nSelect Draft in Resources to return; its cursor and buffer are preserved.",
            source.source_note_name, source.body
        );
    }
    let marker = |index| {
        if focus.zone == FocusZone::Main && focus.main_control_index == index {
            ">"
        } else {
            " "
        }
    };
    let language = workflow
        .and_then(|value| value.language.as_deref())
        .unwrap_or("not selected");
    let style = workflow.map_or("concise", |value| value.style.as_str());
    let state = workflow.map_or("unavailable", |value| value.state.as_str());
    let save = if save_pending {
        "saving"
    } else if compose.is_dirty() {
        "unsaved"
    } else {
        "saved"
    };
    let mut body = format!(
        "Workflow: {state}\nWriting language: {language}\nStyle: {style}\nDraft: {save}\n\n{} Language: US English\n{} Language: South Korean Korean\n{} Style: concise\n{} Style: standard\n{} Style: reflective\n{} Open manual Draft editor{}\n{} Save/flush Draft\n{} Keep local Draft after conflict\n{} Use server Draft after conflict\n{} Review complete entry",
        marker(0),
        marker(1),
        marker(2),
        marker(3),
        marker(4),
        marker(5),
        if manual_pending { " (working)" } else { "" },
        marker(6),
        marker(7),
        marker(8),
        marker(9),
    );
    if settings_pending {
        body.push_str("\n\nWriting settings are being saved.");
    }
    if let Some(server_text) = &compose.conflicting_server_text {
        body.push_str("\n\n--- Local Draft (preserved) ---\n");
        body.push_str(&compose.text);
        body.push_str("\n\n--- Server Draft (canonical) ---\n");
        body.push_str(server_text);
        body.push_str("\n\nChoose Keep local or Use server; no merge or replay is automatic.");
    } else {
        body.push_str("\n\nEnter opens the selected control. Once the editor opens, Esc leaves it and flushes pending edits.");
    }
    body
}

fn review_screen_body(
    focus: &FocusModel,
    workflow: Option<&WorkflowDto>,
    compose: &ComposeViewModel,
    confirmation_pending: bool,
) -> String {
    let marker = |index| {
        if focus.zone == FocusZone::Main && focus.main_control_index == index {
            ">"
        } else {
            " "
        }
    };
    let language = workflow
        .and_then(|value| value.language.as_deref())
        .unwrap_or("not selected");
    let guard = if compose.text.trim().is_empty() {
        "blank entries cannot be confirmed"
    } else if first_unresolved_delimiter(&compose.text).is_some() {
        "resolve inferred-reflection delimiters before confirming"
    } else if compose.is_dirty() {
        "save the Draft before confirming"
    } else {
        "ready"
    };
    format!(
        "Complete entry · {language}\nConfirmation: {guard}\n\n{} Confirm complete entry{}\n{} Back to Draft editor\n{} Find first unresolved reflection marker\n{} Save/flush Draft\n\n--- Full edited entry ---\n{}",
        marker(0),
        if confirmation_pending {
            " (working)"
        } else {
            ""
        },
        marker(1),
        marker(2),
        marker(3),
        compose.text,
    )
}

fn focus_item_count(focus: &FocusModel, resource_count: usize, control_count: usize) -> usize {
    match focus.zone {
        FocusZone::Workflow => ALL_STAGES.len(),
        FocusZone::Resources => resource_count,
        FocusZone::Main => control_count,
    }
}

fn ensure_selected_line_visible(body: &str, current: u16, viewport_height: u16) -> u16 {
    if viewport_height == 0 {
        return 0;
    }
    let selected = body
        .lines()
        .position(|line| line.starts_with('>'))
        .and_then(|index| u16::try_from(index).ok());
    match selected {
        Some(line) if line < current => line,
        Some(line) if line >= current.saturating_add(viewport_height) => {
            line.saturating_sub(viewport_height - 1)
        }
        _ => current,
    }
}

#[allow(clippy::too_many_arguments)]
fn start_screen_body(
    focus: &FocusModel,
    selected_day: Option<&str>,
    day_editing: bool,
    day_draft: &str,
    selected_workflow: Option<&diary_control_contract::WorkflowSummaryDto>,
    has_more: bool,
    capabilities: Option<&CapabilitiesDto>,
    setup: Option<&SetupCheckDto>,
    mutations_enabled: bool,
    start_pending: bool,
    more_pending: bool,
    discard_pending: bool,
    help_open: bool,
) -> String {
    let marker = |index| {
        if focus.zone == FocusZone::Main && focus.main_control_index == index {
            ">"
        } else {
            " "
        }
    };
    let day = if day_editing {
        format!("{day_draft}_")
    } else {
        selected_day.map_or_else(
            || "Mac-local today".to_owned(),
            |day| format_diary_day(day, 16),
        )
    };
    let selected = selected_workflow.map_or_else(
        || "none".to_owned(),
        |workflow| format_diary_day(&workflow.diary_day, 16),
    );
    let manual = capabilities.map_or("checking", |value| {
        if value.manual_workflow_available {
            "available"
        } else {
            "unavailable"
        }
    });
    let setup_label = setup.map_or_else(
        || "checking".to_owned(),
        |value| {
            format!(
                "config={} database={} Apple helper={}",
                ready_label(value.config_safe),
                ready_label(value.database_ready),
                ready_label(value.apple_helper_available)
            )
        },
    );
    let action_state = if mutations_enabled {
        "ready"
    } else {
        "paused during canonical reload"
    };
    let mut body = format!(
        "Setup: {setup_label}\nManual workflow: {manual}\nActions: {action_state}\nSelected workflow: {selected}\n\n{} Day: {day}\n{} Start/resume chosen day{}\n{} Resume selected workflow\n{} Discard selected workflow{}\n{} Load more workflows{}\n{} Retry/refresh\n{} Help\n{} Save/flush (Compose only)\n{} Quit",
        marker(0),
        marker(1),
        if start_pending { " (working)" } else { "" },
        marker(2),
        marker(3),
        if discard_pending { " (working)" } else { "" },
        marker(4),
        if more_pending {
            " (working)"
        } else if has_more {
            ""
        } else {
            " (no more)"
        },
        marker(5),
        marker(6),
        marker(7),
        marker(8),
    );
    if help_open {
        body.push_str(
            "\n\nTab/Shift-Tab or Left/Right changes focus. Up/Down chooses an item. Enter activates it. Ctrl-G opens Command Line; Ctrl-P/Ctrl-N changes stage; Ctrl-D quits.",
        );
    }
    body
}

#[allow(clippy::too_many_arguments)]
fn sources_screen_body(
    focus: &FocusModel,
    sources: &SourcesViewModel,
    explicit_day: Option<&str>,
    date_editing: bool,
    date_draft: &str,
    page_pending: bool,
    detail_pending: bool,
    confirmation_pending: bool,
    job_pending: bool,
) -> String {
    let marker = |index| {
        if focus.zone == FocusZone::Main && focus.main_control_index == index {
            ">"
        } else {
            " "
        }
    };
    let active = sources.candidates.get(focus.resource_index);
    let active_name = active.map_or("none", |candidate| candidate.source_note_name.as_str());
    let directory_day = active
        .and_then(|candidate| candidate.directory_day.as_deref())
        .map_or_else(|| "unknown".to_owned(), |day| format_diary_day(day, 16));
    let header_day = active
        .and_then(|candidate| candidate.header_day.as_deref())
        .map_or_else(|| "unknown".to_owned(), |day| format_diary_day(day, 16));
    let availability = active.map_or("unknown", |candidate| candidate.availability.as_str());
    let warning = active
        .and_then(|candidate| candidate.warning_code.as_deref())
        .unwrap_or("none");
    let extra_day = if date_editing {
        format!("{date_draft}_")
    } else {
        explicit_day.unwrap_or("not set").to_owned()
    };
    let progress = if confirmation_pending {
        "confirming"
    } else if detail_pending {
        "loading source text"
    } else if page_pending {
        "loading candidates"
    } else if job_pending {
        "source job running"
    } else {
        "idle"
    };
    let progress = sources.job_progress_percent.map_or_else(
        || progress.to_owned(),
        |percent| format!("{progress} · {percent}%"),
    );
    let mut body = format!(
        "Active candidate: {active_name}\nDirectory date: {directory_day}\nHeader date: {header_day}\nAvailability: {availability}\nWarning: {warning}\nIncluded/reviewed: {}/{}\nWork: {progress}\n\n{} Discover three-day window\n{} Extra date: {extra_day}\n{} Discover extra date\n{} Include/exclude active candidate\n{} Prepare included sources\n{} Inspect active prepared source\n{} Confirm reviewed source set\n{} Continue without notes\n{} Load more candidates{}\n{} Back to Start",
        sources.included.len(),
        sources.reviewed.len(),
        marker(0),
        marker(1),
        marker(2),
        marker(3),
        marker(4),
        marker(5),
        marker(6),
        marker(7),
        marker(8),
        if sources.next_cursor.is_some() {
            ""
        } else {
            " (no more)"
        },
        marker(9),
    );
    if let Some(detail) = &sources.active_detail {
        body.push_str("\n\n--- Read-only source text ---\n");
        body.push_str(&detail.body);
    }
    body
}

const fn ready_label(ready: bool) -> &'static str {
    if ready { "ready" } else { "unavailable" }
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

#[cfg(test)]
mod start_screen_tests {
    use super::*;

    #[test]
    fn start_screen_exposes_every_command_equivalent_as_a_focused_control() {
        let focus = FocusModel {
            zone: FocusZone::Main,
            main_control_index: 5,
            ..FocusModel::default()
        };
        let capabilities = CapabilitiesDto {
            capabilities: vec!["read".to_owned(), "workflow:write".to_owned()],
            manual_workflow_available: true,
            assistant_backends: Vec::new(),
            disabled_routes: Vec::new(),
        };
        let setup = SetupCheckDto {
            config_safe: true,
            database_ready: true,
            apple_helper_available: false,
            notion_configured: false,
            manual_workflow_available: true,
        };
        let body = start_screen_body(
            &focus,
            None,
            false,
            "",
            None,
            false,
            Some(&capabilities),
            Some(&setup),
            true,
            false,
            false,
            false,
            true,
        );
        for label in [
            "Day:",
            "Start/resume chosen day",
            "Retry/refresh",
            "Help",
            "Save/flush",
            "Quit",
        ] {
            assert!(body.contains(label), "missing focused action: {label}");
        }
        assert!(body.contains("> Retry/refresh"));
        assert!(body.contains("Manual workflow: available"));
    }

    #[test]
    fn focused_main_control_is_scrolled_into_the_visible_window() {
        let body = "header\nline\nline\nline\n> selected\nlast";
        assert_eq!(ensure_selected_line_visible(body, 0, 3), 2);
        assert_eq!(ensure_selected_line_visible(body, 4, 3), 4);
        assert_eq!(ensure_selected_line_visible(body, 5, 3), 4);
    }

    #[test]
    fn sources_screen_keeps_inclusion_separate_from_the_active_read_only_body() {
        let mut sources = SourcesViewModel {
            candidates: vec![diary_control_contract::SourceCandidateDto {
                id: "src_1".to_owned(),
                source_note_name: "2026/10/29/note.md".to_owned(),
                directory_day: Some("2026-10-29".to_owned()),
                header_day: Some("2026-10-28".to_owned()),
                warning_code: Some("source-warning".to_owned()),
                availability: "local".to_owned(),
                selected: false,
            }],
            ..SourcesViewModel::default()
        };
        let _ = sources.toggle_included("src_1");
        sources.apply_source_detail(ClientOutcome::Ok(diary_tui::client::SourceDetailResponse {
            detail: diary_control_contract::SourceDetailDto {
                id: "src_1".to_owned(),
                source_note_name: "2026/10/29/note.md".to_owned(),
                body: "synthetic source text".to_owned(),
                content_digest: "digest".to_owned(),
                directory_day: Some("2026-10-29".to_owned()),
                header_day: Some("2026-10-28".to_owned()),
                warning_code: Some("source-warning".to_owned()),
            },
            preparation_handle: Some("prep_1".to_owned()),
        }));
        let body = sources_screen_body(
            &FocusModel::default(),
            &sources,
            None,
            false,
            "",
            false,
            false,
            false,
            false,
        );
        assert!(body.contains("Directory date: 2026-10-29"));
        assert!(body.contains("Header date: 2026-10-28"));
        assert!(body.contains("Included/reviewed: 1/1"));
        assert!(body.contains("synthetic source text"));
        assert!(sources.included.contains("src_1"));
    }

    fn composing_workflow() -> WorkflowDto {
        WorkflowDto {
            id: "wf_20261029".to_owned(),
            diary_day: "2026-10-29".to_owned(),
            state: "composing".to_owned(),
            revision: 4,
            buffer_version: 2,
            language: Some("en-US".to_owned()),
            style: "concise".to_owned(),
            assistant_backend_id: None,
            composition_path: None,
            selection_digest: Some("selection".to_owned()),
            explicit_empty_selection: false,
            invalidation_code: None,
            entry_text: "server".to_owned(),
            entry_confirmed: false,
            candidates: vec![
                diary_control_contract::SourceCandidateDto {
                    id: "selected".to_owned(),
                    source_note_name: "selected.md".to_owned(),
                    directory_day: Some("2026-10-29".to_owned()),
                    header_day: Some("2026-10-29".to_owned()),
                    warning_code: None,
                    availability: "local".to_owned(),
                    selected: true,
                },
                diary_control_contract::SourceCandidateDto {
                    id: "excluded".to_owned(),
                    source_note_name: "excluded.md".to_owned(),
                    directory_day: Some("2026-10-29".to_owned()),
                    header_day: Some("2026-10-29".to_owned()),
                    warning_code: None,
                    availability: "local".to_owned(),
                    selected: false,
                },
            ],
        }
    }

    #[test]
    fn compose_resources_list_the_draft_and_only_confirmed_sources() {
        let workflow = composing_workflow();
        let resources = compose_resources(Some(&workflow), true);
        assert_eq!(resources.rows.len(), 2);
        assert_eq!(resources.rows[0].label, "Draft");
        assert_eq!(resources.rows[0].state, "unsaved");
        assert_eq!(resources.rows[1].label, "selected.md");
        assert_eq!(
            confirmed_candidate_at(Some(&workflow), 1).unwrap().id,
            "selected"
        );
    }

    #[test]
    fn compose_and_review_show_both_conflict_versions_and_the_full_entry() {
        let workflow = composing_workflow();
        let mut compose = ComposeViewModel::default();
        compose.load_from_canonical(&workflow);
        compose.edit("local complete entry".to_owned());
        let mut newer = workflow.clone();
        newer.buffer_version = 3;
        newer.entry_text = "server complete entry".to_owned();
        compose.load_from_canonical(&newer);

        let compose_body = compose_screen_body(
            &FocusModel::default(),
            Some(&newer),
            &compose,
            None,
            false,
            false,
            false,
        );
        assert!(compose_body.contains("local complete entry"));
        assert!(compose_body.contains("server complete entry"));

        let review_body = review_screen_body(&FocusModel::default(), Some(&newer), &compose, false);
        assert!(review_body.contains("Complete entry · en-US"));
        assert!(review_body.contains("local complete entry"));
        assert!(review_body.contains("save the Draft before confirming"));
    }
}
