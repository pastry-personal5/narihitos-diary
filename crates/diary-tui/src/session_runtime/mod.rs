//! Terminal session state and worker-driven dispatch.

use std::collections::BTreeSet;
use std::env;
use std::time::{Duration, Instant};

use diary_control_contract::{
    CapabilitiesDto, JOB_NOT_FOUND, LocalPreviewDto, PreferencesDto, RemoteDto, RemotePreviewDto,
    ResolveParagraphSuggestionRequest, SetupCheckDto, SourceDetailDto, StartParagraphReviewRequest,
    WorkflowDto, buffer_etag, job_status,
};

use crate::amendment_view;
use crate::assistant_view::{AssistantResource, AssistantViewModel};
use crate::client::{Attachment as ClientAttachment, ClientOutcome, UreqTransport};
use crate::command_line_view::CommandLineWidget;
use crate::compose_view::ComposeViewModel;
use crate::editor::{DraftEditor, DraftEditorPane};
use crate::events::{EventWorker, UreqEventSource};
use crate::focus::{FocusModel, FocusZone, SidebarKind};
use crate::help_overlay::{HelpOverlayWidget, overlay_area};
use crate::ime::{ImeState, ImeTracker, MacInputLanguageNotifications, MacInputLanguageQuery};
use crate::layout::{OverlayClearWidget, adaptive_areas};
use crate::main_pane::{MainPaneWidget, indent_diary_block};
use crate::readiness_view::{CheckedReadiness, ReadinessView};
use crate::remote_view::{RemoteControl, existing_entry_text, remote_screen_body};
use crate::resources::{ResourceRow, ResourcesSidebarModel, ResourcesSidebarWidget};
use crate::review_view::{ReviewViewModel, can_confirm, first_unresolved_delimiter};
use crate::screen_body::*;
use crate::session::{SessionAction, SessionAuthority, reduce_stream_result};
use crate::sidebar::WorkflowSidebarWidget;
use crate::source_shortcuts::{ShortcutContext, help_lines, shortcut_for};
use crate::sources_menu::{
    SourcesAction, action_item, item_action, shortcut_action, visible_items,
};
use crate::sources_view::{JobOutcome, SourceJobKind, SourcesViewModel};
use crate::stage::{
    ALL_STAGES, Progress, Stage, adjacent_accessible_stage, map_workflow, remote_available,
};
use crate::stage_tip::{TipInput, next_step_tip};
use crate::start_view::StartViewModel;
use crate::status_line::{StatusLineModel, StatusLineWidget};
use crate::terminal::TerminalGuard;
use crate::theme::Palette;
use crate::widgets::{
    AssistantWidgetModel, DiaryDayWidgetModel, UtilitySidebarModel, UtilitySidebarWidget,
    read_only_reason, selection_request,
};
use crate::work_status::{WorkStatusModel, WorkStatusWidget};
use crate::worker::{Intent, Worker, WorkerResult};

mod support;
pub use support::*;

// Connected terminal session and worker-driven event loop.

/// Runs the connected shell until the person quits or a render fails.
/// Returns `true` for a clean quit, `false` for a render failure.
///
/// Per the architecture doc ("the event/render loop never performs HTTP...
/// work"), every network operation goes through the bounded [`Worker`]:
/// this function only sends `Intent`s with `try_send` and drains
/// `WorkerResult`s with `try_recv`, never calling `ApiClient` directly.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionEnd {
    Quit,
    Reattach,
    RenderFailed,
}

pub fn run_connected_session(
    guard: &mut TerminalGuard,
    client_attachment: &ClientAttachment,
    paragraph_digest: fn(&[u8]) -> String,
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
    let mut session_language: Option<String> = None;
    let mut session_style: Option<String> = None;
    let mut current_workflow: Option<WorkflowDto> = None;
    let mut local_preview: Option<LocalPreviewDto> = None;
    // Remotes of a completed day. Looking at one is automatic and read-only;
    // readiness is asked for, never automatic, and its answer belongs to one
    // remote of one workflow revision.
    let mut remotes: Vec<RemoteDto> = Vec::new();
    let mut remote_preview: Option<RemotePreviewDto> = None;
    // What the Publish Remote stage already asked the server about.
    let mut remote_requested: Option<(String, String)> = None;
    let mut remotes_listed_for: Option<String> = None;
    let mut remote_loading = false;
    let mut remote_push_pending = false;
    let mut remote_overwrite_confirmation: Option<(String, String)> = None;
    let mut readiness_checked: Option<CheckedReadiness> = None;
    let mut readiness_pending = false;
    let mut amendment_preview: Option<diary_control_contract::AmendmentPreviewDto> = None;
    let mut local_action_pending = false;
    let mut local_publication_job: Option<String> = None;
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
    let mut preview_detail_pending = false;
    let mut source_confirmation_pending = false;
    let mut source_mutation_pending = false;
    let mut source_editor_active = false;
    let mut empty_selection_confirmation = false;
    let mut overwrite_confirmation = false;
    // Scroll offset of the open Sources Help Overlay; `None` when closed.
    let mut sources_help: Option<u16> = None;
    // Workflows whose first automatic discovery was already attempted, so a
    // failed discovery is not retried on its own.
    let mut auto_discovery_attempted: BTreeSet<String> = BTreeSet::new();
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
    let mut command_line = crate::command_line::CommandLineState::default();
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
            let effect = reduce_stream_result(&mut authority, result);
            if let Some(notice) = effect.notice {
                feedback = Some(notice.to_owned());
            }
            match effect.action {
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
                            session_language = workflow.language.clone();
                            session_style = Some(workflow.style.clone());
                            selected_day = Some(workflow.diary_day.clone());
                            local_preview = None;
                            remote_preview = None;
                            remote_requested = None;
                            remotes.clear();
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
                            if remote_push_pending {
                                // Look at the remote again either way, so what
                                // is shown is what is there now.
                                remote_push_pending = false;
                                remote_loading = false;
                                remote_requested = None;
                                remotes_listed_for = None;
                                feedback = Some(match &outcome {
                                    ClientOutcome::Ok(job)
                                        if job.status == job_status::COMPLETED =>
                                    {
                                        "Pushed".to_owned()
                                    }
                                    ClientOutcome::Ok(job)
                                        if job.error_code.as_deref()
                                            == Some("precondition-failed") =>
                                    {
                                        "The remote changed; showing what is there now".to_owned()
                                    }
                                    ClientOutcome::Ok(job) => format!(
                                        "Push did not finish: {}",
                                        job.error_code.as_deref().unwrap_or("unknown")
                                    ),
                                    _ => "Push status is unknown; showing what is there now"
                                        .to_owned(),
                                });
                            } else {
                                feedback = Some(match &outcome {
                                    ClientOutcome::Ok(job)
                                        if job.error_code.as_deref()
                                            == Some("precondition-failed") =>
                                    {
                                        "The file changed after you approved. Open Preview to see it again."
                                            .to_owned()
                                    }
                                    _ => "Local publication finished; reloading status"
                                        .to_owned(),
                                });
                            }
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
                        source_confirmation_pending = false;
                        feedback = Some(
                            "Source job is no longer known to the server; review and retry"
                                .to_owned(),
                        );
                    } else if is_source {
                        let job_kind = match &outcome {
                            ClientOutcome::Ok(job) => Some(job.kind.clone()),
                            _ => None,
                        };
                        let failed_source_error = match &outcome {
                            ClientOutcome::Ok(job) => job.error_code.clone(),
                            _ => None,
                        };
                        let is_preview_job = job_id.is_some()
                            && job_id.as_deref() == sources_view.pending_preview_job.as_deref();
                        let status = sources_view.apply_job_status(outcome);
                        if let Some(error) = &sources_view.last_error {
                            feedback = Some(error.safe_label());
                        }
                        match (job_kind.as_deref(), status) {
                            _ if is_preview_job && status == Some(JobOutcome::Succeeded) => {
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
                            _ if is_preview_job && status == Some(JobOutcome::Failed) => {
                                sources_view.preview_source_id = None;
                                feedback = Some("Preview failed; review and retry".to_owned());
                            }
                            (Some("discovery"), Some(JobOutcome::Succeeded)) => {
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
                            (Some("source-preparation"), Some(JobOutcome::Succeeded)) => {
                                if let (Some(workflow), Some((handle, source_ids))) = (
                                    current_workflow.as_ref(),
                                    sources_view.take_confirm_request(),
                                ) {
                                    if worker
                                        .try_send(Intent::ConfirmSelection {
                                            workflow_id: workflow.id.clone(),
                                            revision: workflow.revision,
                                            preparation_handle: handle,
                                            source_ids,
                                        })
                                        .is_ok()
                                    {
                                        source_confirmation_pending = true;
                                    } else {
                                        sources_view.finish_confirm();
                                        feedback = Some(
                                            "Confirm Source List could not be sent; try again"
                                                .to_owned(),
                                        );
                                    }
                                }
                            }
                            (_, Some(JobOutcome::Failed)) => {
                                if sources_view.confirming() {
                                    sources_view.finish_confirm();
                                    feedback = Some(match failed_source_error {
                                        Some(code) => format!(
                                            "Confirm Source List stopped: {code}; nothing was confirmed"
                                        ),
                                        None => "Confirm Source List stopped: an included note could not be read; nothing was confirmed".to_owned(),
                                    });
                                } else {
                                    feedback =
                                        Some("Source job failed; review and retry".to_owned());
                                }
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
                    }
                }
                WorkerResult::SourceRefineSuggestion(outcome) => match outcome {
                    ClientOutcome::Ok(value) => {
                        let shown = sources_view
                            .candidate_at(
                                focus
                                    .resource_index
                                    .min(sources_view.visible_len().saturating_sub(1)),
                            )
                            .map(|candidate| candidate.id.as_str());
                        if shown == Some(value.source_id.as_str()) {
                            sources_view.active_refine_suggestion = Some(value);
                            feedback =
                                Some("Source refinement is ready: Accept or Reject it".to_owned());
                        } else {
                            feedback = Some(
                                "A refinement finished for a note that is no longer shown; run Refine again"
                                    .to_owned(),
                            );
                        }
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
                    // The server consumed the single-use handle either way.
                    sources_view.finish_confirm();
                    match *outcome {
                        ClientOutcome::Ok(workflow) => {
                            sources_view.load_candidates_from(&workflow);
                            compose_view.load_from_canonical(&workflow);
                            draft_editor = DraftEditor::new(&compose_view.text);
                            current_stage = Stage::Compose;
                            current_workflow = Some(workflow);
                            feedback = Some("Source list confirmed".to_owned());
                        }
                        ClientOutcome::Problem(problem) => {
                            feedback = Some(format!(
                                "Confirm Source List failed: {}; nothing was confirmed",
                                problem.safe_label()
                            ))
                        }
                        ClientOutcome::Transport(error) => {
                            feedback = Some(format!(
                                "Confirm Source List failed: {}; nothing was confirmed",
                                error.safe_label()
                            ))
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
                            let backend = workflow.assistant_backend_id.clone();
                            session_language = language.clone();
                            session_style = Some(workflow.style.clone());
                            compose_view.load_from_canonical(&workflow);
                            current_workflow = Some(workflow);
                            preferences = Some(PreferencesDto {
                                last_language: language,
                                last_assistant_backend_id: backend,
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
                                Some(crate::review_view::ReviewError::Problem(problem));
                        }
                        ClientOutcome::Transport(error) => {
                            feedback = Some(error.safe_label().to_owned());
                            review_view.last_error =
                                Some(crate::review_view::ReviewError::TransportFailure(error));
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
                            current_stage = Stage::PublishLocal;
                            feedback = Some("Exact output plan approved".to_owned());
                        }
                        // The file changed since it was previewed: show it
                        // again instead of leaving the approval stuck.
                        ClientOutcome::Problem(problem)
                            if problem.code == "precondition-failed"
                                && current_workflow.is_some() =>
                        {
                            if let Some(workflow) = &current_workflow {
                                local_action_pending = worker
                                    .try_send(Intent::PreviewLocal {
                                        workflow_id: workflow.id.clone(),
                                        revision: workflow.revision,
                                    })
                                    .is_ok();
                            }
                            feedback =
                                Some("The file changed; showing it again to approve".to_owned());
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
                WorkerResult::RemotesListed(outcome) => match *outcome {
                    ClientOutcome::Ok(list) => {
                        remotes = list.remotes;
                        remotes_listed_for = current_workflow
                            .as_ref()
                            .map(|workflow| workflow.id.clone());
                    }
                    ClientOutcome::Problem(problem) => feedback = Some(problem.safe_label()),
                    ClientOutcome::Transport(error) => {
                        feedback = Some(error.safe_label().to_owned())
                    }
                },
                WorkerResult::RemotePreviewed(outcome) => {
                    remote_loading = false;
                    match *outcome {
                        ClientOutcome::Ok(preview) => {
                            current_workflow = Some(preview.workflow.clone());
                            if let Some(remote) =
                                remotes.iter_mut().find(|remote| remote.id == preview.id)
                            {
                                remote.status = preview.status.clone();
                                remote.reason = preview.reason.clone();
                            }
                            feedback = Some(match preview.status.as_str() {
                                "exists" if preview.id == "icloud" => {
                                    "iCloud already holds an entry; choose Overwrite".to_owned()
                                }
                                "exists" => format!(
                                    "{} already holds an entry; choose Append or Overwrite",
                                    preview.label
                                ),
                                "blocked" => {
                                    format!("{} cannot be pushed to right now", preview.label)
                                }
                                "pushed" => format!("{} already has this entry", preview.label),
                                _ => format!("{} plan ready; inspect it, then Push", preview.label),
                            });
                            remote_preview = Some(preview);
                        }
                        ClientOutcome::Problem(problem) => {
                            if problem.code == "precondition-failed" {
                                remote_requested = None;
                                remotes_listed_for = None;
                                if let Some(workflow) = &current_workflow {
                                    let _ = worker.try_send(Intent::GetWorkflow {
                                        workflow_id: workflow.id.clone(),
                                    });
                                }
                            }
                            feedback = Some(problem.safe_label());
                        }
                        ClientOutcome::Transport(error) => {
                            feedback = Some(error.safe_label().to_owned())
                        }
                    }
                }
                WorkerResult::RemoteChecked(outcome) => {
                    readiness_pending = false;
                    match *outcome {
                        ClientOutcome::Ok(report) => {
                            // A repair in Notion may have cleared a blocked
                            // page. Refresh the menu and sidebar after a check.
                            remote_requested = None;
                            remotes_listed_for = None;
                            readiness_checked = current_workflow.as_ref().map(|workflow| {
                                (
                                    workflow.id.clone(),
                                    report.readiness.destination.clone(),
                                    report,
                                )
                            });
                            feedback = Some("Readiness checked".to_owned());
                        }
                        ClientOutcome::Problem(problem) => feedback = Some(problem.safe_label()),
                        ClientOutcome::Transport(error) => {
                            feedback = Some(error.safe_label().to_owned())
                        }
                    }
                }
                WorkerResult::RemotePushed(outcome) => {
                    match outcome {
                        ClientOutcome::Ok(job) => {
                            local_publication_job = Some(job.job_id);
                            remote_push_pending = true;
                            feedback = Some("Push started".to_owned());
                        }
                        ClientOutcome::Problem(problem) => {
                            remote_loading = false;
                            // A remote that changed since the preview shows its
                            // current content again instead of stalling.
                            remote_requested = None;
                            feedback = Some(problem.safe_label());
                        }
                        ClientOutcome::Transport(error) => {
                            remote_loading = false;
                            feedback = Some(error.safe_label().to_owned())
                        }
                    }
                }
                WorkerResult::AmendmentSeeded(outcome) | WorkerResult::AmendmentReread(outcome) => {
                    local_action_pending = false;
                    match *outcome {
                        ClientOutcome::Ok(workflow) => {
                            current_stage = Stage::Compose;
                            compose_view.load_from_canonical(&workflow);
                            draft_editor = DraftEditor::new(&compose_view.text);
                            feedback = Some(
                                amendment_view::banner(&workflow)
                                    .or_else(|| amendment_view::status_text(&workflow))
                                    .unwrap_or("Amendment opened")
                                    .to_owned(),
                            );
                            current_workflow = Some(workflow);
                            amendment_preview = None;
                            local_preview = None;
                        }
                        ClientOutcome::Problem(problem) => feedback = Some(problem.safe_label()),
                        ClientOutcome::Transport(error) => {
                            feedback = Some(error.safe_label().to_owned())
                        }
                    }
                }
                WorkerResult::AmendmentPreviewed(outcome) => {
                    local_action_pending = false;
                    match *outcome {
                        ClientOutcome::Ok(preview) => {
                            current_workflow = Some(preview.workflow.clone());
                            feedback =
                                amendment_view::status_text(&preview.workflow).map(str::to_owned);
                            amendment_preview = Some(preview);
                        }
                        ClientOutcome::Problem(problem) => feedback = Some(problem.safe_label()),
                        ClientOutcome::Transport(error) => {
                            feedback = Some(error.safe_label().to_owned())
                        }
                    }
                }
                WorkerResult::AmendmentApproved(outcome) => {
                    local_action_pending = false;
                    match *outcome {
                        ClientOutcome::Ok(workflow) => {
                            current_stage = Stage::PublishLocal;
                            feedback = amendment_view::status_text(&workflow).map(str::to_owned);
                            current_workflow = Some(workflow);
                        }
                        ClientOutcome::Problem(problem) => feedback = Some(problem.safe_label()),
                        ClientOutcome::Transport(error) => {
                            feedback = Some(error.safe_label().to_owned())
                        }
                    }
                }
                WorkerResult::AmendmentPublished(outcome) => {
                    local_action_pending = false;
                    match outcome {
                        ClientOutcome::Ok(job) => {
                            local_publication_job = Some(job.job_id);
                            feedback = Some("Replacement started".to_owned());
                        }
                        ClientOutcome::Problem(problem) => feedback = Some(problem.safe_label()),
                        ClientOutcome::Transport(error) => {
                            feedback = Some(error.safe_label().to_owned())
                        }
                    }
                }
                WorkerResult::AmendmentAbandoned(outcome) => {
                    local_action_pending = false;
                    match *outcome {
                        ClientOutcome::Ok(workflow) => {
                            current_stage = Stage::PublishLocal;
                            current_workflow = Some(workflow);
                            amendment_preview = None;
                            feedback = Some(
                                "Amendment abandoned; files and receipts are unchanged".to_owned(),
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
                            preferences = Some(PreferencesDto {
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
        let selected_backend_label = assistant_view.selected_label(current_workflow.as_ref());
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
            || preview_detail_pending
            || sources_view.confirming()
            || source_confirmation_pending;
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
        let focused_row = focus
            .resource_index
            .min(sources_view.visible_len().saturating_sub(1));
        if current_stage == Stage::Sources && !source_editor_active {
            let focused_id = sources_view
                .candidate_at(focused_row)
                .map(|candidate| candidate.id.clone());
            sources_view.close_unfocused_detail(focused_id.as_deref());
        }
        let sources_menu_state = sources_view.menu_state(
            focused_row,
            current_workflow
                .as_ref()
                .is_some_and(|workflow| workflow.assistant_backend_id.is_some()),
        );
        let sources_menu_items = if current_stage == Stage::Sources {
            visible_items(&sources_menu_state)
        } else {
            Vec::new()
        };
        if current_stage == Stage::Sources
            && let Some(workflow) = current_workflow.as_ref()
            && should_auto_discover(
                workflow,
                can_mutate,
                can_read_sources,
                source_job_busy,
                &auto_discovery_attempted,
            )
        {
            auto_discovery_attempted.insert(workflow.id.clone());
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
            } else {
                feedback = Some("Looking for notes from the last three days".to_owned());
            }
        }
        let (compose_resources, assistant_resources) =
            assistant_view.resources(current_workflow.as_ref(), compose_view.is_dirty());
        let utility_model = UtilitySidebarModel {
            diary_day: DiaryDayWidgetModel::from_workflow(current_workflow.as_ref()),
            assistant: AssistantWidgetModel::new(
                &assistant_view.backends,
                current_workflow.as_ref(),
                read_only_reason(
                    current_workflow.is_some(),
                    can_mutate,
                    assistant_job.is_some(),
                    assistant_selection_pending,
                )
                .map(str::to_owned),
            ),
        };
        if current_stage == Stage::PublishRemote
            && !remote_loading
            && !remote_push_pending
            && local_publication_job.is_none()
            && let Some(workflow) = &current_workflow
            && remote_available(workflow)
        {
            if remotes_listed_for.as_deref() != Some(workflow.id.as_str()) {
                if worker
                    .try_send(Intent::ListRemotes {
                        workflow_id: workflow.id.clone(),
                    })
                    .is_ok()
                {
                    remotes_listed_for = Some(workflow.id.clone());
                }
            } else if let Some(remote) = remotes.get(focus.resource_index) {
                let key = (workflow.id.clone(), remote.id.clone());
                if remote_requested.as_ref() != Some(&key)
                    && worker
                        .try_send(Intent::PreviewRemote {
                            workflow_id: workflow.id.clone(),
                            revision: workflow.revision,
                            remote: remote.id.clone(),
                            existing_mode: None,
                        })
                        .is_ok()
                {
                    remote_requested = Some(key);
                    remote_loading = true;
                }
            }
        }
        let selected_remote: Option<&RemoteDto> = remotes.get(focus.resource_index);
        let shown_remote_preview = remote_preview.as_ref().filter(|preview| {
            selected_remote.is_some_and(|remote| remote.id == preview.id)
                && current_workflow
                    .as_ref()
                    .is_some_and(|workflow| workflow.id == preview.workflow.id)
        });
        let visible_resource_count = match current_stage {
            Stage::Start => start_view.workflows.len(),
            Stage::Sources => sources_view.visible_len(),
            Stage::Compose => compose_resources.rows.len(),
            Stage::Review => 1,
            Stage::Preview => 1,
            Stage::PublishLocal => 1,
            Stage::PublishRemote => remotes.len().max(1),
        };
        let visible_control_count = match current_stage {
            Stage::Sources => sources_menu_items.len(),
            Stage::Compose => COMPOSE_CONTROL_COUNT,
            Stage::Review => REVIEW_CONTROL_COUNT,
            Stage::Preview
                if current_workflow
                    .as_ref()
                    .is_some_and(|w| w.amendment.is_some()) =>
            {
                4
            }
            Stage::Preview => {
                2 + if existing_entry_offered(local_preview.as_ref()) {
                    crate::existing_entry_view::EXTRA_CONTROLS
                } else {
                    0
                }
            }
            Stage::PublishLocal => 2,
            Stage::PublishRemote => crate::remote_view::controls(shown_remote_preview).len(),
            _ => START_CONTROL_COUNT,
        };
        focus.clamp(
            ALL_STAGES.len(),
            visible_resource_count,
            visible_control_count,
            utility_model.focusable_rows(),
        );

        let mut displayed_status = work_status.clone();
        displayed_status.current_work = if sources_view.confirming() {
            Some(if source_confirmation_pending {
                "Confirm Source List: confirming the selection".to_owned()
            } else {
                "Confirm Source List: preparing included notes".to_owned()
            })
        } else {
            assistant_view
                .job_label()
                .or_else(|| Some(format!("Composition: {selected_backend_label}")))
        };
        displayed_status.pending_or_blocked = Some(next_step_tip(&TipInput {
            stage: current_stage,
            workflow: current_workflow.as_ref(),
            included_count: sources_view.included.len(),
            candidate_count: sources_view.visible_len(),
            source_work_running: source_job_busy,
            entry_has_text: !compose_view.text.trim().is_empty(),
        }));
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
        } else if let Some(source_id) = &sources_view.pending_delete_confirmation {
            let name = sources_view
                .active_detail
                .as_ref()
                .filter(|detail| &detail.id == source_id)
                .map_or(source_id.as_str(), |detail| {
                    detail.source_note_name.as_str()
                });
            format!("Delete {name} and linked audio? Enter confirms; Esc cancels")
        } else if remote_overwrite_confirmation.is_some() {
            "Overwrite what the remote holds? Enter confirms; Esc cancels".to_owned()
        } else if overwrite_confirmation {
            "Overwrite the existing entry? A backup is kept first. Enter confirms; Esc cancels"
                .to_owned()
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
            Stage::PublishLocal => ResourcesSidebarModel {
                rows: vec![ResourceRow {
                    label: "Local file".to_owned(),
                    state: match current_workflow
                        .as_ref()
                        .and_then(|w| w.local_status.as_deref())
                        .unwrap_or("pending")
                    {
                        "changed" => "file changed",
                        other => other,
                    }
                    .to_owned(),
                    indent: 0,
                }],
            },
            Stage::PublishRemote => ResourcesSidebarModel {
                rows: if remotes.is_empty() {
                    vec![ResourceRow {
                        label: "Remotes".to_owned(),
                        state: "none yet".to_owned(),
                        indent: 0,
                    }]
                } else {
                    remotes
                        .iter()
                        .map(|remote| ResourceRow {
                            label: remote.label.clone(),
                            state: crate::remote_view::status_label(&remote.status).to_owned(),
                            indent: 0,
                        })
                        .collect()
                },
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
                &sources_menu_state,
                &sources_menu_items,
                explicit_source_day.as_deref(),
                source_date_editing,
                &source_date_draft,
                source_page_pending,
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
            Stage::Preview
                if current_workflow
                    .as_ref()
                    .is_some_and(|workflow| workflow.amendment.is_some()) =>
            {
                amendment_preview_screen_body(
                    &focus,
                    current_workflow.as_ref(),
                    amendment_preview.as_ref(),
                )
            }
            Stage::Preview => preview_screen_body(&focus, local_preview.as_ref()),
            Stage::PublishLocal => {
                publish_screen_body(&focus, current_workflow.as_ref(), local_preview.as_ref())
            }
            Stage::PublishRemote => remote_screen_body(
                &focus,
                current_workflow.as_ref().is_some_and(remote_available),
                selected_remote,
                shown_remote_preview,
                current_workflow
                    .as_ref()
                    .map(|workflow| workflow.entry_text.as_str()),
                &ReadinessView::resolve(
                    readiness_pending,
                    readiness_checked.as_ref(),
                    current_workflow.as_ref(),
                    selected_remote.map_or("", |remote| remote.id.as_str()),
                ),
                remote_loading,
                remote_overwrite_confirmation.is_some(),
            ),
        };
        if current_stage != Stage::Sources {
            sources_help = None;
        }
        let existing_entry = match current_stage {
            Stage::PublishLocal => local_preview
                .as_ref()
                .and_then(|preview| preview.existing_text.as_deref())
                .map(indent_diary_block),
            Stage::PublishRemote => shown_remote_preview
                .and_then(existing_entry_text)
                .map(|entry| indent_diary_block(&entry)),
            _ => None,
        };
        let diary_entry = matches!(current_stage, Stage::PublishLocal | Stage::PublishRemote)
            .then(|| {
                current_workflow
                    .as_ref()
                    .map(|workflow| indent_diary_block(&workflow.entry_text))
            })
            .flatten();
        let help_text = sources_help.map(|_| help_lines(status_line.ime));
        let drawn = guard.terminal_mut().draw(|frame| {
            let area = frame.area();
            if !crate::layout::is_supported_size(area.width, area.height) {
                frame.render_widget(crate::layout::TerminalTooSmallWidget, area);
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
            let utility_widget = || {
                UtilitySidebarWidget::new(&utility_model)
                    .focused(focus.zone == FocusZone::Utility)
                    .focused_row((focus.zone == FocusZone::Utility).then_some(focus.utility_index))
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
                        .diary_entry(diary_entry.as_deref())
                        .existing_entry(existing_entry.as_deref())
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
            if let Some(utility_area) = areas.utility_sidebar {
                frame.render_widget(utility_widget(), utility_area);
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
                    SidebarKind::Utility => {
                        frame.render_widget(utility_widget(), overlay.area);
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
            if let (Some(scroll), Some(lines)) = (sources_help, help_text.as_deref()) {
                let help_area = overlay_area(area);
                let clamped = scroll.min(HelpOverlayWidget::max_scroll(lines, help_area));
                sources_help = Some(clamped);
                frame.render_widget(
                    HelpOverlayWidget::new("Sources shortcuts", lines, clamped).palette(palette),
                    help_area,
                );
            }
        });
        if drawn.is_err() {
            return SessionEnd::RenderFailed;
        }

        use crate::command_line::Command;
        use crate::terminal::{InputEvent, InputPoll};
        let shortcut_context = ShortcutContext {
            zone: focus.zone,
            stage: current_stage,
            ime: status_line.ime,
            text_input_active: command_line_active
                || day_editing
                || source_date_editing
                || source_editor_active
                || editor_active,
            confirmation_active: empty_selection_confirmation
                || overwrite_confirmation
                || remote_overwrite_confirmation.is_some()
                || sources_view.pending_delete_confirmation.is_some()
                || discard_confirmation.is_some()
                || quit_confirmation,
            help_overlay_open: sources_help.is_some(),
        };
        let mut pending_sources_action: Option<SourcesAction> = None;
        match crate::terminal::poll_input_event(TICK_INTERVAL) {
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
            other @ (InputPoll::Ctrl(_) | InputPoll::Event(_)) if sources_help.is_some() => {
                // Modal: nothing reaches a buffer, the menu, or a shortcut.
                let scroll = sources_help.unwrap_or(0);
                sources_help = match other {
                    InputPoll::Event(
                        InputEvent::Esc | InputEvent::Enter | InputEvent::Char('?'),
                    ) => None,
                    InputPoll::Event(InputEvent::Up) => Some(scroll.saturating_sub(1)),
                    InputPoll::Event(InputEvent::Down) => Some(scroll.saturating_add(1)),
                    InputPoll::Event(InputEvent::PageUp) => Some(scroll.saturating_sub(10)),
                    InputPoll::Event(InputEvent::PageDown) => Some(scroll.saturating_add(10)),
                    InputPoll::Event(InputEvent::Home) => Some(0),
                    _ => Some(scroll),
                };
            }
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
                                format!("Opened {}", crate::sidebar::stage_label(stage))
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
                    } else if crate::command_line::is_valid_day(&day_draft) {
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
                    if crate::command_line::is_valid_day(&source_date_draft) {
                        explicit_source_day = Some(source_date_draft.clone());
                        source_date_editing = false;
                        feedback = Some(format!(
                            "Extra source date set to {source_date_draft}; use Refresh notes to search it"
                        ));
                        if can_mutate
                            && can_read_sources
                            && !source_job_busy
                            && let Some(workflow) = &current_workflow
                        {
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
                            } else {
                                feedback = Some(format!("Searching notes for {source_date_draft}"));
                            }
                        }
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
            InputPoll::Event(InputEvent::Enter) if overwrite_confirmation => {
                overwrite_confirmation = false;
                if can_mutate
                    && !local_action_pending
                    && let Some(workflow) = &current_workflow
                {
                    local_action_pending = worker
                        .try_send(existing_preview_intent(workflow, "overwrite"))
                        .is_ok();
                }
            }
            InputPoll::Event(InputEvent::Enter) if remote_overwrite_confirmation.is_some() => {
                let confirmed = remote_overwrite_confirmation.take();
                if can_mutate
                    && !remote_loading
                    && let (Some(workflow), Some(remote), Some(preview)) = (
                        current_workflow.as_ref(),
                        selected_remote,
                        shown_remote_preview,
                    )
                    && let Some(digest) = preview.plan_digest.clone()
                    && confirmed.as_ref() == Some(&(remote.id.clone(), digest.clone()))
                    && preview.existing_mode.as_deref() == Some("overwrite")
                {
                    remote_loading = worker
                        .try_send(Intent::PushRemote {
                            workflow_id: workflow.id.clone(),
                            revision: workflow.revision,
                            remote: remote.id.clone(),
                            plan_digest: digest,
                            existing_mode: preview.existing_mode.clone(),
                        })
                        .is_ok();
                }
            }
            InputPoll::Event(InputEvent::Esc) if remote_overwrite_confirmation.is_some() => {
                remote_overwrite_confirmation = None;
                feedback = Some("Overwrite cancelled".to_owned());
            }
            InputPoll::Event(InputEvent::Esc) if overwrite_confirmation => {
                overwrite_confirmation = false;
                feedback = Some("Overwrite cancelled".to_owned());
            }
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
            InputPoll::Event(InputEvent::Char(ch))
                if shortcut_for(&shortcut_context, ch).is_some() =>
            {
                if let Some(shortcut) = shortcut_for(&shortcut_context, ch) {
                    pending_sources_action = Some(shortcut_action(shortcut));
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
                    utility_model.focusable_rows(),
                ));
            }
            InputPoll::Event(InputEvent::Down) => {
                focus.move_next(focus_item_count(
                    &focus,
                    visible_resource_count,
                    visible_control_count,
                    utility_model.focusable_rows(),
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
                FocusZone::Utility => {
                    match selection_request(
                        &utility_model.assistant,
                        &assistant_view.backends,
                        focus.utility_index,
                    ) {
                        Ok(backend_id) => {
                            if let Some(workflow) = &current_workflow {
                                assistant_selection_pending = worker
                                    .try_send(Intent::SelectAssistant {
                                        workflow_id: workflow.id.clone(),
                                        revision: workflow.revision,
                                        backend_id,
                                    })
                                    .is_ok();
                            }
                        }
                        Err(message) => feedback = Some(message),
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
                                language: session_language.clone(),
                                style: session_style.clone(),
                            })
                            .is_ok();
                    }
                }
                FocusZone::Resources if current_stage == Stage::Sources => {
                    if !source_job_busy {
                        request_inspect_source(
                            &worker,
                            &mut sources_view,
                            &current_workflow,
                            focus.resource_index,
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
                                    language: session_language.clone(),
                                    style: session_style.clone(),
                                })
                                .is_ok();
                        }
                        2 if can_start && !start_pending => {
                            if let Some(workflow) = start_view.workflows.get(focus.resource_index) {
                                start_pending = worker
                                    .try_send(Intent::StartOrResumeWorkflow {
                                        diary_day: Some(workflow.diary_day.clone()),
                                        language: session_language.clone(),
                                        style: session_style.clone(),
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
                    pending_sources_action = sources_menu_items
                        .get(focus.main_control_index)
                        .map(|item| item_action(*item));
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
                                        assistant_backend_id: workflow.assistant_backend_id.clone(),
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
                                    selected_text_digest: paragraph_digest(paragraph.as_bytes()),
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
                FocusZone::Main
                    if current_stage == Stage::Preview
                        && current_workflow
                            .as_ref()
                            .is_some_and(|workflow| workflow.amendment.is_some()) =>
                {
                    if can_mutate
                        && !local_action_pending
                        && let Some(workflow) = &current_workflow
                    {
                        let choices = amendment_view::amendment_choices(workflow).unwrap_or(&[]);
                        let sent = match focus.main_control_index {
                            0 => Some(Intent::PreviewAmendment {
                                workflow_id: workflow.id.clone(),
                                revision: workflow.revision,
                            }),
                            1 if workflow.state == "output-planned" => amendment_preview
                                .as_ref()
                                .map(|preview| Intent::ApproveAmendment {
                                    workflow_id: workflow.id.clone(),
                                    revision: workflow.revision,
                                    plan_digest: preview.plan_digest.clone(),
                                }),
                            2 if choices
                                .contains(&amendment_view::AmendmentChoice::AbandonAmendment) =>
                            {
                                Some(Intent::AbandonAmendment {
                                    workflow_id: workflow.id.clone(),
                                    revision: workflow.revision,
                                })
                            }
                            _ => None,
                        };
                        match sent {
                            Some(intent) => local_action_pending = worker.try_send(intent).is_ok(),
                            None => {
                                feedback =
                                    Some("Preview the replacement before approval".to_owned());
                            }
                        }
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
                            index @ (2 | 3) if existing_entry_offered(local_preview.as_ref()) => {
                                if crate::existing_entry_view::mode_for(index) == Some("append") {
                                    local_action_pending = worker
                                        .try_send(existing_preview_intent(workflow, "append"))
                                        .is_ok();
                                } else {
                                    overwrite_confirmation = true;
                                    feedback = Some(
                                        "Overwrite replaces the existing entry; Enter to confirm"
                                            .to_owned(),
                                    );
                                }
                            }
                            _ => {
                                feedback =
                                    Some("Preview the current plan before approval".to_owned())
                            }
                        }
                    }
                }
                FocusZone::Main if current_stage == Stage::PublishRemote => {
                    let controls = crate::remote_view::controls(shown_remote_preview);
                    let completed = current_workflow.as_ref().is_some_and(remote_available);
                    if let (Some(control), Some(remote), Some(workflow), true) = (
                        controls.get(focus.main_control_index).copied(),
                        selected_remote,
                        current_workflow.as_ref(),
                        completed,
                    ) {
                        let remote_id = remote.id.clone();
                        let workflow_id = workflow.id.clone();
                        let revision = workflow.revision;
                        match control {
                            RemoteControl::ShowPlan if !remote_loading => {
                                remote_requested = None;
                                remotes_listed_for = None;
                            }
                            RemoteControl::CheckReadiness if !readiness_pending => {
                                readiness_pending = worker
                                    .try_send(Intent::CheckRemote {
                                        workflow_id,
                                        revision,
                                        remote: remote_id,
                                    })
                                    .is_ok();
                            }
                            RemoteControl::Append | RemoteControl::Overwrite
                                if can_mutate && !remote_loading =>
                            {
                                let mode = if control == RemoteControl::Append {
                                    "append"
                                } else {
                                    "overwrite"
                                };
                                remote_loading = worker
                                    .try_send(Intent::PreviewRemote {
                                        workflow_id,
                                        revision,
                                        remote: remote_id,
                                        existing_mode: Some(mode.to_owned()),
                                    })
                                    .is_ok();
                            }
                            RemoteControl::Push if can_mutate && !remote_loading => {
                                if shown_remote_preview.is_some_and(|p| {
                                    p.existing_mode.as_deref() == Some("overwrite")
                                }) {
                                    remote_overwrite_confirmation = shown_remote_preview
                                        .and_then(|preview| preview.plan_digest.clone())
                                        .map(|digest| (remote_id.clone(), digest));
                                    feedback = Some(
                                        "Overwrite replaces what is there; Enter again to confirm"
                                            .to_owned(),
                                    );
                                } else if let Some(preview) = shown_remote_preview
                                    && let Some(digest) = preview.plan_digest.clone()
                                {
                                    remote_loading = worker
                                        .try_send(Intent::PushRemote {
                                            workflow_id,
                                            revision,
                                            remote: remote_id,
                                            plan_digest: digest,
                                            existing_mode: preview.existing_mode.clone(),
                                        })
                                        .is_ok();
                                }
                            }
                            _ => {}
                        }
                    }
                }
                FocusZone::Main
                    if current_stage == Stage::PublishLocal
                        && can_mutate
                        && !local_action_pending =>
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
                        if amendment_view::offers_amendment(workflow) {
                            local_action_pending = worker
                                .try_send(Intent::SeedAmendment {
                                    workflow_id: workflow.id.clone(),
                                    revision: workflow.revision,
                                })
                                .is_ok();
                        } else if workflow.amendment.is_some() {
                            let intent = match workflow.state.as_str() {
                                "approved" => Some(Intent::PublishAmendment {
                                    workflow_id: workflow.id.clone(),
                                    revision: workflow.revision,
                                }),
                                "partial" => Some(Intent::PublishAmendment {
                                    workflow_id: workflow.id.clone(),
                                    revision: workflow.revision,
                                }),
                                _ => None,
                            };
                            match intent {
                                Some(intent) => {
                                    local_action_pending = worker.try_send(intent).is_ok();
                                }
                                None => {
                                    feedback =
                                        Some("No approved replacement is pending".to_owned());
                                }
                            }
                        } else if workflow.state == "approved" || workflow.state == "partial" {
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

        if let Some(action) = pending_sources_action.take()
            && current_stage == Stage::Sources
        {
            let no_source_job = !source_job_busy;
            let focused_id = sources_view
                .candidate_at(focused_row)
                .map(|candidate| candidate.id.clone());
            if !sources_menu_items.contains(&action_item(action)) {
                feedback = Some("That action is not available right now".to_owned());
            } else {
                match action {
                    SourcesAction::Include
                    | SourcesAction::Exclude
                    | SourcesAction::ToggleInclude => {
                        if !can_mutate || !no_source_job {
                            feedback =
                                Some("Wait for the running source work to finish".to_owned());
                        } else if let Some(id) = focused_id {
                            let included = sources_view.included.contains(&id);
                            let wanted = match action {
                                SourcesAction::Include => true,
                                SourcesAction::Exclude => false,
                                _ => !included,
                            };
                            if wanted != included {
                                let _ = sources_view.toggle_included(&id);
                            }
                        }
                    }
                    SourcesAction::View => {
                        if no_source_job {
                            request_inspect_source(
                                &worker,
                                &mut sources_view,
                                &current_workflow,
                                focused_row,
                                &mut pending_source_job_kind,
                            );
                        } else {
                            feedback =
                                Some("Wait for the running source work to finish".to_owned());
                        }
                    }
                    SourcesAction::Hide => {
                        if !no_source_job {
                            feedback =
                                Some("Wait for the running source work to finish".to_owned());
                        } else if let Some(id) = focused_id {
                            if source_editor_active
                                && source_edit_target.as_ref().map(|target| &target.0) == Some(&id)
                            {
                                feedback =
                                    Some("Save or cancel the source edit before hiding".to_owned());
                            } else if sources_view.hide(&id) {
                                feedback = Some(
                                    "Note hidden for this session; it is excluded (u shows hidden notes)"
                                        .to_owned(),
                                );
                            }
                        }
                    }
                    SourcesAction::ShowHidden => {
                        sources_view.show_hidden();
                        feedback = Some("Hidden notes are back in the list, excluded".to_owned());
                    }
                    SourcesAction::Refine => {
                        if can_mutate && !source_mutation_pending && assistant_job.is_none() {
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
                        } else {
                            feedback =
                                Some("An assistant or note change is already running".to_owned());
                        }
                    }
                    SourcesAction::Accept | SourcesAction::Reject => {
                        if can_mutate
                            && !source_mutation_pending
                            && let (Some(workflow), Some(suggestion)) =
                                (&current_workflow, &sources_view.active_refine_suggestion)
                        {
                            source_mutation_pending = worker
                                .try_send(Intent::ResolveSourceRefine {
                                    workflow_id: workflow.id.clone(),
                                    source_id: suggestion.source_id.clone(),
                                    revision: workflow.revision,
                                    accept: action == SourcesAction::Accept,
                                    digest: suggestion.binding_digest.clone(),
                                })
                                .is_ok();
                        }
                    }
                    SourcesAction::Edit => {
                        if can_mutate
                            && !source_mutation_pending
                            && let Some(detail) = &sources_view.active_detail
                        {
                            source_editor = DraftEditor::new(&detail.body);
                            source_edit_target =
                                Some((detail.id.clone(), detail.content_digest.clone()));
                            source_editor_active = true;
                            focus.zone = FocusZone::Main;
                        }
                    }
                    SourcesAction::Delete => {
                        if can_mutate
                            && !source_mutation_pending
                            && let Some(detail) = &sources_view.active_detail
                        {
                            sources_view.pending_delete_confirmation = Some(detail.id.clone());
                        }
                    }
                    SourcesAction::ConfirmSourceList => {
                        if !can_mutate || !can_read_sources {
                            feedback = Some("Source actions are unavailable right now".to_owned());
                        } else if !no_source_job {
                            feedback =
                                Some("Wait for the running source work to finish".to_owned());
                        } else if sources_view.included.is_empty() {
                            empty_selection_confirmation = true;
                        } else if let Some(workflow) = &current_workflow {
                            match sources_view.begin_confirm() {
                                Err(block) => feedback = Some(block.reason().to_owned()),
                                Ok(source_ids) => {
                                    pending_source_job_kind = Some(SourceJobKind::Preparation);
                                    if worker
                                        .try_send(Intent::PrepareSources {
                                            workflow_id: workflow.id.clone(),
                                            revision: workflow.revision,
                                            source_ids,
                                        })
                                        .is_err()
                                    {
                                        pending_source_job_kind = None;
                                        sources_view.finish_confirm();
                                        feedback =
                                            Some("Confirm Source List could not start".to_owned());
                                    } else {
                                        feedback = Some("Preparing included notes".to_owned());
                                    }
                                }
                            }
                        }
                    }
                    SourcesAction::Refresh => {
                        if can_mutate && can_read_sources && no_source_job {
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
                        } else {
                            feedback =
                                Some("Wait for the running source work to finish".to_owned());
                        }
                    }
                    SourcesAction::AddDate => {
                        source_date_draft = explicit_source_day.clone().unwrap_or_default();
                        source_date_editing = true;
                    }
                    SourcesAction::LoadMore => {
                        if !source_page_pending
                            && let (Some(workflow), Some(cursor)) =
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
                    SourcesAction::Help => sources_help = Some(0),
                    SourcesAction::BackToStart => current_stage = Stage::Start,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;

/// Whether Preview offers Append and Overwrite for the local file.
fn existing_entry_offered(preview: Option<&LocalPreviewDto>) -> bool {
    crate::existing_entry_view::offered(preview)
}

/// The local preview a chosen mode asks for.
fn existing_preview_intent(workflow: &WorkflowDto, mode: &str) -> Intent {
    Intent::PreviewExisting {
        workflow_id: workflow.id.clone(),
        revision: workflow.revision,
        existing_mode: mode.to_owned(),
    }
}
