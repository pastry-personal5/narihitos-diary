#![forbid(unsafe_code)]

use std::collections::BTreeSet;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use diary_connectors::SqliteStore;
use diary_control::{CommandError, CommandService, ServerConfig, serve};
use diary_core::{
    AssistantAction, AssistantBackendDescriptor, AssistantBackendId, AssistantRequest,
    AssistantResponse, AssistantSelection, CatalogRequest, Clock, CompositionPath, DateEvidence,
    DestinationKind, DestinationPlan, DiaryDay, Digest, DraftAssistant, DraftAssistantError,
    DraftAssistantRegistry, DraftEntryResult, FreshnessOutcome, FreshnessStamp, IntentTerminal,
    MemoryCandidate, MemoryExtractionResult, OperationIntent, OperationLog, ParagraphReviewResult,
    Preferences, ReflectionRange, SelectedSource, SourceAccessError, SourceAvailability,
    SourceCandidate, SourceCatalog, SourceDiscovery, SourceReadOutcome, SourceReader, SourceRef,
    SourceSnapshot, SuggestionResolution, Timestamp, WorkflowStore, WritingLanguage, WritingStyle,
};
use tokio::net::TcpListener;
use tokio::sync::oneshot;

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[test]
fn command_service_closes_store_gaps_and_completes_manual_no_notes_flow() {
    let scratch = Scratch::new("commands");
    let store = Arc::new(SqliteStore::open(scratch.path().join("state.sqlite3")).unwrap());
    let service = service(store.clone());
    let day = DiaryDay::parse("2026-10-29").unwrap();
    assert_eq!(
        service.discard(&DiaryDay::parse("2026-10-30").unwrap(), 0),
        Err(CommandError::NotFound)
    );
    let workflow = service.start_workflow(Some(day.clone())).unwrap();
    let workflow = match workflow {
        diary_core::StartOutcome::Created(workflow) => workflow,
        _ => panic!("synthetic workflow should be created"),
    };
    let digest = Digest::of_bytes(b"synthetic");
    let target = DestinationPlan::new(
        DestinationKind::Local,
        "local",
        "synthetic-target",
        digest.clone(),
        digest.clone(),
        digest.clone(),
    )
    .unwrap();
    let intent = OperationIntent::new(
        "synthetic-unresolved",
        "synthetic-plan",
        day.clone(),
        &target,
        digest.clone(),
        digest,
        Timestamp(1),
    )
    .unwrap();
    store.record_intent(&intent).unwrap();
    assert_eq!(
        service.discover(&day, workflow.revision(), None),
        Err(CommandError::Reconciling)
    );
    store
        .mark_terminal("synthetic-unresolved", IntentTerminal::DefiniteNoEffect)
        .unwrap();
    let workflow = service.discover(&day, workflow.revision(), None).unwrap();
    let stale_revision = workflow.revision();
    let workflow = service
        .confirm_empty_selection(&day, workflow.revision())
        .unwrap();
    assert!(workflow.explicit_empty_selection());
    assert!(service.discard(&day, stale_revision).is_err());

    let workflow = service
        .set_writing_settings(
            &day,
            workflow.revision(),
            WritingLanguage::KoKr,
            WritingStyle::Reflective,
            None,
        )
        .unwrap();
    assert_eq!(
        store.load_preferences().unwrap(),
        Preferences {
            last_language: Some(WritingLanguage::KoKr),
            last_assistant_backend_id: None,
        }
    );
    let workflow = service
        .begin_manual_composition(&day, workflow.revision())
        .unwrap();
    let workflow = service
        .save_entry(
            &day,
            workflow.entry().version,
            "합성 테스트 일기".to_owned(),
        )
        .unwrap();
    assert!(
        service
            .save_entry(&day, 0, "stale client".to_owned())
            .is_err()
    );
    let workflow = service
        .confirm_entry(&day, workflow.revision(), workflow.entry().version)
        .unwrap();
    assert!(workflow.entry().confirmed.is_some());
    let summaries = service.list_workflows(256).unwrap();
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].day, day);
}

#[test]
fn command_service_confirms_explicit_no_notes_without_discovery() {
    let scratch = Scratch::new("direct-empty");
    let store = Arc::new(SqliteStore::open(scratch.path().join("state.sqlite3")).unwrap());
    let service = service(store);
    let day = DiaryDay::parse("2026-10-30").unwrap();
    let workflow = match service.start_workflow(Some(day.clone())).unwrap() {
        diary_core::StartOutcome::Created(workflow) => workflow,
        _ => panic!("synthetic workflow should be new"),
    };

    let workflow = service
        .confirm_empty_selection(&day, workflow.revision())
        .unwrap();

    assert_eq!(
        workflow.state(),
        diary_core::WorkflowState::SourcesConfirmed
    );
    assert!(workflow.candidates().is_empty());
    assert!(workflow.explicit_empty_selection());
}

#[test]
fn trusted_preparation_updates_date_evidence_and_confirms_exact_snapshot() {
    let scratch = Scratch::new("preparation");
    let store = Arc::new(SqliteStore::open(scratch.path().join("state.sqlite3")).unwrap());
    let source = Arc::new(PreparedSources);
    let service = CommandService::new(store, source.clone(), source, Arc::new(FixedClock));
    let day = DiaryDay::parse("2026-10-29").unwrap();
    let workflow = match service.start_workflow(Some(day.clone())).unwrap() {
        diary_core::StartOutcome::Created(workflow) => workflow,
        _ => panic!("synthetic workflow should be created"),
    };
    let workflow = service.discover(&day, workflow.revision(), None).unwrap();
    let source_ref = workflow.candidates()[0].source.clone();
    let prepared = service
        .prepare_sources(&day, workflow.revision(), std::slice::from_ref(&source_ref))
        .unwrap();
    let workflow = service
        .confirm_selection(&day, workflow.revision(), prepared)
        .unwrap();
    assert_eq!(workflow.snapshots().len(), 1);
    assert_eq!(workflow.snapshots()[0].body, "Synthetic body.");
    assert_eq!(
        workflow.candidates()[0]
            .date_evidence
            .header_day
            .as_ref()
            .unwrap()
            .as_str(),
        "2026-10-30"
    );
}

#[test]
fn cancelled_discovery_does_not_commit_a_late_result() {
    let scratch = Scratch::new("cancelled-discovery");
    let store = Arc::new(SqliteStore::open(scratch.path().join("state.sqlite3")).unwrap());
    let service = service(store.clone());
    let day = DiaryDay::parse("2026-10-29").unwrap();
    let workflow = match service.start_workflow(Some(day.clone())).unwrap() {
        diary_core::StartOutcome::Created(workflow) => workflow,
        _ => panic!("synthetic workflow should be created"),
    };
    assert_eq!(
        service.discover_guarded(&day, workflow.revision(), None, || false),
        Err(CommandError::Cancelled)
    );
    let unchanged = store.load(&day).unwrap().unwrap();
    assert_eq!(unchanged.revision(), workflow.revision());
    assert!(unchanged.candidates().is_empty());
}

#[test]
fn synthetic_assistant_flow_is_guarded_persisted_and_restart_recoverable() {
    let scratch = Scratch::new("assistant-flow");
    let database = scratch.path().join("state.sqlite3");
    let store = Arc::new(SqliteStore::open(&database).unwrap());
    let source = Arc::new(PreparedSources);
    let service = CommandService::new(store.clone(), source.clone(), source, Arc::new(FixedClock));
    let day = DiaryDay::parse("2026-10-29").unwrap();
    let mut workflow = match service.start_workflow(Some(day.clone())).unwrap() {
        diary_core::StartOutcome::Created(workflow) => workflow,
        _ => panic!("synthetic workflow should be created"),
    };
    workflow = service.discover(&day, workflow.revision(), None).unwrap();
    let prepared = service
        .prepare_sources(
            &day,
            workflow.revision(),
            &[workflow.candidates()[0].source.clone()],
        )
        .unwrap();
    workflow = service
        .confirm_selection(&day, workflow.revision(), prepared)
        .unwrap();

    let backend_id = AssistantBackendId::parse("synthetic-assistant").unwrap();
    let selection = AssistantSelection::new(backend_id.clone(), CompositionPath::Hosted);
    workflow = service
        .set_writing_settings(
            &day,
            workflow.revision(),
            WritingLanguage::KoKr,
            WritingStyle::Reflective,
            Some(selection),
        )
        .unwrap();
    workflow = service
        .begin_manual_composition(&day, workflow.revision())
        .unwrap();
    let registry =
        DraftAssistantRegistry::new(vec![Arc::new(SyntheticAssistant::new(backend_id))], None)
            .unwrap();

    assert_eq!(
        service.extract_memories(
            &day,
            workflow.revision(),
            "cancelled-memory".to_owned(),
            &registry,
            &AtomicBool::new(false),
            || false,
        ),
        Err(CommandError::Cancelled)
    );
    assert!(
        store
            .load(&day)
            .unwrap()
            .unwrap()
            .memory_candidate_sets()
            .is_empty()
    );

    workflow = service
        .extract_memories(
            &day,
            workflow.revision(),
            "memory-set".to_owned(),
            &registry,
            &AtomicBool::new(false),
            || true,
        )
        .unwrap();
    workflow = service
        .replace_reviewed_memory(
            &day,
            workflow.revision(),
            "reviewed-memory".to_owned(),
            "memory-set",
            "Human-reviewed walk".to_owned(),
        )
        .unwrap();
    workflow = service
        .draft_entry(
            &day,
            workflow.revision(),
            "draft-result".to_owned(),
            false,
            &registry,
            &AtomicBool::new(false),
            || true,
        )
        .unwrap();
    assert_eq!(workflow.entry().text, "Synthetic draft.");

    workflow = service
        .review_paragraph(
            &day,
            workflow.revision(),
            "suggestion".to_owned(),
            ReflectionRange {
                start: 0,
                end: workflow.entry().text.len(),
            },
            &registry,
            &AtomicBool::new(false),
            || true,
        )
        .unwrap();
    workflow = service
        .resolve_paragraph_suggestion(
            &day,
            workflow.revision(),
            "suggestion",
            SuggestionResolution::Accepted,
        )
        .unwrap();
    assert_eq!(workflow.entry().text, "Improved paragraph.");

    let reopened = SqliteStore::open(database).unwrap();
    let recovered = reopened.load(&day).unwrap().unwrap();
    assert_eq!(recovered.entry().text, "Improved paragraph.");
    assert_eq!(recovered.memory_candidate_sets().len(), 1);
    assert_eq!(recovered.draft_proposals().len(), 1);
    assert_eq!(recovered.paragraph_suggestions().len(), 1);
}

#[test]
fn real_loopback_enforces_headers_authorization_and_conditional_mutations() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let scratch = Scratch::new("loopback");
        let store = Arc::new(SqliteStore::open(scratch.path().join("state.sqlite3")).unwrap());
        store
            .save_preferences(&Preferences {
                last_language: None,
                last_assistant_backend_id: Some(
                    AssistantBackendId::parse("missing-backend").unwrap(),
                ),
            })
            .unwrap();
        let listener = match TcpListener::bind(("127.0.0.1", 0)).await {
            Ok(listener) => listener,
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => return,
            Err(error) => panic!("test listener bind failed: {error}"),
        };
        let port = listener.local_addr().unwrap().port();
        let token = format!("token_{}", "11".repeat(32));
        let config = ServerConfig {
            instance_id: format!("instance_{}", "22".repeat(32)),
            control_token: token.clone(),
            port,
            commands: prepared_service(store),
            apple_helper_available: true,
            notion_configured: false,
            assistant_registry: Arc::new(
                diary_core::DraftAssistantRegistry::new(Vec::new(), None).unwrap(),
            ),
            diagnostics: None,
            diagnostic_failure: None,
        };
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let server = tokio::spawn(serve(listener, config, async move {
            let _ = shutdown_rx.await;
        }));

        let response = request(port, "GET", "/v1/health", "", &[], None).await;
        assert!(response.starts_with("HTTP/1.1 200"));
        assert!(response.contains("\"contract_version\":8"));
        assert!(
            response
                .to_ascii_lowercase()
                .contains("cache-control: no-store")
        );
        let response = request(
            port,
            "GET",
            "/v1/health",
            "",
            &[("Host", format!("localhost:{port}"))],
            None,
        )
        .await;
        assert!(response.starts_with("HTTP/1.1 400"));
        let response = request(
            port,
            "GET",
            "/v1/health",
            "",
            &[("Origin", "https://example.invalid".to_owned())],
            None,
        )
        .await;
        assert!(response.starts_with("HTTP/1.1 403"));
        let response = request(
            port,
            "GET",
            "/v1/health",
            "",
            &[("X-Forwarded-Host", format!("127.0.0.1:{port}"))],
            None,
        )
        .await;
        assert!(response.starts_with("HTTP/1.1 400"));
        assert!(response.contains("proxy-headers-forbidden"));
        let response = request(port, "GET", "/v1/capabilities", "", &[], None).await;
        assert!(response.starts_with("HTTP/1.1 401"));
        let response = request(port, "GET", "/v1/capabilities", "", &[], Some("wrong")).await;
        assert!(response.starts_with("HTTP/1.1 401"));
        let response = request(port, "GET", "/v1/capabilities", "", &[], Some(&token)).await;
        assert!(response.starts_with("HTTP/1.1 200"));
        assert!(response.contains("\"manual_workflow_available\":true"));
        assert!(response.contains("\"assistant_backends\":[]"));
        assert!(!response.contains("composition_paths"));
        let nested = format!("{}0{}", "[".repeat(17), "]".repeat(17));
        let response = request(port, "PUT", "/v1/preferences", &nested, &[], Some(&token)).await;
        assert!(response.starts_with("HTTP/1.1 400"));
        assert!(response.contains("json-depth-limit"));
        let response = request(
            port,
            "POST",
            "/v1/workflows",
            r#"{"diary_day":"2026-10-29"}"#,
            &[],
            Some(&token),
        )
        .await;
        assert!(response.starts_with("HTTP/1.1 201"));
        assert!(response.to_ascii_lowercase().contains("etag: \"wf-0\""));
        assert!(response.contains("\"assistant_backend_id\":null"));
        let response = request(
            port,
            "POST",
            "/v1/workflows",
            r#"{"diary_day":"2026-10-30"}"#,
            &[],
            Some(&token),
        )
        .await;
        assert!(response.starts_with("HTTP/1.1 201"));
        let response = request(
            port,
            "POST",
            "/v1/workflows/wf_20261030/empty-selection-confirmations",
            "",
            &[("If-Match", "\"wf-0\"".to_owned())],
            Some(&token),
        )
        .await;
        assert!(response.starts_with("HTTP/1.1 200"));
        assert!(response.contains("\"state\":\"sources-confirmed\""));
        assert!(response.contains("\"explicit_empty_selection\":true"));
        let response = request(
            port,
            "PUT",
            "/v1/workflows/wf_20261030/writing-settings",
            r#"{"language":"en-US","style":"concise","assistant_backend_id":"unknown"}"#,
            &[("If-Match", "\"wf-1\"".to_owned())],
            Some(&token),
        )
        .await;
        assert!(response.starts_with("HTTP/1.1 409"));
        assert!(response.contains("assistant-backend-unavailable"));
        let response = request(
            port,
            "PUT",
            "/v1/workflows/wf_20261030/writing-settings",
            r#"{"language":"en-US","style":"concise","assistant_backend_id":null}"#,
            &[("If-Match", "\"wf-1\"".to_owned())],
            Some(&token),
        )
        .await;
        assert!(response.starts_with("HTTP/1.1 200"));
        assert!(response.contains("\"assistant_backend_id\":null"));
        assert!(response.contains("\"composition_path\":null"));

        let sse = open_sse(port, &token).await;
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;

        let response = request(
            port,
            "POST",
            "/v1/workflows/wf_20261029/discoveries",
            r#"{"explicit_date":null}"#,
            &[("If-Match", "\"wf-0\"".to_owned())],
            Some(&token),
        )
        .await;
        assert!(response.starts_with("HTTP/1.1 202"));
        let job_id = json_string(&response, "job_id").unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        loop {
            let response = request(
                port,
                "GET",
                &format!("/v1/jobs/{job_id}"),
                "",
                &[],
                Some(&token),
            )
            .await;
            if response.contains("\"status\":\"completed\"") {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "job did not complete");
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let event_stream = tokio::task::spawn_blocking(move || {
            let mut stream = sse;
            let mut response = Vec::new();
            loop {
                let mut bytes = [0_u8; 4096];
                let count = stream.read(&mut bytes).unwrap();
                if count == 0 {
                    break;
                }
                response.extend_from_slice(&bytes[..count]);
                if response
                    .windows(b"event: workflow.changed".len())
                    .any(|window| window == b"event: workflow.changed")
                {
                    break;
                }
            }
            String::from_utf8(response).unwrap()
        })
        .await
        .unwrap();
        assert!(event_stream.starts_with("HTTP/1.1 200"));
        assert!(event_stream.contains("event: workflow.changed"));
        assert!(!event_stream.contains("diary_day"));
        assert!(!event_stream.contains("entry_text"));

        let response = request(
            port,
            "GET",
            "/v1/workflows/wf_20261029/sources",
            "",
            &[],
            Some(&token),
        )
        .await;
        assert!(response.starts_with("HTTP/1.1 200"));
        assert!(response.contains("\"source_note_name\":\"2026/10/29/note.md\""));
        assert!(response.contains("\"id\":\"src_"));

        let response = request(
            port,
            "DELETE",
            "/v1/workflows/wf_20261029",
            "",
            &[],
            Some(&token),
        )
        .await;
        assert!(response.starts_with("HTTP/1.1 428"));
        let response = request(
            port,
            "DELETE",
            "/v1/workflows/wf_20261029",
            "",
            &[("If-Match", "\"wf-0\"".to_owned())],
            Some(&token),
        )
        .await;
        assert!(response.starts_with("HTTP/1.1 412"));
        let response = request(
            port,
            "DELETE",
            "/v1/workflows/wf_20261029",
            "",
            &[("If-Match", "\"wf-1\"".to_owned())],
            Some(&token),
        )
        .await;
        assert!(response.starts_with("HTTP/1.1 204"));

        let _ = shutdown_tx.send(());
        server.await.unwrap().unwrap();
    });
}

#[test]
fn assistant_choice_survives_restart_and_new_workflows_inherit_session_settings() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let scratch = Scratch::new("assistant-settings");
        let database = scratch.path().join("state.sqlite3");
        let backend_id = AssistantBackendId::parse("synthetic-hosted").unwrap();
        let registry = Arc::new(
            DraftAssistantRegistry::new(
                vec![Arc::new(SyntheticAssistant::new(backend_id.clone()))],
                None,
            )
            .unwrap(),
        );
        let token = format!("token_{}", "33".repeat(32));

        for restart in 0..2 {
            let store = Arc::new(SqliteStore::open(&database).unwrap());
            let listener = match TcpListener::bind(("127.0.0.1", 0)).await {
                Ok(listener) => listener,
                Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => return,
                Err(error) => panic!("test listener bind failed: {error}"),
            };
            let port = listener.local_addr().unwrap().port();
            let config = ServerConfig {
                instance_id: format!("instance_{}", "44".repeat(32)),
                control_token: token.clone(),
                port,
                commands: service(store),
                apple_helper_available: true,
                notion_configured: false,
                assistant_registry: registry.clone(),
                diagnostics: None,
                diagnostic_failure: None,
            };
            let (shutdown_tx, shutdown_rx) = oneshot::channel();
            let server = tokio::spawn(serve(listener, config, async move {
                let _ = shutdown_rx.await;
            }));

            if restart == 0 {
                let created = request(
                    port,
                    "POST",
                    "/v1/workflows",
                    r#"{"diary_day":"2026-10-28"}"#,
                    &[],
                    Some(&token),
                )
                .await;
                assert!(created.starts_with("HTTP/1.1 201"));
                let selected = request(
                    port,
                    "PUT",
                    "/v1/workflows/wf_20261028/assistant-selection",
                    r#"{"backend_id":"synthetic-hosted"}"#,
                    &[("If-Match", "\"wf-0\"".to_owned())],
                    Some(&token),
                )
                .await;
                assert!(selected.starts_with("HTTP/1.1 200"), "{selected}");
                let settings = request(
                    port,
                    "PUT",
                    "/v1/workflows/wf_20261028/writing-settings",
                    r#"{"language":"ko-KR","style":"reflective","assistant_backend_id":"synthetic-hosted"}"#,
                    &[("If-Match", "\"wf-1\"".to_owned())],
                    Some(&token),
                )
                .await;
                assert!(settings.starts_with("HTTP/1.1 200"));
                assert!(settings.contains("\"assistant_backend_id\":\"synthetic-hosted\""));
                let next = request(
                    port,
                    "POST",
                    "/v1/workflows",
                    r#"{"diary_day":"2026-10-29","language":"ko-KR","style":"reflective"}"#,
                    &[],
                    Some(&token),
                )
                .await;
                assert!(next.starts_with("HTTP/1.1 201"));
                assert!(next.contains("\"style\":\"reflective\""));
                assert!(next.contains("\"assistant_backend_id\":\"synthetic-hosted\""));
            } else {
                let restored = request(
                    port,
                    "POST",
                    "/v1/workflows",
                    r#"{"diary_day":"2026-10-30"}"#,
                    &[],
                    Some(&token),
                )
                .await;
                assert!(restored.starts_with("HTTP/1.1 201"));
                assert!(restored.contains("\"assistant_backend_id\":\"synthetic-hosted\""));
                assert!(restored.contains("\"style\":\"concise\""));
                let existing = request(
                    port,
                    "POST",
                    "/v1/workflows",
                    r#"{"diary_day":"2026-10-29","language":"en-US","style":"standard"}"#,
                    &[],
                    Some(&token),
                )
                .await;
                assert!(existing.starts_with("HTTP/1.1 200"));
                assert!(existing.contains("\"style\":\"reflective\""));
                let manual = request(
                    port,
                    "PUT",
                    "/v1/workflows/wf_20261030/assistant-selection",
                    r#"{"backend_id":null}"#,
                    &[("If-Match", "\"wf-1\"".to_owned())],
                    Some(&token),
                )
                .await;
                assert!(manual.starts_with("HTTP/1.1 200"));
                let manual_next = request(
                    port,
                    "POST",
                    "/v1/workflows",
                    r#"{"diary_day":"2026-10-31"}"#,
                    &[],
                    Some(&token),
                )
                .await;
                assert!(manual_next.starts_with("HTTP/1.1 201"));
                assert!(manual_next.contains("\"assistant_backend_id\":null"));
            }
            let _ = shutdown_tx.send(());
            server.await.unwrap().unwrap();
        }
    });
}

#[test]
fn graceful_shutdown_ends_open_event_streams() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let scratch = Scratch::new("sse-shutdown");
        let store = Arc::new(SqliteStore::open(scratch.path().join("state.sqlite3")).unwrap());
        let listener = match TcpListener::bind(("127.0.0.1", 0)).await {
            Ok(listener) => listener,
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => return,
            Err(error) => panic!("test listener bind failed: {error}"),
        };
        let port = listener.local_addr().unwrap().port();
        let token = format!("token_{}", "11".repeat(32));
        let config = ServerConfig {
            instance_id: format!("instance_{}", "22".repeat(32)),
            control_token: token.clone(),
            port,
            commands: prepared_service(store),
            apple_helper_available: true,
            notion_configured: false,
            assistant_registry: Arc::new(
                diary_core::DraftAssistantRegistry::new(Vec::new(), None).unwrap(),
            ),
            diagnostics: None,
            diagnostic_failure: None,
        };
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let server = tokio::spawn(serve(listener, config, async move {
            let _ = shutdown_rx.await;
        }));

        // Held open across shutdown, like the TUI's event connection.
        let _sse = open_sse(port, &token).await;
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;

        let _ = shutdown_tx.send(());
        tokio::time::timeout(std::time::Duration::from_secs(3), server)
            .await
            .expect("server did not stop while an event stream was open")
            .unwrap()
            .unwrap();
    });
}

async fn open_sse(port: u16, token: &str) -> TcpStream {
    let token = token.to_owned();
    tokio::task::spawn_blocking(move || {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(3)))
            .unwrap();
        write!(
            stream,
            "GET /v1/events HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {token}\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        stream
    })
    .await
    .unwrap()
}

fn json_string(response: &str, key: &str) -> Option<String> {
    let body = response.split_once("\r\n\r\n")?.1;
    let prefix = format!("\"{key}\":\"");
    let value = body.split_once(&prefix)?.1;
    Some(value.split_once('"')?.0.to_owned())
}

async fn request(
    port: u16,
    method: &str,
    path: &str,
    body: &str,
    headers: &[(&'static str, String)],
    token: Option<&str>,
) -> String {
    let headers = headers
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect::<Vec<_>>();
    let token = token.map(str::to_owned);
    let method = method.to_owned();
    let path = path.to_owned();
    let body = body.to_owned();
    tokio::task::spawn_blocking(move || {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        write!(
            stream,
            "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
            body.len()
        )
        .unwrap();
        if let Some(token) = token {
            write!(stream, "Authorization: Bearer {token}\r\n").unwrap();
        }
        for (name, value) in headers {
            write!(stream, "{name}: {value}\r\n").unwrap();
        }
        write!(stream, "\r\n{body}").unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response
    })
    .await
    .unwrap()
}

fn service(store: Arc<SqliteStore>) -> CommandService {
    CommandService::new(
        store,
        Arc::new(EmptySources),
        Arc::new(EmptySources),
        Arc::new(FixedClock),
    )
}

fn prepared_service(store: Arc<SqliteStore>) -> CommandService {
    CommandService::new(
        store,
        Arc::new(PreparedSources),
        Arc::new(PreparedSources),
        Arc::new(FixedClock),
    )
}

struct FixedClock;

struct SyntheticAssistant {
    descriptor: AssistantBackendDescriptor,
}

impl SyntheticAssistant {
    fn new(id: AssistantBackendId) -> Self {
        Self {
            descriptor: AssistantBackendDescriptor::new(
                id,
                "Synthetic Assistant",
                CompositionPath::Hosted,
                true,
                None,
                BTreeSet::from([WritingLanguage::EnUs, WritingLanguage::KoKr]),
                BTreeSet::from([
                    AssistantAction::ExtractMemories,
                    AssistantAction::DraftEntry,
                    AssistantAction::ReviewParagraph,
                ]),
            )
            .unwrap(),
        }
    }
}

impl DraftAssistant for SyntheticAssistant {
    fn descriptor(&self) -> AssistantBackendDescriptor {
        self.descriptor.clone()
    }

    fn invoke(
        &self,
        request: &AssistantRequest,
        cancelled: &AtomicBool,
    ) -> Result<AssistantResponse, DraftAssistantError> {
        if cancelled.load(Ordering::Acquire) {
            return Err(DraftAssistantError::Cancelled);
        }
        match request {
            AssistantRequest::ExtractMemories(request) => Ok(AssistantResponse::Memories(
                MemoryExtractionResult::new(vec![
                    MemoryCandidate::new("walk", vec![1], request.sources().len()).unwrap(),
                ])
                .unwrap(),
            )),
            AssistantRequest::DraftEntry(_) => Ok(AssistantResponse::EntryDraft(
                DraftEntryResult::new("Synthetic draft.", vec![]).unwrap(),
            )),
            AssistantRequest::ReviewParagraph(_) => Ok(AssistantResponse::ParagraphReview(
                ParagraphReviewResult::new(
                    "Improved paragraph.",
                    vec![],
                    Some("clearer".to_owned()),
                )
                .unwrap(),
            )),
            AssistantRequest::RefineSource(_) => Err(DraftAssistantError::UnsupportedAction),
        }
    }
}

impl Clock for FixedClock {
    fn now(&self) -> Timestamp {
        Timestamp(1_798_000_000)
    }
    fn local_day(&self) -> DiaryDay {
        DiaryDay::parse("2026-10-29").unwrap()
    }
}

struct EmptySources;

impl SourceCatalog for EmptySources {
    fn discover(&self, _request: CatalogRequest) -> Result<SourceDiscovery, SourceAccessError> {
        Ok(SourceDiscovery {
            candidates: Vec::new(),
            diagnostics: Vec::new(),
        })
    }
}

struct PreparedSources;

impl PreparedSources {
    fn source() -> SourceRef {
        SourceRef::new("synthetic", "2026/10/29/note.md").unwrap()
    }
}

impl SourceCatalog for PreparedSources {
    fn discover(&self, _request: CatalogRequest) -> Result<SourceDiscovery, SourceAccessError> {
        Ok(SourceDiscovery {
            candidates: vec![SourceCandidate {
                source: Self::source(),
                source_note_name: "2026/10/29/note.md".to_owned(),
                date_evidence: DateEvidence {
                    directory_day: Some(DiaryDay::parse("2026-10-29").unwrap()),
                    header_day: None,
                    warning: None,
                },
                availability: SourceAvailability::Resident,
            }],
            diagnostics: Vec::new(),
        })
    }
}

impl SourceReader for PreparedSources {
    fn read_selected(&self, source: &SourceRef) -> Result<SourceReadOutcome, SourceAccessError> {
        Ok(SourceReadOutcome::Read(SelectedSource {
            snapshot: SourceSnapshot {
                source: source.clone(),
                stamp: FreshnessStamp::new(
                    "synthetic-file-1",
                    15,
                    Timestamp(1),
                    Digest::of_bytes(b"Synthetic body."),
                )
                .unwrap(),
                body: "Synthetic body.".to_owned(),
            },
            date_evidence: DateEvidence {
                directory_day: Some(DiaryDay::parse("2026-10-29").unwrap()),
                header_day: Some(DiaryDay::parse("2026-10-30").unwrap()),
                warning: Some("date disagreement".to_owned()),
            },
        }))
    }

    fn revalidate(
        &self,
        _snapshot: &SourceSnapshot,
    ) -> Result<FreshnessOutcome, SourceAccessError> {
        Ok(FreshnessOutcome::Fresh)
    }

    fn request_download(&self, _source: &SourceRef) -> Result<(), SourceAccessError> {
        Ok(())
    }
}

impl SourceReader for EmptySources {
    fn read_selected(&self, source: &SourceRef) -> Result<SourceReadOutcome, SourceAccessError> {
        Ok(SourceReadOutcome::Unverifiable {
            source: source.clone(),
        })
    }
    fn revalidate(
        &self,
        _snapshot: &SourceSnapshot,
    ) -> Result<FreshnessOutcome, SourceAccessError> {
        Ok(FreshnessOutcome::Fresh)
    }
    fn request_download(&self, _source: &SourceRef) -> Result<(), SourceAccessError> {
        Ok(())
    }
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "nd-control-api-{label}-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
