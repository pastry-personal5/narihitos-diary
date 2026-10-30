#![forbid(unsafe_code)]

use std::env;
use std::ffi::OsString;
use std::io::{IsTerminal, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::process::{Command, ExitCode};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use diary_connectors::{
    AppleBridgeClient, AppleFoundationAssistant, AyumiCatalog, AyumiSource, LocalOutput,
    MigrationEvent, MigrationObserver, NotionClient, SqliteStore,
};
use diary_control::{
    CodexAssistant, CommandError, CommandService, DiagnosticEvent, DiagnosticLevel, DiagnosticLog,
    LifecycleConfig, LifecycleError, LogClock, RuntimeAdvertisement, ServerConfig, ServerLock,
    bind_loopback, load_or_create_tag_key, load_runtime_attachment, random_id, serve,
};
use diary_control_contract::CONTRACT_VERSION;
use diary_core::{
    AssistantBackendId, Clock, DiaryDay, Digest, DraftAssistant, DraftAssistantRegistry,
    OperationLog, Timestamp, WorkflowStore,
};
use diary_tui::client::{
    ApiClient, Attachment as ClientAttachment, AttachmentState, UreqTransport, verify_health,
};
use diary_tui::command_line_view::CommandLineWidget;
use diary_tui::focus::FocusZone;
use diary_tui::ime::ImeState;
use diary_tui::layout::adaptive_areas;
use diary_tui::main_pane::MainPaneWidget;
use diary_tui::sidebar::WorkflowSidebarWidget;
use diary_tui::stage::{Stage, map_workflow};
use diary_tui::status_line::{StatusLineModel, StatusLineWidget};
use diary_tui::terminal::TerminalGuard;
use diary_tui::theme::Palette;
use diary_tui::work_status::{WorkStatusModel, WorkStatusWidget};
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

/// Runs one startup recovery pass off the async runtime, with Debug-level
/// `running`/`completed` records and a `failed` record whose reason is the
/// exact diagnostic token. The same token is the startup error.
async fn run_startup_recovery(
    log: &DiagnosticLog,
    kind: &'static str,
    task_failed: &'static str,
    recover: impl FnOnce() -> Result<(), CommandError> + Send + 'static,
) -> Result<(), &'static str> {
    let record = |level, event, reason| {
        log.write(
            level,
            DiagnosticEvent::Job {
                kind,
                event,
                reason,
            },
        )
        .map_err(|_| "diagnostic-write-failed")
    };
    record(DiagnosticLevel::Debug, "running", None)?;
    let outcome = tokio::task::spawn_blocking(recover)
        .await
        .map_err(|_| task_failed)?;
    match outcome {
        Ok(()) => record(DiagnosticLevel::Debug, "completed", None),
        Err(error) => {
            let reason = error.diagnostic_code();
            record(DiagnosticLevel::Warn, "failed", Some(reason))?;
            Err(reason)
        }
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
    let mut commands = CommandService::new_with_writer(
        store.clone(),
        source.clone(),
        source.clone(),
        source,
        clock.clone(),
    )
    .with_local_output(Arc::new(
        LocalOutput::new(lifecycle.entries_root.clone()).map_err(|_| "entry-root-invalid")?,
    ));
    if let Some(notion) = lifecycle
        .notion_runtime_config()
        .map_err(configuration_error)?
    {
        commands = commands.with_notion_output(Arc::new(
            NotionClient::new(notion.token(), notion.root_page_id())
                .map_err(|_| "notion-config-invalid")?,
        ));
    }
    let recovery_commands = commands.clone();
    run_startup_recovery(
        &log,
        "startup-recovery-local",
        "publication-recovery-task-failed",
        move || recovery_commands.reconcile_local_intents(),
    )
    .await?;
    if let Some(root) = &lifecycle.icloud_entries_root {
        let home = env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or("home-unavailable")?;
        commands = commands.with_mirror_output(Arc::new(
            LocalOutput::new_icloud(root.clone(), &home).map_err(|_| "icloud-root-invalid")?,
        ));
        let recovery_commands = commands.clone();
        run_startup_recovery(
            &log,
            "startup-recovery-mirror",
            "mirror-recovery-task-failed",
            move || recovery_commands.reconcile_mirror_intents(),
        )
        .await?;
    }

    let recovery_commands = commands.clone();
    run_startup_recovery(
        &log,
        "startup-recovery-notion",
        "notion-recovery-task-failed",
        move || recovery_commands.reconcile_notion_intents(),
    )
    .await?;

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

mod tui;
use tui::tui_mode;

fn fail(code: &str) -> ExitCode {
    eprintln!("nd serve: {code}");
    ExitCode::FAILURE
}
