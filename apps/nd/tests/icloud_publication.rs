#![forbid(unsafe_code)]

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use diary_connectors::{LocalOutput, SqliteStore};
use diary_control::{CommandError, CommandService, ExistingEntryMode, RemoteStatus};
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
                "nd-icloud-publication-{}-{}",
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
            placement: "end".to_owned(),
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

fn mirror_file(fx: &Fixture) -> PathBuf {
    fx.mirror().target(&fx.day)
}

fn local_bytes(fx: &Fixture) -> Vec<u8> {
    fs::read(fx.local.target(&fx.day)).unwrap()
}

fn icloud_status(service: &CommandService, day: &DiaryDay) -> RemoteStatus {
    service
        .remotes(day)
        .unwrap()
        .into_iter()
        .find(|remote| remote.id == "icloud")
        .unwrap()
        .status
}

fn push(
    service: &CommandService,
    day: &DiaryDay,
    mode: ExistingEntryMode,
) -> Result<DestinationStatus, CommandError> {
    let preview = service.preview_icloud(day, mode).unwrap();
    service.push_icloud(day, mode, &preview.plan_digest, || true)
}

#[test]
fn local_publication_completes_the_day_and_never_touches_icloud() {
    let fx = Fixture::new();
    let service = fx.with_mirror();
    let published = publish(&service, &fx.day);
    assert_eq!(published.state().as_str(), "completed");
    assert_eq!(status_of(&published, "local"), DestinationStatus::Verified);
    assert!(!mirror_file(&fx).exists());
    assert!(
        published
            .destination_progress()
            .iter()
            .all(|item| item.destination != "icloud")
    );
    assert!(
        fx.store
            .receipts(&fx.day)
            .unwrap()
            .iter()
            .all(|receipt| receipt.destination != "icloud")
    );
    // The plan holds the local file only (plus Notion's blocked placeholder).
    assert_eq!(published.output_plan().unwrap().targets.len(), 2);
}

#[test]
fn push_writes_the_local_bytes_and_marks_the_remote_pushed() {
    let fx = Fixture::new();
    let service = fx.with_mirror();
    publish(&service, &fx.day);
    assert_eq!(icloud_status(&service, &fx.day), RemoteStatus::NotPushed);
    let preview = service
        .preview_icloud(&fx.day, ExistingEntryMode::Refuse)
        .unwrap();
    assert_eq!(preview.status, RemoteStatus::NotPushed);
    assert!(preview.existing_text.is_none());
    let status = service
        .push_icloud(
            &fx.day,
            ExistingEntryMode::Refuse,
            &preview.plan_digest,
            || true,
        )
        .unwrap();
    assert_eq!(status, DestinationStatus::Verified);
    assert_eq!(fs::read(mirror_file(&fx)).unwrap(), local_bytes(&fx));
    assert_eq!(icloud_status(&service, &fx.day), RemoteStatus::Pushed);
    assert!(
        fx.store
            .receipts(&fx.day)
            .unwrap()
            .iter()
            .any(|receipt| receipt.destination == "icloud")
    );
    // Pushing again changes nothing.
    let receipts = fx.store.receipts(&fx.day).unwrap().len();
    assert_eq!(
        push(&service, &fx.day, ExistingEntryMode::Refuse).unwrap(),
        DestinationStatus::Verified
    );
    assert_eq!(fx.store.receipts(&fx.day).unwrap().len(), receipts);
}

#[test]
fn a_missing_icloud_folder_blocks_only_the_remote() {
    let fx = Fixture::new();
    let service = fx.with_mirror();
    publish(&service, &fx.day);
    fs::remove_dir_all(&fx.mirror_root).unwrap();
    let preview = service
        .preview_icloud(&fx.day, ExistingEntryMode::Refuse)
        .unwrap();
    assert_eq!(preview.status, RemoteStatus::Blocked);
    assert_eq!(
        push(&service, &fx.day, ExistingEntryMode::Refuse).unwrap(),
        DestinationStatus::Blocked
    );
    assert!(fx.local.target(&fx.day).exists());
    assert!(!fx.mirror_root.exists());
    assert_eq!(
        service.load_workflow(&fx.day).unwrap().state().as_str(),
        "completed"
    );
}

#[test]
fn an_evicted_placeholder_blocks_the_remote() {
    let fx = Fixture::new();
    let service = fx.with_mirror();
    publish(&service, &fx.day);
    let target = mirror_file(&fx);
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::write(target.parent().unwrap().join(".2026-10-30.md.icloud"), "").unwrap();
    assert_eq!(icloud_status(&service, &fx.day), RemoteStatus::Blocked);
    assert!(!target.exists());
}

#[test]
fn existing_content_is_shown_and_only_overwrite_can_resolve_it() {
    let fx = Fixture::new();
    let service = fx.with_mirror();
    publish(&service, &fx.day);
    let target = mirror_file(&fx);
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::write(&target, "an entry written elsewhere\n").unwrap();

    // Shown without asking; not a dead end, and nothing is pushable until a
    // mode is chosen.
    let shown = service
        .preview_icloud(&fx.day, ExistingEntryMode::Refuse)
        .unwrap();
    assert_eq!(shown.status, RemoteStatus::Exists);
    assert_eq!(
        shown.existing_text.as_deref(),
        Some("an entry written elsewhere\n")
    );
    assert!(matches!(
        service.push_icloud(
            &fx.day,
            ExistingEntryMode::Refuse,
            &shown.plan_digest,
            || true
        ),
        Err(CommandError::PreconditionFailed)
    ));
    assert_eq!(
        fs::read_to_string(&target).unwrap(),
        "an entry written elsewhere\n"
    );

    assert!(matches!(
        service.preview_icloud(&fx.day, ExistingEntryMode::Append),
        Err(CommandError::PreconditionFailed)
    ));
    assert_eq!(
        fs::read_to_string(&target).unwrap(),
        "an entry written elsewhere\n"
    );
}

#[test]
fn overwrite_backs_up_and_replaces_with_the_local_bytes() {
    let fx = Fixture::new();
    let service = fx.with_mirror();
    publish(&service, &fx.day);
    let target = mirror_file(&fx);
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::write(&target, "an entry written elsewhere\n").unwrap();
    assert_eq!(
        push(&service, &fx.day, ExistingEntryMode::Overwrite).unwrap(),
        DestinationStatus::Verified
    );
    assert_eq!(fs::read(&target).unwrap(), local_bytes(&fx));
    let backed_up = fs::read_dir(target.parent().unwrap())
        .unwrap()
        .filter_map(Result::ok)
        .any(|entry| entry.file_name().to_string_lossy().contains(".bak-"));
    assert!(backed_up, "the replaced file keeps a backup");
}

#[test]
fn a_folder_that_changed_since_the_preview_refuses_the_push() {
    let fx = Fixture::new();
    let service = fx.with_mirror();
    publish(&service, &fx.day);
    let target = mirror_file(&fx);
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::write(&target, "first version\n").unwrap();
    let preview = service
        .preview_icloud(&fx.day, ExistingEntryMode::Overwrite)
        .unwrap();
    fs::write(&target, "edited after the preview\n").unwrap();
    assert!(matches!(
        service.push_icloud(
            &fx.day,
            ExistingEntryMode::Overwrite,
            &preview.plan_digest,
            || true
        ),
        Err(CommandError::PreconditionFailed)
    ));
    assert_eq!(
        fs::read_to_string(&target).unwrap(),
        "edited after the preview\n"
    );
}

#[test]
fn icloud_and_notion_are_pushed_independently() {
    let fx = Fixture::new();
    let service = fx.with_mirror();
    let published = publish(&service, &fx.day);
    let done = publish_notion(&service, &fx.day, &published);
    assert_eq!(status_of(&done, "notion"), DestinationStatus::Verified);
    assert_eq!(icloud_status(&service, &fx.day), RemoteStatus::NotPushed);
    assert!(!mirror_file(&fx).exists());
    assert_eq!(
        push(&service, &fx.day, ExistingEntryMode::Refuse).unwrap(),
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
fn restart_after_an_ambiguous_push_reconciles_without_writing_again() {
    let fx = Fixture::new();
    let ambiguous = Arc::new(AmbiguousMirror {
        inner: LocalOutput::new_icloud(fx.mirror_root.clone(), &fx.home).unwrap(),
        writes: AtomicU64::new(0),
    });
    let service = fx.base().with_mirror_output(ambiguous.clone());
    publish(&service, &fx.day);
    assert_eq!(
        push(&service, &fx.day, ExistingEntryMode::Refuse).unwrap(),
        DestinationStatus::Pending
    );
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
    assert_eq!(ambiguous.writes.load(Ordering::SeqCst), 1);
    assert_eq!(fs::read(mirror_file(&fx)).unwrap(), local_bytes(&fx));
    assert_eq!(icloud_status(&restarted, &fx.day), RemoteStatus::Pushed);
}
