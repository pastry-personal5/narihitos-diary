//! Assistant routes at the HTTP boundary.

use super::*;

pub(super) async fn set_writing_settings(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<WritingSettingsRequest>,
) -> Result<Response, ApiError> {
    let day = day_from_id(&id)?;
    let revision = workflow_precondition(&headers)?;
    validate_string(&request.language)?;
    validate_string(&request.style)?;
    validate_optional_string(request.assistant_backend_id.as_deref())?;
    let language = WritingLanguage::parse(&request.language)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid-language"))?;
    let style = WritingStyle::parse(&request.style)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid-style"))?;
    let selection = if let Some(raw_id) = request.assistant_backend_id {
        let backend_id = AssistantBackendId::parse(raw_id)
            .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid-assistant-backend"))?;
        let descriptor = state
            .0
            .config
            .assistant_registry
            .descriptors()
            .into_iter()
            .find(|descriptor| descriptor.id == backend_id)
            .ok_or_else(|| ApiError::from(CommandError::UnavailableAssistantBackend))?;
        if !descriptor.available {
            return Err(ApiError::from(CommandError::UnavailableAssistantBackend));
        }
        Some(AssistantSelection::new(
            backend_id,
            descriptor.composition_path,
        ))
    } else {
        None
    };
    let service = state.0.config.commands.clone();
    let workflow =
        blocking(move || service.set_writing_settings(&day, revision, language, style, selection))
            .await?;
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

pub(super) async fn get_assistant_selection(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<AssistantSelectionDto>, ApiError> {
    let day = day_from_id(&id)?;
    let service = state.0.config.commands.clone();
    let workflow = blocking(move || service.load_workflow(&day)).await?;
    Ok(Json(assistant_selection_dto(&workflow)))
}

pub(super) async fn put_assistant_selection(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<PutAssistantSelectionRequest>,
) -> Result<Response, ApiError> {
    let day = day_from_id(&id)?;
    let revision = workflow_precondition(&headers)?;
    validate_optional_string(request.backend_id.as_deref())?;
    let selection = if let Some(raw_id) = request.backend_id {
        let backend_id = AssistantBackendId::parse(raw_id)
            .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid-assistant-backend"))?;
        let descriptor = state
            .0
            .config
            .assistant_registry
            .descriptors()
            .into_iter()
            .find(|descriptor| descriptor.id == backend_id)
            .ok_or_else(|| ApiError::from(CommandError::UnavailableAssistantBackend))?;
        if !descriptor.available {
            return Err(ApiError::from(CommandError::UnavailableAssistantBackend));
        }
        Some(AssistantSelection::new(
            backend_id,
            descriptor.composition_path,
        ))
    } else {
        None
    };
    let service = state.0.config.commands.clone();
    let workflow =
        blocking(move || service.set_assistant_selection(&day, revision, selection)).await?;
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

pub(super) fn assistant_selection_dto(workflow: &Workflow) -> AssistantSelectionDto {
    AssistantSelectionDto {
        backend_id: workflow
            .assistant_selection()
            .map(|selection| selection.backend_id.to_string()),
        composition_path: workflow
            .composition_path()
            .map(CompositionPath::as_str)
            .map(str::to_owned),
    }
}

pub(super) async fn start_memory_extraction(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(_request): Json<StartMemoryExtractionRequest>,
) -> Result<(StatusCode, Json<JobAcceptedDto>), ApiError> {
    let day = day_from_id(&id)?;
    let revision = workflow_precondition(&headers)?;
    let artifact_id = random_id("memory_").map_err(|_| ApiError::internal())?;
    let job_id = new_job(&state, "assistant-memory-extraction", &id)?;
    tokio::spawn(run_assistant_job(
        state,
        job_id.clone(),
        id,
        day,
        AssistantJob::Extract {
            revision,
            artifact_id,
        },
    ));
    Ok((StatusCode::ACCEPTED, Json(JobAcceptedDto { job_id })))
}

pub(super) async fn get_memory_review(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<MemoryReviewDto>, ApiError> {
    let day = day_from_id(&id)?;
    let service = state.0.config.commands.clone();
    let workflow = blocking(move || service.load_workflow(&day)).await?;
    let set = workflow.memory_candidate_sets().last();
    let reviewed = workflow.reviewed_memory_artifact();
    Ok(Json(MemoryReviewDto {
        candidate_set_id: set.map(|value| value.id.clone()),
        candidates: set
            .map(|value| {
                value
                    .candidates
                    .iter()
                    .map(|candidate| MemoryCandidateDto {
                        id: candidate.id.clone(),
                        text: candidate.text.clone(),
                        source_snapshot_digests: candidate
                            .source_snapshot_digests
                            .iter()
                            .map(ToString::to_string)
                            .collect(),
                        stale: value.stale,
                    })
                    .collect()
            })
            .unwrap_or_default(),
        reviewed_memory: reviewed.map_or_else(String::new, |value| value.text.clone()),
        reviewed_memory_revision: reviewed.map_or(0, |value| value.revision),
        stale: set.is_some_and(|value| value.stale) || reviewed.is_some_and(|value| value.stale),
    }))
}

pub(super) async fn put_memory_review(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<PutMemoryReviewRequest>,
) -> Result<Response, ApiError> {
    if request.reviewed_memory.len() > MAX_REVIEWED_MEMORY_BYTES {
        return Err(ApiError::new(StatusCode::PAYLOAD_TOO_LARGE, "string-limit"));
    }
    validate_string(&request.candidate_set_id)?;
    let day = day_from_id(&id)?;
    let revision = workflow_precondition(&headers)?;
    let artifact_id = random_id("reviewed_").map_err(|_| ApiError::internal())?;
    let service = state.0.config.commands.clone();
    let workflow = blocking(move || {
        service.replace_reviewed_memory(
            &day,
            revision,
            artifact_id,
            &request.candidate_set_id,
            request.reviewed_memory,
        )
    })
    .await?;
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

pub(super) async fn start_draft_request(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<StartDraftRequest>,
) -> Result<(StatusCode, Json<JobAcceptedDto>), ApiError> {
    let day = day_from_id(&id)?;
    let revision = workflow_precondition(&headers)?;
    let artifact_id = random_id("draft_").map_err(|_| ApiError::internal())?;
    let job_id = new_job(&state, "assistant-draft", &id)?;
    tokio::spawn(run_assistant_job(
        state,
        job_id.clone(),
        id,
        day,
        AssistantJob::Draft {
            revision,
            artifact_id,
            include_prior: request.include_prior_draft,
        },
    ));
    Ok((StatusCode::ACCEPTED, Json(JobAcceptedDto { job_id })))
}

pub(super) async fn get_draft_result(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<DraftResultDto>, ApiError> {
    let day = day_from_id(&id)?;
    let service = state.0.config.commands.clone();
    let workflow = blocking(move || service.load_workflow(&day)).await?;
    let draft = workflow
        .draft_proposals()
        .last()
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "assistant-draft-not-found"))?;
    let provenance = draft
        .provenance
        .as_ref()
        .ok_or_else(|| ApiError::new(StatusCode::CONFLICT, "assistant-binding-stale"))?;
    Ok(Json(DraftResultDto {
        id: draft.id.clone(),
        backend_id: provenance.backend_id.to_string(),
        composition_path: provenance.composition_path.as_str().to_owned(),
        text: draft.text.clone(),
        reflection_ranges: range_dtos(&draft.reflection_ranges),
        stale: draft.stale,
        populated_buffer_version: draft.populated_buffer_version,
    }))
}

pub(super) async fn start_paragraph_review(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<StartParagraphReviewRequest>,
) -> Result<(StatusCode, Json<JobAcceptedDto>), ApiError> {
    validate_string(&request.buffer_etag)?;
    validate_string(&request.selected_text_digest)?;
    let day = day_from_id(&id)?;
    let revision = workflow_precondition(&headers)?;
    let buffer = parse_buffer_etag(&request.buffer_etag)
        .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "invalid-buffer-etag"))?;
    let digest = Digest::parse(request.selected_text_digest)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid-text-digest"))?;
    let service = state.0.config.commands.clone();
    let lookup_day = day.clone();
    let workflow = blocking(move || service.load_workflow(&lookup_day)).await?;
    let range = ReflectionRange {
        start: request.range_start,
        end: request.range_end,
    };
    let selected = workflow
        .entry()
        .text
        .get(range.start..range.end)
        .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "invalid-paragraph-range"))?;
    if workflow.entry().version != buffer || Digest::of_bytes(selected.as_bytes()) != digest {
        return Err(ApiError::new(
            StatusCode::PRECONDITION_FAILED,
            "assistant-binding-stale",
        ));
    }
    let suggestion_id = random_id("suggestion_").map_err(|_| ApiError::internal())?;
    let job_id = new_job(&state, "assistant-paragraph-review", &id)?;
    tokio::spawn(run_assistant_job(
        state,
        job_id.clone(),
        id,
        day,
        AssistantJob::Paragraph {
            revision,
            suggestion_id,
            range,
        },
    ));
    Ok((StatusCode::ACCEPTED, Json(JobAcceptedDto { job_id })))
}

pub(super) async fn list_paragraph_suggestions(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<ParagraphSuggestionListDto>, ApiError> {
    let day = day_from_id(&id)?;
    let service = state.0.config.commands.clone();
    let workflow = blocking(move || service.load_workflow(&day)).await?;
    Ok(Json(ParagraphSuggestionListDto {
        suggestions: workflow
            .paragraph_suggestions()
            .iter()
            .map(|suggestion| ParagraphSuggestionSummaryDto {
                id: suggestion.id.clone(),
                backend_id: suggestion.provenance.backend_id.to_string(),
                composition_path: suggestion.provenance.composition_path.as_str().to_owned(),
                stale: suggestion.stale,
                resolution: suggestion.resolution.map(|value| value.as_str().to_owned()),
            })
            .collect(),
    }))
}

pub(super) async fn get_paragraph_suggestion(
    State(state): State<AppState>,
    Path((id, suggestion_id)): Path<(String, String)>,
) -> Result<Json<ParagraphSuggestionDto>, ApiError> {
    validate_string(&suggestion_id)?;
    let day = day_from_id(&id)?;
    let service = state.0.config.commands.clone();
    let workflow = blocking(move || service.load_workflow(&day)).await?;
    let suggestion = workflow
        .paragraph_suggestions()
        .iter()
        .find(|suggestion| suggestion.id == suggestion_id)
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "assistant-suggestion-not-found"))?;
    Ok(Json(paragraph_suggestion_dto(suggestion)))
}

pub(super) async fn resolve_paragraph_suggestion(
    State(state): State<AppState>,
    Path((id, suggestion_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(request): Json<ResolveParagraphSuggestionRequest>,
) -> Result<Response, ApiError> {
    validate_string(&suggestion_id)?;
    validate_string(&request.buffer_etag)?;
    validate_string(&request.selected_text_digest)?;
    let resolution = match request.resolution.as_str() {
        "accept" => SuggestionResolution::Accepted,
        "reject" => SuggestionResolution::Rejected,
        _ => return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid-resolution")),
    };
    let revision = workflow_precondition(&headers)?;
    let day = day_from_id(&id)?;
    let service = state.0.config.commands.clone();
    let lookup_day = day.clone();
    let workflow = blocking(move || service.load_workflow(&lookup_day)).await?;
    let suggestion = workflow
        .paragraph_suggestions()
        .iter()
        .find(|suggestion| suggestion.id == suggestion_id)
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "assistant-suggestion-not-found"))?;
    if parse_buffer_etag(&request.buffer_etag) != Some(suggestion.binding.buffer_version)
        || request.range_start != suggestion.binding.range.start
        || request.range_end != suggestion.binding.range.end
        || request.selected_text_digest != suggestion.binding.selected_text_digest.as_str()
    {
        return Err(ApiError::new(
            StatusCode::PRECONDITION_FAILED,
            "assistant-binding-stale",
        ));
    }
    let service = state.0.config.commands.clone();
    let suggestion_for_write = suggestion_id.clone();
    let updated = blocking(move || {
        service.resolve_paragraph_suggestion(&day, revision, &suggestion_for_write, resolution)
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

pub(super) fn range_dtos(ranges: &[ReflectionRange]) -> Vec<ReflectionRangeDto> {
    ranges
        .iter()
        .map(|range| ReflectionRangeDto {
            start: range.start,
            end: range.end,
        })
        .collect()
}

pub(super) fn paragraph_suggestion_dto(
    suggestion: &diary_core::ParagraphSuggestion,
) -> ParagraphSuggestionDto {
    ParagraphSuggestionDto {
        id: suggestion.id.clone(),
        backend_id: suggestion.provenance.backend_id.to_string(),
        composition_path: suggestion.provenance.composition_path.as_str().to_owned(),
        replacement: suggestion.replacement.clone(),
        reflection_ranges: range_dtos(&suggestion.reflection_ranges),
        explanation: suggestion.explanation.clone(),
        buffer_version: suggestion.binding.buffer_version,
        range_start: suggestion.binding.range.start,
        range_end: suggestion.binding.range.end,
        selected_text_digest: suggestion.binding.selected_text_digest.to_string(),
        stale: suggestion.stale,
        resolution: suggestion.resolution.map(|value| value.as_str().to_owned()),
    }
}

pub(super) async fn begin_manual_composition(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let day = day_from_id(&id)?;
    let revision = workflow_precondition(&headers)?;
    let service = state.0.config.commands.clone();
    let workflow = blocking(move || service.begin_manual_composition(&day, revision)).await?;
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

pub(super) async fn save_entry(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<SaveEntryRequest>,
) -> Result<Response, ApiError> {
    let day = day_from_id(&id)?;
    let buffer = buffer_precondition(&headers)?;
    let service = state.0.config.commands.clone();
    let workflow = blocking(move || service.save_entry(&day, buffer, request.text)).await?;
    publish_event(
        &state,
        "entry.changed",
        Some(id),
        Some(workflow.revision()),
        None,
        None,
    );
    workflow_response(StatusCode::OK, &workflow)
}

pub(super) async fn confirm_entry(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<ConfirmEntryRequest>,
) -> Result<Response, ApiError> {
    let day = day_from_id(&id)?;
    let revision = workflow_precondition(&headers)?;
    validate_string(&request.buffer_etag)?;
    let buffer = parse_buffer_etag(&request.buffer_etag)
        .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "invalid-buffer-etag"))?;
    let service = state.0.config.commands.clone();
    let workflow = blocking(move || service.confirm_entry(&day, revision, buffer)).await?;
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
