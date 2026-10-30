#![forbid(unsafe_code)]

use std::env;
use std::ffi::OsString;
use std::io::{IsTerminal, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::process::{Command, ExitCode};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use diary_connectors::{
    AppleBridgeClient, AppleFoundationAssistant, AyumiCatalog, AyumiSource, LocalOutput,
    MigrationEvent, MigrationObserver, SqliteStore,
};
use diary_control::{
    CodexAssistant, CommandService, DiagnosticEvent, DiagnosticLevel, DiagnosticLog,
    LifecycleConfig, LifecycleError, LogClock, RuntimeAdvertisement, ServerConfig, ServerLock,
    bind_loopback, load_or_create_tag_key, load_runtime_attachment, random_id, serve,
};
use diary_control_contract::{
    CONTRACT_VERSION, CapabilitiesDto, JOB_NOT_FOUND, LocalPreviewDto, PreferencesDto,
    ResolveParagraphSuggestionRequest, SetupCheckDto, SourceDetailDto, StartParagraphReviewRequest,
    WorkflowDto, buffer_etag, job_status,
};
use diary_core::{
    AssistantBackendId, Clock, DiaryDay, Digest, DraftAssistant, DraftAssistantRegistry,
    OperationLog, Timestamp, WorkflowStore,
};
use diary_tui::assistant_view::{AssistantResource, AssistantViewModel};
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
use diary_tui::sources_view::{JobPhase, SourceJobKind, SourcesViewModel};
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

/// Writes private-state migration progress to the diagnostic log. Logging is
/// best effort here: a failed write must never abort or roll back a migration.
struct MigrationLog(Arc<DiagnosticLog>);

impl MigrationObserver for MigrationLog {
    fn observe(&self, event: MigrationEvent) {
        let version = |value: i64| u32::try_from(value).unwrap_or(u32::MAX);
        let (level, record) = match event {
            MigrationEvent::Current { version: current } => (
                DiagnosticLevel::Debug,
                DiagnosticEvent::Migration {
                    event: "current",
                    from: version(current),
                    to: version(current),
                    duration_ms: 0,
                    reason: None,
                },
            ),
            MigrationEvent::Started { from, to } => (
                DiagnosticLevel::Debug,
                DiagnosticEvent::Migration {
                    event: "started",
                    from: version(from),
                    to: version(to),
                    duration_ms: 0,
                    reason: None,
                },
            ),
            MigrationEvent::StepApplied {
                from,
                to,
                elapsed_ms,
            } => (
                DiagnosticLevel::Debug,
                DiagnosticEvent::Migration {
                    event: "step",
                    from: version(from),
                    to: version(to),
                    duration_ms: elapsed_ms,
                    reason: None,
                },
            ),
            MigrationEvent::Finished {
                from,
                to,
                elapsed_ms,
            } => (
                DiagnosticLevel::Info,
                DiagnosticEvent::Migration {
                    event: "finished",
                    from: version(from),
                    to: version(to),
                    duration_ms: elapsed_ms,
                    reason: None,
                },
            ),
            MigrationEvent::Failed {
                from,
                to,
                elapsed_ms,
                reason,
            } => (
                DiagnosticLevel::Error,
                DiagnosticEvent::Migration {
                    event: "failed",
                    from: version(from),
                    to: version(to),
                    duration_ms: elapsed_ms,
                    reason: Some(reason),
                },
            ),
        };
        let _ = self.0.write(level, record);
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
    let log_level = env::var("ND_LOG_LEVEL")
        .ok()
        .and_then(|value| DiagnosticLevel::parse(&value))
        .unwrap_or(DiagnosticLevel::Info);
    let tag_key = load_or_create_tag_key(
        &lifecycle.application_support.join("diagnostic-tag-key"),
        lifecycle.expected_uid,
    )
    .map_err(|_| "diagnostic-tag-key-failed")?;
    let log = Arc::new(
        DiagnosticLog::open_with_min_level(
            &lifecycle.log_dir,
            lifecycle.expected_uid,
            clock.clone(),
            log_level,
        )
        .map_err(|error| error.startup_code())?
        .with_tag_key(tag_key),
    );
    log.write(
        DiagnosticLevel::Info,
        DiagnosticEvent::Lifecycle {
            component: "server",
            event: "startup",
        },
    )
    .map_err(|_| "diagnostic-write-failed")?;
    // What startup validation made of the retained log segments. Records this
    // build does not recognize (e.g. after a downgrade) are tolerated, so they
    // are a warning-level fact rather than a startup failure.
    let report = log.validation_report();
    log.write(
        if report.unrecognized > 0 {
            DiagnosticLevel::Warn
        } else {
            DiagnosticLevel::Debug
        },
        DiagnosticEvent::Validation(report),
    )
    .map_err(|_| "diagnostic-write-failed")?;

    lifecycle
        .prepare_database()
        .map_err(|_| "database-permission-failed")?;
    lifecycle
        .validate_database_files()
        .map_err(|_| "database-permission-failed")?;
    let store = Arc::new(
        SqliteStore::open_observed(&lifecycle.database_path, &MigrationLog(log.clone()))
            .map_err(|_| "database-open-failed")?,
    );
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
    let apple_assistant = AppleFoundationAssistant::new(bridge.clone(), helper_available);
    let source = Arc::new(AyumiSource::new(catalog, bridge));
    let commands = CommandService::new_with_writer(
        store.clone(),
        source.clone(),
        source.clone(),
        source,
        clock.clone(),
    )
    .with_local_output(Arc::new(
        LocalOutput::new(lifecycle.entries_root.clone()).map_err(|_| "entry-root-invalid")?,
    ));
    let recovery_commands = commands.clone();
    tokio::task::spawn_blocking(move || recovery_commands.reconcile_local_intents())
        .await
        .map_err(|_| "publication-recovery-task-failed")?
        .map_err(|_| "publication-recovery-failed")?;

    let (listener, port) = bind_loopback(lifecycle.port_fallback_count).await?;
    let instance_id = random_id("instance_").map_err(|_| "random-failed")?;
    let control_token = random_id("token_").map_err(|_| "random-failed")?;
    let mut assistant_backends = lifecycle
        .codex_profiles
        .iter()
        .map(|profile| {
            CodexAssistant::production(profile, lifecycle.expected_uid)
                .map(|adapter| Arc::new(adapter) as Arc<dyn DraftAssistant>)
        })
        .collect::<Result<Vec<_>, _>>()?;
    assistant_backends.push(Arc::new(apple_assistant) as Arc<dyn DraftAssistant>);
    let hosted_default = lifecycle
        .codex_profiles
        .first()
        .map(|profile| AssistantBackendId::parse(format!("codex.{}", profile.alias)))
        .transpose()
        .map_err(|_| "assistant-registry-invalid")?;
    let assistant_registry = Arc::new(
        DraftAssistantRegistry::new(assistant_backends, hosted_default)
            .map_err(|_| "assistant-registry-invalid")?,
    );
    let config = ServerConfig {
        instance_id: instance_id.clone(),
        control_token: control_token.clone(),
        port,
        commands,
        apple_helper_available: helper_available,
        notion_configured: lifecycle.notion_configured,
        assistant_registry,
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
/// server is a screen with a retry backoff, never an exit. Ctrl-C, pressed
/// twice, quits from any screen.
fn tui_mode_interactive() -> ExitCode {
    let mut guard = match TerminalGuard::acquire() {
        Ok(guard) => guard,
        Err(_) => return fail_tui("terminal-unavailable"),
    };
    let client = ApiClient::new(UreqTransport::new());
    let mut backoff = RETRY_INITIAL_BACKOFF;
    let mut quit_armed = false;

    loop {
        let (runtime_dir, expected_uid) = match resolve_runtime_location() {
            Ok(value) => value,
            Err(_) => {
                draw_phase_screen(
                    &mut guard,
                    "Cannot resolve the runtime location.",
                    quit_armed,
                );
                match diary_tui::terminal::poll_event(backoff) {
                    diary_tui::terminal::PollOutcome::Quit => {
                        if quit_armed {
                            drop(guard);
                            return ExitCode::SUCCESS;
                        }
                        quit_armed = true;
                    }
                    diary_tui::terminal::PollOutcome::Other => {
                        quit_armed = false;
                    }
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
                    quit_armed = false;
                }
                SessionEnd::RenderFailed => {
                    drop(guard);
                    return fail_tui("render-failed");
                }
            },
            Err(phase) => {
                draw_phase_screen(&mut guard, attach_phase_message(phase), quit_armed);
                match diary_tui::terminal::poll_event(backoff) {
                    diary_tui::terminal::PollOutcome::Quit => {
                        if quit_armed {
                            drop(guard);
                            return ExitCode::SUCCESS;
                        }
                        quit_armed = true;
                    }
                    diary_tui::terminal::PollOutcome::Other => {
                        quit_armed = false;
                    }
                    diary_tui::terminal::PollOutcome::TimedOut => {
                        backoff = (backoff * 2).min(RETRY_BACKOFF_CEILING);
                    }
                }
            }
        }
    }
}

fn draw_phase_screen(guard: &mut TerminalGuard, message: &str, quit_armed: bool) {
    let notice = if quit_armed {
        "Retrying automatically. Press Ctrl-C again to quit."
    } else {
        "Retrying automatically. Press Ctrl-C twice to quit."
    };
    let work_status = WorkStatusModel {
        current_work: Some(message.to_owned()),
        pending_or_blocked: None,
        connection_and_save_safety: Some("Not connected".to_owned()),
        notice: Some(notice.to_owned()),
    };
    let stage_model = map_workflow(None, false);
    let status_line = StatusLineModel {
        diary_day: None,
        connection_label: "Not connected",
        ime: ImeState::Unknown,
        action_hint: Some("Ctrl-C twice: quit".to_owned()),
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
        notice: Some("Press Ctrl-G for Command Line, Ctrl-C twice to quit".to_owned()),
    };
    let mut status_line = StatusLineModel {
        diary_day: None,
        connection_label: "Connected",
        ime: ImeState::Unknown,
        action_hint: Some("Ctrl-C twice: quit".to_owned()),
    };

    let worker = Worker::spawn(UreqTransport::new(), client_attachment.clone());
    let event_worker = EventWorker::spawn(UreqEventSource::new(), client_attachment.clone(), None);
    let mut authority = SessionAuthority::connected(client_attachment.instance_id().to_owned());
    let mut start_view = StartViewModel::default();
    let mut refresh_pending = worker.try_send(Intent::RefreshWorkflows).is_ok();
    let _ = worker.try_send(Intent::GetCapabilities);
    let _ = worker.try_send(Intent::GetAssistantBackends);
    let _ = worker.try_send(Intent::GetSetupCheck);
    let _ = worker.try_send(Intent::GetPreferences);
    let mut recovery_list_required = false;
    let mut recovery_workflow: Option<(String, bool)> = None;
    let mut recovery_job: Option<String> = None;
    // The one job whose `PollJob` is in flight. The worker answers in FIFO
    // order without echoing a job ID on failure, so polls are serialized
    // and each reply is routed by this ID, never by which caller asked.
    let mut job_poll_in_flight: Option<String> = None;
    let mut prefer_assistant_poll = false;
    let mut capabilities: Option<CapabilitiesDto> = None;
    let mut setup_check: Option<SetupCheckDto> = None;
    let mut preferences: Option<PreferencesDto> = None;
    let mut current_workflow: Option<WorkflowDto> = None;
    let mut local_preview: Option<LocalPreviewDto> = None;
    let mut local_action_pending = false;
    let mut local_publication_job: Option<String> = None;
    let mut conflict_day_editing = false;
    let mut conflict_day_draft = String::new();
    let mut current_stage = Stage::Start;
    let mut focus = FocusModel::default();
    let mut start_pending = false;
    let mut more_pending = false;
    let mut discard_pending: Option<usize> = None;
    let mut discard_confirmation: Option<usize> = None;
    let mut help_open = false;
    let mut sources_view = SourcesViewModel::default();
    let mut pending_source_job_kind: Option<SourceJobKind> = None;
    let mut source_page_pending = false;
    let mut source_page_append = false;
    let mut source_detail_pending = false;
    let mut preview_detail_pending = false;
    let mut source_confirmation_pending = false;
    let mut source_mutation_pending = false;
    let mut source_editor_active = false;
    let mut empty_selection_confirmation = false;
    let mut source_date_editing = false;
    let mut source_date_draft = String::new();
    let mut explicit_source_day: Option<String> = None;
    let mut compose_view = ComposeViewModel::default();
    let mut assistant_view = AssistantViewModel::default();
    let mut pending_assistant_job_kind: Option<String> = None;
    let mut assistant_job: Option<(String, String)> = None;
    let mut assistant_selection_pending = false;
    let mut memory_review_pending = false;
    let mut draft_editor = DraftEditor::new("");
    // The Sources-stage source-body editor. Never shares `draft_editor`: a
    // canonical reload resets that one to the entry text (which a Ctrl-S
    // here would then write over the source note), and a leftover source
    // body there would replace the diary entry on the next Compose keystroke.
    let mut source_editor = DraftEditor::new("");
    // The source (ID, content digest) `source_editor` was opened on. Ctrl-S
    // saves to this, never to whatever source is displayed by then: a preview
    // finishing mid-edit would otherwise write this text over that source.
    let mut source_edit_target: Option<(String, String)> = None;
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
    const DEFAULT_NOTICE: &str = "Press Ctrl-G for Command Line, Ctrl-C twice to quit";

    let mut command_line_active = false;
    let mut command_line = diary_tui::command_line::CommandLineState::default();
    let mut command_line_feedback: Option<String> = None;
    let mut feedback: Option<String> = None;
    let mut selected_day: Option<String> = None;
    let mut day_draft = String::new();
    let mut day_editing = false;
    let mut main_scroll = 0u16;
    let mut shown_source_detail_id: Option<String> = None;
    let mut focused_stage = current_stage;
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
                        recovery_job = Some(job_id);
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
                WorkerResult::AssistantBackends(outcome) => match outcome {
                    ClientOutcome::Ok(value) => assistant_view.backends = value.backends,
                    ClientOutcome::Problem(problem) => feedback = Some(problem.safe_label()),
                    ClientOutcome::Transport(error) => {
                        feedback = Some(error.safe_label().to_owned());
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
                    // No focus clamp here: an SSE-driven list refresh also
                    // arrives while another stage is shown, and clamping with
                    // Start's counts would move that stage's Resources/Main
                    // focus. The per-frame clamp uses the current stage's.
                }
                WorkerResult::MoreWorkflows(outcome) => {
                    more_pending = false;
                    start_view.apply_more(outcome);
                    if let Some(error) = &start_view.last_error {
                        feedback = Some(error.safe_label());
                    }
                }
                WorkerResult::StartedWorkflow(outcome) => {
                    start_pending = false;
                    match *outcome {
                        ClientOutcome::Ok(workflow) => {
                            selected_day = Some(workflow.diary_day.clone());
                            local_preview = None;
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
                            let focused_source_id =
                                focused_source_id(current_stage, &focus, &sources_view);
                            sources_view.load_candidates_from(workflow);
                            reanchor_source_focus(
                                &mut focus,
                                &sources_view,
                                focused_source_id.as_deref(),
                            );
                            compose_view.load_from_canonical(workflow);
                            if !compose_view.is_dirty() {
                                // Reloads follow every autosave (its own
                                // `entry.changed` event); rebuilding an editor
                                // that already shows this text would snap the
                                // cursor back to the top mid-writing.
                                if draft_editor.text() != compose_view.text {
                                    draft_editor = DraftEditor::new(&compose_view.text);
                                }
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
                    let polled_id = job_poll_in_flight.take();
                    let job_id = match &outcome {
                        ClientOutcome::Ok(job) => Some(job.id.clone()),
                        _ => polled_id,
                    };
                    let job_lost = matches!(
                        &outcome,
                        ClientOutcome::Problem(problem) if problem.code == JOB_NOT_FOUND
                    );
                    let is_recovery = job_id.is_some() && recovery_job == job_id;
                    let is_local_publication = job_id.is_some() && local_publication_job == job_id;
                    let is_assistant = job_id.is_some()
                        && assistant_job.as_ref().map(|(id, _)| id) == job_id.as_ref();
                    let is_source = job_id
                        .as_deref()
                        .is_some_and(|id| sources_view.is_pending_job(id));
                    if is_recovery && (job_lost || matches!(&outcome, ClientOutcome::Ok(_))) {
                        // A vanished job has no canonical state left to reload.
                        recovery_job = None;
                        authority.canonical_job_loaded();
                    }
                    if is_local_publication {
                        let terminal = job_lost
                            || matches!(&outcome, ClientOutcome::Ok(job) if job_status::is_terminal(&job.status));
                        if terminal {
                            local_publication_job = None;
                            if let Some(workflow) = &current_workflow {
                                let _ = worker.try_send(Intent::GetWorkflow {
                                    workflow_id: workflow.id.clone(),
                                });
                            }
                            feedback =
                                Some("Local publication finished; reloading status".to_owned());
                        }
                    }
                    if is_assistant {
                        match outcome {
                            ClientOutcome::Ok(job) => {
                                let terminal = job_status::is_terminal(&job.status);
                                let succeeded = job.status == job_status::COMPLETED;
                                let kind = job.kind.clone();
                                let error_code = job.error_code.clone();
                                // The server reports a refinement's opaque
                                // source ID as its result handle.
                                let refined_source_id = job.result_handle.clone();
                                assistant_view.active_job = Some(job);
                                if terminal {
                                    assistant_job = None;
                                }
                                if succeeded && let Some(workflow) = &current_workflow {
                                    let intent = match kind.as_str() {
                                        "assistant-memory-extraction" => Intent::GetMemoryReview {
                                            workflow_id: workflow.id.clone(),
                                        },
                                        "assistant-draft" => Intent::GetDraftResult {
                                            workflow_id: workflow.id.clone(),
                                        },
                                        "assistant-paragraph-review" => {
                                            Intent::GetParagraphSuggestions {
                                                workflow_id: workflow.id.clone(),
                                            }
                                        }
                                        "assistant-source-refine" => {
                                            if let Some(source_id) =
                                                refined_source_id.or_else(|| {
                                                    sources_view
                                                        .active_detail
                                                        .as_ref()
                                                        .map(|detail| detail.id.clone())
                                                })
                                            {
                                                Intent::GetSourceRefineSuggestion {
                                                    workflow_id: workflow.id.clone(),
                                                    source_id,
                                                }
                                            } else {
                                                Intent::GetWorkflow {
                                                    workflow_id: workflow.id.clone(),
                                                }
                                            }
                                        }
                                        _ => Intent::GetWorkflow {
                                            workflow_id: workflow.id.clone(),
                                        },
                                    };
                                    let _ = worker.try_send(intent);
                                    let _ = worker.try_send(Intent::GetWorkflow {
                                        workflow_id: workflow.id.clone(),
                                    });
                                    feedback = Some("Assistant action complete".to_owned());
                                } else if terminal {
                                    feedback = Some(match error_code {
                                        Some(code) => format!(
                                            "Assistant action failed: {code}; manual Draft remains available"
                                        ),
                                        None => "Assistant action did not complete; manual Draft remains available".to_owned(),
                                    });
                                }
                            }
                            ClientOutcome::Problem(_) if job_lost => {
                                // Polling a 404 forever would also keep every
                                // assistant action disabled.
                                assistant_job = None;
                                feedback = Some(
                                    "Assistant job is no longer known to the server; manual Draft remains available"
                                        .to_owned(),
                                );
                            }
                            ClientOutcome::Problem(problem) => {
                                feedback = Some(problem.safe_label())
                            }
                            ClientOutcome::Transport(error) => {
                                feedback = Some(error.safe_label().to_owned());
                            }
                        }
                    } else if is_source && job_lost {
                        if let Some(job_id) = &job_id {
                            sources_view.forget_job(job_id);
                        }
                        feedback = Some(
                            "Source job is no longer known to the server; review and retry"
                                .to_owned(),
                        );
                    } else if is_source {
                        let job_kind = match &outcome {
                            ClientOutcome::Ok(job) => Some(job.kind.clone()),
                            _ => None,
                        };
                        let is_preview_job = job_id.is_some()
                            && job_id.as_deref() == sources_view.pending_preview_job.as_deref();
                        let phase = sources_view.apply_job_status(outcome);
                        if let Some(error) = &sources_view.last_error {
                            feedback = Some(error.safe_label());
                        }
                        match (job_kind.as_deref(), phase) {
                            _ if is_preview_job && phase == Some(JobPhase::Succeeded) => {
                                if let (Some(workflow), Some(source_id), Some(handle)) = (
                                    current_workflow.as_ref(),
                                    sources_view.preview_source_id.clone(),
                                    sources_view.preview_handle.take(),
                                ) {
                                    if worker
                                        .try_send(Intent::SourceDetail {
                                            workflow_id: workflow.id.clone(),
                                            source_id,
                                            preparation_handle: handle,
                                        })
                                        .is_ok()
                                    {
                                        preview_detail_pending = true;
                                    } else {
                                        sources_view.preview_source_id = None;
                                    }
                                }
                            }
                            _ if is_preview_job && phase == Some(JobPhase::Failed) => {
                                sources_view.preview_source_id = None;
                                feedback = Some("Preview failed; review and retry".to_owned());
                            }
                            (Some("discovery"), Some(JobPhase::Succeeded)) => {
                                sources_view.accept_fresh_discovery();
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
                    }
                    // Otherwise a recovery-only read with no stage state to
                    // apply; a failed one stays in `recovery_job` and is retried.
                }
                WorkerResult::JobAccepted(outcome) => {
                    if let Some(kind) = pending_source_job_kind.take() {
                        sources_view.apply_job_accepted(kind, outcome);
                        if let Some(error) = &sources_view.last_error {
                            feedback = Some(error.safe_label());
                        }
                    } else if let Some(kind) = pending_assistant_job_kind.take() {
                        match outcome {
                            ClientOutcome::Ok(accepted) => {
                                assistant_job = Some((accepted.job_id, kind));
                                feedback = Some("Assistant action started".to_owned());
                            }
                            ClientOutcome::Problem(problem) => {
                                feedback = Some(problem.safe_label());
                            }
                            ClientOutcome::Transport(error) => {
                                feedback = Some(error.safe_label().to_owned());
                            }
                        }
                    }
                }
                WorkerResult::Sources(outcome) => {
                    source_page_pending = false;
                    let focused_source_id = focused_source_id(current_stage, &focus, &sources_view);
                    sources_view.apply_source_page(source_page_append, outcome);
                    reanchor_source_focus(&mut focus, &sources_view, focused_source_id.as_deref());
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
                    } else if preview_detail_pending {
                        preview_detail_pending = false;
                        sources_view.apply_preview_detail(*outcome);
                        if let Some(error) = &sources_view.last_error {
                            feedback = Some(error.safe_label());
                        }
                    } else {
                        source_detail_pending = false;
                        sources_view.apply_source_detail(*outcome);
                        if let Some(error) = &sources_view.last_error {
                            feedback = Some(error.safe_label());
                        }
                    }
                }
                WorkerResult::SourceRefineSuggestion(outcome) => match outcome {
                    ClientOutcome::Ok(value) => {
                        sources_view.active_refine_suggestion = Some(value);
                        feedback =
                            Some("Source refinement is ready for approval or rejection".to_owned());
                    }
                    ClientOutcome::Problem(problem) => feedback = Some(problem.safe_label()),
                    ClientOutcome::Transport(error) => {
                        feedback = Some(error.safe_label().to_owned())
                    }
                },
                WorkerResult::SourceMutation(outcome) => {
                    source_mutation_pending = false;
                    match *outcome {
                        ClientOutcome::Ok(workflow) => {
                            let focused_source_id =
                                focused_source_id(current_stage, &focus, &sources_view);
                            sources_view.load_candidates_from(&workflow);
                            reanchor_source_focus(
                                &mut focus,
                                &sources_view,
                                focused_source_id.as_deref(),
                            );
                            sources_view.active_detail = None;
                            sources_view.active_refine_suggestion = None;
                            sources_view.pending_delete_confirmation = None;
                            current_workflow = Some(workflow);
                            source_editor_active = false;
                            feedback = Some("Source change saved".to_owned());
                        }
                        ClientOutcome::Problem(problem) => feedback = Some(problem.safe_label()),
                        ClientOutcome::Transport(error) => {
                            feedback = Some(error.safe_label().to_owned())
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
                    // A save for a day the person has since left already
                    // landed there; applying it would bring that workflow
                    // back as current and mix its versions into this draft.
                    let other_workflow = matches!(
                        &*outcome,
                        ClientOutcome::Ok(saved) if current_workflow
                            .as_ref()
                            .is_none_or(|current| current.id != saved.id)
                    );
                    if let Some(generation) = generation.filter(|_| !other_workflow) {
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
                WorkerResult::LocalPreviewed(outcome) => {
                    local_action_pending = false;
                    match *outcome {
                        ClientOutcome::Ok(preview) => {
                            current_workflow = Some(preview.workflow.clone());
                            local_preview = Some(preview);
                            feedback = Some(
                                "Output plan ready; inspect the complete file before approval"
                                    .to_owned(),
                            );
                        }
                        ClientOutcome::Problem(problem) => feedback = Some(problem.safe_label()),
                        ClientOutcome::Transport(error) => {
                            feedback = Some(error.safe_label().to_owned())
                        }
                    }
                }
                WorkerResult::LocalApproved(outcome) => {
                    local_action_pending = false;
                    match *outcome {
                        ClientOutcome::Ok(workflow) => {
                            current_workflow = Some(workflow);
                            current_stage = Stage::Publish;
                            feedback = Some("Exact output plan approved".to_owned());
                        }
                        ClientOutcome::Problem(problem) => feedback = Some(problem.safe_label()),
                        ClientOutcome::Transport(error) => {
                            feedback = Some(error.safe_label().to_owned())
                        }
                    }
                }
                WorkerResult::LocalPublished(outcome) => {
                    local_action_pending = false;
                    match outcome {
                        ClientOutcome::Ok(job) => {
                            local_publication_job = Some(job.job_id);
                            feedback = Some("Local publication started".to_owned());
                        }
                        ClientOutcome::Problem(problem) => feedback = Some(problem.safe_label()),
                        ClientOutcome::Transport(error) => {
                            feedback = Some(error.safe_label().to_owned())
                        }
                    }
                }
                WorkerResult::SavedUnderDay(outcome) => {
                    local_action_pending = false;
                    match *outcome {
                        ClientOutcome::Ok(workflow) => {
                            selected_day = Some(workflow.diary_day.clone());
                            current_stage = map_workflow(Some(&workflow), false).default_current;
                            sources_view.load_candidates_from(&workflow);
                            compose_view.load_from_canonical(&workflow);
                            draft_editor = DraftEditor::new(&compose_view.text);
                            current_workflow = Some(workflow);
                            local_preview = None;
                            feedback = Some(
                                "Target day opened; review and confirm its entry before output"
                                    .to_owned(),
                            );
                        }
                        ClientOutcome::Problem(problem) => feedback = Some(problem.safe_label()),
                        ClientOutcome::Transport(error) => {
                            feedback = Some(error.safe_label().to_owned())
                        }
                    }
                }
                WorkerResult::AssistantSelected(outcome) => {
                    assistant_selection_pending = false;
                    match *outcome {
                        ClientOutcome::Ok(workflow) => {
                            let backend = workflow.assistant_backend_id.clone();
                            compose_view.load_from_canonical(&workflow);
                            current_workflow = Some(workflow);
                            let _ = worker.try_send(Intent::SavePreferences {
                                last_language: preferences
                                    .as_ref()
                                    .and_then(|value| value.last_language.clone()),
                                last_assistant_backend_id: backend,
                            });
                            feedback =
                                Some("Assistant backend selected; no content was sent".to_owned());
                        }
                        ClientOutcome::Problem(problem) => feedback = Some(problem.safe_label()),
                        ClientOutcome::Transport(error) => {
                            feedback = Some(error.safe_label().to_owned())
                        }
                    }
                }
                WorkerResult::MemoryReview(outcome) => {
                    memory_review_pending = false;
                    match outcome {
                        ClientOutcome::Ok(review) => assistant_view.load_memory_review(review),
                        ClientOutcome::Problem(problem) => feedback = Some(problem.safe_label()),
                        ClientOutcome::Transport(error) => {
                            feedback = Some(error.safe_label().to_owned())
                        }
                    }
                }
                WorkerResult::MemoryReviewSaved(outcome) => {
                    memory_review_pending = false;
                    match *outcome {
                        ClientOutcome::Ok(workflow) => {
                            let workflow_id = workflow.id.clone();
                            compose_view.load_from_canonical(&workflow);
                            current_workflow = Some(workflow);
                            let _ = worker.try_send(Intent::GetMemoryReview { workflow_id });
                            feedback = Some("Reviewed memories confirmed".to_owned());
                        }
                        ClientOutcome::Problem(problem) => feedback = Some(problem.safe_label()),
                        ClientOutcome::Transport(error) => {
                            feedback = Some(error.safe_label().to_owned())
                        }
                    }
                }
                WorkerResult::DraftResult(outcome) => match outcome {
                    ClientOutcome::Ok(draft) => {
                        assistant_view.draft_result = Some(draft);
                        feedback = Some("Assistant draft is available".to_owned());
                    }
                    ClientOutcome::Problem(problem) => feedback = Some(problem.safe_label()),
                    ClientOutcome::Transport(error) => {
                        feedback = Some(error.safe_label().to_owned())
                    }
                },
                WorkerResult::ParagraphSuggestions(outcome) => match outcome {
                    ClientOutcome::Ok(value) => assistant_view.suggestions = value.suggestions,
                    ClientOutcome::Problem(problem) => feedback = Some(problem.safe_label()),
                    ClientOutcome::Transport(error) => {
                        feedback = Some(error.safe_label().to_owned())
                    }
                },
                WorkerResult::ParagraphSuggestion(outcome) => match outcome {
                    ClientOutcome::Ok(value) => assistant_view.active_suggestion = Some(value),
                    ClientOutcome::Problem(problem) => feedback = Some(problem.safe_label()),
                    ClientOutcome::Transport(error) => {
                        feedback = Some(error.safe_label().to_owned())
                    }
                },
                WorkerResult::ParagraphSuggestionResolved(outcome) => match *outcome {
                    ClientOutcome::Ok(workflow) => {
                        compose_view.load_from_canonical(&workflow);
                        draft_editor = DraftEditor::new(&compose_view.text);
                        let workflow_id = workflow.id.clone();
                        current_workflow = Some(workflow);
                        assistant_view.active_suggestion = None;
                        let _ = worker.try_send(Intent::GetParagraphSuggestions { workflow_id });
                        feedback = Some("Paragraph suggestion resolved".to_owned());
                    }
                    ClientOutcome::Problem(problem) => feedback = Some(problem.safe_label()),
                    ClientOutcome::Transport(error) => {
                        feedback = Some(error.safe_label().to_owned())
                    }
                },
                WorkerResult::JobCancelled(outcome) => match outcome {
                    ClientOutcome::Ok(()) => feedback = Some("Cancellation requested".to_owned()),
                    ClientOutcome::Problem(problem) => feedback = Some(problem.safe_label()),
                    ClientOutcome::Transport(error) => {
                        feedback = Some(error.safe_label().to_owned())
                    }
                },
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
        if job_poll_in_flight.is_none()
            && let Some(job_id) = next_job_poll(
                recovery_job.as_deref().or(local_publication_job.as_deref()),
                sources_view
                    .pending_discovery_job
                    .as_deref()
                    .or(sources_view.pending_preparation_job.as_deref())
                    .or(sources_view.pending_preview_job.as_deref()),
                assistant_job.as_ref().map(|(job_id, _)| job_id.as_str()),
                prefer_assistant_poll,
            )
            .map(str::to_owned)
            && worker
                .try_send(Intent::PollJob {
                    job_id: job_id.clone(),
                })
                .is_ok()
        {
            if assistant_job.as_ref().is_some_and(|(id, _)| *id == job_id) {
                prefer_assistant_poll = false;
            } else if sources_view.is_pending_job(&job_id) {
                prefer_assistant_poll = true;
            }
            job_poll_in_flight = Some(job_id);
        }

        status_line.ime = ime_tracker.tick(std::time::Instant::now());
        status_line.diary_day = current_workflow
            .as_ref()
            .map(|workflow| workflow.diary_day.clone())
            .or_else(|| selected_day.clone());
        let selected_backend_label = current_workflow
            .as_ref()
            .and_then(|workflow| workflow.assistant_backend_id.as_deref())
            .and_then(|id| {
                assistant_view
                    .backends
                    .iter()
                    .find(|backend| backend.id == id)
            })
            .map_or_else(
                || "manual".to_owned(),
                |backend| format!("{} · {}", backend.display_label, backend.composition_path),
            );
        status_line.action_hint = Some(format!("{selected_backend_label} | Ctrl-C twice: quit"));

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
            || sources_view.pending_preparation_job.is_some()
            || sources_view.pending_preview_job.is_some()
            || preview_detail_pending;
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
        if current_stage != focused_stage {
            // Each stage lists different resources, so the previous stage's
            // row index would point at an unrelated row here.
            focused_stage = current_stage;
            focus.resource_index =
                stage_entry_resource_index(current_stage, &sources_view, compose_active_resource);
        }
        let (compose_resources, assistant_resources) =
            assistant_view.resources(current_workflow.as_ref(), compose_view.is_dirty());
        let visible_resource_count = match current_stage {
            Stage::Start => start_view.workflows.len(),
            Stage::Sources => sources_view.candidates.len(),
            Stage::Compose => compose_resources.rows.len(),
            Stage::Review => 1,
            Stage::Preview => 1,
            Stage::Publish => 2,
        };
        let visible_control_count = match current_stage {
            Stage::Sources => SOURCES_CONTROL_COUNT,
            Stage::Compose => COMPOSE_CONTROL_COUNT,
            Stage::Review => REVIEW_CONTROL_COUNT,
            Stage::Preview => 3,
            Stage::Publish => 2,
            _ => START_CONTROL_COUNT,
        };
        focus.clamp(
            ALL_STAGES.len(),
            visible_resource_count,
            visible_control_count,
        );

        let mut displayed_status = work_status.clone();
        displayed_status.current_work = assistant_view
            .job_label()
            .or_else(|| Some(format!("Composition: {selected_backend_label}")));
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
        } else if conflict_day_editing {
            format!("New diary day: {conflict_day_draft} (Enter confirms; Esc cancels)")
        } else if source_date_editing {
            format!("Extra source date: {source_date_draft}")
        } else if empty_selection_confirmation {
            "Continue without notes? Enter confirms; Esc cancels".to_owned()
        } else if let Some(source_id) = &sources_view.pending_delete_confirmation {
            let name = sources_view
                .active_detail
                .as_ref()
                .filter(|detail| &detail.id == source_id)
                .map_or(source_id.as_str(), |detail| {
                    detail.source_note_name.as_str()
                });
            format!("Delete {name} and linked audio? Enter confirms; Esc cancels")
        } else if quit_confirmation {
            if compose_view.is_dirty() || save_in_flight.is_some() {
                "Draft is not safely saved. Ctrl-C again to quit; Esc cancels".to_owned()
            } else if source_editor_active {
                "Source edit is not saved (Ctrl-S saves). Ctrl-C again abandons it; Esc cancels"
                    .to_owned()
            } else {
                "Press Ctrl-C again to quit; Esc cancels".to_owned()
            }
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
            Stage::Preview => ResourcesSidebarModel {
                rows: vec![ResourceRow {
                    label: "Local Markdown".to_owned(),
                    state: "preview".to_owned(),
                    indent: 0,
                }],
            },
            Stage::Publish => ResourcesSidebarModel {
                rows: vec![
                    ResourceRow {
                        label: "Local file".to_owned(),
                        state: current_workflow
                            .as_ref()
                            .and_then(|w| w.local_status.as_deref())
                            .unwrap_or("pending")
                            .to_owned(),
                        indent: 0,
                    },
                    ResourceRow {
                        label: "Notion".to_owned(),
                        state: "blocked".to_owned(),
                        indent: 0,
                    },
                ],
            },
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
                preferences.as_ref(),
                &assistant_view,
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
                source_detail_pending,
                source_confirmation_pending,
                source_job_busy,
            ),
            Stage::Compose => compose_screen_body(
                &focus,
                current_workflow.as_ref(),
                &compose_view,
                &assistant_view,
                assistant_resources.get(compose_active_resource),
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
            Stage::Preview => preview_screen_body(&focus, local_preview.as_ref()),
            Stage::Publish => publish_screen_body(&focus, current_workflow.as_ref()),
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
                        Stage::Sources => sources_view.active_candidate_index(),
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
            main_scroll = resolve_main_scroll(
                current_stage,
                &sources_view,
                &main_body,
                main_scroll,
                main_height,
                &mut shown_source_detail_id,
            );
            let editor_pane = if current_stage == Stage::Sources && source_editor_active {
                Some(&source_editor)
            } else if current_stage == Stage::Compose
                && editor_active
                && compose_active_resource == 0
            {
                Some(&draft_editor)
            } else {
                None
            };
            if let Some(editor) = editor_pane {
                frame.render_widget(
                    DraftEditorPane::new(editor, focus.zone == FocusZone::Main).palette(palette),
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
                if quit_confirmation {
                    return SessionEnd::Quit;
                }
                quit_confirmation = true;
                feedback = Some(if compose_view.is_dirty() || save_in_flight.is_some() {
                    save_requested = true;
                    "Saving before quit; Ctrl-C again abandons unsaved edits".to_owned()
                } else if source_editor_active {
                    "Source edit is not saved; Ctrl-C again abandons it".to_owned()
                } else {
                    "Press Ctrl-C again to quit".to_owned()
                });
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
                if current_stage == Stage::Sources
                    && source_editor_active
                    && !source_mutation_pending
                {
                    if let (Some(workflow), Some((source_id, digest))) =
                        (&current_workflow, &source_edit_target)
                    {
                        source_mutation_pending = worker
                            .try_send(Intent::EditSourceBody {
                                workflow_id: workflow.id.clone(),
                                source_id: source_id.clone(),
                                revision: workflow.revision,
                                text: source_editor.text().to_owned(),
                                digest: digest.clone(),
                            })
                            .is_ok();
                    }
                } else if current_stage == Stage::Compose && compose_view.is_dirty() {
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
            InputPoll::Event(event) if conflict_day_editing => match event {
                InputEvent::Char(ch)
                    if (ch.is_ascii_digit() || ch == '-') && conflict_day_draft.len() < 10 =>
                {
                    conflict_day_draft.push(ch)
                }
                InputEvent::Backspace => {
                    conflict_day_draft.pop();
                }
                InputEvent::Enter => {
                    if diary_tui::command_line::is_valid_day(&conflict_day_draft) {
                        if let Some(workflow) = &current_workflow {
                            local_action_pending = worker
                                .try_send(Intent::SaveUnderDay {
                                    workflow_id: workflow.id.clone(),
                                    revision: workflow.revision,
                                    new_day: conflict_day_draft.clone(),
                                })
                                .is_ok();
                        }
                        conflict_day_editing = false;
                    } else {
                        feedback = Some("New day must be YYYY-MM-DD".to_owned());
                    }
                }
                InputEvent::Esc => {
                    conflict_day_editing = false;
                    feedback = Some("Save under another day cancelled".to_owned());
                }
                _ => {}
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
            InputPoll::Event(InputEvent::Enter)
                if sources_view.pending_delete_confirmation.is_some() =>
            {
                let source_id = sources_view
                    .pending_delete_confirmation
                    .take()
                    .unwrap_or_default();
                if let Some(workflow) = &current_workflow {
                    source_mutation_pending = worker
                        .try_send(Intent::DeleteSource {
                            workflow_id: workflow.id.clone(),
                            source_id,
                            revision: workflow.revision,
                        })
                        .is_ok();
                }
            }
            InputPoll::Event(InputEvent::Esc)
                if sources_view.pending_delete_confirmation.is_some() =>
            {
                sources_view.pending_delete_confirmation = None;
                feedback = Some("Source deletion cancelled".to_owned());
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
            InputPoll::Event(InputEvent::Esc)
                if source_editor_active && current_stage == Stage::Sources =>
            {
                source_editor_active = false;
                feedback = Some("Source edit cancelled".to_owned());
            }
            InputPoll::Event(event) if source_editor_active && current_stage == Stage::Sources => {
                let _ = source_editor.handle_input(&event);
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
                    if !source_job_busy && !source_detail_pending {
                        request_inspect_source(
                            &worker,
                            &mut sources_view,
                            &current_workflow,
                            focus.resource_index,
                            &mut source_detail_pending,
                            &mut pending_source_job_kind,
                        );
                    }
                }
                FocusZone::Resources if current_stage == Stage::Compose => {
                    compose_active_resource = focus.resource_index;
                    match assistant_resources.get(focus.resource_index) {
                        Some(AssistantResource::Draft) => {
                            confirmed_source_detail = None;
                            assistant_view.active_suggestion = None;
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
                        }
                        Some(AssistantResource::MemoryReview) => {
                            confirmed_source_detail = None;
                            editor_active = false;
                            if !memory_review_pending && let Some(workflow) = &current_workflow {
                                memory_review_pending = worker
                                    .try_send(Intent::GetMemoryReview {
                                        workflow_id: workflow.id.clone(),
                                    })
                                    .is_ok();
                            }
                        }
                        Some(AssistantResource::MemoryCandidate(index)) => {
                            if let Some(id) = assistant_view
                                .memory_review
                                .as_ref()
                                .and_then(|review| review.candidates.get(*index))
                                .map(|candidate| candidate.id.clone())
                            {
                                assistant_view.toggle_memory(&id);
                                feedback = Some(
                                    "Memory inclusion changed; confirm review to persist"
                                        .to_owned(),
                                );
                            }
                        }
                        Some(AssistantResource::Backend(index))
                            if can_mutate && !assistant_selection_pending =>
                        {
                            if let (Some(workflow), Some(backend)) = (
                                current_workflow.as_ref(),
                                assistant_view.backends.get(*index),
                            ) {
                                if backend.available {
                                    assistant_selection_pending = worker
                                        .try_send(Intent::SelectAssistant {
                                            workflow_id: workflow.id.clone(),
                                            revision: workflow.revision,
                                            backend_id: Some(backend.id.clone()),
                                        })
                                        .is_ok();
                                } else {
                                    feedback = Some(
                                        "That backend is unavailable; nothing was sent".to_owned(),
                                    );
                                }
                            }
                        }
                        Some(AssistantResource::ConfirmedSource(index))
                            if !confirmed_source_pending =>
                        {
                            let candidate = current_workflow.as_ref().and_then(|workflow| {
                                workflow
                                    .candidates
                                    .iter()
                                    .filter(|candidate| candidate.selected)
                                    .nth(*index)
                            });
                            if let (Some(workflow), Some(candidate)) =
                                (current_workflow.as_ref(), candidate)
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
                        Some(AssistantResource::Suggestion(index)) => {
                            if let (Some(workflow), Some(suggestion)) = (
                                current_workflow.as_ref(),
                                assistant_view.suggestions.get(*index),
                            ) {
                                let _ = worker.try_send(Intent::GetParagraphSuggestion {
                                    workflow_id: workflow.id.clone(),
                                    suggestion_id: suggestion.id.clone(),
                                });
                                editor_active = false;
                                confirmed_source_detail = None;
                            }
                        }
                        _ => {}
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
                                pending_source_job_kind = Some(SourceJobKind::Discovery);
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
                                pending_source_job_kind = Some(SourceJobKind::Discovery);
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
                            }
                        }
                        4 if can_mutate
                            && can_read_sources
                            && !sources_view.included.is_empty()
                            && no_source_job =>
                        {
                            if let Some(workflow) = &current_workflow {
                                sources_view.invalidate_preparation();
                                pending_source_job_kind = Some(SourceJobKind::Preparation);
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
                        5 if no_source_job && !source_detail_pending => {
                            request_inspect_source(
                                &worker,
                                &mut sources_view,
                                &current_workflow,
                                focus.resource_index,
                                &mut source_detail_pending,
                                &mut pending_source_job_kind,
                            );
                        }
                        6 if can_mutate && !source_mutation_pending && assistant_job.is_none() => {
                            if let (Some(workflow), Some(detail)) =
                                (&current_workflow, &sources_view.active_detail)
                            {
                                pending_assistant_job_kind =
                                    Some("assistant-source-refine".to_owned());
                                if worker
                                    .try_send(Intent::RefineSource {
                                        workflow_id: workflow.id.clone(),
                                        source_id: detail.id.clone(),
                                        revision: workflow.revision,
                                        digest: detail.content_digest.clone(),
                                    })
                                    .is_err()
                                {
                                    pending_assistant_job_kind = None;
                                }
                            }
                        }
                        7 | 8 if can_mutate && !source_mutation_pending => {
                            if let (Some(workflow), Some(suggestion)) =
                                (&current_workflow, &sources_view.active_refine_suggestion)
                            {
                                source_mutation_pending = worker
                                    .try_send(Intent::ResolveSourceRefine {
                                        workflow_id: workflow.id.clone(),
                                        source_id: suggestion.source_id.clone(),
                                        revision: workflow.revision,
                                        accept: focus.main_control_index == 7,
                                        digest: suggestion.binding_digest.clone(),
                                    })
                                    .is_ok();
                            }
                        }
                        9 if can_mutate && !source_mutation_pending => {
                            if let Some(detail) = &sources_view.active_detail {
                                source_editor = DraftEditor::new(&detail.body);
                                source_edit_target =
                                    Some((detail.id.clone(), detail.content_digest.clone()));
                                source_editor_active = true;
                                focus.zone = FocusZone::Main;
                            }
                        }
                        10 if can_mutate && !source_mutation_pending => {
                            if let Some(detail) = &sources_view.active_detail {
                                sources_view.pending_delete_confirmation = Some(detail.id.clone());
                            }
                        }
                        11 if can_mutate
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
                        12 if can_mutate
                            && no_source_job
                            && sources_view.included.is_empty()
                            && !source_confirmation_pending =>
                        {
                            empty_selection_confirmation = true;
                        }
                        13 if !source_page_pending => {
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
                        14 => current_stage = Stage::Start,
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
                        10 if can_mutate && assistant_job.is_none() => {
                            if let Some(workflow) = &current_workflow {
                                pending_assistant_job_kind =
                                    Some("assistant-memory-extraction".to_owned());
                                if worker
                                    .try_send(Intent::ExtractMemories {
                                        workflow_id: workflow.id.clone(),
                                        revision: workflow.revision,
                                    })
                                    .is_err()
                                {
                                    pending_assistant_job_kind = None;
                                }
                            }
                        }
                        11 if can_mutate && !memory_review_pending => {
                            if let (Some(workflow), Some(review)) =
                                (&current_workflow, &assistant_view.memory_review)
                                && let Some(candidate_set_id) = &review.candidate_set_id
                            {
                                memory_review_pending = worker
                                    .try_send(Intent::SaveMemoryReview {
                                        workflow_id: workflow.id.clone(),
                                        revision: workflow.revision,
                                        candidate_set_id: candidate_set_id.clone(),
                                        reviewed_memory: assistant_view
                                            .reviewed_memory_draft
                                            .clone(),
                                    })
                                    .is_ok();
                            }
                        }
                        12 if can_mutate && assistant_job.is_none() => {
                            if compose_view.is_dirty() || save_in_flight.is_some() {
                                save_requested = true;
                                feedback = Some(
                                    "Drafting waits until the local Draft is flushed; activate again after save"
                                        .to_owned(),
                                );
                            } else if let Some(workflow) = &current_workflow {
                                pending_assistant_job_kind = Some("assistant-draft".to_owned());
                                if worker
                                    .try_send(Intent::RequestDraft {
                                        workflow_id: workflow.id.clone(),
                                        revision: workflow.revision,
                                        include_prior_draft: !compose_view.text.is_empty(),
                                    })
                                    .is_err()
                                {
                                    pending_assistant_job_kind = None;
                                }
                            }
                        }
                        13 if can_mutate && assistant_job.is_none() => {
                            if compose_view.is_dirty() || save_in_flight.is_some() {
                                save_requested = true;
                                feedback = Some(
                                    "Paragraph review waits until the local Draft is flushed; activate again after save"
                                        .to_owned(),
                                );
                            } else if let (Some(workflow), Some((start, end, paragraph))) =
                                (&current_workflow, draft_editor.focused_paragraph())
                            {
                                pending_assistant_job_kind =
                                    Some("assistant-paragraph-review".to_owned());
                                let request = StartParagraphReviewRequest {
                                    buffer_etag: buffer_etag(compose_view.buffer_version),
                                    range_start: start,
                                    range_end: end,
                                    selected_text_digest: Digest::of_bytes(paragraph.as_bytes())
                                        .to_string(),
                                };
                                if worker
                                    .try_send(Intent::ReviewParagraph {
                                        workflow_id: workflow.id.clone(),
                                        revision: workflow.revision,
                                        request,
                                    })
                                    .is_err()
                                {
                                    pending_assistant_job_kind = None;
                                }
                            }
                        }
                        14 => {
                            if let Some((job_id, _)) = &assistant_job {
                                let _ = worker.try_send(Intent::CancelJob {
                                    job_id: job_id.clone(),
                                });
                            } else {
                                feedback = Some("No assistant action is running".to_owned());
                            }
                        }
                        15 => {
                            let _ = worker.try_send(Intent::GetAssistantBackends);
                            if let Some(workflow) = &current_workflow {
                                let _ = worker.try_send(Intent::GetMemoryReview {
                                    workflow_id: workflow.id.clone(),
                                });
                                let _ = worker.try_send(Intent::GetDraftResult {
                                    workflow_id: workflow.id.clone(),
                                });
                                let _ = worker.try_send(Intent::GetParagraphSuggestions {
                                    workflow_id: workflow.id.clone(),
                                });
                            }
                        }
                        16..=17 if can_mutate => {
                            if let (Some(workflow), Some(suggestion)) =
                                (&current_workflow, &assistant_view.active_suggestion)
                            {
                                if suggestion.stale || suggestion.resolution.is_some() {
                                    feedback = Some(
                                        "That suggestion is stale or already resolved; it is inspect-only"
                                            .to_owned(),
                                    );
                                } else {
                                    let request = ResolveParagraphSuggestionRequest {
                                        resolution: if focus.main_control_index == 16 {
                                            "accept"
                                        } else {
                                            "reject"
                                        }
                                        .to_owned(),
                                        buffer_etag: buffer_etag(suggestion.buffer_version),
                                        range_start: suggestion.range_start,
                                        range_end: suggestion.range_end,
                                        selected_text_digest: suggestion
                                            .selected_text_digest
                                            .clone(),
                                    };
                                    let _ = worker.try_send(Intent::ResolveParagraphSuggestion {
                                        workflow_id: workflow.id.clone(),
                                        suggestion_id: suggestion.id.clone(),
                                        revision: workflow.revision,
                                        request,
                                    });
                                }
                            }
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
                FocusZone::Main if current_stage == Stage::Preview => {
                    if can_mutate
                        && !local_action_pending
                        && let Some(workflow) = &current_workflow
                    {
                        match focus.main_control_index {
                            0 => {
                                local_action_pending = worker
                                    .try_send(Intent::PreviewLocal {
                                        workflow_id: workflow.id.clone(),
                                        revision: workflow.revision,
                                    })
                                    .is_ok();
                            }
                            1 if workflow.state == "output-planned" => {
                                if let Some(preview) = &local_preview {
                                    local_action_pending = worker
                                        .try_send(Intent::ApproveLocal {
                                            workflow_id: workflow.id.clone(),
                                            revision: workflow.revision,
                                            plan_digest: preview.plan_digest.clone(),
                                        })
                                        .is_ok();
                                }
                            }
                            2 if workflow.local_status.as_deref() == Some("conflict")
                                || local_preview
                                    .as_ref()
                                    .is_some_and(|preview| preview.local_status == "conflict") =>
                            {
                                conflict_day_draft.clear();
                                conflict_day_editing = true;
                                feedback = Some("Enter a different diary day".to_owned());
                            }
                            _ => {
                                feedback =
                                    Some("Preview the current plan before approval".to_owned())
                            }
                        }
                    }
                }
                FocusZone::Main
                    if current_stage == Stage::Publish && can_mutate && !local_action_pending =>
                {
                    if focus.main_control_index == 1 {
                        if let Some(job_id) = &local_publication_job {
                            let _ = worker.try_send(Intent::CancelJob {
                                job_id: job_id.clone(),
                            });
                        }
                    } else if local_publication_job.is_none()
                        && let Some(workflow) = &current_workflow
                    {
                        if workflow.state == "approved" || workflow.state == "partial" {
                            local_action_pending = worker
                                .try_send(Intent::PublishLocal {
                                    workflow_id: workflow.id.clone(),
                                    revision: workflow.revision,
                                })
                                .is_ok();
                        } else {
                            feedback = Some("No approved local write is pending".to_owned());
                        }
                    }
                }
                _ => {}
            },
            InputPoll::Event(_) => {}
        }
    }
}

/// Requests the body of a candidate for display, regardless of whether it's
/// included: activating a source and including it are independent actions
/// (product-behavior.md, ux-tui.md). An already bulk-prepared included
/// source reuses the confirm-bound handle; any other candidate gets an ad
/// hoc, single-source preview that never touches the bulk preparation
/// bookkeeping `confirm_selection` depends on.
#[allow(clippy::too_many_arguments)]
fn request_inspect_source(
    worker: &Worker,
    sources_view: &mut SourcesViewModel,
    current_workflow: &Option<WorkflowDto>,
    resource_index: usize,
    source_detail_pending: &mut bool,
    pending_source_job_kind: &mut Option<SourceJobKind>,
) {
    let Some(workflow) = current_workflow.as_ref() else {
        return;
    };
    let Some(candidate) = sources_view.candidates.get(resource_index) else {
        return;
    };
    if sources_view.included.contains(&candidate.id)
        && let Some(handle) = sources_view.latest_preparation_handle.clone()
    {
        if worker
            .try_send(Intent::SourceDetail {
                workflow_id: workflow.id.clone(),
                source_id: candidate.id.clone(),
                preparation_handle: handle,
            })
            .is_ok()
        {
            *source_detail_pending = true;
        }
        return;
    }
    let source_id = candidate.id.clone();
    sources_view.preview_source_id = Some(source_id.clone());
    *pending_source_job_kind = Some(SourceJobKind::Preview);
    if worker
        .try_send(Intent::PrepareSources {
            workflow_id: workflow.id.clone(),
            revision: workflow.revision,
            source_ids: vec![source_id],
        })
        .is_err()
    {
        sources_view.preview_source_id = None;
        *pending_source_job_kind = None;
    }
}

const START_CONTROL_COUNT: usize = 9;
const SOURCES_CONTROL_COUNT: usize = 15;
const COMPOSE_CONTROL_COUNT: usize = 18;
const REVIEW_CONTROL_COUNT: usize = 4;

#[allow(clippy::too_many_arguments)]
fn compose_screen_body(
    focus: &FocusModel,
    workflow: Option<&WorkflowDto>,
    compose: &ComposeViewModel,
    assistant: &AssistantViewModel,
    active_resource: Option<&AssistantResource>,
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
    let backend = workflow
        .and_then(|value| value.assistant_backend_id.as_deref())
        .unwrap_or("manual");
    let path = workflow
        .and_then(|value| value.composition_path.as_deref())
        .unwrap_or("manual");
    let save = if save_pending {
        "saving"
    } else if compose.is_dirty() {
        "unsaved"
    } else {
        "saved"
    };
    let mut body = format!(
        "Workflow: {state}\nWriting language: {language}\nStyle: {style}\nAssistant: {backend} · {path}\nDraft: {save}\n\n{} Language: US English\n{} Language: Korean\n{} Style: concise\n{} Style: standard\n{} Style: reflective\n{} Open manual Draft editor{}\n{} Save/flush Draft\n{} Keep local Draft after conflict\n{} Use server Draft after conflict\n{} Review complete entry\n{} Extract memories\n{} Confirm reviewed memories\n{} Draft with selected assistant\n{} Review focused paragraph\n{} Cancel assistant action\n{} Refresh assistant resources\n{} Accept current suggestion\n{} Reject current suggestion",
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
        marker(10),
        marker(11),
        marker(12),
        marker(13),
        marker(14),
        marker(15),
        marker(16),
        marker(17),
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
    if matches!(
        active_resource,
        Some(AssistantResource::MemoryReview | AssistantResource::MemoryCandidate(_))
    ) {
        body.push_str("\n\n--- Reviewed memories (editable selection) ---\n");
        body.push_str(&assistant.reviewed_memory_draft);
        body.push_str("\n\nActivate a candidate in Resources to include/exclude it, then Confirm reviewed memories.");
    }
    if let Some(draft) = &assistant.draft_result {
        body.push_str("\n\n--- Current assistant draft artifact");
        if draft.stale {
            body.push_str(" (stale; inspect only)");
        }
        body.push_str(" ---\n");
        body.push_str(&draft.text);
    }
    if let Some(suggestion) = &assistant.active_suggestion {
        body.push_str("\n\n--- Paragraph suggestion ---\n");
        body.push_str(&suggestion.replacement);
        if suggestion.stale {
            body.push_str("\n(stale; cannot apply)");
        }
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

fn preview_screen_body(focus: &FocusModel, preview: Option<&LocalPreviewDto>) -> String {
    let marker = |index| {
        if focus.zone == FocusZone::Main && focus.main_control_index == index {
            ">"
        } else {
            " "
        }
    };
    let mut text = format!(
        "{} Generate/refresh preview\n{} Approve exact plan\n{} Save conflict under another day\n\n",
        marker(0),
        marker(1),
        marker(2)
    );
    if let Some(preview) = preview {
        text.push_str(&format!(
            "Target: {}\nSHA-256: {}\nLocal: {}\nNotion: {}\n\n{}\n",
            preview.target,
            preview.payload_digest,
            preview.local_status,
            preview.notion_status,
            preview.payload
        ));
        for change in &preview.intended_changes {
            text.push_str(&format!("Planned: {change}\n"));
        }
        if let Some(existing) = &preview.existing_text {
            text.push_str(&format!("\nExisting target (conflict):\n{existing}\n"));
        }
    } else {
        text.push_str("Generate a read-only output preview to inspect the exact file.\n");
    }
    text
}

fn publish_screen_body(focus: &FocusModel, workflow: Option<&WorkflowDto>) -> String {
    let marker = |index| {
        if focus.zone == FocusZone::Main && focus.main_control_index == index {
            ">"
        } else {
            " "
        }
    };
    format!(
        "Local publication: {}\nNotion: {} (P1-M10)\n\n{} Publish approved local file\n{} Cancel pending publication",
        workflow
            .and_then(|w| w.local_status.as_deref())
            .unwrap_or("pending"),
        workflow
            .and_then(|w| w.notion_status.as_deref())
            .unwrap_or("blocked"),
        marker(0),
        marker(1)
    )
}

fn focus_item_count(focus: &FocusModel, resource_count: usize, control_count: usize) -> usize {
    match focus.zone {
        FocusZone::Workflow => ALL_STAGES.len(),
        FocusZone::Resources => resource_count,
        FocusZone::Main => control_count,
    }
}

fn focused_source_id(
    stage: Stage,
    focus: &FocusModel,
    sources: &SourcesViewModel,
) -> Option<String> {
    (stage == Stage::Sources)
        .then(|| sources.candidates.get(focus.resource_index))
        .flatten()
        .map(|candidate| candidate.id.clone())
}

/// The Resources row focused on entering `stage`: the source being shown in
/// Sources, the active draft resource in Compose, and the top row otherwise.
fn stage_entry_resource_index(
    stage: Stage,
    sources: &SourcesViewModel,
    compose_active_resource: usize,
) -> usize {
    match stage {
        Stage::Sources => sources.active_candidate_index().unwrap_or(0),
        Stage::Compose => compose_active_resource,
        _ => 0,
    }
}

/// Picks the one job to poll next. Recovery reads come first because
/// mutations stay disabled until they land; source and assistant jobs then
/// alternate so one long-running job cannot starve the other's polling.
fn next_job_poll<'a>(
    recovery: Option<&'a str>,
    source: Option<&'a str>,
    assistant: Option<&'a str>,
    prefer_assistant: bool,
) -> Option<&'a str> {
    recovery.or(if prefer_assistant {
        assistant.or(source)
    } else {
        source.or(assistant)
    })
}

/// Keeps the Resources Sidebar cursor on the same source after a canonical
/// reload or page response replaces the candidate vector. If that source was
/// removed, or another stage is shown (no focused source), focus is left
/// alone: the per-frame clamp bounds it with the current stage's counts,
/// whereas clamping here with Sources' counts would move Compose's focus.
fn reanchor_source_focus(
    focus: &mut FocusModel,
    sources: &SourcesViewModel,
    previous_source_id: Option<&str>,
) {
    if let Some(index) = previous_source_id.and_then(|source_id| {
        sources
            .candidates
            .iter()
            .position(|candidate| candidate.id == source_id)
    }) {
        focus.resource_index = index;
    }
}

/// Picks the Main Pane's scroll offset for this frame. Opening a source's
/// text (or a refinement suggestion) resets the view to the top so freshly
/// revealed content starts predictably rather than at whatever offset a
/// previously shown source left behind. While that text is open, its own
/// scrolling governs the viewport: the focused Main-zone control's header
/// line (e.g. "Inspect active prepared source") must not keep pinning the
/// view back up and away from the body text rendered below it, which would
/// make the viewport snap back every frame the moment a person tries to
/// page down into the text.
fn resolve_main_scroll(
    stage: Stage,
    sources_view: &SourcesViewModel,
    body: &str,
    current: u16,
    viewport_height: u16,
    shown_source_detail_id: &mut Option<String>,
) -> u16 {
    let current_detail_id = sources_view.active_detail.as_ref().map(|detail| &detail.id);
    let mut scroll = current;
    if current_detail_id != shown_source_detail_id.as_ref() {
        *shown_source_detail_id = current_detail_id.cloned();
        scroll = 0;
    }
    let viewing_source_text = stage == Stage::Sources
        && (sources_view.active_detail.is_some()
            || sources_view.active_refine_suggestion.is_some());
    if viewing_source_text {
        scroll
    } else {
        ensure_selected_line_visible(body, scroll, viewport_height)
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
    preferences: Option<&PreferencesDto>,
    assistant: &AssistantViewModel,
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
    let selected_assistant = preferences
        .and_then(|value| value.last_assistant_backend_id.as_deref())
        .and_then(|id| assistant.backends.iter().find(|backend| backend.id == id))
        .map_or_else(
            || "manual".to_owned(),
            |backend| {
                format!(
                    "{} · {} ({})",
                    backend.display_label,
                    backend.composition_path,
                    if backend.available {
                        "available"
                    } else {
                        "unavailable"
                    }
                )
            },
        );
    let mut body = format!(
        "Setup: {setup_label}\nManual workflow: {manual}\nAssistant: {selected_assistant}\nActions: {action_state}\nSelected workflow: {selected}\n\n{} Day: {day}\n{} Start/resume chosen day{}\n{} Resume selected workflow\n{} Discard selected workflow{}\n{} Load more workflows{}\n{} Retry/refresh\n{} Help\n{} Save/flush (Compose only)\n{} Quit",
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
            "\n\nTab/Shift-Tab or Left/Right changes focus. Up/Down chooses an item. Enter activates it. Ctrl-G opens Command Line; Ctrl-P/Ctrl-N changes stage; Ctrl-C twice quits.",
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
        "Active candidate: {active_name}\nDirectory date: {directory_day}\nHeader date: {header_day}\nAvailability: {availability}\nWarning: {warning}\nIncluded/reviewed: {}/{}\nWork: {progress}\n\n{} Discover three-day window\n{} Extra date: {extra_day}\n{} Discover extra date\n{} Include/exclude active candidate\n{} Prepare included sources\n{} Inspect active prepared source\n{} Refine inspected source with assistant\n{} Approve refinement\n{} Reject refinement\n{} Edit inspected source (Ctrl-S saves)\n{} Delete inspected source and linked audio\n{} Confirm reviewed source set\n{} Continue without notes\n{} Load more candidates{}\n{} Back to Start",
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
        marker(9),
        marker(10),
        marker(11),
        marker(12),
        marker(13),
        if sources.next_cursor.is_some() {
            ""
        } else {
            " (no more)"
        },
        marker(14),
    );
    if let Some(detail) = &sources.active_detail {
        body.push_str("\n\n--- Source text ---\n");
        body.push_str(&detail.body);
    }
    if let Some(suggestion) = &sources.active_refine_suggestion {
        body.push_str("\n\n--- Suggested refinement ---\n");
        body.push_str(&suggestion.suggested_body);
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
            None,
            &AssistantViewModel::default(),
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

    fn sources_view_with_detail(id: &str) -> SourcesViewModel {
        let mut view = SourcesViewModel::default();
        view.apply_source_detail(ClientOutcome::Ok(diary_tui::client::SourceDetailResponse {
            detail: diary_control_contract::SourceDetailDto {
                id: id.to_owned(),
                source_note_name: "note".to_owned(),
                body: "synthetic body".to_owned(),
                content_digest: "digest".to_owned(),
                directory_day: None,
                header_day: None,
                warning_code: None,
            },
            preparation_handle: None,
        }));
        view
    }

    #[test]
    fn opening_source_text_stops_the_focused_control_from_pinning_the_scroll_back() {
        // A line starting with '>' near the top, as the Sources header
        // always renders for whichever Main-zone control is focused.
        let body = "> Inspect active prepared source\nmore header\n\n--- Source text ---\nfirst body line\nsecond body line";
        let sources = sources_view_with_detail("src_1");
        let mut shown = Some("src_1".to_owned());
        // The person paged down to read the body; before the fix this would
        // snap back up toward the '>' line every frame.
        let scroll = resolve_main_scroll(Stage::Sources, &sources, body, 4, 3, &mut shown);
        assert_eq!(scroll, 4);
    }

    #[test]
    fn opening_a_different_source_resets_the_scroll_to_the_top() {
        let sources = sources_view_with_detail("src_2");
        let mut shown = Some("src_1".to_owned());
        let scroll = resolve_main_scroll(Stage::Sources, &sources, "body", 7, 3, &mut shown);
        assert_eq!(scroll, 0);
        assert_eq!(shown, Some("src_2".to_owned()));
    }

    #[test]
    fn canonical_source_reordering_keeps_focus_on_the_same_source() {
        let mut sources = SourcesViewModel {
            candidates: vec![
                diary_control_contract::SourceCandidateDto {
                    id: "src_1".to_owned(),
                    source_note_name: "first".to_owned(),
                    directory_day: None,
                    header_day: None,
                    warning_code: None,
                    availability: "local".to_owned(),
                    selected: false,
                },
                diary_control_contract::SourceCandidateDto {
                    id: "src_2".to_owned(),
                    source_note_name: "second".to_owned(),
                    directory_day: None,
                    header_day: None,
                    warning_code: None,
                    availability: "local".to_owned(),
                    selected: false,
                },
            ],
            ..SourcesViewModel::default()
        };
        let mut focus = FocusModel {
            zone: FocusZone::Resources,
            resource_index: 0,
            ..FocusModel::default()
        };
        let focused_id = focused_source_id(Stage::Sources, &focus, &sources);

        sources.candidates.swap(0, 1);
        reanchor_source_focus(&mut focus, &sources, focused_id.as_deref());

        assert_eq!(focus.resource_index, 1);
        assert_eq!(sources.candidates[focus.resource_index].id, "src_1");
    }

    #[test]
    fn entering_a_stage_focuses_its_own_resource_not_the_previous_stages_row() {
        let shown = sources_view_with_detail("src_2");
        let mut sources = SourcesViewModel {
            candidates: ["src_1", "src_2"]
                .into_iter()
                .map(|id| diary_control_contract::SourceCandidateDto {
                    id: id.to_owned(),
                    source_note_name: id.to_owned(),
                    directory_day: None,
                    header_day: None,
                    warning_code: None,
                    availability: "local".to_owned(),
                    selected: false,
                })
                .collect(),
            ..SourcesViewModel::default()
        };
        assert_eq!(stage_entry_resource_index(Stage::Sources, &sources, 4), 0);
        sources.active_detail = shown.active_detail;
        assert_eq!(stage_entry_resource_index(Stage::Sources, &sources, 4), 1);
        assert_eq!(stage_entry_resource_index(Stage::Compose, &sources, 4), 4);
        assert_eq!(stage_entry_resource_index(Stage::Start, &sources, 4), 0);
    }

    #[test]
    fn job_polls_put_recovery_first_and_alternate_source_and_assistant() {
        assert_eq!(
            next_job_poll(Some("recovery"), Some("source"), Some("assistant"), true),
            Some("recovery")
        );
        assert_eq!(
            next_job_poll(None, Some("source"), Some("assistant"), false),
            Some("source")
        );
        assert_eq!(
            next_job_poll(None, Some("source"), Some("assistant"), true),
            Some("assistant")
        );
        assert_eq!(
            next_job_poll(None, Some("source"), None, true),
            Some("source")
        );
        assert_eq!(next_job_poll(None, None, None, false), None);
    }

    #[test]
    fn a_source_reload_outside_sources_leaves_that_stages_focus_alone() {
        let sources = SourcesViewModel::default();
        let mut focus = FocusModel {
            zone: FocusZone::Main,
            resource_index: 3,
            main_control_index: 17,
            ..FocusModel::default()
        };
        let focused_id = focused_source_id(Stage::Compose, &focus, &sources);

        reanchor_source_focus(&mut focus, &sources, focused_id.as_deref());

        assert_eq!(focus.resource_index, 3);
        assert_eq!(focus.main_control_index, 17);
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
        for action in ["Refine", "Approve", "Reject", "Edit", "Delete"] {
            assert!(body.contains(action), "missing Sources action: {action}");
        }
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
            local_status: None,
            notion_status: None,
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
        let (resources, kinds) = AssistantViewModel::default().resources(Some(&workflow), true);
        assert_eq!(resources.rows.len(), 3);
        assert_eq!(resources.rows[0].label, "Draft");
        assert_eq!(resources.rows[0].state, "unsaved");
        assert_eq!(resources.rows[2].label, "selected.md");
        assert!(matches!(kinds[2], AssistantResource::ConfirmedSource(0)));
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
            &AssistantViewModel::default(),
            Some(&AssistantResource::Draft),
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

    #[test]
    fn preview_and_publish_show_exact_file_conflict_and_separate_statuses() {
        let mut workflow = composing_workflow();
        workflow.local_status = Some("conflict".to_owned());
        workflow.notion_status = Some("blocked".to_owned());
        let preview = LocalPreviewDto {
            workflow: workflow.clone(),
            target: "/tmp/entry.md".to_owned(),
            payload: "---\ndate: \"2026-10-29\"\n---\n\nplanned".to_owned(),
            payload_digest: "digest".to_owned(),
            plan_digest: "plan".to_owned(),
            existing_text: Some("existing content".to_owned()),
            intended_changes: Vec::new(),
            local_status: "conflict".to_owned(),
            notion_status: "blocked".to_owned(),
        };
        let focus = FocusModel {
            zone: FocusZone::Main,
            main_control_index: 2,
            ..FocusModel::default()
        };
        let body = preview_screen_body(&focus, Some(&preview));
        assert!(body.contains("planned"));
        assert!(body.contains("existing content"));
        assert!(body.contains("> Save conflict under another day"));
        assert!(
            publish_screen_body(&focus, Some(&workflow))
                .contains("Local publication: conflict\nNotion: blocked")
        );
    }
}
