#![forbid(unsafe_code)]

use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use diary_connectors::SqliteStore;
use diary_control::{CommandError, CommandService, ServerConfig, serve};
use diary_core::{
    CatalogRequest, Clock, CompositionPath, DateEvidence, DestinationKind, DestinationPlan,
    DiaryDay, Digest, FreshnessOutcome, FreshnessStamp, IntentTerminal, OperationIntent,
    OperationLog, Preferences, SelectedSource, SourceAccessError, SourceAvailability,
    SourceCandidate, SourceCatalog, SourceDiscovery, SourceReadOutcome, SourceReader, SourceRef,
    SourceSnapshot, Timestamp, WorkflowStore, WritingLanguage, WritingStyle,
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
            CompositionPath::Manual,
        )
        .unwrap();
    assert_eq!(
        store.load_preferences().unwrap(),
        Preferences {
            last_language: Some(WritingLanguage::KoKr),
            last_composition_path: CompositionPath::Manual,
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
fn real_loopback_enforces_headers_authorization_and_conditional_mutations() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let scratch = Scratch::new("loopback");
        let store = Arc::new(SqliteStore::open(scratch.path().join("state.sqlite3")).unwrap());
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let token = format!("token_{}", "11".repeat(32));
        let config = ServerConfig {
            instance_id: format!("instance_{}", "22".repeat(32)),
            control_token: token.clone(),
            port,
            commands: service(store),
            apple_helper_available: true,
            notion_configured: false,
            diagnostics: None,
            diagnostic_failure: None,
        };
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let server = tokio::spawn(serve(listener, config, async move {
            let _ = shutdown_rx.await;
        }));

        let response = request(port, "GET", "/v1/health", "", &[], None).await;
        assert!(response.starts_with("HTTP/1.1 200"));
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

struct FixedClock;

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
            "nd-p1-m6-{label}-{}-{}",
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
