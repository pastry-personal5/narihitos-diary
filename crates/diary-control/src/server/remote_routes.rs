//! Remote routes at the HTTP boundary: list, preview, readiness, and push for
//! iCloud and Notion.

use super::*;

use crate::ExistingEntryMode;
use crate::commands::{RemoteStatus, RemoteSummary};

const ICLOUD: &str = "icloud";
const NOTION: &str = "notion";

fn mode_from(value: Option<&str>) -> Result<ExistingEntryMode, ApiError> {
    match value {
        None => Ok(ExistingEntryMode::Refuse),
        Some(value) => ExistingEntryMode::parse(value)
            .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "invalid-existing-mode")),
    }
}

fn known_remote(name: &str) -> Result<&'static str, ApiError> {
    match name {
        ICLOUD => Ok(ICLOUD),
        NOTION => Ok(NOTION),
        _ => Err(ApiError::new(StatusCode::NOT_FOUND, "unknown-remote")),
    }
}

fn remote_dto(summary: RemoteSummary) -> RemoteDto {
    RemoteDto {
        id: summary.id.to_owned(),
        label: summary.label.to_owned(),
        status: summary.status.as_str().to_owned(),
        reason: summary.reason,
    }
}

pub(super) async fn list_remotes(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<RemoteListDto>, ApiError> {
    let day = day_from_id(&id)?;
    let service = state.0.config.commands.clone();
    let remotes = blocking(move || service.remotes(&day)).await?;
    Ok(Json(RemoteListDto {
        remotes: remotes.into_iter().map(remote_dto).collect(),
    }))
}

fn blocks(items: Vec<diary_core::NotionBlockPreview>) -> Vec<NotionBlockDto> {
    items
        .into_iter()
        .map(|block| NotionBlockDto {
            kind: block.kind,
            text: block.text,
            depth: block.depth,
        })
        .collect()
}

fn icloud_preview(
    service: &CommandService,
    day: &DiaryDay,
    mode: ExistingEntryMode,
) -> Result<RemotePreviewDto, CommandError> {
    let workflow = service.load_workflow(day)?;
    let base = |status: RemoteStatus, reason: Option<String>| RemotePreviewDto {
        id: ICLOUD.to_owned(),
        label: "iCloud".to_owned(),
        status: status.as_str().to_owned(),
        reason,
        workflow: workflow_dto(&workflow),
        target: Vec::new(),
        payload: None,
        existing_text: None,
        intended_changes: Vec::new(),
        placement: String::new(),
        blocks: Vec::new(),
        current_blocks: Vec::new(),
        existing_mode: None,
        plan_digest: None,
        page_url: None,
    };
    if !service.icloud_configured() {
        return Ok(base(
            RemoteStatus::Blocked,
            Some("icloud-not-configured".to_owned()),
        ));
    }
    let preview = match service.preview_icloud(day, mode) {
        Ok(preview) => preview,
        Err(CommandError::Workflow(_)) => {
            return Ok(base(
                RemoteStatus::Blocked,
                Some("local-not-published".to_owned()),
            ));
        }
        Err(error) => return Err(error),
    };
    // Content already at the remote is shown, but nothing can be pushed until
    // the person picks Append or Overwrite.
    let pushable = match preview.status {
        RemoteStatus::NotPushed => true,
        RemoteStatus::Exists => mode.chosen(),
        RemoteStatus::Pushed | RemoteStatus::Blocked => false,
    };
    let mut dto = base(preview.status, preview.reason);
    dto.target = vec![preview.target];
    dto.payload = Some(preview.payload);
    dto.existing_text = preview.existing_text;
    dto.intended_changes = preview.intended_changes;
    dto.existing_mode = mode.chosen().then(|| mode.as_str().to_owned());
    dto.plan_digest = pushable.then(|| preview.plan_digest.as_str().to_owned());
    Ok(dto)
}

fn notion_preview(
    service: &CommandService,
    day: &DiaryDay,
    revision: u64,
    mode: ExistingEntryMode,
) -> Result<RemotePreviewDto, CommandError> {
    let mut preview = service.preview_notion_with(day, revision, mode)?;
    let mut exists = false;
    let entry_blocks = preview.blocks.clone();
    if preview.status == diary_core::DestinationStatus::Conflict
        && !mode.chosen()
        && preview.reason.as_deref() == Some("notion-same-day-conflict")
    {
        // Show the existing section without asking: an Append preview reads
        // it. A page that cannot be appended to stays blocked below.
        let next = service.preview_notion_with(
            day,
            preview.workflow.revision(),
            ExistingEntryMode::Append,
        )?;
        if next.status == diary_core::DestinationStatus::Pending {
            exists = true;
            preview = next;
        }
    }
    // A section this app wrote itself is updated in place: no choice, no
    // prompt. Anything else already there is shown for Append or Overwrite.
    let mut owned_update = false;
    if exists && service.notion_section_is_ours(day) {
        let over = service.preview_notion_with(
            day,
            preview.workflow.revision(),
            ExistingEntryMode::Overwrite,
        )?;
        if over.status == diary_core::DestinationStatus::Pending {
            preview = over;
            owned_update = true;
            exists = false;
        }
    }
    let (status, plan_digest, page_url) = match preview.status {
        diary_core::DestinationStatus::Pending if owned_update => (
            RemoteStatus::NotPushed,
            preview.plan_digest.clone().map(|d| d.as_str().to_owned()),
            None,
        ),
        diary_core::DestinationStatus::Pending if exists => (RemoteStatus::Exists, None, None),
        diary_core::DestinationStatus::Pending if mode.chosen() => (
            RemoteStatus::Exists,
            preview.plan_digest.clone().map(|d| d.as_str().to_owned()),
            None,
        ),
        diary_core::DestinationStatus::Pending => (
            RemoteStatus::NotPushed,
            preview.plan_digest.clone().map(|d| d.as_str().to_owned()),
            None,
        ),
        _ => {
            let fixable = matches!(
                preview.reason.as_deref(),
                Some("notion-same-day-conflict" | "notion-hierarchy-ambiguous")
            );
            (
                RemoteStatus::Blocked,
                None,
                fixable.then(|| service.notion_fix_url(day)).flatten(),
            )
        }
    };
    Ok(RemotePreviewDto {
        id: NOTION.to_owned(),
        label: "Notion".to_owned(),
        status: status.as_str().to_owned(),
        reason: preview.reason,
        workflow: workflow_dto(&preview.workflow),
        target: vec![preview.root_label, preview.year_title, preview.month_title],
        payload: None,
        existing_text: None,
        intended_changes: Vec::new(),
        placement: preview.placement,
        blocks: if exists {
            blocks(entry_blocks)
        } else {
            blocks(preview.blocks)
        },
        current_blocks: blocks(preview.current_blocks),
        existing_mode: (preview.mode != ExistingEntryMode::Refuse && !owned_update)
            .then(|| preview.mode.as_str().to_owned()),
        plan_digest,
        page_url,
    })
}

pub(super) async fn preview_remote(
    State(state): State<AppState>,
    Path((id, name)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<RemotePreviewDto>, ApiError> {
    let day = day_from_id(&id)?;
    let revision = workflow_precondition(&headers)?;
    let remote = known_remote(&name)?;
    let request: RemotePreviewRequest = if body.is_empty() {
        RemotePreviewRequest::default()
    } else {
        serde_json::from_slice(&body)
            .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid-request"))?
    };
    let mode = mode_from(request.existing_mode.as_deref())?;
    let service = state.0.config.commands.clone();
    let preview = blocking(move || {
        if remote == ICLOUD {
            icloud_preview(&service, &day, mode)
        } else {
            notion_preview(&service, &day, revision, mode)
        }
    })
    .await?;
    publish_event(
        &state,
        "workflow.changed",
        Some(id),
        Some(preview.workflow.revision),
        None,
        None,
    );
    Ok(Json(preview))
}

pub(super) async fn check_remote_readiness(
    State(state): State<AppState>,
    Path((id, name)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<RemoteReadinessDto>, ApiError> {
    let day = day_from_id(&id)?;
    let revision = workflow_precondition(&headers)?;
    let remote = known_remote(&name)?;
    let service = state.0.config.commands.clone();
    let tag_day = day.clone();
    let outcome =
        tokio::task::spawn_blocking(move || service.check_publish_readiness(&day, revision))
            .await
            .map_err(|_| ApiError::internal())?;
    let report = match outcome {
        Ok(report) => report,
        Err(error) => {
            log_debug(
                &state,
                DiagnosticEvent::ReadinessRefused {
                    reason: match &error {
                        CommandError::PreconditionFailed => "stale-revision",
                        CommandError::Reconciling => "reconciling",
                        CommandError::NotFound => "not-found",
                        _ => "state-unavailable",
                    },
                },
            );
            return Err(ApiError::from(error));
        }
    };
    let item = report
        .destinations
        .iter()
        .find(|item| item.destination == remote)
        .ok_or_else(ApiError::internal)?;
    log_debug(
        &state,
        DiagnosticEvent::PublishReadiness {
            destination: item.destination,
            readiness: item.readiness.as_str(),
            reason: item.reason,
            next_step: item.next_step,
            decided_by: item.decided_by,
            duration_ms: item.duration_ms,
        },
    );
    if let Some(workflow) = state
        .0
        .config
        .diagnostics
        .as_ref()
        .and_then(|log| log.tag("workflow", tag_day.as_str().as_bytes()))
    {
        log_debug(
            &state,
            DiagnosticEvent::ReadinessCheck {
                workflow,
                revision: report.revision,
                local_verified: report.local_verified,
                notion_configured: report.notion_configured,
                icloud_configured: report.icloud_configured,
                duration_ms: report.duration_ms,
            },
        );
    }
    Ok(Json(RemoteReadinessDto {
        revision: report.revision,
        readiness: DestinationReadinessDto {
            destination: item.destination.to_owned(),
            readiness: item.readiness.as_str().to_owned(),
            reason: item.reason.to_owned(),
            next_step: item.next_step.to_owned(),
        },
    }))
}

pub(super) async fn push_remote(
    State(state): State<AppState>,
    Path((id, name)): Path<(String, String)>,
    headers: HeaderMap,
    Json(request): Json<RemotePushRequest>,
) -> Result<(StatusCode, Json<JobAcceptedDto>), ApiError> {
    let day = day_from_id(&id)?;
    let revision = workflow_precondition(&headers)?;
    let remote = known_remote(&name)?;
    let mode = mode_from(request.existing_mode.as_deref())?;
    let digest = Digest::parse(request.plan_digest)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid-plan-digest"))?;
    let job_id = new_job(&state, "remote-push", &id)?;
    tokio::spawn(run_remote_push_job(
        state,
        job_id.clone(),
        id,
        day,
        revision,
        remote,
        mode,
        digest,
    ));
    Ok((StatusCode::ACCEPTED, Json(JobAcceptedDto { job_id })))
}

enum Pushed {
    Icloud(diary_core::DestinationStatus),
    Notion(Box<Workflow>),
}

#[allow(clippy::too_many_arguments)]
async fn run_remote_push_job(
    state: AppState,
    job_id: String,
    workflow_id: String,
    day: DiaryDay,
    revision: u64,
    remote: &'static str,
    mode: ExistingEntryMode,
    digest: Digest,
) {
    let Ok(_permit) = Arc::clone(&state.0.workers).acquire_owned().await else {
        return;
    };
    if !mark_running(&state, &job_id) {
        return;
    }
    let Some((commit_state, cancelled)) = assistant_job_state(&state, &job_id) else {
        fail_job(&state, &job_id, "internal-failure");
        return;
    };
    if commit_state
        .compare_exchange(
            COMMIT_ACTIVE,
            COMMITTING,
            Ordering::AcqRel,
            Ordering::Acquire,
        )
        .is_err()
    {
        return;
    }
    let snapshot_day = day.clone();
    let service = state.0.config.commands.clone();
    let outcome = tokio::task::spawn_blocking(move || {
        if remote == ICLOUD {
            service
                .push_icloud(&day, mode, &digest, || true)
                .map(Pushed::Icloud)
        } else {
            service
                .push_notion(&day, revision, &digest, &cancelled)
                .map(|workflow| Pushed::Notion(Box::new(workflow)))
        }
    })
    .await;
    match outcome {
        Ok(Ok(pushed)) => {
            let (verified, revision) = match &pushed {
                Pushed::Icloud(status) => {
                    (*status == diary_core::DestinationStatus::Verified, None)
                }
                Pushed::Notion(workflow) => (
                    workflow.destination_progress().iter().any(|item| {
                        item.destination == NOTION
                            && item.status == diary_core::DestinationStatus::Verified
                    }),
                    Some(workflow.revision()),
                ),
            };
            if !verified {
                let code = match pushed {
                    Pushed::Icloud(diary_core::DestinationStatus::Conflict) => {
                        "precondition-failed"
                    }
                    Pushed::Icloud(diary_core::DestinationStatus::Blocked) => "remote-unavailable",
                    _ => "remote-not-verified",
                };
                log_publication_snapshot(&state, "remote-push", "failed", &snapshot_day).await;
                fail_job(&state, &job_id, code);
                return;
            }
            log_publication_snapshot(&state, "remote-push", "finished", &snapshot_day).await;
            publish_event(
                &state,
                "workflow.changed",
                Some(workflow_id),
                revision,
                None,
                None,
            );
            finish_job(&state, &job_id, None, None);
        }
        Ok(Err(error)) => {
            log_publication_snapshot(&state, "remote-push", "failed", &snapshot_day).await;
            fail_job(&state, &job_id, error.diagnostic_code());
        }
        Err(_) => fail_job(&state, &job_id, "internal-failure"),
    }
}
