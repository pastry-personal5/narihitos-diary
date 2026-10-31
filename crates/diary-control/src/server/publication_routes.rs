//! Publication routes at the HTTP boundary.

use super::*;

use crate::ExistingEntryMode;

/// Reads the optional preview body: no body, or no mode, refuses to touch
/// existing content.
fn existing_mode_from(body: &Bytes) -> Result<ExistingEntryMode, ApiError> {
    let request: LocalPreviewRequest = if body.is_empty() {
        LocalPreviewRequest::default()
    } else {
        serde_json::from_slice(body)
            .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid-request"))?
    };
    match request.existing_mode.as_deref() {
        None => Ok(ExistingEntryMode::Refuse),
        Some(value) => ExistingEntryMode::parse(value)
            .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "invalid-existing-mode")),
    }
}

pub(super) async fn preview_local_output(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<LocalPreviewDto>, ApiError> {
    let day = day_from_id(&id)?;
    let revision = workflow_precondition(&headers)?;
    let mode = existing_mode_from(&body)?;
    let service = state.0.config.commands.clone();
    let preview = blocking(move || service.preview_local_with(&day, revision, mode)).await?;
    let plan = preview
        .workflow
        .output_plan()
        .ok_or_else(ApiError::internal)?;
    let local_status = preview
        .workflow
        .destination_progress()
        .iter()
        .find(|item| item.destination == "local")
        .map(|item| item.status.as_str())
        .unwrap_or("blocked");
    let result = LocalPreviewDto {
        workflow: workflow_dto(&preview.workflow),
        target: preview.target,
        payload: preview.payload,
        payload_digest: preview.payload_digest.as_str().to_owned(),
        plan_digest: plan.plan_digest.as_str().to_owned(),
        existing_text: preview.existing_text,
        intended_changes: preview.intended_changes,
        local_status: local_status.to_owned(),
        existing_mode: (preview.mode != ExistingEntryMode::Refuse)
            .then(|| preview.mode.as_str().to_owned()),
    };
    publish_event(
        &state,
        "workflow.changed",
        Some(id),
        Some(result.workflow.revision),
        None,
        None,
    );
    Ok(Json(result))
}

pub(super) async fn approve_local_output(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<LocalApprovalRequest>,
) -> Result<Response, ApiError> {
    let day = day_from_id(&id)?;
    let revision = workflow_precondition(&headers)?;
    let digest = Digest::parse(request.plan_digest)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid-plan-digest"))?;
    let service = state.0.config.commands.clone();
    let workflow = blocking(move || service.approve_local(&day, revision, &digest)).await?;
    publish_event(
        &state,
        "workflow.changed",
        Some(id),
        Some(workflow.revision()),
        None,
        None,
    );
    workflow_response(StatusCode::OK, &workflow)
}

pub(super) async fn publish_local_output(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<(StatusCode, Json<JobAcceptedDto>), ApiError> {
    let day = day_from_id(&id)?;
    let revision = workflow_precondition(&headers)?;
    let job_id = new_job(&state, "local-publication", &id)?;
    tokio::spawn(run_local_publication_job(
        state,
        job_id.clone(),
        id,
        day,
        revision,
    ));
    Ok((StatusCode::ACCEPTED, Json(JobAcceptedDto { job_id })))
}

pub(super) async fn run_local_publication_job(
    state: AppState,
    job_id: String,
    workflow_id: String,
    day: DiaryDay,
    revision: u64,
) {
    let Ok(_permit) = Arc::clone(&state.0.workers).acquire_owned().await else {
        return;
    };
    if !mark_running(&state, &job_id) {
        return;
    }
    let Some(commit_state) = job_commit_state(&state, &job_id) else {
        fail_job(&state, &job_id, "internal-failure");
        return;
    };
    let snapshot_day = day.clone();
    let service = state.0.config.commands.clone();
    let outcome = tokio::task::spawn_blocking(move || {
        service.publish_local_guarded(&day, revision, || {
            commit_state
                .compare_exchange(
                    COMMIT_ACTIVE,
                    COMMITTING,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_ok()
        })
    })
    .await;
    match outcome {
        Ok(Ok(workflow)) => {
            log_publication_snapshot(&state, "local-publication", "finished", &snapshot_day).await;
            publish_event(
                &state,
                "workflow.changed",
                Some(workflow_id),
                Some(workflow.revision()),
                None,
                None,
            );
            finish_job(&state, &job_id, None, None);
        }
        Ok(Err(error)) => {
            log_publication_snapshot(&state, "local-publication", "failed", &snapshot_day).await;
            fail_job(&state, &job_id, error.diagnostic_code());
        }
        Err(_) => fail_job(&state, &job_id, "internal-failure"),
    }
}

/// Read-only: no workflow event is published and no revision changes.
pub(super) async fn seed_amendment(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let day = day_from_id(&id)?;
    let revision = workflow_precondition(&headers)?;
    let service = state.0.config.commands.clone();
    let seeded = blocking(move || service.seed_amendment(&day, revision)).await?;
    publish_event(
        &state,
        "workflow.changed",
        Some(id),
        Some(seeded.workflow.revision()),
        None,
        None,
    );
    workflow_response(StatusCode::CREATED, &seeded.workflow)
}

pub(super) async fn reread_amendment(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let day = day_from_id(&id)?;
    let revision = workflow_precondition(&headers)?;
    let service = state.0.config.commands.clone();
    let seeded = blocking(move || service.reread_amendment(&day, revision)).await?;
    publish_event(
        &state,
        "workflow.changed",
        Some(id),
        Some(seeded.workflow.revision()),
        None,
        None,
    );
    workflow_response(StatusCode::OK, &seeded.workflow)
}

pub(super) async fn abandon_amendment(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let day = day_from_id(&id)?;
    let revision = workflow_precondition(&headers)?;
    let service = state.0.config.commands.clone();
    let workflow = blocking(move || service.abandon_amendment(&day, revision)).await?;
    publish_event(
        &state,
        "workflow.changed",
        Some(id),
        Some(workflow.revision()),
        None,
        None,
    );
    workflow_response(StatusCode::OK, &workflow)
}

pub(super) async fn preview_amendment(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<AmendmentPreviewDto>, ApiError> {
    let day = day_from_id(&id)?;
    let revision = workflow_precondition(&headers)?;
    let service = state.0.config.commands.clone();
    let preview = blocking(move || service.preview_amendment(&day, revision)).await?;
    let plan_digest = preview
        .workflow
        .output_plan()
        .map(|plan| plan.plan_digest.as_str().to_owned())
        .ok_or_else(|| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal-failure"))?;
    let result = AmendmentPreviewDto {
        workflow: workflow_dto(&preview.workflow),
        plan_digest,
        local_target: preview.local_target,
        local_current: preview.local_current,
        local_payload: preview.local_payload,
        local_status: preview.local_status.as_str().to_owned(),
    };
    publish_event(
        &state,
        "workflow.changed",
        Some(id),
        Some(result.workflow.revision),
        None,
        None,
    );
    Ok(Json(result))
}

pub(super) async fn approve_amendment(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<LocalApprovalRequest>,
) -> Result<Response, ApiError> {
    let day = day_from_id(&id)?;
    let revision = workflow_precondition(&headers)?;
    let digest = Digest::parse(request.plan_digest)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid-plan-digest"))?;
    let service = state.0.config.commands.clone();
    let workflow = blocking(move || service.approve_amendment(&day, revision, &digest)).await?;
    publish_event(
        &state,
        "workflow.changed",
        Some(id),
        Some(workflow.revision()),
        None,
        None,
    );
    workflow_response(StatusCode::OK, &workflow)
}

pub(super) async fn publish_amendment(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<(StatusCode, Json<JobAcceptedDto>), ApiError> {
    let day = day_from_id(&id)?;
    let revision = workflow_precondition(&headers)?;
    let job_id = new_job(&state, "amendment-publication", &id)?;
    tokio::spawn(run_amendment_job(state, job_id.clone(), id, day, revision));
    Ok((StatusCode::ACCEPTED, Json(JobAcceptedDto { job_id })))
}

pub(super) async fn run_amendment_job(
    state: AppState,
    job_id: String,
    workflow_id: String,
    day: DiaryDay,
    revision: u64,
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
    let amendment_kind = "amendment-publication";
    let snapshot_day = day.clone();
    let service = state.0.config.commands.clone();
    let outcome =
        tokio::task::spawn_blocking(move || service.publish_amendment(&day, revision, &cancelled))
            .await;
    match outcome {
        Ok(Ok(workflow)) => {
            log_publication_snapshot(&state, amendment_kind, "finished", &snapshot_day).await;
            publish_event(
                &state,
                "workflow.changed",
                Some(workflow_id),
                Some(workflow.revision()),
                None,
                None,
            );
            finish_job(&state, &job_id, None, None);
        }
        Ok(Err(error)) => {
            log_publication_snapshot(&state, amendment_kind, "failed", &snapshot_day).await;
            fail_job(&state, &job_id, error.diagnostic_code());
        }
        Err(_) => fail_job(&state, &job_id, "internal-failure"),
    }
}

pub(super) async fn save_under_day(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<SaveUnderDayRequest>,
) -> Result<Response, ApiError> {
    let day = day_from_id(&id)?;
    let revision = workflow_precondition(&headers)?;
    let new_day = DiaryDay::parse(request.diary_day)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid-diary-day"))?;
    let service = state.0.config.commands.clone();
    let workflow =
        blocking(move || service.save_local_conflict_under_day(&day, revision, new_day)).await?;
    let target_id = workflow_id(workflow.day().as_str()).ok_or_else(ApiError::internal)?;
    publish_event(
        &state,
        "workflow.changed",
        Some(target_id),
        Some(workflow.revision()),
        None,
        None,
    );
    workflow_response(StatusCode::OK, &workflow)
}
