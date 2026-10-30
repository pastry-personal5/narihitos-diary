#![forbid(unsafe_code)]

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use diary_connectors::{LocalOutput, LocalState, SqliteStore};
use diary_control::CommandService;
use diary_control::ServerConfig;
use diary_core::{
    CatalogRequest, Clock, DiaryDay, DraftAssistantRegistry, FreshnessOutcome, LocalOutputError,
    LocalOutputPort, LocalOutputState, OperationIntent, OperationLog, Receipt, SourceAccessError,
    SourceCatalog, SourceDiscovery, SourceReadOutcome, SourceReader, SourceRef, SourceSnapshot,
    StartOutcome, Timestamp, Workflow, WorkflowStore, WritingLanguage, WritingStyle,
};
use diary_tui::client::{Attachment, ClientOutcome, UreqTransport};
use diary_tui::worker::{Intent, Worker, WorkerResult};
use std::time::{Duration, Instant};

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct FixedClock;
impl Clock for FixedClock {
    fn now(&self) -> Timestamp {
        Timestamp(100)
    }
    fn local_day(&self) -> DiaryDay {
        DiaryDay::parse("2026-10-30").unwrap()
    }
}

struct EmptySources;
impl SourceCatalog for EmptySources {
    fn discover(&self, _: CatalogRequest) -> Result<SourceDiscovery, SourceAccessError> {
        Ok(SourceDiscovery {
            candidates: Vec::new(),
            diagnostics: Vec::new(),
        })
    }
}
impl SourceReader for EmptySources {
    fn read_selected(&self, _: &SourceRef) -> Result<SourceReadOutcome, SourceAccessError> {
        Err(SourceAccessError::NotFound)
    }
    fn revalidate(&self, _: &SourceSnapshot) -> Result<FreshnessOutcome, SourceAccessError> {
        Err(SourceAccessError::NotFound)
    }
    fn request_download(&self, _: &SourceRef) -> Result<(), SourceAccessError> {
        Err(SourceAccessError::NotFound)
    }
}

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        loop {
            let path = std::env::temp_dir().join(format!(
                "nd-p1-m9-{}-{}",
                std::process::id(),
                SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self(fs::canonicalize(path).unwrap()),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => panic!("scratch create failed: {error}"),
            }
        }
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn service(store: Arc<SqliteStore>, output: Arc<LocalOutput>) -> CommandService {
    CommandService::new(
        store,
        Arc::new(EmptySources),
        Arc::new(EmptySources),
        Arc::new(FixedClock),
    )
    .with_local_output(output)
}

struct AmbiguousOutput {
    inner: LocalOutput,
    unavailable: AtomicBool,
}

struct CancellingOutput {
    inner: LocalOutput,
    store: Arc<SqliteStore>,
}
impl LocalOutputPort for CancellingOutput {
    fn target(&self, day: &DiaryDay) -> String {
        self.inner.target(day).to_string_lossy().into_owned()
    }
    fn inspect(&self, day: &DiaryDay, payload: &str) -> Result<LocalOutputState, LocalOutputError> {
        self.inner.inspect(day, payload)
    }
    fn write(&self, day: &DiaryDay, payload: &str) -> Result<LocalOutputState, LocalOutputError> {
        let intent = self
            .store
            .unresolved_intents(Some(day))
            .unwrap()
            .pop()
            .unwrap();
        self.store.request_cancellation(&intent.attempt_id).unwrap();
        self.inner.write(day, payload)
    }
}
impl LocalOutputPort for AmbiguousOutput {
    fn target(&self, day: &DiaryDay) -> String {
        self.inner.target(day).to_string_lossy().into_owned()
    }
    fn inspect(&self, day: &DiaryDay, payload: &str) -> Result<LocalOutputState, LocalOutputError> {
        if self.unavailable.load(Ordering::Acquire) {
            Err(LocalOutputError::Unavailable)
        } else {
            self.inner.inspect(day, payload)
        }
    }
    fn write(&self, day: &DiaryDay, payload: &str) -> Result<LocalOutputState, LocalOutputError> {
        let result = self.inner.write(day, payload);
        self.unavailable.store(true, Ordering::Release);
        result.map(|_| LocalOutputState::Absent)
    }
}

fn confirmed(service: &CommandService, day: &DiaryDay) -> Workflow {
    let mut workflow = match service.start_workflow(Some(day.clone())).unwrap() {
        StartOutcome::Created(workflow) => workflow,
        _ => panic!("expected a new workflow"),
    };
    workflow = service.discover(day, workflow.revision(), None).unwrap();
    workflow = service
        .confirm_empty_selection(day, workflow.revision())
        .unwrap();
    workflow = service
        .set_writing_settings(
            day,
            workflow.revision(),
            WritingLanguage::KoKr,
            WritingStyle::Concise,
            None,
        )
        .unwrap();
    workflow = service
        .begin_manual_composition(day, workflow.revision())
        .unwrap();
    workflow = service
        .save_entry(day, workflow.entry().version, "Today was quiet.".to_owned())
        .unwrap();
    service
        .confirm_entry(day, workflow.revision(), workflow.entry().version)
        .unwrap()
}

#[test]
fn local_publication_requires_exact_approval_and_persists_receipt() {
    let scratch = Scratch::new();
    let store = Arc::new(SqliteStore::open(scratch.0.join("state.sqlite3")).unwrap());
    let output = Arc::new(LocalOutput::new(scratch.0.join("entries")).unwrap());
    let service = service(store.clone(), output.clone());
    let day = DiaryDay::parse("2026-10-30").unwrap();
    let confirmed = confirmed(&service, &day);
    assert!(service.publish_local(&day, confirmed.revision()).is_err());
    let preview = service.preview_local(&day, confirmed.revision()).unwrap();
    assert_eq!(
        preview.payload,
        "---\ndate: \"2026-10-30\"\nlanguage: \"ko-KR\"\n---\n\nToday was quiet.\n"
    );
    assert!(
        service
            .approve_local(
                &day,
                preview.workflow.revision(),
                &diary_core::Digest::of_bytes(b"wrong")
            )
            .is_err()
    );
    fs::create_dir_all(output.target(&day).parent().unwrap()).unwrap();
    fs::write(output.target(&day), "intervening content").unwrap();
    assert!(
        service
            .approve_local(
                &day,
                preview.workflow.revision(),
                &preview.workflow.output_plan().unwrap().plan_digest
            )
            .is_err()
    );
    assert_eq!(
        fs::read_to_string(output.target(&day)).unwrap(),
        "intervening content"
    );
    fs::remove_file(output.target(&day)).unwrap();
    let approved = service
        .approve_local(
            &day,
            preview.workflow.revision(),
            &preview.workflow.output_plan().unwrap().plan_digest,
        )
        .unwrap();
    assert!(
        service
            .publish_local_guarded(&day, approved.revision(), || false)
            .is_err()
    );
    assert!(store.unresolved_intents(Some(&day)).unwrap().is_empty());
    assert!(!output.target(&day).exists());
    fs::write(output.target(&day), "later content").unwrap();
    assert!(service.publish_local(&day, approved.revision()).is_err());
    assert_eq!(
        fs::read_to_string(output.target(&day)).unwrap(),
        "later content"
    );
    fs::remove_file(output.target(&day)).unwrap();
    let published = service.publish_local(&day, approved.revision()).unwrap();
    assert!(service.publish_local(&day, approved.revision()).is_err());
    assert_eq!(
        published.destination_progress()[0].status.as_str(),
        "verified"
    );
    assert_eq!(
        fs::read_to_string(output.target(&day)).unwrap(),
        preview.payload
    );
    assert_eq!(store.receipts(&day).unwrap().len(), 1);
    assert!(store.unresolved_intents(Some(&day)).unwrap().is_empty());
}

#[test]
fn restart_reconciles_write_after_lost_response_and_blocks_a_different_file() {
    let scratch = Scratch::new();
    let db = scratch.0.join("state.sqlite3");
    let store = Arc::new(SqliteStore::open(&db).unwrap());
    let output = Arc::new(LocalOutput::new(scratch.0.join("entries")).unwrap());
    let publication_service = service(store.clone(), output.clone());
    let day = DiaryDay::parse("2026-10-30").unwrap();
    let confirmed_entry = confirmed(&publication_service, &day);
    let preview = publication_service
        .preview_local(&day, confirmed_entry.revision())
        .unwrap();
    let approved = publication_service
        .approve_local(
            &day,
            preview.workflow.revision(),
            &preview.workflow.output_plan().unwrap().plan_digest,
        )
        .unwrap();
    let mut publishing = approved.clone();
    publishing
        .begin_publication(approved.revision(), Timestamp(100))
        .unwrap();
    store
        .compare_and_swap(approved.revision(), None, &publishing)
        .unwrap();
    let plan = publishing.output_plan().unwrap();
    let target = plan
        .targets
        .iter()
        .find(|target| target.destination == "local")
        .unwrap();
    let intent = OperationIntent::new(
        "lost-reply",
        plan.plan_id.clone(),
        day.clone(),
        target,
        plan.entry_digest.clone(),
        plan.selection_digest.clone(),
        Timestamp(100),
    )
    .unwrap();
    assert!(store.record_intent(&intent).unwrap());
    assert_eq!(
        output.write(&day, &preview.payload),
        Ok(LocalState::Identical)
    );
    drop(publication_service);
    drop(store);
    let reopened = Arc::new(SqliteStore::open(&db).unwrap());
    let restarted = service(reopened.clone(), output.clone());
    restarted.reconcile_local_intents().unwrap();
    assert_eq!(reopened.receipts(&day).unwrap().len(), 1);
    assert_eq!(
        restarted
            .load_workflow(&day)
            .unwrap()
            .destination_progress()[0]
            .status
            .as_str(),
        "verified"
    );
    assert!(reopened.unresolved_intents(Some(&day)).unwrap().is_empty());

    let other_day = DiaryDay::parse("2026-10-31").unwrap();
    let confirmed = confirmed(&restarted, &other_day);
    fs::create_dir_all(output.target(&other_day).parent().unwrap()).unwrap();
    fs::write(output.target(&other_day), "other person's file").unwrap();
    let conflict = restarted
        .preview_local(&other_day, confirmed.revision())
        .unwrap();
    assert_eq!(
        conflict.existing_text.as_deref(),
        Some("other person's file")
    );
    assert!(
        restarted
            .approve_local(
                &other_day,
                conflict.workflow.revision(),
                &conflict.workflow.output_plan().unwrap().plan_digest
            )
            .is_err()
    );
    assert_eq!(
        fs::read_to_string(output.target(&other_day)).unwrap(),
        "other person's file"
    );
    let carried = restarted
        .save_local_conflict_under_day(
            &other_day,
            conflict.workflow.revision(),
            DiaryDay::parse("2026-11-01").unwrap(),
        )
        .unwrap();
    assert_eq!(carried.entry().text, "Today was quiet.");
    assert!(carried.entry().confirmed.is_none());
    assert_eq!(carried.state().as_str(), "discovered");
    assert!(carried.selection_digest().is_none());
    let reopened_day = restarted
        .save_local_conflict_under_day(
            &other_day,
            conflict.workflow.revision(),
            DiaryDay::parse("2026-11-01").unwrap(),
        )
        .unwrap();
    assert_eq!(reopened_day, carried);
}

#[test]
fn real_api_worker_previews_approves_and_publishes_local_file() {
    let scratch = Scratch::new();
    let store = Arc::new(SqliteStore::open(scratch.0.join("state.sqlite3")).unwrap());
    let output = Arc::new(LocalOutput::new(scratch.0.join("entries")).unwrap());
    let service = service(store.clone(), output.clone());
    let day = DiaryDay::parse("2026-10-30").unwrap();
    let confirmed = confirmed(&service, &day);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let listener =
        match runtime.block_on(async { tokio::net::TcpListener::bind("127.0.0.1:0").await }) {
            Ok(listener) => listener,
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => return,
            Err(error) => panic!("listener bind failed: {error}"),
        };
    let port = listener.local_addr().unwrap().port();
    let config = ServerConfig {
        instance_id: "synthetic-instance".to_owned(),
        control_token: "synthetic-token".to_owned(),
        port,
        commands: service,
        apple_helper_available: false,
        notion_configured: false,
        assistant_registry: Arc::new(DraftAssistantRegistry::new(Vec::new(), None).unwrap()),
        diagnostics: None,
        diagnostic_failure: None,
    };
    let server = runtime.spawn(diary_control::serve(listener, config, async {
        std::future::pending::<()>().await
    }));
    let attachment = Attachment::new(
        "synthetic-instance".to_owned(),
        "127.0.0.1".to_owned(),
        port,
        "synthetic-token".to_owned(),
        diary_control_contract::CONTRACT_VERSION,
    );
    let worker = Worker::spawn(UreqTransport::new(), attachment);
    let id = "wf_20261030".to_owned();
    worker
        .try_send(Intent::PreviewLocal {
            workflow_id: id.clone(),
            revision: confirmed.revision(),
        })
        .unwrap();
    let preview = match wait_result(&worker) {
        WorkerResult::LocalPreviewed(outcome) => match *outcome {
            ClientOutcome::Ok(preview) => preview,
            _ => panic!("preview failed"),
        },
        _ => panic!("wrong worker result"),
    };
    assert_eq!(preview.notion_status, "blocked");
    worker
        .try_send(Intent::ApproveLocal {
            workflow_id: id.clone(),
            revision: preview.workflow.revision,
            plan_digest: preview.plan_digest.clone(),
        })
        .unwrap();
    let approved = match wait_result(&worker) {
        WorkerResult::LocalApproved(outcome) => match *outcome {
            ClientOutcome::Ok(workflow) => workflow,
            _ => panic!("approval failed"),
        },
        _ => panic!("wrong worker result"),
    };
    worker
        .try_send(Intent::PublishLocal {
            workflow_id: id.clone(),
            revision: approved.revision,
        })
        .unwrap();
    let job_id = match wait_result(&worker) {
        WorkerResult::LocalPublished(ClientOutcome::Ok(job)) => job.job_id,
        _ => panic!("publication job not accepted"),
    };
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        worker
            .try_send(Intent::PollJob {
                job_id: job_id.clone(),
            })
            .unwrap();
        match wait_result(&worker) {
            WorkerResult::JobStatus(ClientOutcome::Ok(job))
                if job.status == diary_control_contract::job_status::COMPLETED =>
            {
                break;
            }
            WorkerResult::JobStatus(ClientOutcome::Ok(job)) if Instant::now() < deadline => {
                assert!(diary_control_contract::job_status::is_active(&job.status));
                std::thread::sleep(Duration::from_millis(20));
            }
            _ => panic!("publication job failed"),
        }
    }
    worker
        .try_send(Intent::GetWorkflow {
            workflow_id: id.clone(),
        })
        .unwrap();
    match wait_result(&worker) {
        WorkerResult::WorkflowFetched(outcome) => match *outcome {
            ClientOutcome::Ok(workflow) => {
                assert_eq!(workflow.local_status.as_deref(), Some("verified"));
                assert_eq!(workflow.notion_status.as_deref(), Some("blocked"));
            }
            _ => panic!("reload failed"),
        },
        _ => panic!("wrong worker result"),
    }
    assert!(output.target(&day).exists());
    drop(worker);
    runtime.block_on(async {
        server.abort();
        let _ = server.await;
    });

    drop(store);
    let reopened = Arc::new(SqliteStore::open(scratch.0.join("state.sqlite3")).unwrap());
    let restarted_service = crate::service(reopened.clone(), output.clone());
    restarted_service.reconcile_local_intents().unwrap();
    let conflict_day = DiaryDay::parse("2026-10-31").unwrap();
    let conflict_entry = crate::confirmed(&restarted_service, &conflict_day);
    fs::write(output.target(&conflict_day), "existing different text").unwrap();
    let listener =
        runtime.block_on(async { tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap() });
    let port = listener.local_addr().unwrap().port();
    let config = ServerConfig {
        instance_id: "restarted-instance".to_owned(),
        control_token: "restarted-token".to_owned(),
        port,
        commands: restarted_service,
        apple_helper_available: false,
        notion_configured: false,
        assistant_registry: Arc::new(DraftAssistantRegistry::new(Vec::new(), None).unwrap()),
        diagnostics: None,
        diagnostic_failure: None,
    };
    let server = runtime.spawn(diary_control::serve(listener, config, async {
        std::future::pending::<()>().await
    }));
    let attachment = Attachment::new(
        "restarted-instance".to_owned(),
        "127.0.0.1".to_owned(),
        port,
        "restarted-token".to_owned(),
        diary_control_contract::CONTRACT_VERSION,
    );
    let worker = Worker::spawn(UreqTransport::new(), attachment);
    worker
        .try_send(Intent::GetWorkflow { workflow_id: id })
        .unwrap();
    match wait_result(&worker) {
        WorkerResult::WorkflowFetched(outcome) => match *outcome {
            ClientOutcome::Ok(workflow) => {
                assert_eq!(workflow.local_status.as_deref(), Some("verified"));
                assert_eq!(workflow.notion_status.as_deref(), Some("blocked"));
            }
            _ => panic!("restarted workflow failed"),
        },
        _ => panic!("wrong worker result"),
    }
    worker
        .try_send(Intent::PreviewLocal {
            workflow_id: "wf_20261031".to_owned(),
            revision: conflict_entry.revision(),
        })
        .unwrap();
    let conflict = match wait_result(&worker) {
        WorkerResult::LocalPreviewed(outcome) => match *outcome {
            ClientOutcome::Ok(preview) => preview,
            _ => panic!("conflict preview failed"),
        },
        _ => panic!("wrong worker result"),
    };
    assert_eq!(
        conflict.existing_text.as_deref(),
        Some("existing different text")
    );
    worker
        .try_send(Intent::ApproveLocal {
            workflow_id: "wf_20261031".to_owned(),
            revision: conflict.workflow.revision,
            plan_digest: conflict.plan_digest,
        })
        .unwrap();
    assert!(
        matches!(wait_result(&worker), WorkerResult::LocalApproved(outcome) if matches!(*outcome, ClientOutcome::Problem(_)))
    );
    assert_eq!(
        fs::read_to_string(output.target(&conflict_day)).unwrap(),
        "existing different text"
    );
    worker
        .try_send(Intent::SaveUnderDay {
            workflow_id: "wf_20261031".to_owned(),
            revision: conflict.workflow.revision,
            new_day: "2026-11-01".to_owned(),
        })
        .unwrap();
    match wait_result(&worker) {
        WorkerResult::SavedUnderDay(outcome) => match *outcome {
            ClientOutcome::Ok(workflow) => {
                assert_eq!(workflow.diary_day, "2026-11-01");
                assert_eq!(workflow.state, "discovered");
                assert!(!workflow.entry_confirmed);
                assert_eq!(workflow.entry_text, "Today was quiet.");
            }
            _ => panic!("save under another day failed"),
        },
        _ => panic!("wrong worker result"),
    }
    drop(worker);
    runtime.block_on(async {
        server.abort();
        let _ = server.await;
    });
}

fn wait_result(worker: &Worker) -> WorkerResult {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(result) = worker.try_recv() {
            return result;
        }
        assert!(Instant::now() < deadline, "worker reply timed out");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn restart_recovers_before_intent_and_after_receipt_boundaries() {
    let scratch = Scratch::new();
    let store = Arc::new(SqliteStore::open(scratch.0.join("state.sqlite3")).unwrap());
    let output = Arc::new(LocalOutput::new(scratch.0.join("entries")).unwrap());
    let service = service(store.clone(), output.clone());
    for (day_text, after_receipt) in [("2026-10-30", false), ("2026-10-31", true)] {
        let day = DiaryDay::parse(day_text).unwrap();
        let confirmed = confirmed(&service, &day);
        let preview = service.preview_local(&day, confirmed.revision()).unwrap();
        let approved = service
            .approve_local(
                &day,
                preview.workflow.revision(),
                &preview.workflow.output_plan().unwrap().plan_digest,
            )
            .unwrap();
        let mut publishing = approved.clone();
        publishing
            .begin_publication(approved.revision(), Timestamp(100))
            .unwrap();
        store
            .compare_and_swap(approved.revision(), None, &publishing)
            .unwrap();
        if after_receipt {
            let plan = publishing.output_plan().unwrap();
            let target = plan
                .targets
                .iter()
                .find(|target| target.destination == "local")
                .unwrap();
            let intent = OperationIntent::new(
                format!("attempt-{day_text}"),
                plan.plan_id.clone(),
                day.clone(),
                target,
                plan.entry_digest.clone(),
                plan.selection_digest.clone(),
                Timestamp(100),
            )
            .unwrap();
            store.record_intent(&intent).unwrap();
            assert_eq!(
                output.write(&day, &preview.payload),
                Ok(LocalState::Identical)
            );
            store
                .record_receipt(&Receipt {
                    receipt_id: format!("receipt-{day_text}"),
                    attempt_id: intent.attempt_id,
                    day: day.clone(),
                    destination: "local".into(),
                    target_identity: target.target_identity.clone(),
                    version: 1,
                    entry_digest: plan.entry_digest.clone(),
                    selection_digest: plan.selection_digest.clone(),
                    payload_digest: target.payload_digest.clone(),
                    returned_ids: Vec::new(),
                    verified_at: Timestamp(100),
                })
                .unwrap();
        }
        service.reconcile_local_intents().unwrap();
        let recovered = service.load_workflow(&day).unwrap();
        assert_eq!(
            recovered.destination_progress()[0].status.as_str(),
            if after_receipt { "verified" } else { "pending" }
        );
        if !after_receipt {
            assert_eq!(
                service
                    .publish_local(&day, recovered.revision())
                    .unwrap()
                    .destination_progress()[0]
                    .status
                    .as_str(),
                "verified"
            );
        }
    }
}

#[test]
fn uncertain_readback_gates_only_its_day_until_reconciliation() {
    let scratch = Scratch::new();
    let store = Arc::new(SqliteStore::open(scratch.0.join("state.sqlite3")).unwrap());
    let output = Arc::new(AmbiguousOutput {
        inner: LocalOutput::new(scratch.0.join("entries")).unwrap(),
        unavailable: AtomicBool::new(false),
    });
    let service = CommandService::new(
        store.clone(),
        Arc::new(EmptySources),
        Arc::new(EmptySources),
        Arc::new(FixedClock),
    )
    .with_local_output(output.clone());
    let day = DiaryDay::parse("2026-10-30").unwrap();
    let confirmed = confirmed(&service, &day);
    let preview = service.preview_local(&day, confirmed.revision()).unwrap();
    let approved = service
        .approve_local(
            &day,
            preview.workflow.revision(),
            &preview.workflow.output_plan().unwrap().plan_digest,
        )
        .unwrap();
    let publishing = service.publish_local(&day, approved.revision()).unwrap();
    assert_eq!(publishing.state().as_str(), "publishing");
    assert_eq!(store.unresolved_intents(Some(&day)).unwrap().len(), 1);
    assert!(service.start_workflow(Some(day.clone())).is_err());
    assert!(
        service
            .start_workflow(Some(DiaryDay::parse("2026-10-31").unwrap()))
            .is_ok()
    );
    service.reconcile_local_intents().unwrap();
    assert_eq!(store.unresolved_intents(Some(&day)).unwrap().len(), 1);
    output.unavailable.store(false, Ordering::Release);
    service.reconcile_local_intents().unwrap();
    assert!(store.unresolved_intents(Some(&day)).unwrap().is_empty());
    assert_eq!(
        service.load_workflow(&day).unwrap().destination_progress()[0]
            .status
            .as_str(),
        "verified"
    );
}

#[test]
fn cancellation_requested_after_intent_does_not_undo_verified_write() {
    let scratch = Scratch::new();
    let store = Arc::new(SqliteStore::open(scratch.0.join("state.sqlite3")).unwrap());
    let output = Arc::new(CancellingOutput {
        inner: LocalOutput::new(scratch.0.join("entries")).unwrap(),
        store: store.clone(),
    });
    let service = CommandService::new(
        store.clone(),
        Arc::new(EmptySources),
        Arc::new(EmptySources),
        Arc::new(FixedClock),
    )
    .with_local_output(output.clone());
    let day = DiaryDay::parse("2026-10-30").unwrap();
    let confirmed = confirmed(&service, &day);
    let preview = service.preview_local(&day, confirmed.revision()).unwrap();
    let approved = service
        .approve_local(
            &day,
            preview.workflow.revision(),
            &preview.workflow.output_plan().unwrap().plan_digest,
        )
        .unwrap();
    let finished = service.publish_local(&day, approved.revision()).unwrap();
    assert_eq!(
        finished.destination_progress()[0].status.as_str(),
        "verified"
    );
    assert_eq!(store.receipts(&day).unwrap().len(), 1);
    assert_eq!(
        fs::read_to_string(output.inner.target(&day)).unwrap(),
        preview.payload
    );
}
