//! Workflow routes at the HTTP boundary.

use super::*;

pub(super) async fn get_preferences(
    State(state): State<AppState>,
) -> Result<Json<PreferencesDto>, ApiError> {
    let service = state.0.config.commands.clone();
    let preferences = blocking(move || service.preferences()).await?;
    Ok(Json(preferences_dto(&preferences)))
}

pub(super) async fn put_preferences(
    State(state): State<AppState>,
    Json(request): Json<PreferencesDto>,
) -> Result<Json<PreferencesDto>, ApiError> {
    validate_optional_string(request.last_language.as_deref())?;
    validate_optional_string(request.last_assistant_backend_id.as_deref())?;
    let preferences = parse_preferences(request)?;
    let result = preferences.clone();
    let service = state.0.config.commands.clone();
    blocking(move || service.save_preferences(&preferences)).await?;
    Ok(Json(preferences_dto(&result)))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ListQuery {
    cursor: Option<usize>,
    limit: Option<usize>,
}

pub(super) async fn list_workflows(
    State(state): State<AppState>,
    Query(query): Query<ListQuery>,
) -> Result<Json<WorkflowListDto>, ApiError> {
    let offset = query.cursor.unwrap_or(0);
    let limit = query.limit.unwrap_or(WORKFLOW_PAGE_SIZE);
    if limit == 0 || limit > WORKFLOW_PAGE_SIZE || offset > MAX_WORKFLOWS {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid-pagination"));
    }
    let service = state.0.config.commands.clone();
    let summaries = blocking(move || service.list_workflows(MAX_WORKFLOWS)).await?;
    let page: Vec<_> = summaries
        .iter()
        .skip(offset)
        .take(limit)
        .map(summary_dto)
        .collect();
    let next = (offset + page.len() < summaries.len()).then(|| (offset + page.len()).to_string());
    Ok(Json(WorkflowListDto {
        workflows: page,
        next_cursor: next,
    }))
}

pub(super) async fn start_workflow(
    State(state): State<AppState>,
    Json(request): Json<StartWorkflowRequest>,
) -> Result<Response, ApiError> {
    validate_optional_string(request.diary_day.as_deref())?;
    let day = request
        .diary_day
        .map(DiaryDay::parse)
        .transpose()
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid-diary-day"))?;
    let language = request
        .language
        .as_deref()
        .map(WritingLanguage::parse)
        .transpose()
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid-language"))?;
    let style = request
        .style
        .as_deref()
        .map(WritingStyle::parse)
        .transpose()
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid-style"))?;
    let service = state.0.config.commands.clone();
    let descriptors = state.0.config.assistant_registry.descriptors();
    let outcome =
        blocking(move || service.start_workflow_with_settings(day, language, style, &descriptors))
            .await?;
    let (status, workflow, created) = match outcome {
        StartOutcome::Created(workflow) => (StatusCode::CREATED, workflow, true),
        StartOutcome::Resumed(workflow) | StartOutcome::Completed(workflow) => {
            (StatusCode::OK, workflow, false)
        }
    };
    if created {
        publish_event(
            &state,
            "workflow.changed",
            Some(workflow_id(workflow.day().as_str()).expect("validated diary day")),
            Some(workflow.revision()),
            None,
            None,
        );
    }
    workflow_response(status, &workflow)
}

pub(super) async fn get_workflow(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let day = day_from_id(&id)?;
    let service = state.0.config.commands.clone();
    let workflow = blocking(move || service.load_workflow(&day)).await?;
    workflow_response(StatusCode::OK, &workflow)
}

pub(super) async fn discard_workflow(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let day = day_from_id(&id)?;
    let revision = workflow_precondition(&headers)?;
    let service = state.0.config.commands.clone();
    blocking(move || service.discard(&day, revision)).await?;
    publish_event(
        &state,
        "workflow.changed",
        Some(id),
        Some(revision + 1),
        None,
        None,
    );
    Ok(StatusCode::NO_CONTENT)
}
