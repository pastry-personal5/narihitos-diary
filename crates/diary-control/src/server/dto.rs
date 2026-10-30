//! Wire mapping, request validation, and conditional headers.

use super::*;

pub(super) fn workflow_response(
    status: StatusCode,
    workflow: &Workflow,
) -> Result<Response, ApiError> {
    let body = serde_json::to_vec(&workflow_dto(workflow)).map_err(|_| ApiError::internal())?;
    if body.len() > MAX_RESPONSE_BYTES {
        return Err(ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "response-limit",
        ));
    }
    let mut response = (status, [(CONTENT_TYPE, "application/json")], body).into_response();
    response.headers_mut().insert(
        ETAG,
        HeaderValue::from_str(&workflow_etag(workflow.revision()))
            .map_err(|_| ApiError::internal())?,
    );
    response.headers_mut().insert(
        "x-entry-buffer-etag",
        HeaderValue::from_str(&buffer_etag(workflow.entry().version))
            .map_err(|_| ApiError::internal())?,
    );
    Ok(response)
}

pub(super) fn workflow_dto(workflow: &Workflow) -> WorkflowDto {
    WorkflowDto {
        id: workflow_id(workflow.day().as_str()).expect("validated diary day"),
        diary_day: workflow.day().as_str().to_owned(),
        state: workflow.state().as_str().to_owned(),
        revision: workflow.revision(),
        buffer_version: workflow.entry().version,
        language: workflow.language().map(|value| value.as_str().to_owned()),
        style: workflow.style().as_str().to_owned(),
        assistant_backend_id: workflow
            .assistant_selection()
            .map(|selection| selection.backend_id.as_str().to_owned()),
        composition_path: workflow
            .composition_path()
            .map(|path| path.as_str().to_owned()),
        selection_digest: workflow
            .selection_digest()
            .map(|value| value.as_str().to_owned()),
        explicit_empty_selection: workflow.explicit_empty_selection(),
        invalidation_code: workflow.invalidation_reason().map(invalidation_code),
        entry_text: workflow.entry().text.clone(),
        entry_confirmed: workflow.entry().confirmed.is_some(),
        local_status: if workflow.state() == diary_core::WorkflowState::Publishing {
            Some("reconciling".to_owned())
        } else {
            workflow
                .destination_progress()
                .iter()
                .find(|item| item.destination == "local")
                .map(|item| item.status.as_str().to_owned())
        },
        notion_status: workflow
            .destination_progress()
            .iter()
            .find(|item| item.destination == "notion")
            .map(|item| item.status.as_str().to_owned()),
        icloud_status: workflow
            .destination_progress()
            .iter()
            .find(|item| item.destination == "icloud")
            .map(|item| item.status.as_str().to_owned()),
        amendment: workflow.amendment().map(|seed| AmendmentDto {
            hand_edited: seed.hand_edited,
        }),
        candidates: candidate_dtos(workflow),
    }
}

pub(super) fn summary_dto(summary: &diary_core::WorkflowSummary) -> WorkflowSummaryDto {
    WorkflowSummaryDto {
        id: workflow_id(summary.day.as_str()).expect("validated diary day"),
        diary_day: summary.day.as_str().to_owned(),
        state: summary.state.as_str().to_owned(),
        revision: summary.revision,
        buffer_version: summary.buffer_version,
        updated_at_unix_ms: summary.updated_at.0.saturating_mul(1000),
        discarded: summary.discarded_at.is_some(),
    }
}

pub(super) fn candidate_dtos(workflow: &Workflow) -> Vec<SourceCandidateDto> {
    workflow
        .candidates()
        .iter()
        .map(|candidate| SourceCandidateDto {
            id: opaque_source_id(&candidate.source),
            source_note_name: candidate.source_note_name.clone(),
            local_time: crate::local_time::local_time(&candidate.source_note_name),
            directory_day: candidate
                .date_evidence
                .directory_day
                .as_ref()
                .map(|day| day.as_str().to_owned()),
            header_day: candidate
                .date_evidence
                .header_day
                .as_ref()
                .map(|day| day.as_str().to_owned()),
            warning_code: candidate
                .date_evidence
                .warning
                .as_ref()
                .map(|_| "source-warning".to_owned()),
            availability: candidate.availability.as_str().to_owned(),
            selected: workflow
                .snapshots()
                .iter()
                .any(|snapshot| snapshot.source == candidate.source),
        })
        .collect()
}

pub(super) fn opaque_source_id(source: &SourceRef) -> String {
    format!(
        "src_{}",
        Digest::of_bytes(source.stable_key().as_bytes()).as_str()
    )
}

pub(super) fn invalidation_code(reason: &InvalidationReason) -> String {
    match reason {
        InvalidationReason::SelectionChanged => "selection-changed",
        InvalidationReason::SourceChanged(_) => "source-changed",
        InvalidationReason::FreshnessUnverifiable(_) => "freshness-unverifiable",
        InvalidationReason::EntryEdited => "entry-edited",
        InvalidationReason::CompositionChoiceChanged => "composition-choice-changed",
        InvalidationReason::OutputBindingChanged => "output-binding-changed",
    }
    .to_owned()
}

pub(super) fn preferences_dto(preferences: &Preferences) -> PreferencesDto {
    PreferencesDto {
        last_language: preferences
            .last_language
            .map(|value| value.as_str().to_owned()),
        last_assistant_backend_id: preferences
            .last_assistant_backend_id
            .as_ref()
            .map(|backend_id| backend_id.as_str().to_owned()),
    }
}

pub(super) fn optional_single_header<'a>(
    headers: &'a HeaderMap,
    name: &'static str,
) -> Result<Option<&'a str>, ApiError> {
    let values = headers.get_all(name).iter().collect::<Vec<_>>();
    match values.as_slice() {
        [] => Ok(None),
        [value] => value
            .to_str()
            .map(Some)
            .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid-header")),
        _ => Err(ApiError::new(StatusCode::BAD_REQUEST, "duplicate-header")),
    }
}

pub(super) fn validate_string(value: &str) -> Result<(), ApiError> {
    if value.len() > MAX_STRING_BYTES {
        Err(ApiError::new(StatusCode::PAYLOAD_TOO_LARGE, "string-limit"))
    } else {
        Ok(())
    }
}

pub(super) fn validate_optional_string(value: Option<&str>) -> Result<(), ApiError> {
    if let Some(value) = value {
        validate_string(value)?;
    }
    Ok(())
}

pub(super) fn validate_unique_strings(values: &[String]) -> Result<(), ApiError> {
    let mut seen = BTreeSet::new();
    for value in values {
        validate_string(value)?;
        if !seen.insert(value.as_str()) {
            return Err(ApiError::new(StatusCode::BAD_REQUEST, "duplicate-source"));
        }
    }
    Ok(())
}

pub(super) fn parse_preferences(dto: PreferencesDto) -> Result<Preferences, ApiError> {
    let language = dto
        .last_language
        .as_deref()
        .map(WritingLanguage::parse)
        .transpose()
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid-language"))?;
    let backend_id = dto
        .last_assistant_backend_id
        .map(AssistantBackendId::parse)
        .transpose()
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid-assistant-backend-id"))?;
    if backend_id.is_some() {
        return Err(ApiError::from(CommandError::UnavailableAssistantBackend));
    }
    Ok(Preferences {
        last_language: language,
        last_assistant_backend_id: None,
    })
}

pub(super) fn day_from_id(id: &str) -> Result<DiaryDay, ApiError> {
    workflow_day(id)
        .and_then(|day| DiaryDay::parse(day).ok())
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "workflow-not-found"))
}

pub(super) fn workflow_precondition(headers: &HeaderMap) -> Result<u64, ApiError> {
    let value = headers
        .get(IF_MATCH)
        .ok_or_else(|| ApiError::new(StatusCode::PRECONDITION_REQUIRED, "precondition-required"))?;
    parse_workflow_etag(value.to_str().unwrap_or_default())
        .ok_or_else(|| ApiError::new(StatusCode::PRECONDITION_FAILED, "precondition-failed"))
}

pub(super) fn buffer_precondition(headers: &HeaderMap) -> Result<u64, ApiError> {
    let value = headers
        .get(IF_MATCH)
        .ok_or_else(|| ApiError::new(StatusCode::PRECONDITION_REQUIRED, "precondition-required"))?;
    parse_buffer_etag(value.to_str().unwrap_or_default())
        .ok_or_else(|| ApiError::new(StatusCode::PRECONDITION_FAILED, "precondition-failed"))
}

pub(super) fn job_dto(record: &JobRecord) -> JobDto {
    JobDto {
        id: record.id.clone(),
        kind: record.kind.to_owned(),
        status: record.status.to_owned(),
        workflow_id: record.workflow_id.clone(),
        progress_percent: record.progress,
        result_handle: record.result_handle.clone(),
        error_code: record.error_code.clone(),
        expires_at_unix_ms: record.expires_at_ms,
    }
}
