//! Source routes at the HTTP boundary.

use super::*;

pub(super) async fn discover(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<DiscoveryRequest>,
) -> Result<(StatusCode, Json<JobAcceptedDto>), ApiError> {
    let day = day_from_id(&id)?;
    let revision = workflow_precondition(&headers)?;
    validate_optional_string(request.explicit_date.as_deref())?;
    let explicit = request
        .explicit_date
        .map(DiaryDay::parse)
        .transpose()
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid-diary-day"))?;
    let job_id = new_job(&state, "discovery", &id)?;
    let worker_state = state.clone();
    let worker_job = job_id.clone();
    tokio::spawn(async move {
        run_discovery_job(worker_state, worker_job, id, day, revision, explicit).await;
    });
    Ok((StatusCode::ACCEPTED, Json(JobAcceptedDto { job_id })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SourceListQuery {
    cursor: Option<usize>,
    limit: Option<usize>,
}

pub(super) async fn list_sources(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<SourceListQuery>,
) -> Result<Json<SourceListDto>, ApiError> {
    let offset = query.cursor.unwrap_or(0);
    let limit = query.limit.unwrap_or(SOURCE_PAGE_SIZE);
    if limit == 0 || limit > SOURCE_PAGE_SIZE || offset > MAX_CANDIDATES {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid-pagination"));
    }
    let day = day_from_id(&id)?;
    let service = state.0.config.commands.clone();
    let workflow = blocking(move || service.load_workflow(&day)).await?;
    let candidates = candidate_dtos(&workflow);
    let sources = candidates
        .iter()
        .skip(offset)
        .take(limit)
        .cloned()
        .collect::<Vec<_>>();
    let next_cursor =
        (offset + sources.len() < candidates.len()).then(|| (offset + sources.len()).to_string());
    Ok(Json(SourceListDto {
        sources,
        next_cursor,
    }))
}

pub(super) async fn source_detail(
    State(state): State<AppState>,
    Path((id, source_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    validate_string(&source_id)?;
    let day = day_from_id(&id)?;
    let prepared_handle = optional_single_header(&headers, "x-nd-preparation-handle")?;
    let service = state.0.config.commands.clone();
    let workflow = blocking(move || service.load_workflow(&day)).await?;
    let candidate = workflow
        .candidates()
        .iter()
        .find(|candidate| opaque_source_id(&candidate.source) == source_id)
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "source-not-found"))?;
    let source_note_name = candidate.source_note_name.clone();
    let local_time = crate::local_time::local_time(&source_note_name);
    let (snapshot, evidence) = if let Some(handle) = prepared_handle {
        validate_string(handle)?;
        let selected = prepared_source(&state, handle, &id, &source_id)?;
        (selected.snapshot, selected.date_evidence)
    } else {
        let snapshot = workflow
            .snapshots()
            .iter()
            .find(|snapshot| opaque_source_id(&snapshot.source) == source_id)
            .cloned()
            .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "source-not-confirmed"))?;
        let evidence = workflow
            .candidates()
            .iter()
            .find(|candidate| candidate.source == snapshot.source)
            .map(|candidate| candidate.date_evidence.clone())
            .ok_or_else(ApiError::internal)?;
        (snapshot, evidence)
    };
    let body = serde_json::to_vec(&SourceDetailDto {
        id: source_id,
        source_note_name,
        local_time,
        body: snapshot.body.clone(),
        content_digest: snapshot.stamp.content_digest.as_str().to_owned(),
        directory_day: evidence.directory_day.map(|day| day.as_str().to_owned()),
        header_day: evidence.header_day.map(|day| day.as_str().to_owned()),
        warning_code: evidence.warning.map(|_| "source-warning".to_owned()),
    })
    .map_err(|_| ApiError::internal())?;
    if body.len() > MAX_SOURCE_BYTES + 64 * 1024 {
        return Err(ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "response-limit",
        ));
    }
    Ok(([(CONTENT_TYPE, "application/json")], body).into_response())
}

pub(super) async fn start_source_refine(
    State(state): State<AppState>,
    Path((id, source_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(request): Json<RefineSourceRequest>,
) -> Result<(StatusCode, Json<JobAcceptedDto>), ApiError> {
    validate_string(&source_id)?;
    validate_string(&request.content_digest)?;
    let digest = Digest::parse(request.content_digest)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid-content-digest"))?;
    let day = day_from_id(&id)?;
    let revision = workflow_precondition(&headers)?;
    let service = state.0.config.commands.clone();
    let lookup = day.clone();
    let workflow = blocking(move || service.load_workflow(&lookup)).await?;
    let source = workflow
        .candidates()
        .iter()
        .find(|value| opaque_source_id(&value.source) == source_id)
        .map(|value| value.source.clone())
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "source-not-found"))?;
    let job_id = new_job(&state, "assistant-source-refine", &id)?;
    tokio::spawn(run_assistant_job(
        state,
        job_id.clone(),
        id,
        day,
        AssistantJob::Refine {
            revision,
            source,
            digest,
        },
    ));
    Ok((StatusCode::ACCEPTED, Json(JobAcceptedDto { job_id })))
}

pub(super) async fn get_source_refine_suggestion(
    State(state): State<AppState>,
    Path((id, source_id)): Path<(String, String)>,
) -> Result<Json<SourceRefineSuggestionDto>, ApiError> {
    validate_string(&source_id)?;
    let day = day_from_id(&id)?;
    let service = state.0.config.commands.clone();
    let workflow = blocking(move || service.load_workflow(&day)).await?;
    let suggestion = workflow
        .source_refine_suggestions()
        .iter()
        .find(|value| opaque_source_id(&value.source) == source_id)
        .ok_or_else(|| {
            ApiError::new(StatusCode::NOT_FOUND, "source-refine-suggestion-not-found")
        })?;
    Ok(Json(SourceRefineSuggestionDto {
        source_id,
        original_body: suggestion.original_body.clone(),
        suggested_body: suggestion.suggested_body.clone(),
        binding_digest: suggestion.binding_digest.to_string(),
    }))
}

pub(super) async fn resolve_source_refine(
    State(state): State<AppState>,
    Path((id, source_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(request): Json<ResolveSourceRefineRequest>,
) -> Result<Response, ApiError> {
    ensure_no_assistant_job(&state, &id)?;
    let revision = workflow_precondition(&headers)?;
    let day = day_from_id(&id)?;
    let digest = Digest::parse(request.binding_digest)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid-content-digest"))?;
    let service = state.0.config.commands.clone();
    let lookup = day.clone();
    let workflow = blocking(move || service.load_workflow(&lookup)).await?;
    let source = workflow
        .candidates()
        .iter()
        .find(|value| opaque_source_id(&value.source) == source_id)
        .map(|value| value.source.clone())
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "source-not-found"))?;
    let service = state.0.config.commands.clone();
    let updated = blocking(move || {
        service.resolve_source_refinement(&day, revision, &source, request.accept, &digest)
    })
    .await?;
    publish_event(
        &state,
        "workflow.changed",
        Some(id),
        Some(updated.revision()),
        None,
        None,
    );
    workflow_response(StatusCode::OK, &updated)
}

pub(super) async fn edit_source_body(
    State(state): State<AppState>,
    Path((id, source_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(request): Json<EditSourceBodyRequest>,
) -> Result<Response, ApiError> {
    ensure_no_assistant_job(&state, &id)?;
    if request.text.len() > MAX_SOURCE_BYTES {
        return Err(ApiError::new(StatusCode::PAYLOAD_TOO_LARGE, "string-limit"));
    }
    let revision = workflow_precondition(&headers)?;
    let day = day_from_id(&id)?;
    let digest = Digest::parse(request.expected_content_digest)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid-content-digest"))?;
    let service = state.0.config.commands.clone();
    let lookup = day.clone();
    let workflow = blocking(move || service.load_workflow(&lookup)).await?;
    let source = workflow
        .candidates()
        .iter()
        .find(|value| opaque_source_id(&value.source) == source_id)
        .map(|value| value.source.clone())
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "source-not-found"))?;
    let service = state.0.config.commands.clone();
    let updated =
        blocking(move || service.edit_source_body(&day, revision, &source, &request.text, &digest))
            .await?;
    publish_event(
        &state,
        "workflow.changed",
        Some(id),
        Some(updated.revision()),
        None,
        None,
    );
    workflow_response(StatusCode::OK, &updated)
}

pub(super) async fn delete_source_route(
    State(state): State<AppState>,
    Path((id, source_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(request): Json<DeleteSourceRequest>,
) -> Result<Response, ApiError> {
    ensure_no_assistant_job(&state, &id)?;
    let revision = workflow_precondition(&headers)?;
    let day = day_from_id(&id)?;
    let service = state.0.config.commands.clone();
    let lookup = day.clone();
    let workflow = blocking(move || service.load_workflow(&lookup)).await?;
    let source = workflow
        .candidates()
        .iter()
        .find(|value| opaque_source_id(&value.source) == source_id)
        .map(|value| value.source.clone())
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "source-not-found"))?;
    let service = state.0.config.commands.clone();
    let updated =
        blocking(move || service.delete_source(&day, revision, &source, request.confirm)).await?;
    publish_event(
        &state,
        "workflow.changed",
        Some(id),
        Some(updated.revision()),
        None,
        None,
    );
    workflow_response(StatusCode::OK, &updated)
}

pub(super) async fn prepare_sources(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<PrepareSourcesRequest>,
) -> Result<(StatusCode, Json<JobAcceptedDto>), ApiError> {
    let day = day_from_id(&id)?;
    let revision = workflow_precondition(&headers)?;
    if request.source_ids.is_empty() || request.source_ids.len() > MAX_SELECTED_SOURCES {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid-source-selection",
        ));
    }
    validate_unique_strings(&request.source_ids)?;
    let service = state.0.config.commands.clone();
    let lookup_day = day.clone();
    let workflow = blocking(move || service.load_workflow(&lookup_day)).await?;
    let mut sources = Vec::new();
    for source_id in &request.source_ids {
        let source = workflow
            .candidates()
            .iter()
            .find(|candidate| opaque_source_id(&candidate.source) == *source_id)
            .map(|candidate| candidate.source.clone())
            .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "unknown-source"))?;
        sources.push(source);
    }
    let job_id = new_job(&state, "source-preparation", &id)?;
    let worker_state = state.clone();
    let worker_job = job_id.clone();
    tokio::spawn(async move {
        run_prepare_job(
            worker_state,
            worker_job,
            id,
            day,
            revision,
            request.source_ids,
            sources,
        )
        .await;
    });
    Ok((StatusCode::ACCEPTED, Json(JobAcceptedDto { job_id })))
}

pub(super) async fn confirm_selection(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<ConfirmSelectionRequest>,
) -> Result<Response, ApiError> {
    let day = day_from_id(&id)?;
    let revision = workflow_precondition(&headers)?;
    validate_string(&request.preparation_handle)?;
    if request.source_ids.is_empty() || request.source_ids.len() > MAX_SELECTED_SOURCES {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid-source-selection",
        ));
    }
    validate_unique_strings(&request.source_ids)?;
    let selected = take_preparation(
        &state,
        &request.preparation_handle,
        &id,
        revision,
        &request.source_ids,
    )?;
    let service = state.0.config.commands.clone();
    let workflow = blocking(move || service.confirm_selection(&day, revision, selected)).await?;
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

pub(super) async fn confirm_empty_selection(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let day = day_from_id(&id)?;
    let revision = workflow_precondition(&headers)?;
    let service = state.0.config.commands.clone();
    let workflow = blocking(move || service.confirm_empty_selection(&day, revision)).await?;
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
