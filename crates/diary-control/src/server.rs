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

mod assistant_routes;
mod dto;
mod event_routes;
mod jobs;
mod publication_routes;
mod source_routes;
mod workflow_routes;
use crate::diagnostics::{
    AssistantSnapshot, DiagnosticEvent, DiagnosticLevel, DiagnosticLog, NOTION_PREVIEW_REASONS,
    NOTION_PREVIEW_STATUSES, PublicationSnapshot,
};
use crate::lifecycle::random_id;
use assistant_routes::*;
use dto::*;
use event_routes::*;
use jobs::*;
use publication_routes::*;
use source_routes::*;
use workflow_routes::*;

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
            "/v1/workflows/{id}/readiness",
            post(check_publish_readiness),
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

async fn blocking<T: Send + 'static>(
    operation: impl FnOnce() -> Result<T, CommandError> + Send + 'static,
) -> Result<T, ApiError> {
    tokio::task::spawn_blocking(operation)
        .await
        .map_err(|_| ApiError::internal())?
        .map_err(ApiError::from)
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
