#![forbid(unsafe_code)]

use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use diary_connectors::{LocalOutput, SqliteStore};
use diary_control::{
    CommandError, CommandService, DestinationReadiness, DiagnosticLevel, DiagnosticLog, LogClock,
    Readiness, ServerConfig,
};
use diary_core::{
    CatalogRequest, Clock, DiaryDay, Digest, DraftAssistantRegistry, FreshnessOutcome,
    InspectionOutcome, LocalOutputError, LocalOutputPort, LocalOutputState, NotionOutputPort,
    NotionPreflight, NotionPreflightStatus, NotionRequest, OperationLog, SourceAccessError,
    SourceCatalog, SourceDiscovery, SourceReadOutcome, SourceReader, SourceRef, SourceSnapshot,
    StartOutcome, Timestamp, Workflow, WriteOutcome, WritingLanguage, WritingStyle,
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
                "nd-readiness-{}-{}",
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

/// A Notion port whose preflight outcome the test sets.
struct Scripted(Mutex<(NotionPreflightStatus, Option<&'static str>)>);

impl Scripted {
    fn new() -> Arc<Self> {
        Arc::new(Self(Mutex::new((NotionPreflightStatus::Ready, None))))
    }
    fn set(&self, status: NotionPreflightStatus, reason: Option<&'static str>) {
        *self.0.lock().unwrap() = (status, reason);
    }
}

impl NotionOutputPort for Scripted {
    fn preflight(&self, day: &DiaryDay, markdown: &str) -> Result<NotionPreflight, String> {
        let (status, reason) = *self.0.lock().unwrap();
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
            observed_state_digest: Digest::of_bytes(b"empty"),
            blocks: Vec::new(),
            status,
            reason: reason.map(str::to_owned),
        })
    }
    fn write(&self, _: &NotionRequest, _: &AtomicBool) -> WriteOutcome {
        panic!("a readiness check must never write");
    }
    fn inspect(&self, _: &NotionRequest) -> InspectionOutcome {
        InspectionOutcome::Absent
    }
}

/// A Notion port whose preflight fails with free text, as a connector might.
struct Failing;
impl NotionOutputPort for Failing {
    fn preflight(&self, _: &DiaryDay, _: &str) -> Result<NotionPreflight, String> {
        Err("page 7f3a9 not found at https://example.invalid".to_owned())
    }
    fn write(&self, _: &NotionRequest, _: &AtomicBool) -> WriteOutcome {
        panic!("a readiness check must never write");
    }
    fn inspect(&self, _: &NotionRequest) -> InspectionOutcome {
        InspectionOutcome::Absent
    }
}

/// A mirror whose folder cannot be inspected.
struct UnavailableMirror;
impl LocalOutputPort for UnavailableMirror {
    fn target(&self, _: &DiaryDay) -> String {
        "unavailable".to_owned()
    }
    fn inspect(&self, _: &DiaryDay, _: &str) -> Result<LocalOutputState, LocalOutputError> {
        Err(LocalOutputError::Unavailable)
    }
    fn write(&self, _: &DiaryDay, _: &str) -> Result<LocalOutputState, LocalOutputError> {
        panic!("a readiness check must never write");
    }
}

struct TickClock;
impl LogClock for TickClock {
    fn now_unix_ms(&self) -> u64 {
        1_793_360_000_000
    }
}

struct Fixture {
    _scratch: Scratch,
    store: Arc<SqliteStore>,
    day: DiaryDay,
}

impl Fixture {
    fn new() -> Self {
        let scratch = Scratch::new();
        let store = Arc::new(SqliteStore::open(scratch.0.join("state.sqlite3")).unwrap());
        Self {
            _scratch: scratch,
            store,
            day: DiaryDay::parse("2026-10-30").unwrap(),
        }
    }

    fn entries(&self) -> Arc<LocalOutput> {
        Arc::new(LocalOutput::new(self._scratch.0.join("entries")).unwrap())
    }

    fn mirror(&self) -> Arc<LocalOutput> {
        Arc::new(LocalOutput::new(self._scratch.0.join("icloud")).unwrap())
    }

    fn service(&self) -> CommandService {
        CommandService::new(
            self.store.clone(),
            Arc::new(EmptySources),
            Arc::new(EmptySources),
            Arc::new(FixedClock),
        )
        .with_local_output(self.entries())
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

fn of<'a>(
    report: &'a diary_control::PublishReadiness,
    destination: &str,
) -> &'a DestinationReadiness {
    report
        .destinations
        .iter()
        .find(|item| item.destination == destination)
        .unwrap()
}

#[test]
fn both_destinations_wait_for_the_local_file() {
    let fx = Fixture::new();
    let service = fx
        .service()
        .with_notion_output(Scripted::new())
        .with_mirror_output(fx.mirror());
    let confirmed = confirmed(&service, &fx.day);
    let report = service
        .check_publish_readiness(&fx.day, confirmed.revision())
        .unwrap();
    for destination in ["notion", "icloud"] {
        let item = of(&report, destination);
        assert_eq!(item.readiness, Readiness::NotReady);
        assert_eq!(item.reason, "local-file-not-verified");
        assert_eq!(item.next_step, "publish-local-first");
    }
}

#[test]
fn notion_ready_points_to_preview_and_approval_not_publication() {
    let fx = Fixture::new();
    let notion = Scripted::new();
    let service = fx.service().with_notion_output(notion);
    let partial = partial(&service, &fx.day);
    let report = service
        .check_publish_readiness(&fx.day, partial.revision())
        .unwrap();
    let item = of(&report, "notion");
    assert_eq!(item.readiness, Readiness::Ready);
    assert_eq!(item.reason, "none");
    assert_eq!(item.next_step, "preview-and-approve");
}

#[test]
fn notion_blocked_and_conflict_name_their_reason_and_next_step() {
    let fx = Fixture::new();
    let notion = Scripted::new();
    let service = fx.service().with_notion_output(notion.clone());
    let partial = partial(&service, &fx.day);
    let cases = [
        (
            NotionPreflightStatus::Blocked,
            "notion-permission-denied",
            Readiness::NotReady,
            "fix-notion-access",
        ),
        (
            NotionPreflightStatus::Blocked,
            "notion-hierarchy-ambiguous",
            Readiness::NotReady,
            "check-notion-page",
        ),
        (
            NotionPreflightStatus::Blocked,
            "notion-remote-unavailable",
            Readiness::NotReady,
            "retry-later",
        ),
        (
            NotionPreflightStatus::Conflict,
            "notion-same-day-conflict",
            Readiness::Conflict,
            "check-notion-page",
        ),
    ];
    for (status, reason, readiness, next_step) in cases {
        notion.set(status, Some(reason));
        let report = service
            .check_publish_readiness(&fx.day, partial.revision())
            .unwrap();
        let item = of(&report, "notion");
        assert_eq!(item.readiness, readiness, "{reason}");
        assert_eq!(item.reason, reason);
        assert_eq!(item.next_step, next_step, "{reason}");
    }
}

#[test]
fn unlisted_connector_text_never_reaches_the_response() {
    let fx = Fixture::new();
    let service = fx.service().with_notion_output(Arc::new(Failing));
    let partial = partial(&service, &fx.day);
    let report = service
        .check_publish_readiness(&fx.day, partial.revision())
        .unwrap();
    let item = of(&report, "notion");
    assert_eq!(item.readiness, Readiness::NotReady);
    assert_eq!(item.reason, "notion-unavailable");
    assert_eq!(item.next_step, "retry-later");
}

#[test]
fn unconfigured_destinations_say_so() {
    let fx = Fixture::new();
    let service = fx.service();
    let partial = partial(&service, &fx.day);
    let report = service
        .check_publish_readiness(&fx.day, partial.revision())
        .unwrap();
    let notion = of(&report, "notion");
    assert_eq!(notion.reason, "notion-not-configured");
    assert_eq!(notion.next_step, "configure-notion");
    let icloud = of(&report, "icloud");
    assert_eq!(icloud.reason, "icloud-not-configured");
    assert_eq!(icloud.next_step, "configure-icloud");
}

#[test]
fn icloud_reports_ready_different_file_and_unavailable_folder() {
    let fx = Fixture::new();
    let partial = partial(&fx.service(), &fx.day);

    let mirror = fx.mirror();
    let service = fx.service().with_mirror_output(mirror.clone());
    let report = service
        .check_publish_readiness(&fx.day, partial.revision())
        .unwrap();
    let item = of(&report, "icloud");
    assert_eq!(item.readiness, Readiness::Ready);
    assert_eq!(item.next_step, "preview-and-approve");

    let target = mirror.target(&fx.day);
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::write(&target, "someone else's text").unwrap();
    let report = service
        .check_publish_readiness(&fx.day, partial.revision())
        .unwrap();
    let item = of(&report, "icloud");
    // Existing iCloud content is not an obstacle: the Remote menu offers
    // Append and Overwrite for it.
    assert_eq!(item.readiness, Readiness::Ready);
    assert_eq!(item.next_step, "preview-and-approve");

    let service = fx.service().with_mirror_output(Arc::new(UnavailableMirror));
    let report = service
        .check_publish_readiness(&fx.day, partial.revision())
        .unwrap();
    let item = of(&report, "icloud");
    assert_eq!(item.readiness, Readiness::NotReady);
    assert_eq!(item.reason, "icloud-folder-unavailable");
    assert_eq!(item.next_step, "check-icloud-folder");
}

#[test]
fn the_check_is_read_only_and_refuses_a_stale_revision() {
    let fx = Fixture::new();
    let notion = Scripted::new();
    let service = fx
        .service()
        .with_notion_output(notion)
        .with_mirror_output(fx.mirror());
    let partial = partial(&service, &fx.day);
    let intents = fx.store.unresolved_intents(Some(&fx.day)).unwrap().len();
    let receipts = fx.store.receipts(&fx.day).unwrap().len();

    let report = service
        .check_publish_readiness(&fx.day, partial.revision())
        .unwrap();
    assert_eq!(report.revision, partial.revision());
    assert_eq!(
        service.load_workflow(&fx.day).unwrap().revision(),
        partial.revision()
    );
    assert_eq!(
        fx.store.unresolved_intents(Some(&fx.day)).unwrap().len(),
        intents
    );
    assert_eq!(fx.store.receipts(&fx.day).unwrap().len(), receipts);

    assert_eq!(
        service
            .check_publish_readiness(&fx.day, partial.revision() - 1)
            .unwrap_err(),
        CommandError::PreconditionFailed
    );
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
fn readiness_round_trips_through_the_real_server_without_changing_the_workflow() {
    let fx = Fixture::new();
    // Publish locally without a mirror, then serve a service that has one, so
    // the mirror is configured but not yet written.
    let partial = partial(&fx.service(), &fx.day);
    let service = fx
        .service()
        .with_notion_output(Scripted::new())
        .with_mirror_output(fx.mirror());
    let revision = partial.revision();
    let log_root = fx._scratch.0.join("logs");
    let uid = fs::metadata(&fx._scratch.0).unwrap().uid();
    let log = Arc::new(
        DiagnosticLog::open_with_min_level(
            &log_root,
            uid,
            Arc::new(TickClock),
            DiagnosticLevel::Debug,
        )
        .unwrap()
        .with_tag_key([9; 32]),
    );
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
        instance_id: "readiness".to_owned(),
        control_token: "readiness-token".to_owned(),
        port,
        commands: service,
        apple_helper_available: false,
        notion_configured: true,
        assistant_registry: Arc::new(DraftAssistantRegistry::new(Vec::new(), None).unwrap()),
        diagnostics: Some(Arc::clone(&log)),
        diagnostic_failure: None,
    };
    let server = runtime.spawn(diary_control::serve(listener, config, async {
        std::future::pending::<()>().await
    }));
    let worker = Worker::spawn(
        UreqTransport::new(),
        Attachment::new(
            "readiness".to_owned(),
            "127.0.0.1".to_owned(),
            port,
            "readiness-token".to_owned(),
            diary_control_contract::CONTRACT_VERSION,
        ),
    );
    let id = "wf_20261030".to_owned();

    for (remote, expected, next_step) in [
        ("notion", "ready", "preview-and-approve"),
        ("icloud", "ready", "preview-and-approve"),
    ] {
        worker
            .try_send(Intent::CheckRemote {
                workflow_id: id.clone(),
                revision,
                remote: remote.to_owned(),
            })
            .unwrap();
        let report = match wait_result(&worker) {
            WorkerResult::RemoteChecked(outcome) => match *outcome {
                ClientOutcome::Ok(report) => report,
                _ => panic!("readiness check failed"),
            },
            _ => panic!("wrong worker result"),
        };
        assert_eq!(report.revision, revision);
        assert_eq!(report.readiness.destination, remote);
        assert_eq!(report.readiness.readiness, expected, "{remote}");
        let _ = next_step;
    }

    worker
        .try_send(Intent::CheckRemote {
            workflow_id: id.clone(),
            revision: revision - 1,
            remote: "notion".to_owned(),
        })
        .unwrap();
    match wait_result(&worker) {
        WorkerResult::RemoteChecked(outcome) => {
            assert!(!matches!(*outcome, ClientOutcome::Ok(_)), "stale revision");
        }
        _ => panic!("wrong worker result"),
    }

    worker
        .try_send(Intent::GetWorkflow { workflow_id: id })
        .unwrap();
    match wait_result(&worker) {
        WorkerResult::WorkflowFetched(outcome) => match *outcome {
            ClientOutcome::Ok(workflow) => assert_eq!(workflow.revision, revision),
            _ => panic!("reload failed"),
        },
        _ => panic!("wrong worker result"),
    }
    drop(worker);
    runtime.block_on(async {
        server.abort();
        let _ = server.await;
    });

    // The route's records reach the log (a rejected record would be dropped
    // silently), and they carry closed tokens only: no day, path, or text.
    let text = fs::read_to_string(log_root.join("control-api.log")).unwrap();
    for expected in [
        "component=publish event=readiness decided_by=preflight-status destination=notion",
        "component=publish event=readiness decided_by=mirror-inspect destination=icloud",
        "component=publish event=readiness-check ",
        "local_verified=1 notion_configured=1",
        "component=publish event=readiness-refused reason=stale-revision",
    ] {
        assert!(
            text.contains(expected),
            "missing log record: {expected}\n{text}"
        );
    }
    assert!(!text.contains("2026-10-30"));
    assert!(!text.contains("quiet"));
}
