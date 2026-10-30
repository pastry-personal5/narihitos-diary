use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};
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
    AssistantBackendId, AssistantSelection, CompositionPath, DiaryDay, Digest,
    DraftAssistantRegistry, InvalidationReason, Preferences, ReflectionRange, SelectedSource,
    SourceRef, StartOutcome, SuggestionResolution, Workflow, WritingLanguage, WritingStyle,
};
use futures_core::Stream;
use serde::Deserialize;
use subtle::ConstantTimeEq;
use tokio::net::TcpListener;
use tokio::sync::{Semaphore, mpsc, oneshot, watch};

use crate::commands::{CommandError, CommandService};
use crate::diagnostics::{
    AssistantSnapshot, DiagnosticEvent, DiagnosticLevel, DiagnosticLog, NOTION_PREVIEW_REASONS,
    NOTION_PREVIEW_STATUSES, PublicationSnapshot,
};
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
    pub assistant_registry: Arc<DraftAssistantRegistry>,
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
    /// Flips to `true` when graceful shutdown starts so long-lived event
    /// streams end instead of holding the server open until the timeout.
    shutdown: watch::Sender<bool>,
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
    cancelled: Arc<AtomicBool>,
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
    shutdown: watch::Sender<bool>,
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
        shutdown: watch::channel(false).0,
    }));
    let shutdown = state.0.shutdown.clone();
    let router = Router::new()
        .route("/v1/health", get(health))
        .route("/v1/capabilities", get(capabilities))
        .route("/v1/setup-check", get(setup_check))
        .route("/v1/assistant-backends", get(assistant_backends))
        .route("/v1/preferences", get(get_preferences).put(put_preferences))
        .route("/v1/workflows", get(list_workflows).post(start_workflow))
        .route(
            "/v1/workflows/{id}",
            get(get_workflow).delete(discard_workflow),
        )
        .route(
            "/v1/workflows/{id}/output-previews",
            post(preview_local_output),
        )
        .route(
            "/v1/workflows/{id}/output-approvals",
            post(approve_local_output),
        )
        .route(
            "/v1/workflows/{id}/local-publications",
            post(publish_local_output),
        )
        .route(
            "/v1/workflows/{id}/notion-previews",
            post(preview_notion_output),
        )
        .route(
            "/v1/workflows/{id}/notion-approvals",
            post(approve_notion_output),
        )
        .route(
            "/v1/workflows/{id}/notion-publications",
            post(publish_notion_output),
        )
        .route("/v1/workflows/{id}/amendments", post(seed_amendment))
        .route(
            "/v1/workflows/{id}/amendments/previews",
            post(preview_amendment),
        )
        .route(
            "/v1/workflows/{id}/amendments/approvals",
            post(approve_amendment),
        )
        .route(
            "/v1/workflows/{id}/amendments/publications",
            post(publish_amendment),
        )
        .route(
            "/v1/workflows/{id}/amendments/notion-retries",
            post(retry_amendment_notion),
        )
        .route(
            "/v1/workflows/{id}/amendments/rereads",
            post(reread_amendment),
        )
        .route(
            "/v1/workflows/{id}/amendments/abandonments",
            post(abandon_amendment),
        )
        .route("/v1/workflows/{id}/save-under-day", post(save_under_day))
        .route("/v1/workflows/{id}/discoveries", post(discover))
        .route("/v1/workflows/{id}/sources", get(list_sources))
        .route(
            "/v1/workflows/{id}/sources/{source_id}",
            get(source_detail).delete(delete_source_route),
        )
        .route(
            "/v1/workflows/{id}/sources/{source_id}/refine-requests",
            post(start_source_refine),
        )
        .route(
            "/v1/workflows/{id}/sources/{source_id}/refine-suggestion",
            get(get_source_refine_suggestion),
        )
        .route(
            "/v1/workflows/{id}/sources/{source_id}/refine-suggestion/resolution",
            put(resolve_source_refine),
        )
        .route(
            "/v1/workflows/{id}/sources/{source_id}/body",
            put(edit_source_body),
        )
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
            "/v1/workflows/{id}/assistant-selection",
            get(get_assistant_selection).put(put_assistant_selection),
        )
        .route(
            "/v1/workflows/{id}/memory-extractions",
            post(start_memory_extraction),
        )
        .route(
            "/v1/workflows/{id}/memory-review",
            get(get_memory_review).put(put_memory_review),
        )
        .route(
            "/v1/workflows/{id}/draft-requests",
            post(start_draft_request),
        )
        .route("/v1/workflows/{id}/draft-result", get(get_draft_result))
        .route(
            "/v1/workflows/{id}/paragraph-review-requests",
            post(start_paragraph_review),
        )
        .route(
            "/v1/workflows/{id}/paragraph-suggestions",
            get(list_paragraph_suggestions),
        )
        .route(
            "/v1/workflows/{id}/paragraph-suggestions/{suggestion_id}",
            get(get_paragraph_suggestion),
        )
        .route(
            "/v1/workflows/{id}/paragraph-suggestions/{suggestion_id}/resolution",
            put(resolve_paragraph_suggestion),
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
    ServerHandle { router, shutdown }
}

pub async fn serve(
    listener: TcpListener,
    config: ServerConfig,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<(), std::io::Error> {
    let handle = build_router(config);
    let streams = handle.shutdown;
    axum::serve(listener, handle.router)
        .with_graceful_shutdown(async move {
            shutdown.await;
            let _ = streams.send(true);
        })
        .await
}

pub async fn bind_loopback(port_fallback_count: u16) -> Result<(TcpListener, u16), &'static str> {
    let ports = loopback_ports(port_fallback_count)?;
    bind_loopback_ports(ports).await
}

fn loopback_ports(port_fallback_count: u16) -> Result<std::ops::RangeInclusive<u16>, &'static str> {
    if port_fallback_count > 16 {
        return Err("listener-port-count-invalid");
    }
    let last = 8772_u16.saturating_add(port_fallback_count);
    Ok(8771..=last)
}

async fn bind_loopback_ports(
    ports: impl IntoIterator<Item = u16>,
) -> Result<(TcpListener, u16), &'static str> {
    for port in ports {
        match TcpListener::bind(("127.0.0.1", port)).await {
            Ok(listener) => {
                let bound_port = listener
                    .local_addr()
                    .map_err(|_| "listener-bind-failed")?
                    .port();
                return Ok((listener, bound_port));
            }
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
        "draft-editor"
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

/// Best-effort Debug-level write for job/preparation lifecycle events: a
/// failure here never fails the request, matching how every other
/// diagnostics write in this module is treated.
fn log_debug(state: &AppState, event: DiagnosticEvent) {
    if let Some(log) = &state.0.config.diagnostics {
        let _ = log.write(DiagnosticLevel::Debug, event);
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
        manual_workflow_available: true,
        assistant_backends: backend_dtos(&state),
        disabled_routes: vec![
            "/mcp".to_owned(),
            "/v1/drafts".to_owned(),
            "/v1/publications".to_owned(),
        ],
    })
}

async fn assistant_backends(State(state): State<AppState>) -> Json<AssistantBackendListDto> {
    Json(AssistantBackendListDto {
        backends: backend_dtos(&state),
    })
}

fn backend_dtos(state: &AppState) -> Vec<AssistantBackendDto> {
    state
        .0
        .config
        .assistant_registry
        .descriptors()
        .into_iter()
        .map(|descriptor| AssistantBackendDto {
            id: descriptor.id.to_string(),
            display_label: descriptor.display_label,
            composition_path: descriptor.composition_path.as_str().to_owned(),
            available: descriptor.available,
            reason_code: descriptor.reason_code,
            supported_languages: descriptor
                .supported_languages
                .into_iter()
                .map(|language| language.as_str().to_owned())
                .collect(),
            supported_actions: descriptor
                .supported_actions
                .into_iter()
                .map(|action| action.as_str().to_owned())
                .collect(),
        })
        .collect()
}

async fn setup_check(State(state): State<AppState>) -> Json<SetupCheckDto> {
    Json(SetupCheckDto {
        config_safe: true,
        database_ready: true,
        apple_helper_available: state.0.config.apple_helper_available,
        notion_configured: state.0.config.notion_configured,
        notion_reason_code: (!state.0.config.notion_configured)
            .then(|| "notion-not-configured".to_owned()),
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
    validate_optional_string(request.last_assistant_backend_id.as_deref())?;
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
    let outcome = blocking(move || {
        let outcome = service.start_workflow(day)?;
        if let StartOutcome::Created(workflow) = outcome {
            let preferences = service.preferences()?;
            let preferred = preferences
                .last_assistant_backend_id
                .and_then(|backend_id| {
                    descriptors
                        .iter()
                        .find(|descriptor| descriptor.id == backend_id && descriptor.available)
                        .map(|descriptor| {
                            AssistantSelection::new(backend_id, descriptor.composition_path)
                        })
                });
            if preferred.is_some() || language.is_some() || style.is_some() {
                let effective_language = language
                    .or(workflow.language())
                    .unwrap_or(WritingLanguage::EnUs);
                let effective_style = style.unwrap_or(workflow.style());
                let workflow = service.set_writing_settings(
                    workflow.day(),
                    workflow.revision(),
                    effective_language,
                    effective_style,
                    preferred,
                )?;
                Ok(StartOutcome::Created(workflow))
            } else {
                Ok(StartOutcome::Created(workflow))
            }
        } else {
            Ok(outcome)
        }
    })
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

async fn start_source_refine(
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

async fn get_source_refine_suggestion(
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

async fn resolve_source_refine(
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

async fn edit_source_body(
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

async fn delete_source_route(
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

async fn get_assistant_selection(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<AssistantSelectionDto>, ApiError> {
    let day = day_from_id(&id)?;
    let service = state.0.config.commands.clone();
    let workflow = blocking(move || service.load_workflow(&day)).await?;
    Ok(Json(assistant_selection_dto(&workflow)))
}

async fn put_assistant_selection(
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

fn assistant_selection_dto(workflow: &Workflow) -> AssistantSelectionDto {
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

async fn start_memory_extraction(
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

async fn get_memory_review(
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

async fn put_memory_review(
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

async fn start_draft_request(
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

async fn get_draft_result(
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

async fn start_paragraph_review(
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

async fn list_paragraph_suggestions(
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

async fn get_paragraph_suggestion(
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

async fn resolve_paragraph_suggestion(
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

fn range_dtos(ranges: &[ReflectionRange]) -> Vec<ReflectionRangeDto> {
    ranges
        .iter()
        .map(|range| ReflectionRangeDto {
            start: range.start,
            end: range.end,
        })
        .collect()
}

fn paragraph_suggestion_dto(
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

async fn preview_local_output(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<LocalPreviewDto>, ApiError> {
    let day = day_from_id(&id)?;
    let revision = workflow_precondition(&headers)?;
    let service = state.0.config.commands.clone();
    let preview = blocking(move || service.preview_local(&day, revision)).await?;
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
        notion_status: "blocked".to_owned(),
        icloud_target: preview.mirror_target,
        icloud_status: preview
            .mirror_status
            .map(|status| status.as_str().to_owned()),
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

async fn approve_local_output(
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

async fn publish_local_output(
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

async fn run_local_publication_job(
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

async fn preview_notion_output(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<NotionPreviewDto>, ApiError> {
    let day = day_from_id(&id)?;
    let revision = workflow_precondition(&headers)?;
    let service = state.0.config.commands.clone();
    let preview = blocking(move || service.preview_notion(&day, revision)).await?;
    log_debug(
        &state,
        DiagnosticEvent::NotionPreview {
            status: NOTION_PREVIEW_STATUSES
                .iter()
                .copied()
                .find(|status| *status == preview.status.as_str())
                .unwrap_or("blocked"),
            reason: match preview.reason.as_deref() {
                None => "none",
                Some(reason) => NOTION_PREVIEW_REASONS
                    .iter()
                    .copied()
                    .find(|known| *known == reason)
                    .unwrap_or("other"),
            },
        },
    );
    let result = NotionPreviewDto {
        workflow: workflow_dto(&preview.workflow),
        path: vec![preview.root_label, preview.year_title, preview.month_title],
        year_exists: preview.year_exists,
        month_exists: preview.month_exists,
        renderer_version: preview.renderer_version,
        api_version: preview.api_version,
        payload_digest: preview.payload_digest.as_str().to_owned(),
        plan_digest: preview.plan_digest.map(|digest| digest.as_str().to_owned()),
        blocks: preview
            .blocks
            .into_iter()
            .map(|block| NotionBlockDto {
                kind: block.kind,
                text: block.text,
                depth: block.depth,
            })
            .collect(),
        status: preview.status.as_str().to_owned(),
        reason: preview.reason,
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

async fn approve_notion_output(
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
    let workflow = blocking(move || service.approve_notion(&day, revision, &digest)).await?;
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

async fn publish_notion_output(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<(StatusCode, Json<JobAcceptedDto>), ApiError> {
    let day = day_from_id(&id)?;
    let revision = workflow_precondition(&headers)?;
    let job_id = new_job(&state, "notion-publication", &id)?;
    tokio::spawn(run_notion_publication_job(
        state,
        job_id.clone(),
        id,
        day,
        revision,
    ));
    Ok((StatusCode::ACCEPTED, Json(JobAcceptedDto { job_id })))
}

async fn run_notion_publication_job(
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
    let snapshot_day = day.clone();
    let service = state.0.config.commands.clone();
    let outcome = tokio::task::spawn_blocking(move || {
        service.publish_notion_guarded(&day, revision, &cancelled)
    })
    .await;
    match outcome {
        Ok(Ok(workflow)) => {
            log_publication_snapshot(&state, "notion-publication", "finished", &snapshot_day).await;
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
            log_publication_snapshot(&state, "notion-publication", "failed", &snapshot_day).await;
            fail_job(&state, &job_id, error.diagnostic_code());
        }
        Err(_) => fail_job(&state, &job_id, "internal-failure"),
    }
}

async fn seed_amendment(
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

async fn reread_amendment(
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

async fn abandon_amendment(
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

async fn preview_amendment(
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
    let blocks = |blocks: Vec<diary_core::NotionBlockPreview>| {
        blocks
            .into_iter()
            .map(|block| NotionBlockDto {
                kind: block.kind,
                text: block.text,
                depth: block.depth,
            })
            .collect()
    };
    let result = AmendmentPreviewDto {
        workflow: workflow_dto(&preview.workflow),
        plan_digest,
        local_target: preview.local_target,
        local_current: preview.local_current,
        local_payload: preview.local_payload,
        local_status: preview.local_status.as_str().to_owned(),
        notion: preview.notion.map(|notion| AmendmentNotionDto {
            current_blocks: blocks(notion.current_blocks),
            replacement_blocks: blocks(notion.replacement_blocks),
            status: notion.status.as_str().to_owned(),
            reason: notion.reason,
        }),
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

async fn approve_amendment(
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

async fn publish_amendment(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<(StatusCode, Json<JobAcceptedDto>), ApiError> {
    let day = day_from_id(&id)?;
    let revision = workflow_precondition(&headers)?;
    let job_id = new_job(&state, "amendment-publication", &id)?;
    tokio::spawn(run_amendment_job(
        state,
        job_id.clone(),
        id,
        day,
        revision,
        false,
    ));
    Ok((StatusCode::ACCEPTED, Json(JobAcceptedDto { job_id })))
}

async fn retry_amendment_notion(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<(StatusCode, Json<JobAcceptedDto>), ApiError> {
    let day = day_from_id(&id)?;
    let revision = workflow_precondition(&headers)?;
    let job_id = new_job(&state, "amendment-notion-retry", &id)?;
    tokio::spawn(run_amendment_job(
        state,
        job_id.clone(),
        id,
        day,
        revision,
        true,
    ));
    Ok((StatusCode::ACCEPTED, Json(JobAcceptedDto { job_id })))
}

async fn run_amendment_job(
    state: AppState,
    job_id: String,
    workflow_id: String,
    day: DiaryDay,
    revision: u64,
    notion_only: bool,
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
    let amendment_kind = if notion_only {
        "amendment-notion-retry"
    } else {
        "amendment-publication"
    };
    let snapshot_day = day.clone();
    let service = state.0.config.commands.clone();
    let outcome = tokio::task::spawn_blocking(move || {
        if notion_only {
            service.retry_amendment_notion(&day, revision, &cancelled)
        } else {
            service.publish_amendment(&day, revision, &cancelled)
        }
    })
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

async fn save_under_day(
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

async fn get_job(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<JobDto>, ApiError> {
    cleanup_jobs(&state);
    let jobs = state.0.jobs.lock().map_err(|_| ApiError::internal())?;
    let record = jobs
        .jobs
        .get(&id)
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, JOB_NOT_FOUND))?;
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
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, JOB_NOT_FOUND))?;
    if record.kind == "local-publication"
        && record.commit_state.load(Ordering::Acquire) == COMMITTING
    {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "publication-commit-started",
        ));
    }
    if record.status == job_status::QUEUED {
        record.cancelled.store(true, Ordering::Release);
        record
            .commit_state
            .store(COMMIT_CANCELLED, Ordering::Release);
        record.status = job_status::CANCELLED;
        record.expires_at_ms = now_ms().saturating_add(JOB_EXPIRY_MS);
    } else if record.status == job_status::RUNNING
        && (record.kind == "notion-publication"
            || record.kind == "amendment-publication"
            || record.kind == "amendment-notion-retry"
            || record
                .commit_state
                .compare_exchange(
                    COMMIT_ACTIVE,
                    COMMIT_CANCELLED,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_ok())
    {
        record.cancelled.store(true, Ordering::Release);
        record.status = job_status::CANCELLATION_REQUESTED;
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
    // Put one body frame on every newly established stream immediately.
    // Besides giving HTTP intermediaries something to flush, this means the
    // client can prove the SSE body is flowing before the first mutation;
    // comments consume no event ID and are ignored by the typed parser.
    let _ = tx.try_send(Ok(Bytes::from_static(b": connected\n\n")));
    let count = Arc::clone(&state.0.subscriber_count);
    let timer_state = state.clone();
    let mut shutdown_rx = state.0.shutdown.subscribe();
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
                _ = shutdown_rx.wait_for(|stopping| *stopping) => break,
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
        .filter(|job| job_status::is_active(job.status))
        .count();
    if is_assistant_job(kind)
        && jobs.jobs.values().any(|job| {
            job.workflow_id == workflow_id
                && is_assistant_job(job.kind)
                && job_status::is_active(job.status)
        })
    {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "assistant-job-already-active",
        ));
    }
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
            status: job_status::QUEUED,
            workflow_id: workflow_id.to_owned(),
            progress: 0,
            result_handle: None,
            error_code: None,
            expires_at_ms: now_ms().saturating_add(JOB_EXPIRY_MS),
            commit_state: Arc::new(AtomicU8::new(COMMIT_ACTIVE)),
            cancelled: Arc::new(AtomicBool::new(false)),
        },
    );
    drop(jobs);
    log_debug(
        state,
        DiagnosticEvent::Job {
            kind,
            event: "accepted",
            reason: None,
        },
    );
    Ok(id)
}

fn is_assistant_job(kind: &str) -> bool {
    matches!(
        kind,
        "assistant-memory-extraction"
            | "assistant-draft"
            | "assistant-paragraph-review"
            | "assistant-source-refine"
    )
}

fn ensure_no_assistant_job(state: &AppState, workflow_id: &str) -> Result<(), ApiError> {
    let jobs = state.0.jobs.lock().map_err(|_| ApiError::internal())?;
    if jobs.jobs.values().any(|job| {
        job.workflow_id == workflow_id
            && is_assistant_job(job.kind)
            && job_status::is_active(job.status)
    }) {
        Err(ApiError::new(
            StatusCode::CONFLICT,
            "assistant-job-already-active",
        ))
    } else {
        Ok(())
    }
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
        Ok(Err(error)) => fail_job(&state, &job_id, error.diagnostic_code()),
        Err(_) => fail_job(&state, &job_id, "internal-failure"),
    }
}

enum AssistantJob {
    Extract {
        revision: u64,
        artifact_id: String,
    },
    Draft {
        revision: u64,
        artifact_id: String,
        include_prior: bool,
    },
    Paragraph {
        revision: u64,
        suggestion_id: String,
        range: ReflectionRange,
    },
    Refine {
        revision: u64,
        source: SourceRef,
        digest: Digest,
    },
}

impl AssistantJob {
    /// The `Job.kind` token, shared with the job's own lifecycle records.
    const fn kind(&self) -> &'static str {
        match self {
            Self::Extract { .. } => "assistant-memory-extraction",
            Self::Draft { .. } => "assistant-draft",
            Self::Paragraph { .. } => "assistant-paragraph-review",
            Self::Refine { .. } => "assistant-source-refine",
        }
    }
}

async fn run_assistant_job(
    state: AppState,
    job_id: String,
    workflow_id: String,
    day: DiaryDay,
    operation: AssistantJob,
) {
    let Ok(permit) = Arc::clone(&state.0.workers).acquire_owned().await else {
        return;
    };
    if !mark_running(&state, &job_id) {
        return;
    }
    let Some((commit_state, cancelled)) = assistant_job_state(&state, &job_id) else {
        fail_job(&state, &job_id, "internal-failure");
        return;
    };
    let kind = operation.kind();
    log_assistant_snapshot(&state, kind, "start", &day).await;
    let snapshot_day = day.clone();
    let service = state.0.config.commands.clone();
    let registry = Arc::clone(&state.0.config.assistant_registry);
    let commit_for_worker = Arc::clone(&commit_state);
    let cancelled_for_worker = Arc::clone(&cancelled);
    let result_handle = match &operation {
        AssistantJob::Extract { artifact_id, .. } | AssistantJob::Draft { artifact_id, .. } => {
            artifact_id.clone()
        }
        AssistantJob::Paragraph { suggestion_id, .. } => suggestion_id.clone(),
        AssistantJob::Refine { source, .. } => opaque_source_id(source),
    };
    let mut task = tokio::task::spawn_blocking(move || {
        let commit = || {
            commit_for_worker
                .compare_exchange(
                    COMMIT_ACTIVE,
                    COMMITTING,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_ok()
        };
        match operation {
            AssistantJob::Extract {
                revision,
                artifact_id,
            } => service.extract_memories(
                &day,
                revision,
                artifact_id,
                &registry,
                &cancelled_for_worker,
                commit,
            ),
            AssistantJob::Draft {
                revision,
                artifact_id,
                include_prior,
            } => service.draft_entry(
                &day,
                revision,
                artifact_id,
                include_prior,
                &registry,
                &cancelled_for_worker,
                commit,
            ),
            AssistantJob::Paragraph {
                revision,
                suggestion_id,
                range,
            } => service.review_paragraph(
                &day,
                revision,
                suggestion_id,
                range,
                &registry,
                &cancelled_for_worker,
                commit,
            ),
            AssistantJob::Refine {
                revision,
                source,
                digest,
            } => service.refine_source(
                &day,
                revision,
                &source,
                &digest,
                &registry,
                &cancelled_for_worker,
                commit,
            ),
        }
    });
    let result = match tokio::time::timeout(Duration::from_secs(120), &mut task).await {
        Ok(result) => result,
        Err(_) => {
            cancelled.store(true, Ordering::Release);
            if commit_state
                .compare_exchange(
                    COMMIT_ACTIVE,
                    COMMIT_CANCELLED,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_ok()
            {
                drop(permit);
                fail_job(&state, &job_id, "assistant-timeout");
                return;
            }
            task.await
        }
    };
    drop(permit);
    match result {
        Ok(Ok(workflow)) => {
            finish_job(&state, &job_id, Some(result_handle), None);
            publish_event(
                &state,
                "workflow.changed",
                Some(workflow_id),
                Some(workflow.revision()),
                Some(job_id),
                Some(100),
            );
        }
        Ok(Err(error)) => {
            log_assistant_snapshot(&state, kind, "failed", &snapshot_day).await;
            fail_job(&state, &job_id, error.diagnostic_code());
        }
        Err(_) => fail_job(&state, &job_id, "internal-failure"),
    }
}

/// Best-effort Debug-level snapshot of the workflow an assistant job runs
/// against. Reads on a blocking thread because the store is synchronous
/// SQLite.
async fn log_assistant_snapshot(
    state: &AppState,
    kind: &'static str,
    event_status: &'static str,
    day: &DiaryDay,
) {
    let Some(log) = state.0.config.diagnostics.clone() else {
        return;
    };
    if !log.enabled(DiagnosticLevel::Debug) {
        return;
    }
    let service = state.0.config.commands.clone();
    let day = day.clone();
    let _ = tokio::task::spawn_blocking(move || {
        let Ok(workflow) = service.load_workflow(&day) else {
            return;
        };
        if let Some(snapshot) = AssistantSnapshot::capture(&log, &workflow, kind, event_status) {
            let _ = log.write(
                DiagnosticLevel::Debug,
                DiagnosticEvent::AssistantState(snapshot),
            );
        }
    })
    .await;
}

/// Best-effort Debug-level snapshot of a day's publication state when a
/// publication job ends. Reads on a blocking thread because the store is
/// synchronous SQLite.
async fn log_publication_snapshot(
    state: &AppState,
    kind: &'static str,
    event_status: &'static str,
    day: &DiaryDay,
) {
    let Some(log) = state.0.config.diagnostics.clone() else {
        return;
    };
    if !log.enabled(DiagnosticLevel::Debug) {
        return;
    }
    let service = state.0.config.commands.clone();
    let day = day.clone();
    let _ = tokio::task::spawn_blocking(move || {
        let Ok(workflow) = service.load_workflow(&day) else {
            return;
        };
        let (receipts, unresolved) = service.publication_counts(&day).unwrap_or((0, 0));
        if let Some(snapshot) =
            PublicationSnapshot::capture(&log, &workflow, kind, event_status, receipts, unresolved)
        {
            let _ = log.write(
                DiagnosticLevel::Debug,
                DiagnosticEvent::PublicationState(snapshot),
            );
        }
    })
    .await;
}

fn assistant_job_state(state: &AppState, id: &str) -> Option<(Arc<AtomicU8>, Arc<AtomicBool>)> {
    let jobs = state.0.jobs.lock().ok()?;
    let job = jobs.jobs.get(id)?;
    Some((Arc::clone(&job.commit_state), Arc::clone(&job.cancelled)))
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
            let mut jobs = lock_job_transitions(&state);
            if jobs
                .jobs
                .get(&job_id)
                .is_some_and(|job| job.status == job_status::CANCELLATION_REQUESTED)
            {
                if let Some(job) = jobs.jobs.get_mut(&job_id) {
                    job.status = job_status::CANCELLED;
                    job.expires_at_ms = now_ms().saturating_add(JOB_EXPIRY_MS);
                }
                drop(jobs);
                log_debug(
                    &state,
                    DiagnosticEvent::Job {
                        kind: "source-preparation",
                        event: "cancelled",
                        reason: None,
                    },
                );
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
                job.status = job_status::COMPLETED;
                job.progress = 100;
                job.result_handle = Some(handle);
                job.expires_at_ms = expires;
            }
            drop(jobs);
            log_debug(&state, DiagnosticEvent::Preparation { event: "issued" });
            log_debug(
                &state,
                DiagnosticEvent::Job {
                    kind: "source-preparation",
                    event: "completed",
                    reason: None,
                },
            );
            publish_event(
                &state,
                "job.changed",
                Some(workflow_id),
                Some(revision),
                Some(job_id),
                Some(100),
            );
        }
        Ok(Ok(Err(error))) => fail_job(&state, &job_id, error.diagnostic_code()),
        // The blocking task panicked or was aborted: not a timeout.
        Ok(Err(_)) => fail_job(&state, &job_id, "internal-failure"),
        Err(_) => fail_job(&state, &job_id, "job-timeout"),
    }
}

fn mark_running(state: &AppState, id: &str) -> bool {
    let kind = {
        let mut jobs = lock_job_transitions(state);
        let Some(job) = jobs.jobs.get_mut(id) else {
            return false;
        };
        if job.status != job_status::QUEUED {
            return false;
        }
        job.status = job_status::RUNNING;
        job.progress = 1;
        job.kind
    };
    log_debug(
        state,
        DiagnosticEvent::Job {
            kind,
            event: "running",
            reason: None,
        },
    );
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

fn finish_job(state: &AppState, id: &str, handle: Option<String>, error: Option<&'static str>) {
    let outcome = {
        let mut jobs = lock_job_transitions(state);
        let Some(job) = jobs.jobs.get_mut(id) else {
            return;
        };
        if job.status == job_status::CANCELLATION_REQUESTED {
            job.status = job_status::CANCELLED;
            job.result_handle = None;
            job.expires_at_ms = now_ms().saturating_add(JOB_EXPIRY_MS);
            (job.kind, "cancelled")
        } else {
            let event = if error.is_some() {
                job_status::FAILED
            } else {
                job_status::COMPLETED
            };
            job.status = event;
            job.progress = 100;
            job.result_handle = handle;
            job.error_code = error.map(str::to_owned);
            job.expires_at_ms = now_ms().saturating_add(JOB_EXPIRY_MS);
            (job.kind, event)
        }
    };
    log_debug(
        state,
        DiagnosticEvent::Job {
            kind: outcome.0,
            event: outcome.1,
            reason: (outcome.1 == "failed").then_some(error).flatten(),
        },
    );
}

fn fail_job(state: &AppState, id: &str, code: &'static str) {
    finish_job(state, id, None, Some(code));
}

/// Locks the job table for a lifecycle transition, recovering a poisoned
/// guard. Giving up instead would strand the job `queued`/`running`, states
/// `cleanup_job_state` never evicts, so it would hold capacity forever.
fn lock_job_transitions(state: &AppState) -> std::sync::MutexGuard<'_, JobState> {
    state
        .0
        .jobs
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
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
    jobs.jobs
        .retain(|_, job| job.expires_at_ms > now || job_status::is_active(job.status));
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
    let Some(prepared) = jobs.preparations.remove(handle) else {
        drop(jobs);
        log_debug(state, DiagnosticEvent::Preparation { event: "expired" });
        return Err(ApiError::new(
            StatusCode::PRECONDITION_FAILED,
            "preparation-expired",
        ));
    };
    drop(jobs);
    if prepared.workflow_id != workflow_id
        || prepared.workflow_revision != revision
        || prepared.source_ids != source_ids
    {
        log_debug(
            state,
            DiagnosticEvent::Preparation {
                event: "binding-mismatch",
            },
        );
        return Err(ApiError::new(
            StatusCode::PRECONDITION_FAILED,
            "preparation-binding-changed",
        ));
    }
    log_debug(state, DiagnosticEvent::Preparation { event: "consumed" });
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
    let Some(prepared) = jobs.preparations.get(handle) else {
        log_debug(state, DiagnosticEvent::Preparation { event: "expired" });
        return Err(ApiError::new(
            StatusCode::PRECONDITION_FAILED,
            "preparation-expired",
        ));
    };
    if prepared.workflow_id != workflow_id {
        log_debug(
            state,
            DiagnosticEvent::Preparation {
                event: "binding-mismatch",
            },
        );
        return Err(ApiError::new(
            StatusCode::PRECONDITION_FAILED,
            "preparation-binding-changed",
        ));
    }
    let Some(position) = prepared
        .source_ids
        .iter()
        .position(|prepared_id| prepared_id == source_id)
    else {
        log_debug(
            state,
            DiagnosticEvent::Preparation {
                event: "binding-mismatch",
            },
        );
        return Err(ApiError::new(
            StatusCode::PRECONDITION_FAILED,
            "preparation-binding-changed",
        ));
    };
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
        last_assistant_backend_id: preferences
            .last_assistant_backend_id
            .as_ref()
            .map(|backend_id| backend_id.as_str().to_owned()),
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
            | CommandError::NotionBindingChanged(_)
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
        assert_eq!(
            loopback_ports(2).unwrap().collect::<Vec<_>>(),
            [8771, 8772, 8773, 8774]
        );

        let first = match StdTcpListener::bind(("127.0.0.1", 0)) {
            Ok(listener) => listener,
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
                assert_eq!(
                    loopback_ports(17).unwrap_err(),
                    "listener-port-count-invalid"
                );
                return;
            }
            Err(error) => panic!("test listener bind failed: {error}"),
        };
        let occupied_port = first.local_addr().unwrap().port();
        let (listener, port) = runtime
            .block_on(bind_loopback_ports([occupied_port, 0]))
            .unwrap();
        assert_ne!(port, occupied_port);
        assert_ne!(port, 0);
        drop(listener);

        assert_eq!(
            runtime
                .block_on(bind_loopback_ports([occupied_port]))
                .unwrap_err(),
            "listener-ports-exhausted"
        );
        drop(first);
        assert_eq!(
            loopback_ports(17).unwrap_err(),
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
                cancelled: Arc::new(AtomicBool::new(false)),
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
                cancelled: Arc::new(AtomicBool::new(false)),
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
