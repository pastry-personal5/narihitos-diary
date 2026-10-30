#![forbid(unsafe_code)]

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use diary_connectors::{LocalOutput, SqliteStore};
use diary_control::{CommandError, CommandService};
use diary_core::{
    CatalogRequest, Clock, DestinationStatus, DiaryDay, Digest, FreshnessOutcome,
    InspectionOutcome, LocalOutputError, LocalOutputPort, LocalOutputState, NotionBlockPreview,
    NotionOutputPort, NotionPreflight, NotionPreflightStatus, NotionRequest, OperationLog,
    SourceAccessError, SourceCatalog, SourceDiscovery, SourceReadOutcome, SourceReader, SourceRef,
    SourceSnapshot, StartOutcome, Timestamp, Workflow, WriteOutcome, WritingLanguage, WritingStyle,
};

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
                "nd-p1-m14-{}-{}",
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

struct SyntheticNotion;

impl NotionOutputPort for SyntheticNotion {
    fn preflight(&self, day: &DiaryDay, markdown: &str) -> Result<NotionPreflight, String> {
        Ok(NotionPreflight {
            target_identity: "synthetic-root".into(),
            root_label: "Notion root".into(),
            year_title: day.as_str()[..4].to_owned(),
            month_title: day.as_str()[..7].to_owned(),
            year_exists: false,
            month_exists: false,
            renderer_version: 1,
            api_version: "2026-03-11".into(),
            payload_digest: Digest::of_bytes(markdown.as_bytes()),
            observed_state_digest: Digest::of_bytes(b"synthetic-empty"),
            blocks: vec![NotionBlockPreview {
                kind: "heading_2".into(),
                text: format!("{} (FRI)", day.as_str()),
                depth: 0,
            }],
            status: NotionPreflightStatus::Ready,
            reason: None,
        })
    }

    fn write(&self, request: &NotionRequest, _: &AtomicBool) -> WriteOutcome {
        if request.expected_payload_digest == Digest::of_bytes(request.markdown.as_bytes()) {
            WriteOutcome::Verified {
                receipt_id: "synthetic-month".into(),
                returned_ids: vec!["synthetic-heading".into()],
            }
        } else {
            WriteOutcome::Conflict
        }
    }

    fn inspect(&self, _: &NotionRequest) -> InspectionOutcome {
        InspectionOutcome::Matching {
            receipt_id: "synthetic-month".into(),
            returned_ids: vec!["synthetic-heading".into()],
        }
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

struct Fixture {
    _scratch: Scratch,
    store: Arc<SqliteStore>,
    local: Arc<LocalOutput>,
    home: PathBuf,
    mirror_root: PathBuf,
    day: DiaryDay,
}

impl Fixture {
    fn new() -> Self {
        let scratch = Scratch::new();
        let home = scratch.0.join("home");
        let mirror_root = home.join("Library/Mobile Documents/com~apple~CloudDocs/entries");
        fs::create_dir_all(&mirror_root).unwrap();
        let store = Arc::new(SqliteStore::open(scratch.0.join("state.sqlite3")).unwrap());
        let local = Arc::new(LocalOutput::new(scratch.0.join("entries")).unwrap());
        Self {
            _scratch: scratch,
            store,
            local,
            home,
            mirror_root,
            day: DiaryDay::parse("2026-10-30").unwrap(),
        }
    }

    fn mirror(&self) -> Arc<LocalOutput> {
        Arc::new(LocalOutput::new_icloud(self.mirror_root.clone(), &self.home).unwrap())
    }

    fn base(&self) -> CommandService {
        CommandService::new(
            self.store.clone(),
            Arc::new(EmptySources),
            Arc::new(EmptySources),
            Arc::new(FixedClock),
        )
        .with_local_output(self.local.clone())
        .with_notion_output(Arc::new(SyntheticNotion))
    }

    fn with_mirror(&self) -> CommandService {
        self.base().with_mirror_output(self.mirror())
    }
}

fn status_of(workflow: &Workflow, destination: &str) -> DestinationStatus {
    workflow
        .destination_progress()
        .iter()
        .find(|item| item.destination == destination)
        .unwrap()
        .status
}

/// Previews, approves, and publishes the local entry, returning the workflow.
fn publish(service: &CommandService, day: &DiaryDay) -> Workflow {
    let confirmed = confirmed(service, day);
    let preview = service.preview_local(day, confirmed.revision()).unwrap();
    let approved = service
        .approve_local(
            day,
            preview.workflow.revision(),
            &preview.workflow.output_plan().unwrap().plan_digest,
        )
        .unwrap();
    service.publish_local(day, approved.revision()).unwrap()
}

fn publish_notion(service: &CommandService, day: &DiaryDay, workflow: &Workflow) -> Workflow {
    let notion = service.preview_notion(day, workflow.revision()).unwrap();
    let approved = service
        .approve_notion(
            day,
            notion.workflow.revision(),
            notion.plan_digest.as_ref().unwrap(),
        )
        .unwrap();
    service.publish_notion(day, approved.revision()).unwrap()
}

#[test]
fn mirror_is_byte_identical_and_day_completes_without_extra_approval() {
    let fx = Fixture::new();
    let service = fx.with_mirror();
    let published = publish(&service, &fx.day);
    assert_eq!(status_of(&published, "local"), DestinationStatus::Verified);
    assert_eq!(status_of(&published, "icloud"), DestinationStatus::Verified);
    let mirrored = fs::read(fx.mirror().target(&fx.day)).unwrap();
    assert_eq!(mirrored, fs::read(fx.local.target(&fx.day)).unwrap());
    let completed = publish_notion(&service, &fx.day, &published);
    assert_eq!(completed.state().as_str(), "completed");
    assert!(
        fx.store
            .receipts(&fx.day)
            .unwrap()
            .iter()
            .any(|receipt| receipt.destination == "icloud")
    );
}

#[test]
fn no_mirror_configured_leaves_plan_and_behavior_unchanged() {
    let fx = Fixture::new();
    let service = fx.base();
    let confirmed = confirmed(&service, &fx.day);
    let preview = service
        .preview_local(&fx.day, confirmed.revision())
        .unwrap();
    assert!(preview.mirror_target.is_none());
    assert_eq!(preview.workflow.output_plan().unwrap().targets.len(), 2);
    assert_eq!(preview.workflow.destination_progress().len(), 2);
    assert!(!fx.mirror().target(&fx.day).exists());
}

#[test]
fn missing_icloud_root_blocks_mirror_but_local_publishes_day_partial() {
    let fx = Fixture::new();
    let service = fx.with_mirror();
    fs::remove_dir_all(&fx.mirror_root).unwrap();
    let published = publish(&service, &fx.day);
    assert_eq!(published.state().as_str(), "partial");
    assert_eq!(status_of(&published, "local"), DestinationStatus::Verified);
    assert_eq!(status_of(&published, "icloud"), DestinationStatus::Blocked);
    assert!(fx.local.target(&fx.day).exists());
    assert!(!fx.mirror_root.exists());
    assert!(
        fx.store
            .unresolved_intents(Some(&fx.day))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn evicted_placeholder_blocks_mirror() {
    let fx = Fixture::new();
    let service = fx.with_mirror();
    let target = fx.mirror().target(&fx.day);
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::write(target.parent().unwrap().join(".2026-10-30.md.icloud"), "").unwrap();
    let published = publish(&service, &fx.day);
    assert_eq!(status_of(&published, "local"), DestinationStatus::Verified);
    assert_eq!(status_of(&published, "icloud"), DestinationStatus::Blocked);
    assert!(!target.exists());
}

#[test]
fn different_mirror_content_is_conflict_and_local_notion_unaffected() {
    let fx = Fixture::new();
    let service = fx.with_mirror();
    let target = fx.mirror().target(&fx.day);
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::write(&target, "someone else's file").unwrap();
    let published = publish(&service, &fx.day);
    assert_eq!(status_of(&published, "local"), DestinationStatus::Verified);
    assert_eq!(status_of(&published, "icloud"), DestinationStatus::Conflict);
    assert_eq!(fs::read_to_string(&target).unwrap(), "someone else's file");
    let after_notion = publish_notion(&service, &fx.day, &published);
    assert_eq!(
        status_of(&after_notion, "notion"),
        DestinationStatus::Verified
    );
    assert_eq!(
        status_of(&after_notion, "icloud"),
        DestinationStatus::Conflict
    );
    assert_eq!(after_notion.state().as_str(), "partial");
}

#[test]
fn mirror_failure_does_not_block_notion_preview_and_publish() {
    let fx = Fixture::new();
    let service = fx.with_mirror();
    fs::remove_dir_all(&fx.mirror_root).unwrap();
    let published = publish(&service, &fx.day);
    let after_notion = publish_notion(&service, &fx.day, &published);
    assert_eq!(
        status_of(&after_notion, "notion"),
        DestinationStatus::Verified
    );
    assert_eq!(
        status_of(&after_notion, "icloud"),
        DestinationStatus::Blocked
    );
    assert_eq!(
        status_of(&after_notion, "local"),
        DestinationStatus::Verified
    );
}

/// Writes the file, then reports `Absent` as a crash between write and
/// read-back would. Counts writes to prove recovery never writes again.
struct AmbiguousMirror {
    inner: LocalOutput,
    writes: AtomicU64,
}
impl LocalOutputPort for AmbiguousMirror {
    fn target(&self, day: &DiaryDay) -> String {
        self.inner.target(day).to_string_lossy().into_owned()
    }
    fn inspect(&self, day: &DiaryDay, payload: &str) -> Result<LocalOutputState, LocalOutputError> {
        self.inner.inspect(day, payload)
    }
    fn write(&self, day: &DiaryDay, payload: &str) -> Result<LocalOutputState, LocalOutputError> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        self.inner.write(day, payload)?;
        Ok(LocalOutputState::Absent)
    }
}

#[test]
fn restart_after_ambiguous_mirror_write_reconciles_to_verified() {
    let fx = Fixture::new();
    let ambiguous = Arc::new(AmbiguousMirror {
        inner: LocalOutput::new_icloud(fx.mirror_root.clone(), &fx.home).unwrap(),
        writes: AtomicU64::new(0),
    });
    let service = fx.base().with_mirror_output(ambiguous.clone());
    let published = publish(&service, &fx.day);
    assert_eq!(status_of(&published, "local"), DestinationStatus::Verified);
    assert_eq!(status_of(&published, "icloud"), DestinationStatus::Pending);
    assert_eq!(fx.store.unresolved_intents(Some(&fx.day)).unwrap().len(), 1);
    assert_eq!(ambiguous.writes.load(Ordering::SeqCst), 1);

    // Restart: a fresh service over the same store reconciles by inspection.
    let restarted = fx.with_mirror();
    restarted.reconcile_mirror_intents().unwrap();
    assert!(
        fx.store
            .unresolved_intents(Some(&fx.day))
            .unwrap()
            .is_empty()
    );
    let workflow = restarted.load_workflow(&fx.day).unwrap();
    assert_eq!(status_of(&workflow, "icloud"), DestinationStatus::Verified);
    assert_eq!(ambiguous.writes.load(Ordering::SeqCst), 1);
    assert_eq!(
        fs::read(fx.mirror().target(&fx.day)).unwrap(),
        fs::read(fx.local.target(&fx.day)).unwrap()
    );
}

#[test]
fn mirror_retry_after_blocked_completes_day() {
    let fx = Fixture::new();
    let service = fx.with_mirror();
    fs::remove_dir_all(&fx.mirror_root).unwrap();
    let published = publish(&service, &fx.day);
    assert_eq!(status_of(&published, "icloud"), DestinationStatus::Blocked);
    fs::create_dir_all(&fx.mirror_root).unwrap();
    let retried = service
        .publish_mirror_guarded(&fx.day, published.revision(), || true)
        .unwrap();
    assert_eq!(status_of(&retried, "icloud"), DestinationStatus::Verified);
    let completed = publish_notion(&service, &fx.day, &retried);
    assert_eq!(completed.state().as_str(), "completed");
    assert_eq!(
        fs::read(fx.mirror().target(&fx.day)).unwrap(),
        fs::read(fx.local.target(&fx.day)).unwrap()
    );
}

#[test]
fn config_change_between_preview_and_publish_forces_new_preview() {
    let fx = Fixture::new();
    // Mirror removed after preview.
    let with = fx.with_mirror();
    let confirmed_with = confirmed(&with, &fx.day);
    let preview = with
        .preview_local(&fx.day, confirmed_with.revision())
        .unwrap();
    let digest = preview.workflow.output_plan().unwrap().plan_digest.clone();
    let without = fx.base();
    assert!(matches!(
        without.approve_local(&fx.day, preview.workflow.revision(), &digest),
        Err(CommandError::PreconditionFailed)
    ));
    // Approved with the mirror, then removed before publish.
    let approved = with
        .approve_local(&fx.day, preview.workflow.revision(), &digest)
        .unwrap();
    assert!(matches!(
        without.publish_local(&fx.day, approved.revision()),
        Err(CommandError::PreconditionFailed)
    ));
    assert!(!fx.local.target(&fx.day).exists());
}

#[test]
fn mirror_added_after_preview_forces_new_preview() {
    let fx = Fixture::new();
    let without = fx.base();
    let confirmed = confirmed(&without, &fx.day);
    let preview = without
        .preview_local(&fx.day, confirmed.revision())
        .unwrap();
    let digest = preview.workflow.output_plan().unwrap().plan_digest.clone();
    let with = fx.with_mirror();
    assert!(matches!(
        with.approve_local(&fx.day, preview.workflow.revision(), &digest),
        Err(CommandError::PreconditionFailed)
    ));
}
