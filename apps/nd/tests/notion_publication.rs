#![forbid(unsafe_code)]

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use diary_connectors::{LocalOutput, SqliteStore};
use diary_control::CommandService;
use diary_control::ServerConfig;
use diary_core::{
    CatalogRequest, Clock, DestinationStatus, DiaryDay, Digest, DraftAssistantRegistry,
    FreshnessOutcome, InspectionOutcome, NotionBlockPreview, NotionOutputPort, NotionPreflight,
    NotionPreflightStatus, NotionRequest, OperationLog, SourceAccessError, SourceCatalog,
    SourceDiscovery, SourceReadOutcome, SourceReader, SourceRef, SourceSnapshot, StartOutcome,
    Timestamp, Workflow, WriteOutcome, WritingLanguage, WritingStyle,
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
                "nd-local-publication-{}-{}",
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

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Verify,
    Ambiguous,
    NoEffect,
    Conflict,
}

struct Inner {
    status: NotionPreflightStatus,
    reason: Option<&'static str>,
    observed: &'static [u8],
    mode: Mode,
    inspect: InspectionOutcome,
    writes: usize,
}

struct Scripted(Mutex<Inner>);

impl Scripted {
    fn new() -> Arc<Self> {
        Arc::new(Self(Mutex::new(Inner {
            status: NotionPreflightStatus::Ready,
            reason: None,
            observed: b"empty",
            mode: Mode::Verify,
            inspect: InspectionOutcome::Absent,
            writes: 0,
        })))
    }
    fn set(&self, change: impl FnOnce(&mut Inner)) {
        change(&mut self.0.lock().unwrap());
    }
    fn writes(&self) -> usize {
        self.0.lock().unwrap().writes
    }
}

impl NotionOutputPort for Scripted {
    fn preflight(&self, day: &DiaryDay, markdown: &str) -> Result<NotionPreflight, String> {
        let inner = self.0.lock().unwrap();
        Ok(NotionPreflight {
            target_identity: "synthetic-root".into(),
            root_label: "Notion root".into(),
            year_title: day.as_str()[..4].to_owned(),
            month_title: day.as_str()[..7].to_owned(),
            year_exists: false,
            month_exists: false,
            placement: "end".to_owned(),
            renderer_version: 1,
            api_version: "2026-03-11".into(),
            payload_digest: Digest::of_bytes(markdown.as_bytes()),
            observed_state_digest: Digest::of_bytes(inner.observed),
            blocks: vec![NotionBlockPreview {
                kind: "heading_2".into(),
                text: format!("{} (FRI)", day.as_str()),
                depth: 0,
            }],
            status: inner.status,
            reason: (inner.status != NotionPreflightStatus::Ready).then(|| {
                inner
                    .reason
                    .unwrap_or("notion-permission-denied")
                    .to_owned()
            }),
        })
    }

    fn month_page_url(&self, _: &DiaryDay) -> Option<String> {
        Some("https://www.notion.so/0123456789abcdef0123456789abcdef".to_owned())
    }

    fn write(&self, _: &NotionRequest, cancelled: &AtomicBool) -> WriteOutcome {
        let mut inner = self.0.lock().unwrap();
        if cancelled.load(Ordering::Acquire) {
            return WriteOutcome::DefiniteNoEffect;
        }
        inner.writes += 1;
        match inner.mode {
            Mode::Verify => WriteOutcome::Verified {
                receipt_id: "synthetic-month".into(),
                returned_ids: vec!["synthetic-heading".into()],
            },
            Mode::Ambiguous => WriteOutcome::AmbiguousWrite,
            Mode::NoEffect => WriteOutcome::DefiniteNoEffect,
            Mode::Conflict => WriteOutcome::Conflict,
        }
    }

    fn inspect(&self, _: &NotionRequest) -> InspectionOutcome {
        self.0.lock().unwrap().inspect.clone()
    }
}

/// Local file verified and receipted; Notion still blocked (`partial`).
fn partial(service: &CommandService, day: &DiaryDay) -> Workflow {
    let confirmed = confirmed(service, day);
    let local = service.preview_local(day, confirmed.revision()).unwrap();
    let local = service
        .approve_local(
            day,
            local.workflow.revision(),
            &local.workflow.output_plan().unwrap().plan_digest,
        )
        .unwrap();
    service.publish_local(day, local.revision()).unwrap()
}

fn setup() -> (Scratch, Arc<SqliteStore>, Arc<Scripted>, CommandService) {
    let scratch = Scratch::new();
    let store = Arc::new(SqliteStore::open(scratch.0.join("state.sqlite3")).unwrap());
    let output = Arc::new(LocalOutput::new(scratch.0.join("entries")).unwrap());
    let notion = Scripted::new();
    let service = service(store.clone(), output).with_notion_output(notion.clone());
    (scratch, store, notion, service)
}

fn notion_status(workflow: &Workflow) -> &'static str {
    workflow
        .destination_progress()
        .iter()
        .find(|item| item.destination == "notion")
        .map_or("none", |item| item.status.as_str())
}

fn approved(service: &CommandService, day: &DiaryDay) -> Workflow {
    let current = service.load_workflow(day).unwrap();
    let preview = service.preview_notion(day, current.revision()).unwrap();
    service
        .approve_notion(
            day,
            preview.workflow.revision(),
            preview.plan_digest.as_ref().unwrap(),
        )
        .unwrap()
}

#[test]
fn approval_and_publication_void_when_remote_state_changes() {
    let (_scratch, _store, notion, service) = setup();
    let day = DiaryDay::parse("2026-10-30").unwrap();
    let partial = partial(&service, &day);
    let preview = service.preview_notion(&day, partial.revision()).unwrap();
    assert_eq!(preview.status.as_str(), "pending");
    assert_eq!(preview.year_title, "2026");
    assert_eq!(preview.month_title, "2026-10");
    notion.set(|inner| inner.observed = b"changed");
    assert!(
        service
            .approve_notion(
                &day,
                preview.workflow.revision(),
                preview.plan_digest.as_ref().unwrap()
            )
            .is_err()
    );
    let approved = approved(&service, &day);
    notion.set(|inner| inner.observed = b"changed-again");
    assert!(service.publish_notion(&day, approved.revision()).is_err());
    assert_eq!(notion.writes(), 0);
}

/// Notion reports the month page as the receipt, so every day of one month
/// shares that ID. Receipts are keyed by ID, so a second day used to fail to
/// save its receipt after the write landed, leaving the day reconciling.
#[test]
fn two_days_in_one_month_each_save_their_own_notion_receipt() {
    let (_scratch, store, notion, service) = setup();
    for text in ["2026-10-30", "2026-10-29"] {
        let day = DiaryDay::parse(text).unwrap();
        partial(&service, &day);
        let approved = approved(&service, &day);
        let done = service
            .publish_notion(&day, approved.revision())
            .unwrap_or_else(|error| panic!("{text}: {error:?}"));
        assert_eq!(notion_status(&done), "verified", "{text}");
        assert!(
            store.unresolved_intents(Some(&day)).unwrap().is_empty(),
            "{text} must not be left reconciling"
        );
        assert!(
            store
                .receipts(&day)
                .unwrap()
                .iter()
                .any(|receipt| receipt.destination == "notion"),
            "{text}"
        );
    }
    assert_eq!(notion.writes(), 2);
}

/// The stuck-day shape: another day of the same month already holds the
/// month-page receipt, and this day's write landed but was not confirmed.
/// Recovery must adopt it instead of failing to save a colliding receipt.
#[test]
fn restart_adopts_a_landed_write_when_another_day_of_the_month_is_published() {
    let (_scratch, store, notion, service) = setup();
    let first = DiaryDay::parse("2026-10-29").unwrap();
    partial(&service, &first);
    let approved_first = approved(&service, &first);
    service
        .publish_notion(&first, approved_first.revision())
        .unwrap();

    let day = DiaryDay::parse("2026-10-30").unwrap();
    partial(&service, &day);
    let approved = approved(&service, &day);
    notion.set(|inner| inner.mode = Mode::Ambiguous);
    let reconciling = service.publish_notion(&day, approved.revision()).unwrap();
    assert_eq!(reconciling.state().as_str(), "publishing");
    assert_eq!(store.unresolved_intents(Some(&day)).unwrap().len(), 1);

    notion.set(|inner| {
        inner.inspect = InspectionOutcome::Matching {
            receipt_id: "synthetic-month".into(),
            returned_ids: vec!["synthetic-heading".into()],
        };
    });
    service.reconcile_notion_intents().unwrap();
    assert!(store.unresolved_intents(Some(&day)).unwrap().is_empty());
    let done = service.load_workflow(&day).unwrap();
    assert_eq!(done.state().as_str(), "completed");
    assert_eq!(notion_status(&done), "verified");
    assert_eq!(notion.writes(), 2);
}

#[test]
fn blocked_and_conflict_previews_carry_no_plan_and_need_a_new_approval() {
    let (_scratch, _store, notion, service) = setup();
    let day = DiaryDay::parse("2026-10-30").unwrap();
    let partial = partial(&service, &day);
    for status in [
        NotionPreflightStatus::Blocked,
        NotionPreflightStatus::Existing,
    ] {
        notion.set(|inner| inner.status = status);
        let preview = service.preview_notion(&day, partial.revision()).unwrap();
        assert!(preview.plan_digest.is_none());
        assert_eq!(preview.reason.as_deref(), Some("notion-permission-denied"));
        assert!(service.publish_notion(&day, partial.revision()).is_err());
    }
    notion.set(|inner| inner.status = NotionPreflightStatus::Ready);
    let approved = approved(&service, &day);
    let done = service.publish_notion(&day, approved.revision()).unwrap();
    assert_eq!(notion_status(&done), "verified");
    assert_eq!(notion.writes(), 1);
}

#[test]
fn ambiguous_write_gates_its_day_and_restart_adopts_without_a_second_write() {
    let (_scratch, store, notion, service) = setup();
    let day = DiaryDay::parse("2026-10-30").unwrap();
    partial(&service, &day);
    let approved = approved(&service, &day);
    notion.set(|inner| inner.mode = Mode::Ambiguous);
    let reconciling = service.publish_notion(&day, approved.revision()).unwrap();
    assert_eq!(reconciling.state().as_str(), "publishing");
    assert_eq!(notion.writes(), 1);
    assert_eq!(store.unresolved_intents(Some(&day)).unwrap().len(), 1);
    // Duplicate commands and previews are gated; another day is not.
    assert!(
        service
            .publish_notion(&day, reconciling.revision())
            .is_err()
    );
    assert!(
        service
            .preview_notion(&day, reconciling.revision())
            .is_err()
    );
    assert!(
        service
            .start_workflow(Some(DiaryDay::parse("2026-10-31").unwrap()))
            .is_ok()
    );
    // Undecidable inspection keeps the gate.
    notion.set(|inner| inner.inspect = InspectionOutcome::Unavailable);
    service.reconcile_notion_intents().unwrap();
    assert_eq!(store.unresolved_intents(Some(&day)).unwrap().len(), 1);
    // Restart: the exact section is found and adopted.
    notion.set(|inner| {
        inner.inspect = InspectionOutcome::Matching {
            receipt_id: "synthetic-month".into(),
            returned_ids: vec!["synthetic-heading".into()],
        };
    });
    service.reconcile_notion_intents().unwrap();
    assert!(store.unresolved_intents(Some(&day)).unwrap().is_empty());
    let done = service.load_workflow(&day).unwrap();
    assert_eq!(done.state().as_str(), "completed");
    assert_eq!(notion_status(&done), "verified");
    assert_eq!(notion.writes(), 1);
    assert!(
        store
            .receipts(&day)
            .unwrap()
            .iter()
            .any(|receipt| receipt.destination == "notion")
    );
}

#[test]
fn no_effect_and_cancellation_stay_pending_and_retry_reuses_the_approval() {
    let (_scratch, _store, notion, service) = setup();
    let day = DiaryDay::parse("2026-10-30").unwrap();
    partial(&service, &day);
    let approved = approved(&service, &day);
    let cancelled = service
        .publish_notion_guarded(&day, approved.revision(), &AtomicBool::new(true))
        .unwrap();
    assert_eq!(notion_status(&cancelled), "pending");
    notion.set(|inner| inner.mode = Mode::NoEffect);
    let deferred = service.publish_notion(&day, cancelled.revision()).unwrap();
    assert_eq!(notion_status(&deferred), "pending");
    assert_eq!(notion.writes(), 1);
    // Same complete plan digest: no new preview or approval is needed.
    notion.set(|inner| inner.mode = Mode::Verify);
    let done = service.publish_notion(&day, deferred.revision()).unwrap();
    assert_eq!(notion_status(&done), "verified");
    assert_eq!(done.state().as_str(), "completed");
}

#[test]
fn conflicting_remote_stops_only_notion_and_keeps_local_verified() {
    let (_scratch, _store, notion, service) = setup();
    let day = DiaryDay::parse("2026-10-30").unwrap();
    partial(&service, &day);
    let approved = approved(&service, &day);
    notion.set(|inner| inner.mode = Mode::Conflict);
    let conflicted = service.publish_notion(&day, approved.revision()).unwrap();
    assert_eq!(notion_status(&conflicted), "changed");
    assert_eq!(
        conflicted.destination_progress()[0].status.as_str(),
        "verified"
    );
    // A changed remote outcome must leave the day open to a fresh preview.
    notion.set(|inner| inner.mode = Mode::Verify);
    let refreshed = service.preview_notion(&day, conflicted.revision()).unwrap();
    assert!(refreshed.plan_digest.is_some());
    assert_eq!(refreshed.status, DestinationStatus::Pending);
}

#[test]
fn real_api_worker_runs_the_notion_only_flow_through_restart() {
    let (scratch, store, notion, service) = setup();
    let day = DiaryDay::parse("2026-10-30").unwrap();
    let partial = partial(&service, &day);
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
        instance_id: "m10".to_owned(),
        control_token: "m10-token".to_owned(),
        port,
        commands: service,
        apple_helper_available: false,
        notion_configured: true,
        assistant_registry: Arc::new(DraftAssistantRegistry::new(Vec::new(), None).unwrap()),
        diagnostics: None,
        diagnostic_failure: None,
    };
    let server = runtime.spawn(diary_control::serve(listener, config, async {
        std::future::pending::<()>().await
    }));
    let worker = Worker::spawn(
        UreqTransport::new(),
        Attachment::new(
            "m10".to_owned(),
            "127.0.0.1".to_owned(),
            port,
            "m10-token".to_owned(),
            diary_control_contract::CONTRACT_VERSION,
        ),
    );
    let id = "wf_20261030".to_owned();
    worker
        .try_send(Intent::PreviewRemote {
            workflow_id: id.clone(),
            revision: partial.revision(),
            remote: "notion".to_owned(),
            existing_mode: None,
        })
        .unwrap();
    let preview = match wait_result(&worker) {
        WorkerResult::RemotePreviewed(outcome) => match *outcome {
            ClientOutcome::Ok(preview) => preview,
            _ => panic!("preview failed"),
        },
        _ => panic!("wrong worker result"),
    };
    assert_eq!(preview.status, "not-pushed");
    assert_eq!(preview.target, ["Notion root", "2026", "2026-10"]);
    assert_eq!(preview.blocks[0].kind, "heading_2");
    // Push is the approval: one request acts on exactly the previewed plan.
    worker
        .try_send(Intent::PushRemote {
            workflow_id: id.clone(),
            revision: preview.workflow.revision,
            remote: "notion".to_owned(),
            plan_digest: preview.plan_digest.clone().unwrap(),
            existing_mode: None,
        })
        .unwrap();
    let job_id = match wait_result(&worker) {
        WorkerResult::RemotePushed(ClientOutcome::Ok(job)) => job.job_id,
        _ => panic!("job not accepted"),
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
            WorkerResult::JobStatus(ClientOutcome::Ok(_)) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            _ => panic!("notion job failed"),
        }
    }
    worker
        .try_send(Intent::GetWorkflow {
            workflow_id: id.clone(),
        })
        .unwrap();
    let completed_revision = match wait_result(&worker) {
        WorkerResult::WorkflowFetched(outcome) => match *outcome {
            ClientOutcome::Ok(workflow) => {
                assert_eq!(workflow.state, "completed");
                assert_eq!(workflow.notion_status.as_deref(), Some("verified"));
                workflow.revision
            }
            _ => panic!("reload failed"),
        },
        _ => panic!("wrong worker result"),
    };
    notion.set(|inner| {
        inner.status = NotionPreflightStatus::Existing;
        inner.reason = Some("notion-same-day-conflict");
    });
    worker
        .try_send(Intent::PreviewRemote {
            workflow_id: id.clone(),
            revision: completed_revision,
            remote: "notion".to_owned(),
            existing_mode: None,
        })
        .unwrap();
    match wait_result(&worker) {
        WorkerResult::RemotePreviewed(outcome) => match *outcome {
            ClientOutcome::Ok(preview) => {
                assert_eq!(preview.status, "blocked");
                assert!(preview.plan_digest.is_none());
                assert_eq!(
                    preview.page_url.as_deref(),
                    Some("https://www.notion.so/0123456789abcdef0123456789abcdef")
                );
            }
            _ => panic!("blocked preview failed"),
        },
        _ => panic!("wrong worker result"),
    }
    notion.set(|inner| {
        inner.status = NotionPreflightStatus::Ready;
        inner.reason = None;
    });
    worker
        .try_send(Intent::CheckRemote {
            workflow_id: id.clone(),
            revision: completed_revision,
            remote: "notion".to_owned(),
        })
        .unwrap();
    match wait_result(&worker) {
        WorkerResult::RemoteChecked(outcome) => match *outcome {
            ClientOutcome::Ok(report) => assert_eq!(report.readiness.readiness, "ready"),
            _ => panic!("readiness failed"),
        },
        _ => panic!("wrong worker result"),
    }
    worker
        .try_send(Intent::PreviewRemote {
            workflow_id: id.clone(),
            revision: completed_revision,
            remote: "notion".to_owned(),
            existing_mode: None,
        })
        .unwrap();
    let repaired = match wait_result(&worker) {
        WorkerResult::RemotePreviewed(outcome) => match *outcome {
            ClientOutcome::Ok(preview) => preview,
            _ => panic!("repaired preview failed"),
        },
        _ => panic!("wrong worker result"),
    };
    assert_eq!(repaired.status, "not-pushed");
    let stale_digest = repaired.plan_digest.clone().unwrap();

    // The remote moves after the preview. The asynchronous job exposes the
    // v9 precondition token, and another preview binds a fresh plan instead of
    // leaving the day in a conflict/stall state.
    notion.set(|inner| inner.observed = b"remote-changed");
    worker
        .try_send(Intent::PushRemote {
            workflow_id: id.clone(),
            revision: repaired.workflow.revision,
            remote: "notion".to_owned(),
            plan_digest: stale_digest.clone(),
            existing_mode: None,
        })
        .unwrap();
    let job_id = match wait_result(&worker) {
        WorkerResult::RemotePushed(ClientOutcome::Ok(job)) => job.job_id,
        _ => panic!("changed-remote job not accepted"),
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
                if job.status == diary_control_contract::job_status::FAILED =>
            {
                assert_eq!(job.error_code.as_deref(), Some("precondition-failed"));
                break;
            }
            WorkerResult::JobStatus(ClientOutcome::Ok(_)) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            _ => panic!("changed-remote job did not fail safely"),
        }
    }
    worker
        .try_send(Intent::PreviewRemote {
            workflow_id: id.clone(),
            revision: repaired.workflow.revision,
            remote: "notion".to_owned(),
            existing_mode: None,
        })
        .unwrap();
    match wait_result(&worker) {
        WorkerResult::RemotePreviewed(outcome) => match *outcome {
            ClientOutcome::Ok(preview) => {
                assert_eq!(preview.status, "not-pushed");
                assert_ne!(preview.plan_digest.as_deref(), Some(stale_digest.as_str()));
            }
            _ => panic!("changed-remote refresh failed"),
        },
        _ => panic!("wrong worker result"),
    }
    drop(worker);
    runtime.block_on(async {
        server.abort();
        let _ = server.await;
    });
    drop(store);
    // Restart: the completed workflow and its Notion receipt survive reopen.
    let reopened = SqliteStore::open(scratch.0.join("state.sqlite3")).unwrap();
    assert!(
        reopened
            .receipts(&day)
            .unwrap()
            .iter()
            .any(|receipt| receipt.destination == "notion")
    );
}
