#![forbid(unsafe_code)]

use std::env;
use std::ffi::OsString;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::process::{Command, ExitCode};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use diary_connectors::{AppleBridgeClient, AyumiCatalog, AyumiSource, SqliteStore};
use diary_control::{
    CommandService, DiagnosticEvent, DiagnosticLevel, DiagnosticLog, LifecycleConfig, LogClock,
    RuntimeAdvertisement, ServerConfig, ServerLock, bind_loopback, random_id, serve,
};
use diary_control_contract::CONTRACT_VERSION;
use diary_core::{Clock, DiaryDay, OperationLog, Timestamp, WorkflowStore};
use tokio::sync::{mpsc, oneshot};

const HELP: &str = "\
nd — Narihito's Diary

Usage:
  nd           Open the TUI (not implemented yet)
  nd serve     Start the Control API Server
  nd --help    Show this help
";

fn main() -> ExitCode {
    run(env::args_os().skip(1))
}

fn run(mut arguments: impl Iterator<Item = OsString>) -> ExitCode {
    match (arguments.next(), arguments.next()) {
        (None, None) => not_implemented("default TUI mode"),
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

fn not_implemented(mode: &str) -> ExitCode {
    eprintln!("nd: {mode} is not implemented yet");
    ExitCode::FAILURE
}

fn fail(code: &str) -> ExitCode {
    eprintln!("nd serve: {code}");
    ExitCode::FAILURE
}
