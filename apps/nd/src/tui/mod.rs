//! Terminal client orchestration and presentation.

use super::*;

/// Dispatches to the real interactive TUI when stdout is a tty, or to a
/// one-shot headless path otherwise. The headless path exists because an
/// automated harness (the `command_shells`/`tui_attachment` integration
/// tests) has no terminal to render a persistent retry screen into; a
/// real interactive session must never exit just because the server
/// is not running yet (see AGENTS.md "Server lifecycle").
pub(super) fn tui_mode() -> ExitCode {
    if std::io::stdout().is_terminal() {
        tui_mode_interactive()
    } else {
        tui_mode_headless()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AttachmentFailure {
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
) -> Result<ClientAttachment, AttachmentFailure> {
    let control_attachment = match load_runtime_attachment(runtime_dir, expected_uid) {
        Ok(attachment) => attachment,
        Err(LifecycleError::MissingRuntime) => return Err(AttachmentFailure::NotRunning),
        Err(LifecycleError::IncompatibleContract) => {
            return Err(AttachmentFailure::VersionMismatch);
        }
        Err(_) => return Err(AttachmentFailure::Unsafe),
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
        AttachmentState::NotRunning => Err(AttachmentFailure::NotRunning),
        AttachmentState::InstanceMismatch => Err(AttachmentFailure::InstanceMismatch),
        AttachmentState::VersionMismatch => Err(AttachmentFailure::VersionMismatch),
        AttachmentState::UnsafeAttachment => Err(AttachmentFailure::Unsafe),
    }
}

fn attachment_failure_message(status: AttachmentFailure) -> &'static str {
    match status {
        AttachmentFailure::NotRunning => {
            "The Control API Server is not running. Start it in another terminal with: nd serve"
        }
        AttachmentFailure::Unsafe => "The server responded unsafely; refusing to attach.",
        AttachmentFailure::VersionMismatch => {
            "This client does not support the running server's contract version."
        }
        AttachmentFailure::InstanceMismatch => {
            "A different server instance is now running; reattaching."
        }
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
                draw_connection_screen(
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
            Ok(client_attachment) => {
                match run_connected_session(&mut guard, &client_attachment, |bytes| {
                    Digest::of_bytes(bytes).to_string()
                }) {
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
                }
            }
            Err(status) => {
                draw_connection_screen(&mut guard, attachment_failure_message(status), quit_armed);
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

fn draw_connection_screen(guard: &mut TerminalGuard, message: &str, quit_armed: bool) {
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

use diary_tui::session_runtime::{SessionEnd, run_connected_session};

fn tui_mode_headless() -> ExitCode {
    let (runtime_dir, expected_uid) = match resolve_runtime_location() {
        Ok(value) => value,
        Err(code) => return fail_tui(code),
    };
    let client = ApiClient::new(UreqTransport::new());
    let client_attachment = match attempt_attach(&runtime_dir, expected_uid, &client) {
        Ok(attachment) => attachment,
        Err(AttachmentFailure::NotRunning) => return not_running(),
        Err(status) => {
            eprintln!("nd: {}", attachment_failure_message(status));
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
