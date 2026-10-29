use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::body::{Body, Bytes, to_bytes};
use axum::extract::{DefaultBodyLimit, Path, Query, Request, State};
use axum::http::header::{
    AUTHORIZATION, CACHE_CONTROL, CONTENT_TYPE, ETAG, HOST, IF_MATCH, ORIGIN,
};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use diary_control_contract::*;
use diary_core::{
    CompositionPath, DiaryDay, Digest, InvalidationReason, Preferences, SelectedSource, SourceRef,
    StartOutcome, Workflow, WritingLanguage, WritingStyle,
};
use futures_core::Stream;
use serde::Deserialize;
use subtle::ConstantTimeEq;
use tokio::net::TcpListener;
use tokio::sync::{Semaphore, mpsc, oneshot};

use crate::commands::{CommandError, CommandService};
use crate::diagnostics::{DiagnosticEvent, DiagnosticLevel, DiagnosticLog};
use crate::lifecycle::random_id;

const JOB_EXPIRY_MS: i64 = 10 * 60 * 1000;
const HEARTBEAT: Duration = Duration::from_secs(15);
const STREAM_LIFETIME: Duration = Duration::from_secs(30 * 60);
const COMMIT_ACTIVE: u8 = 0;
const COMMITTING: u8 = 1;
const COMMIT_CANCELLED: u8 = 2;

#[derive(Clone)]
pub struct ServerConfig {
    pub instance_id: String,
    pub control_token: String,
    pub port: u16,
    pub commands: CommandService,
    pub apple_helper_available: bool,
    pub notion_configured: bool,
    pub diagnostics: Option<Arc<DiagnosticLog>>,
    pub diagnostic_failure: Option<mpsc::UnboundedSender<()>>,
}

#[derive(Clone)]
struct AppState(Arc<Inner>);

struct Inner {
    config: ServerConfig,
    jobs: Mutex<JobState>,
    events: Mutex<EventState>,
    subscriber_count: Arc<AtomicUsize>,
    workers: Arc<Semaphore>,
    requests: Arc<Semaphore>,
}

#[derive(Default)]
struct JobState {
    jobs: BTreeMap<String, JobRecord>,
    preparations: BTreeMap<String, PreparedRecord>,
}

#[derive(Clone)]
struct JobRecord {
    id: String,
    kind: &'static str,
    status: &'static str,
    workflow_id: String,
    progress: u8,
    result_handle: Option<String>,
    error_code: Option<String>,
    expires_at_ms: i64,
    commit_state: Arc<AtomicU8>,
}

struct PreparedRecord {
    workflow_id: String,
    workflow_revision: u64,
    source_ids: Vec<String>,
    selected: Vec<SelectedSource>,
    expires_at_ms: i64,
}

#[derive(Default)]
struct EventState {
    next_id: u64,
    next_subscriber_id: u64,
    replay: VecDeque<EventDto>,
    subscribers: Vec<Subscriber>,
}

struct Subscriber {
    id: u64,
    sender: mpsc::Sender<Result<Bytes, Infallible>>,
    workflow_id: Option<String>,
    overflow: Option<oneshot::Sender<()>>,
}

pub struct ServerHandle {
    pub router: Router,
}

pub fn build_router(config: ServerConfig) -> ServerHandle {
    let state = AppState(Arc::new(Inner {
        config,
        jobs: Mutex::new(JobState::default()),
        events: Mutex::new(EventState {
            next_id: 1,
            ..EventState::default()
        }),
        subscriber_count: Arc::new(AtomicUsize::new(0)),
        workers: Arc::new(Semaphore::new(JOB_CONCURRENCY)),
        requests: Arc::new(Semaphore::new(64)),
    }));
    let router = Router::new()
        .route("/v1/health", get(health))
        .route("/v1/capabilities", get(capabilities))
        .route("/v1/setup-check", get(setup_check))
        .route("/v1/preferences", get(get_preferences).put(put_preferences))
        .route("/v1/workflows", get(list_workflows).post(start_workflow))
        .route(
            "/v1/workflows/{id}",
            get(get_workflow).delete(discard_workflow),
        )
        .route("/v1/workflows/{id}/discoveries", post(discover))
        .route("/v1/workflows/{id}/sources", get(list_sources))
        .route("/v1/workflows/{id}/sources/{source_id}", get(source_detail))
        .route("/v1/workflows/{id}/sources/prepare", post(prepare_sources))
        .route("/v1/workflows/{id}/selection", put(confirm_selection))
        .route(
            "/v1/workflows/{id}/empty-selection-confirmations",
            post(confirm_empty_selection),
        )
        .route(
            "/v1/workflows/{id}/writing-settings",
            put(set_writing_settings),
        )
        .route(
            "/v1/workflows/{id}/manual-compositions",
            post(begin_manual_composition),
        )
        .route("/v1/workflows/{id}/entry", put(save_entry))
        .route(
            "/v1/workflows/{id}/entry-confirmations",
            post(confirm_entry),
        )
        .route("/v1/jobs/{id}", get(get_job).delete(cancel_job))
        .route("/v1/events", get(events))
        .layer(DefaultBodyLimit::max(MAX_REQUEST_BYTES))
        .layer(middleware::from_fn_with_state(state.clone(), security))
        .with_state(state);
    ServerHandle { router }
}

pub async fn serve(
    listener: TcpListener,
    config: ServerConfig,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<(), std::io::Error> {
    axum::serve(listener, build_router(config).router)
        .with_graceful_shutdown(shutdown)
        .await
}

pub async fn bind_loopback(port_fallback_count: u16) -> Result<(TcpListener, u16), &'static str> {
    if port_fallback_count > 16 {
        return Err("listener-port-count-invalid");
    }
    let last = 8772_u16.saturating_add(port_fallback_count);
    for port in 8771..=last {
        match TcpListener::bind(("127.0.0.1", port)).await {
            Ok(listener) => return Ok((listener, port)),
            Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => continue,
            Err(_) => return Err("listener-bind-failed"),
        }
    }
    Err("listener-ports-exhausted")
}

async fn security(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let started = Instant::now();
    let route = route_code(request.uri().path());
    let permit = match Arc::clone(&state.0.requests).try_acquire_owned() {
        Ok(permit) => permit,
        Err(_) => {
            return logged_response(
                &state,
                route,
                started,
                ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "request-capacity").into_response(),
            )
            .await;
        }
    };
    let expected_host = format!("127.0.0.1:{}", state.0.config.port);
    let hosts: Vec<_> = request.headers().get_all(HOST).iter().collect();
    if hosts.len() != 1 || hosts[0].to_str().ok() != Some(expected_host.as_str()) {
        return logged_response(
            &state,
            route,
            started,
            ApiError::new(StatusCode::BAD_REQUEST, "unsafe-host").into_response(),
        )
        .await;
    }
    if [
        "forwarded",
        "x-forwarded-for",
        "x-forwarded-host",
        "x-forwarded-proto",
        "x-real-ip",
    ]
    .iter()
    .any(|name| request.headers().contains_key(*name))
    {
        return logged_response(
            &state,
            route,
            started,
            ApiError::new(StatusCode::BAD_REQUEST, "proxy-headers-forbidden").into_response(),
        )
        .await;
    }
    let origins: Vec<_> = request.headers().get_all(ORIGIN).iter().collect();
    if origins.len() > 1 {
        return logged_response(
            &state,
            route,
            started,
            ApiError::new(StatusCode::FORBIDDEN, "unsafe-origin").into_response(),
        )
        .await;
    }
    if let Some(origin) = origins.first() {
        let expected_origin = format!("http://{expected_host}");
        if origin.to_str().ok() != Some(expected_origin.as_str()) {
            return logged_response(
                &state,
                route,
                started,
                ApiError::new(StatusCode::FORBIDDEN, "unsafe-origin").into_response(),
            )
            .await;
        }
    }
    if request.uri().path() != "/v1/health" {
        let authorizations: Vec<_> = request.headers().get_all(AUTHORIZATION).iter().collect();
        let Some(value) = authorizations
            .first()
            .filter(|_| authorizations.len() == 1)
            .and_then(|value| value.to_str().ok())
        else {
            return logged_response(
                &state,
                route,
                started,
                ApiError::new(StatusCode::UNAUTHORIZED, "authorization-required").into_response(),
            )
            .await;
        };
        let Some(token) = value.strip_prefix("Bearer ") else {
            return logged_response(
                &state,
                route,
                started,
                ApiError::new(StatusCode::UNAUTHORIZED, "authorization-required").into_response(),
            )
            .await;
        };
        if !constant_time_token(token, &state.0.config.control_token) {
            return logged_response(
                &state,
                route,
                started,
                ApiError::new(StatusCode::UNAUTHORIZED, "authorization-failed").into_response(),
            )
            .await;
        }
    }
    if source_helper_required(request.uri().path()) && !state.0.config.apple_helper_available {
        return logged_response(
            &state,
            route,
            started,
            ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "source-capability-unavailable",
            )
            .into_response(),
        )
        .await;
    }
    let (parts, body) = request.into_parts();
    let bytes = match to_bytes(body, MAX_REQUEST_BYTES).await {
        Ok(bytes) => bytes,
        Err(_) => {
            return logged_response(
                &state,
                route,
                started,
                ApiError::new(StatusCode::PAYLOAD_TOO_LARGE, "request-limit").into_response(),
            )
            .await;
        }
    };
    if !bytes.is_empty() && !json_depth_valid(&bytes, 16) {
        return logged_response(
            &state,
            route,
            started,
            ApiError::new(StatusCode::BAD_REQUEST, "json-depth-limit").into_response(),
        )
        .await;
    }
    let request = Request::from_parts(parts, Body::from(bytes));
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    let response = logged_response(&state, route, started, response).await;
    drop(permit);
    response
}

fn source_helper_required(path: &str) -> bool {
    path.ends_with("/discoveries") || path.ends_with("/sources/prepare")
}

fn route_code(path: &str) -> &'static str {
    if path == "/v1/health" {
        "health"
    } else if path == "/v1/capabilities" {
        "capabilities"
    } else if path == "/v1/setup-check" {
        "setup"
    } else if path == "/v1/preferences" {
        "preferences"
    } else if path == "/v1/workflows" {
        "workflows"
    } else if path.ends_with("/discoveries") {
        "discovery"
    } else if path.ends_with("/sources/prepare") {
        "preparation"
    } else if path.ends_with("/sources") {
        "sources"
    } else if path.contains("/sources/") {
        "source"
    } else if path.ends_with("/selection") || path.ends_with("/empty-selection-confirmations") {
        "selection"
    } else if path.ends_with("/writing-settings") {
        "settings"
    } else if path.ends_with("/manual-compositions") {
        "manual"
    } else if path.ends_with("/entry-confirmations") {
        "confirmation"
    } else if path.ends_with("/entry") {
        "entry"
    } else if path.starts_with("/v1/jobs/") {
        "job"
    } else if path == "/v1/events" {
        "events"
    } else if path.starts_with("/v1/workflows/") {
        "workflow"
    } else {
        "unknown"
    }
}

async fn logged_response(
    state: &AppState,
    route: &'static str,
    started: Instant,
    response: Response,
) -> Response {
    if let Some(log) = &state.0.config.diagnostics {
        let status = response.status();
        let outcome = if status.is_success() || status.is_redirection() {
            "ok"
        } else if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
            "denied"
        } else if matches!(
            status,
            StatusCode::TOO_MANY_REQUESTS | StatusCode::SERVICE_UNAVAILABLE
        ) {
            "busy"
        } else if status.is_client_error() {
            "client-error"
        } else {
            "failed"
        };
        let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let log = Arc::clone(log);
        let write = tokio::task::spawn_blocking(move || {
            log.write(
                DiagnosticLevel::Info,
                DiagnosticEvent::Http {
                    route,
                    outcome,
                    status: status.as_u16(),
                    duration_ms,
                },
            )
        })
        .await;
        if !matches!(write, Ok(Ok(())))
            && let Some(failure) = &state.0.config.diagnostic_failure
        {
            let _ = failure.send(());
        }
    }
    response
}

fn constant_time_token(candidate: &str, expected: &str) -> bool {
    candidate.len() == expected.len() && bool::from(candidate.as_bytes().ct_eq(expected.as_bytes()))
}

fn json_depth_valid(bytes: &[u8], maximum: usize) -> bool {
    let mut depth = 0_usize;
    let mut quoted = false;
    let mut escaped = false;
    for byte in bytes {
        if quoted {
            if escaped {
                escaped = false;
            } else if *byte == b'\\' {
                escaped = true;
            } else if *byte == b'"' {
                quoted = false;
            }
            continue;
        }
        match *byte {
            b'"' => quoted = true,
            b'{' | b'[' => {
                depth += 1;
                if depth > maximum {
                    return false;
                }
            }
            b'}' | b']' => {
                let Some(next) = depth.checked_sub(1) else {
                    return false;
                };
                depth = next;
            }
            _ => {}
        }
    }
    !quoted && depth == 0
}

async fn health(State(state): State<AppState>) -> Json<HealthDto> {
    Json(HealthDto {
        api_version: API_VERSION.to_owned(),
        contract_version: CONTRACT_VERSION,
        instance_id: state.0.config.instance_id.clone(),
        build_version: env!("CARGO_PKG_VERSION").to_owned(),
        ready: true,
    })
}

async fn capabilities(State(state): State<AppState>) -> Json<CapabilitiesDto> {
    let mut capabilities = vec!["read".to_owned(), "workflow:write".to_owned()];
    if state.0.config.apple_helper_available {
        capabilities.push("source:read".to_owned());
    }
    Json(CapabilitiesDto {
        capabilities,
        composition_paths: vec![
            CompositionPathDto {
                id: "manual".to_owned(),
                available: true,
                reason_code: None,
            },
            CompositionPathDto {
                id: "hosted".to_owned(),
                available: false,
                reason_code: Some("milestone-not-implemented".to_owned()),
            },
            CompositionPathDto {
                id: "apple-on-device".to_owned(),
                available: false,
                reason_code: Some(
                    if state.0.config.apple_helper_available {
                        "milestone-not-implemented"
                    } else {
                        "helper-unavailable"
                    }
                    .to_owned(),
                ),
            },
        ],
        disabled_routes: vec![
            "/mcp".to_owned(),
            "/v1/drafts".to_owned(),
            "/v1/publications".to_owned(),
            "/v1/amendments".to_owned(),
        ],
    })
}

async fn setup_check(State(state): State<AppState>) -> Json<SetupCheckDto> {
    Json(SetupCheckDto {
        config_safe: true,
        database_ready: true,
        apple_helper_available: state.0.config.apple_helper_available,
        notion_configured: state.0.config.notion_configured,
        manual_workflow_available: true,
    })
}

async fn get_preferences(State(state): State<AppState>) -> Result<Json<PreferencesDto>, ApiError> {
    let service = state.0.config.commands.clone();
    let preferences = blocking(move || service.preferences()).await?;
    Ok(Json(preferences_dto(&preferences)))
}

async fn put_preferences(
    State(state): State<AppState>,
    Json(request): Json<PreferencesDto>,
) -> Result<Json<PreferencesDto>, ApiError> {
    validate_optional_string(request.last_language.as_deref())?;
    validate_string(&request.last_composition_path)?;
    let preferences = parse_preferences(request)?;
    let result = preferences.clone();
    let service = state.0.config.commands.clone();
    blocking(move || service.save_preferences(&preferences)).await?;
    Ok(Json(preferences_dto(&result)))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListQuery {
    cursor: Option<usize>,
    limit: Option<usize>,
}

async fn list_workflows(
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

async fn start_workflow(
    State(state): State<AppState>,
    Json(request): Json<StartWorkflowRequest>,
) -> Result<Response, ApiError> {
    validate_optional_string(request.diary_day.as_deref())?;
    let day = request
        .diary_day
        .map(DiaryDay::parse)
        .transpose()
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid-diary-day"))?;
    let service = state.0.config.commands.clone();
    let outcome = blocking(move || service.start_workflow(day)).await?;
    let (status, workflow) = match outcome {
        StartOutcome::Created(workflow) => (StatusCode::CREATED, workflow),
        StartOutcome::Resumed(workflow) | StartOutcome::Completed(workflow) => {
            (StatusCode::OK, workflow)
        }
    };
    workflow_response(status, &workflow)
}

async fn get_workflow(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let day = day_from_id(&id)?;
    let service = state.0.config.commands.clone();
    let workflow = blocking(move || service.load_workflow(&day)).await?;
    workflow_response(StatusCode::OK, &workflow)
}

async fn discard_workflow(
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

async fn discover(
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
struct SourceListQuery {
    cursor: Option<usize>,
    limit: Option<usize>,
}

async fn list_sources(
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

async fn source_detail(
    State(state): State<AppState>,
    Path((id, source_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    validate_string(&source_id)?;
    let day = day_from_id(&id)?;
    let prepared_handle = optional_single_header(&headers, "x-nd-preparation-handle")?;
    let (snapshot, evidence) = if let Some(handle) = prepared_handle {
        validate_string(handle)?;
        let selected = prepared_source(&state, handle, &id, &source_id)?;
        (selected.snapshot, selected.date_evidence)
    } else {
        let service = state.0.config.commands.clone();
        let workflow = blocking(move || service.load_workflow(&day)).await?;
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

async fn prepare_sources(
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

async fn confirm_selection(
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

async fn confirm_empty_selection(
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

async fn set_writing_settings(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<WritingSettingsRequest>,
) -> Result<Response, ApiError> {
    let day = day_from_id(&id)?;
    let revision = workflow_precondition(&headers)?;
    validate_string(&request.language)?;
    validate_string(&request.style)?;
    validate_string(&request.composition_path)?;
    let language = WritingLanguage::parse(&request.language)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid-language"))?;
    let style = WritingStyle::parse(&request.style)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid-style"))?;
    let path = CompositionPath::parse(&request.composition_path)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid-composition-path"))?;
    if path != CompositionPath::Manual {
        return Err(ApiError::from(CommandError::UnavailablePath));
    }
    let service = state.0.config.commands.clone();
    let workflow =
        blocking(move || service.set_writing_settings(&day, revision, language, style, path))
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

async fn begin_manual_composition(
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

async fn save_entry(
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

async fn confirm_entry(
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

async fn get_job(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<JobDto>, ApiError> {
    cleanup_jobs(&state);
    let jobs = state.0.jobs.lock().map_err(|_| ApiError::internal())?;
    let record = jobs
        .jobs
        .get(&id)
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "job-not-found"))?;
    Ok(Json(job_dto(record)))
}

async fn cancel_job(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let mut jobs = state.0.jobs.lock().map_err(|_| ApiError::internal())?;
    let record = jobs
        .jobs
        .get_mut(&id)
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "job-not-found"))?;
    if record.status == "queued" {
        record
            .commit_state
            .store(COMMIT_CANCELLED, Ordering::Release);
        record.status = "cancelled";
        record.expires_at_ms = now_ms().saturating_add(JOB_EXPIRY_MS);
    } else if record.status == "running"
        && record
            .commit_state
            .compare_exchange(
                COMMIT_ACTIVE,
                COMMIT_CANCELLED,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
    {
        record.status = "cancellation-requested";
    }
    if let Some(handle) = record.result_handle.take() {
        jobs.preparations.remove(&handle);
    }
    Ok(StatusCode::ACCEPTED)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EventsQuery {
    workflow_id: Option<String>,
}

async fn events(
    State(state): State<AppState>,
    Query(query): Query<EventsQuery>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    if let Some(workflow_id) = &query.workflow_id {
        day_from_id(workflow_id)?;
    }
    if state.0.subscriber_count.fetch_add(1, Ordering::AcqRel) >= MAX_SUBSCRIBERS {
        state.0.subscriber_count.fetch_sub(1, Ordering::AcqRel);
        return Err(ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "subscriber-capacity",
        ));
    }
    let last_headers = headers.get_all("last-event-id").iter().collect::<Vec<_>>();
    let (last_id, invalid_last_id) = match last_headers.as_slice() {
        [] => (None, false),
        [value] => match value
            .to_str()
            .ok()
            .filter(|value| value.len() <= 20)
            .and_then(|value| value.parse::<u64>().ok())
        {
            Some(value) => (Some(value), false),
            None => (None, true),
        },
        _ => (None, true),
    };
    let (tx, rx) = mpsc::channel(SUBSCRIBER_QUEUE_CAPACITY);
    let (overflow_tx, mut overflow_rx) = oneshot::channel();
    let subscriber_id = {
        let mut events = match state.0.events.lock() {
            Ok(events) => events,
            Err(_) => {
                state.0.subscriber_count.fetch_sub(1, Ordering::AcqRel);
                return Err(ApiError::internal());
            }
        };
        let subscriber_id = events.next_subscriber_id;
        events.next_subscriber_id = events.next_subscriber_id.saturating_add(1);
        match replay_decision(
            &events,
            query.workflow_id.as_deref(),
            last_id,
            invalid_last_id,
        ) {
            ReplayDecision::Reset => {
                let reset = EventDto {
                    id: events.next_id.saturating_sub(1),
                    instance_id: state.0.config.instance_id.clone(),
                    event_type: "reset".to_owned(),
                    workflow_id: query.workflow_id.clone(),
                    revision: None,
                    job_id: None,
                    progress_percent: None,
                };
                let _ = tx.try_send(Ok(Bytes::from(sse_bytes(&reset))));
            }
            ReplayDecision::Events(replay) => {
                for event in replay {
                    let _ = tx.try_send(Ok(Bytes::from(sse_bytes(&event))));
                }
            }
        }
        events.subscribers.push(Subscriber {
            id: subscriber_id,
            sender: tx.clone(),
            workflow_id: query.workflow_id.clone(),
            overflow: Some(overflow_tx),
        });
        subscriber_id
    };
    let count = Arc::clone(&state.0.subscriber_count);
    let timer_state = state.clone();
    tokio::spawn(async move {
        let deadline = tokio::time::sleep(STREAM_LIFETIME);
        tokio::pin!(deadline);
        loop {
            tokio::select! {
                _ = tokio::time::sleep(HEARTBEAT) => {
                    if tx.try_send(Ok(Bytes::from_static(b": keep-alive\n\n"))).is_err() { break; }
                }
                _ = &mut overflow_rx => break,
                _ = &mut deadline => break,
            }
        }
        remove_subscriber(&timer_state, subscriber_id);
    });
    let stream = EventStream {
        receiver: rx,
        count,
        state,
        subscriber_id,
    };
    let mut response = Response::new(Body::from_stream(stream));
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("text/event-stream"));
    Ok(response)
}

struct EventStream {
    receiver: mpsc::Receiver<Result<Bytes, Infallible>>,
    count: Arc<AtomicUsize>,
    state: AppState,
    subscriber_id: u64,
}

impl Stream for EventStream {
    type Item = Result<Bytes, Infallible>;
    fn poll_next(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.receiver.poll_recv(context)
    }
}

impl Drop for EventStream {
    fn drop(&mut self) {
        remove_subscriber(&self.state, self.subscriber_id);
        self.count.fetch_sub(1, Ordering::AcqRel);
    }
}

fn remove_subscriber(state: &AppState, subscriber_id: u64) {
    if let Ok(mut events) = state.0.events.lock() {
        events
            .subscribers
            .retain(|subscriber| subscriber.id != subscriber_id);
    }
}

enum ReplayDecision {
    Reset,
    Events(Vec<EventDto>),
}

fn replay_decision(
    events: &EventState,
    workflow_id: Option<&str>,
    last_id: Option<u64>,
    invalid_last_id: bool,
) -> ReplayDecision {
    if invalid_last_id {
        return ReplayDecision::Reset;
    }
    let Some(last) = last_id else {
        return ReplayDecision::Events(Vec::new());
    };
    let first = events.replay.front().map(|event| event.id);
    if first.is_some_and(|first| last.saturating_add(1) < first) || last >= events.next_id {
        return ReplayDecision::Reset;
    }
    let replay = events
        .replay
        .iter()
        .filter(|event| {
            event.id > last && workflow_id.is_none_or(|id| event.workflow_id.as_deref() == Some(id))
        })
        .cloned()
        .collect::<Vec<_>>();
    if replay.len() > SUBSCRIBER_QUEUE_CAPACITY {
        ReplayDecision::Reset
    } else {
        ReplayDecision::Events(replay)
    }
}

async fn blocking<T: Send + 'static>(
    operation: impl FnOnce() -> Result<T, CommandError> + Send + 'static,
) -> Result<T, ApiError> {
    tokio::task::spawn_blocking(operation)
        .await
        .map_err(|_| ApiError::internal())?
        .map_err(ApiError::from)
}

fn new_job(state: &AppState, kind: &'static str, workflow_id: &str) -> Result<String, ApiError> {
    cleanup_jobs(state);
    let mut jobs = state.0.jobs.lock().map_err(|_| ApiError::internal())?;
    let active = jobs
        .jobs
        .values()
        .filter(|job| matches!(job.status, "queued" | "running" | "cancellation-requested"))
        .count();
    if active >= JOB_QUEUE_CAPACITY + JOB_CONCURRENCY || jobs.jobs.len() >= MAX_JOBS {
        return Err(ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "job-capacity",
        ));
    }
    let id = random_id("job_").map_err(|_| ApiError::internal())?;
    jobs.jobs.insert(
        id.clone(),
        JobRecord {
            id: id.clone(),
            kind,
            status: "queued",
            workflow_id: workflow_id.to_owned(),
            progress: 0,
            result_handle: None,
            error_code: None,
            expires_at_ms: now_ms().saturating_add(JOB_EXPIRY_MS),
            commit_state: Arc::new(AtomicU8::new(COMMIT_ACTIVE)),
        },
    );
    Ok(id)
}

async fn run_discovery_job(
    state: AppState,
    job_id: String,
    workflow_id: String,
    day: DiaryDay,
    revision: u64,
    explicit: Option<DiaryDay>,
) {
    let Ok(permit) = Arc::clone(&state.0.workers).acquire_owned().await else {
        return;
    };
    if !mark_running(&state, &job_id) {
        return;
    }
    let Some(commit_state) = job_commit_state(&state, &job_id) else {
        fail_job(&state, &job_id, "internal-failure");
        return;
    };
    let service = state.0.config.commands.clone();
    let worker_commit_state = Arc::clone(&commit_state);
    let mut task = tokio::task::spawn_blocking(move || {
        service.discover_guarded(&day, revision, explicit, || {
            worker_commit_state
                .compare_exchange(
                    COMMIT_ACTIVE,
                    COMMITTING,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_ok()
        })
    });
    let result = match tokio::time::timeout(Duration::from_secs(120), &mut task).await {
        Ok(result) => result,
        Err(_) => {
            match commit_state.compare_exchange(
                COMMIT_ACTIVE,
                COMMIT_CANCELLED,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    drop(permit);
                    fail_job(&state, &job_id, "job-timeout");
                    return;
                }
                Err(COMMIT_CANCELLED) => {
                    drop(permit);
                    fail_job(&state, &job_id, "operation-cancelled");
                    return;
                }
                Err(_) => {}
            }
            task.await
        }
    };
    drop(permit);
    match result {
        Ok(Ok(workflow)) => {
            finish_job(&state, &job_id, None, None);
            publish_event(
                &state,
                "workflow.changed",
                Some(workflow_id),
                Some(workflow.revision()),
                Some(job_id),
                Some(100),
            );
        }
        Ok(Err(error)) => fail_job(&state, &job_id, error.code()),
        Err(_) => fail_job(&state, &job_id, "internal-failure"),
    }
}

async fn run_prepare_job(
    state: AppState,
    job_id: String,
    workflow_id: String,
    day: DiaryDay,
    revision: u64,
    source_ids: Vec<String>,
    sources: Vec<SourceRef>,
) {
    let Ok(permit) = Arc::clone(&state.0.workers).acquire_owned().await else {
        return;
    };
    if !mark_running(&state, &job_id) {
        return;
    }
    let service = state.0.config.commands.clone();
    let result = tokio::time::timeout(
        Duration::from_secs(120),
        tokio::task::spawn_blocking(move || service.prepare_sources(&day, revision, &sources)),
    )
    .await;
    drop(permit);
    match result {
        Ok(Ok(Ok(selected))) => {
            let handle = match random_id("prep_") {
                Ok(handle) => handle,
                Err(_) => {
                    fail_job(&state, &job_id, "internal-failure");
                    return;
                }
            };
            let expires = now_ms().saturating_add(JOB_EXPIRY_MS);
            let mut jobs = match state.0.jobs.lock() {
                Ok(jobs) => jobs,
                Err(_) => return,
            };
            if jobs
                .jobs
                .get(&job_id)
                .is_some_and(|job| job.status == "cancellation-requested")
            {
                if let Some(job) = jobs.jobs.get_mut(&job_id) {
                    job.status = "cancelled";
                    job.expires_at_ms = now_ms().saturating_add(JOB_EXPIRY_MS);
                }
                return;
            }
            jobs.preparations.insert(
                handle.clone(),
                PreparedRecord {
                    workflow_id: workflow_id.clone(),
                    workflow_revision: revision,
                    source_ids,
                    selected,
                    expires_at_ms: expires,
                },
            );
            if let Some(job) = jobs.jobs.get_mut(&job_id) {
                job.status = "completed";
                job.progress = 100;
                job.result_handle = Some(handle);
                job.expires_at_ms = expires;
            }
            drop(jobs);
            publish_event(
                &state,
                "job.changed",
                Some(workflow_id),
                Some(revision),
                Some(job_id),
                Some(100),
            );
        }
        Ok(Ok(Err(error))) => fail_job(&state, &job_id, error.code()),
        _ => fail_job(&state, &job_id, "job-timeout"),
    }
}

fn mark_running(state: &AppState, id: &str) -> bool {
    let Ok(mut jobs) = state.0.jobs.lock() else {
        return false;
    };
    let Some(job) = jobs.jobs.get_mut(id) else {
        return false;
    };
    if job.status != "queued" {
        return false;
    }
    job.status = "running";
    job.progress = 1;
    true
}

fn job_commit_state(state: &AppState, id: &str) -> Option<Arc<AtomicU8>> {
    state
        .0
        .jobs
        .lock()
        .ok()?
        .jobs
        .get(id)
        .map(|job| Arc::clone(&job.commit_state))
}

fn finish_job(state: &AppState, id: &str, handle: Option<String>, error: Option<String>) {
    let Ok(mut jobs) = state.0.jobs.lock() else {
        return;
    };
    if let Some(job) = jobs.jobs.get_mut(id) {
        if job.status == "cancellation-requested" {
            job.status = "cancelled";
            job.result_handle = None;
            job.expires_at_ms = now_ms().saturating_add(JOB_EXPIRY_MS);
        } else {
            job.status = if error.is_some() {
                "failed"
            } else {
                "completed"
            };
            job.progress = 100;
            job.result_handle = handle;
            job.error_code = error;
            job.expires_at_ms = now_ms().saturating_add(JOB_EXPIRY_MS);
        }
    }
}

fn fail_job(state: &AppState, id: &str, code: &str) {
    finish_job(state, id, None, Some(code.to_owned()));
}

fn cleanup_jobs(state: &AppState) {
    let now = now_ms();
    let Ok(mut jobs) = state.0.jobs.lock() else {
        return;
    };
    cleanup_job_state(&mut jobs, now);
}

fn cleanup_job_state(jobs: &mut JobState, now: i64) {
    let expired_handles: Vec<_> = jobs
        .preparations
        .iter()
        .filter(|(_, prepared)| prepared.expires_at_ms <= now)
        .map(|(handle, _)| handle.clone())
        .collect();
    for handle in expired_handles {
        jobs.preparations.remove(&handle);
    }
    jobs.jobs.retain(|_, job| {
        job.expires_at_ms > now
            || matches!(job.status, "queued" | "running" | "cancellation-requested")
    });
}

fn take_preparation(
    state: &AppState,
    handle: &str,
    workflow_id: &str,
    revision: u64,
    source_ids: &[String],
) -> Result<Vec<SelectedSource>, ApiError> {
    cleanup_jobs(state);
    let mut jobs = state.0.jobs.lock().map_err(|_| ApiError::internal())?;
    let prepared = jobs
        .preparations
        .remove(handle)
        .ok_or_else(|| ApiError::new(StatusCode::PRECONDITION_FAILED, "preparation-expired"))?;
    if prepared.workflow_id != workflow_id
        || prepared.workflow_revision != revision
        || prepared.source_ids != source_ids
    {
        return Err(ApiError::new(
            StatusCode::PRECONDITION_FAILED,
            "preparation-binding-changed",
        ));
    }
    Ok(prepared.selected)
}

fn prepared_source(
    state: &AppState,
    handle: &str,
    workflow_id: &str,
    source_id: &str,
) -> Result<SelectedSource, ApiError> {
    cleanup_jobs(state);
    let jobs = state.0.jobs.lock().map_err(|_| ApiError::internal())?;
    let prepared = jobs
        .preparations
        .get(handle)
        .ok_or_else(|| ApiError::new(StatusCode::PRECONDITION_FAILED, "preparation-expired"))?;
    if prepared.workflow_id != workflow_id {
        return Err(ApiError::new(
            StatusCode::PRECONDITION_FAILED,
            "preparation-binding-changed",
        ));
    }
    let position = prepared
        .source_ids
        .iter()
        .position(|prepared_id| prepared_id == source_id)
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::PRECONDITION_FAILED,
                "preparation-binding-changed",
            )
        })?;
    prepared
        .selected
        .get(position)
        .cloned()
        .ok_or_else(ApiError::internal)
}

fn publish_event(
    state: &AppState,
    event_type: &str,
    workflow_id: Option<String>,
    revision: Option<u64>,
    job_id: Option<String>,
    progress: Option<u8>,
) {
    let Ok(mut events) = state.0.events.lock() else {
        return;
    };
    let event = EventDto {
        id: events.next_id,
        instance_id: state.0.config.instance_id.clone(),
        event_type: event_type.to_owned(),
        workflow_id,
        revision,
        job_id,
        progress_percent: progress,
    };
    events.next_id = events.next_id.saturating_add(1);
    if events.replay.len() == EVENT_REPLAY_CAPACITY {
        events.replay.pop_front();
    }
    events.replay.push_back(event.clone());
    let bytes = Bytes::from(sse_bytes(&event));
    events.subscribers.retain_mut(|subscriber| {
        if subscriber
            .workflow_id
            .as_ref()
            .is_some_and(|workflow_id| event.workflow_id.as_ref() != Some(workflow_id))
        {
            return true;
        }
        match subscriber.sender.try_send(Ok(bytes.clone())) {
            Ok(()) => true,
            Err(mpsc::error::TrySendError::Full(_)) => {
                if let Some(overflow) = subscriber.overflow.take() {
                    let _ = overflow.send(());
                }
                false
            }
            Err(mpsc::error::TrySendError::Closed(_)) => false,
        }
    });
}

fn sse_bytes(event: &EventDto) -> Vec<u8> {
    let json = serde_json::to_vec(event).unwrap_or_else(|_| b"{}".to_vec());
    let mut bytes = format!("id: {}\nevent: {}\ndata: ", event.id, event.event_type).into_bytes();
    bytes.extend(json);
    bytes.extend_from_slice(b"\n\n");
    bytes
}

fn workflow_response(status: StatusCode, workflow: &Workflow) -> Result<Response, ApiError> {
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

fn workflow_dto(workflow: &Workflow) -> WorkflowDto {
    WorkflowDto {
        id: workflow_id(workflow.day().as_str()).expect("validated diary day"),
        diary_day: workflow.day().as_str().to_owned(),
        state: workflow.state().as_str().to_owned(),
        revision: workflow.revision(),
        buffer_version: workflow.entry().version,
        language: workflow.language().map(|value| value.as_str().to_owned()),
        style: workflow.style().as_str().to_owned(),
        composition_path: workflow.composition_path().as_str().to_owned(),
        selection_digest: workflow
            .selection_digest()
            .map(|value| value.as_str().to_owned()),
        explicit_empty_selection: workflow.explicit_empty_selection(),
        invalidation_code: workflow.invalidation_reason().map(invalidation_code),
        entry_text: workflow.entry().text.clone(),
        entry_confirmed: workflow.entry().confirmed.is_some(),
        candidates: candidate_dtos(workflow),
    }
}

fn summary_dto(summary: &diary_core::WorkflowSummary) -> WorkflowSummaryDto {
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

fn candidate_dtos(workflow: &Workflow) -> Vec<SourceCandidateDto> {
    workflow
        .candidates()
        .iter()
        .map(|candidate| SourceCandidateDto {
            id: opaque_source_id(&candidate.source),
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

fn opaque_source_id(source: &SourceRef) -> String {
    format!(
        "src_{}",
        Digest::of_bytes(source.stable_key().as_bytes()).as_str()
    )
}

fn invalidation_code(reason: &InvalidationReason) -> String {
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

fn preferences_dto(preferences: &Preferences) -> PreferencesDto {
    PreferencesDto {
        last_language: preferences
            .last_language
            .map(|value| value.as_str().to_owned()),
        last_composition_path: preferences.last_composition_path.as_str().to_owned(),
    }
}

fn optional_single_header<'a>(
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

fn validate_string(value: &str) -> Result<(), ApiError> {
    if value.len() > MAX_STRING_BYTES {
        Err(ApiError::new(StatusCode::PAYLOAD_TOO_LARGE, "string-limit"))
    } else {
        Ok(())
    }
}

fn validate_optional_string(value: Option<&str>) -> Result<(), ApiError> {
    if let Some(value) = value {
        validate_string(value)?;
    }
    Ok(())
}

fn validate_unique_strings(values: &[String]) -> Result<(), ApiError> {
    let mut seen = BTreeSet::new();
    for value in values {
        validate_string(value)?;
        if !seen.insert(value.as_str()) {
            return Err(ApiError::new(StatusCode::BAD_REQUEST, "duplicate-source"));
        }
    }
    Ok(())
}

fn parse_preferences(dto: PreferencesDto) -> Result<Preferences, ApiError> {
    let language = dto
        .last_language
        .as_deref()
        .map(WritingLanguage::parse)
        .transpose()
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid-language"))?;
    let path = CompositionPath::parse(&dto.last_composition_path)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid-composition-path"))?;
    if path != CompositionPath::Manual {
        return Err(ApiError::from(CommandError::UnavailablePath));
    }
    Ok(Preferences {
        last_language: language,
        last_composition_path: path,
    })
}

fn day_from_id(id: &str) -> Result<DiaryDay, ApiError> {
    workflow_day(id)
        .and_then(|day| DiaryDay::parse(day).ok())
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "workflow-not-found"))
}

fn workflow_precondition(headers: &HeaderMap) -> Result<u64, ApiError> {
    let value = headers
        .get(IF_MATCH)
        .ok_or_else(|| ApiError::new(StatusCode::PRECONDITION_REQUIRED, "precondition-required"))?;
    parse_workflow_etag(value.to_str().unwrap_or_default())
        .ok_or_else(|| ApiError::new(StatusCode::PRECONDITION_FAILED, "precondition-failed"))
}

fn buffer_precondition(headers: &HeaderMap) -> Result<u64, ApiError> {
    let value = headers
        .get(IF_MATCH)
        .ok_or_else(|| ApiError::new(StatusCode::PRECONDITION_REQUIRED, "precondition-required"))?;
    parse_buffer_etag(value.to_str().unwrap_or_default())
        .ok_or_else(|| ApiError::new(StatusCode::PRECONDITION_FAILED, "precondition-failed"))
}

fn job_dto(record: &JobRecord) -> JobDto {
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

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
        })
}

struct ApiError {
    status: StatusCode,
    code: &'static str,
}

impl ApiError {
    const fn new(status: StatusCode, code: &'static str) -> Self {
        Self { status, code }
    }
    const fn internal() -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal-failure")
    }
}

impl From<CommandError> for ApiError {
    fn from(error: CommandError) -> Self {
        let status = match error {
            CommandError::NotFound => StatusCode::NOT_FOUND,
            CommandError::PreconditionRequired => StatusCode::PRECONDITION_REQUIRED,
            CommandError::PreconditionFailed
            | CommandError::Store(
                diary_core::StoreError::StaleRevision | diary_core::StoreError::StaleBuffer,
            ) => StatusCode::PRECONDITION_FAILED,
            CommandError::LimitExceeded => StatusCode::PAYLOAD_TOO_LARGE,
            CommandError::Source(diary_core::SourceAccessError::Busy) => {
                StatusCode::SERVICE_UNAVAILABLE
            }
            CommandError::Source(diary_core::SourceAccessError::TimedOut) => {
                StatusCode::GATEWAY_TIMEOUT
            }
            CommandError::Store(_) => StatusCode::INTERNAL_SERVER_ERROR,
            _ => StatusCode::CONFLICT,
        };
        Self::new(status, error.code())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let problem = ProblemDto {
            type_uri: format!("urn:nd:problem:{}", self.code),
            title: self.code.replace('-', " "),
            status: self.status.as_u16(),
            code: self.code.to_owned(),
            instance: random_id("corr_").ok(),
        };
        let mut response = (self.status, Json(problem)).into_response();
        response.headers_mut().insert(
            CONTENT_TYPE,
            HeaderValue::from_static("application/problem+json"),
        );
        response
            .headers_mut()
            .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
        response
    }
}

#[cfg(test)]
mod tests {
    use std::net::TcpListener as StdTcpListener;

    use super::*;

    #[test]
    fn loopback_binding_uses_the_frozen_fallback_sequence_and_reports_exhaustion() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let first = StdTcpListener::bind(("127.0.0.1", 8771)).ok();
        let (listener, port) = runtime.block_on(bind_loopback(2)).unwrap();
        assert!((8771..=8774).contains(&port));
        if first.is_some() {
            assert_ne!(port, 8771);
        }
        drop(listener);
        drop(first);

        let reservations: Vec<_> = [8771_u16, 8772]
            .into_iter()
            .filter_map(|port| StdTcpListener::bind(("127.0.0.1", port)).ok())
            .collect();
        assert_eq!(
            runtime.block_on(bind_loopback(0)).unwrap_err(),
            "listener-ports-exhausted"
        );
        drop(reservations);
        assert_eq!(
            runtime.block_on(bind_loopback(17)).unwrap_err(),
            "listener-port-count-invalid"
        );
    }

    #[test]
    fn job_and_preparation_expiry_preserve_only_active_work() {
        let mut state = JobState::default();
        state.jobs.insert(
            "expired".to_owned(),
            JobRecord {
                id: "expired".to_owned(),
                kind: "discovery",
                status: "completed",
                workflow_id: "wf_20261029".to_owned(),
                progress: 100,
                result_handle: Some("prep_expired".to_owned()),
                error_code: None,
                expires_at_ms: 10,
                commit_state: Arc::new(AtomicU8::new(COMMIT_ACTIVE)),
            },
        );
        state.jobs.insert(
            "running".to_owned(),
            JobRecord {
                id: "running".to_owned(),
                kind: "discovery",
                status: "running",
                workflow_id: "wf_20261029".to_owned(),
                progress: 1,
                result_handle: None,
                error_code: None,
                expires_at_ms: 10,
                commit_state: Arc::new(AtomicU8::new(COMMIT_ACTIVE)),
            },
        );
        state.preparations.insert(
            "prep_expired".to_owned(),
            PreparedRecord {
                workflow_id: "wf_20261029".to_owned(),
                workflow_revision: 1,
                source_ids: Vec::new(),
                selected: Vec::new(),
                expires_at_ms: 10,
            },
        );
        cleanup_job_state(&mut state, 10);
        assert!(!state.jobs.contains_key("expired"));
        assert!(state.jobs.contains_key("running"));
        assert!(state.preparations.is_empty());
    }

    #[test]
    fn depth_scanner_ignores_delimiters_in_strings_and_fails_closed() {
        assert!(json_depth_valid(br#"{"text":"[[{{"}"#, 1));
        assert!(!json_depth_valid(b"[[0]]", 1));
        assert!(!json_depth_valid(br#"{"unterminated":"#, 16));
        assert!(!json_depth_valid(b"}", 16));
    }

    #[test]
    fn sse_encoding_contains_only_the_typed_notification_shape() {
        let event = EventDto {
            id: 1,
            instance_id: "instance_synthetic".to_owned(),
            event_type: "workflow.changed".to_owned(),
            workflow_id: Some("wf_20261029".to_owned()),
            revision: Some(2),
            job_id: None,
            progress_percent: None,
        };
        let encoded = String::from_utf8(sse_bytes(&event)).unwrap();
        assert!(encoded.contains("event: workflow.changed"));
        for prohibited in ["entry_text", "body", "token", "credential", "source_id"] {
            assert!(!encoded.contains(prohibited));
        }
    }

    #[test]
    fn replay_resets_instead_of_silently_overflowing_the_subscriber_queue() {
        let replay = (1..=u64::try_from(SUBSCRIBER_QUEUE_CAPACITY + 1).unwrap())
            .map(|id| EventDto {
                id,
                instance_id: "instance_synthetic".to_owned(),
                event_type: "workflow.changed".to_owned(),
                workflow_id: Some("wf_20261029".to_owned()),
                revision: Some(id),
                job_id: None,
                progress_percent: None,
            })
            .collect();
        let state = EventState {
            next_id: u64::try_from(SUBSCRIBER_QUEUE_CAPACITY + 2).unwrap(),
            next_subscriber_id: 0,
            replay,
            subscribers: Vec::new(),
        };
        assert!(matches!(
            replay_decision(&state, None, Some(0), false),
            ReplayDecision::Reset
        ));
        assert!(matches!(
            replay_decision(&state, None, Some(u64::MAX), false),
            ReplayDecision::Reset
        ));
    }

    #[test]
    fn request_strings_and_source_lists_enforce_frozen_bounds() {
        assert!(validate_string(&"x".repeat(MAX_STRING_BYTES)).is_ok());
        assert!(validate_string(&"x".repeat(MAX_STRING_BYTES + 1)).is_err());
        assert!(validate_unique_strings(&["src_a".to_owned(), "src_b".to_owned()]).is_ok());
        assert!(validate_unique_strings(&["src_a".to_owned(), "src_a".to_owned()]).is_err());
    }

    #[test]
    fn helper_capability_guards_only_routes_that_touch_apple_source_services() {
        assert!(source_helper_required(
            "/v1/workflows/wf_20261029/discoveries"
        ));
        assert!(source_helper_required(
            "/v1/workflows/wf_20261029/sources/prepare"
        ));
        assert!(!source_helper_required(
            "/v1/workflows/wf_20261029/empty-selection-confirmations"
        ));
    }
}
