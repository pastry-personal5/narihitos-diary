#![forbid(unsafe_code)]

//! Amendment acceptance over the command layer: amendment of a completed day,
//! with invented content, fake sources, and a scripted Notion port.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use diary_connectors::{LocalOutput, SqliteStore};
use diary_control::{CommandError, CommandService};
use diary_core::WorkflowStore;
use diary_core::{
    CatalogRequest, Clock, DiaryDay, Digest, FreshnessOutcome, InspectionOutcome,
    NotionBlockPreview, NotionOutputPort, NotionPreflight, NotionPreflightStatus,
    NotionReplacePreflight, NotionReplaceRequest, NotionRequest, OperationIntent, OperationLog,
    SourceAccessError, SourceCatalog, SourceDiscovery, SourceReadOutcome, SourceReader, SourceRef,
    SourceSnapshot, StartOutcome, Timestamp, Workflow, WorkflowState, WriteOutcome,
    WritingLanguage, WritingStyle,
};

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// A clock the test can move; every command reads time through it.
struct TestClock(Arc<AtomicI64>);
impl Clock for TestClock {
    fn now(&self) -> Timestamp {
        Timestamp(self.0.load(Ordering::SeqCst))
    }
    fn local_day(&self) -> DiaryDay {
        DiaryDay::parse("2026-10-30").unwrap()
    }
}

/// Any source access fails: an amendment must never read sources.
struct NoSources;
impl SourceCatalog for NoSources {
    fn discover(&self, _: CatalogRequest) -> Result<SourceDiscovery, SourceAccessError> {
        Ok(SourceDiscovery {
            candidates: Vec::new(),
            diagnostics: Vec::new(),
        })
    }
}
static SOURCE_READS: AtomicU64 = AtomicU64::new(0);
impl SourceReader for NoSources {
    fn read_selected(&self, _: &SourceRef) -> Result<SourceReadOutcome, SourceAccessError> {
        SOURCE_READS.fetch_add(1, Ordering::SeqCst);
        Err(SourceAccessError::NotFound)
    }
    fn revalidate(&self, _: &SourceSnapshot) -> Result<FreshnessOutcome, SourceAccessError> {
        SOURCE_READS.fetch_add(1, Ordering::SeqCst);
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
                "nd-amendment-{}-{}",
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

#[derive(Clone, Copy, PartialEq)]
enum Replace {
    Verify,
    NoEffect,
    Ambiguous,
    Conflict,
}

struct Inner {
    preflight: NotionPreflightStatus,
    replace: Replace,
    owned_state: &'static [u8],
    reconcile: InspectionOutcome,
    replaced: Vec<NotionReplaceRequest>,
}

struct Scripted(Mutex<Inner>);
impl Scripted {
    fn new() -> Arc<Self> {
        Arc::new(Self(Mutex::new(Inner {
            preflight: NotionPreflightStatus::Ready,
            replace: Replace::Verify,
            owned_state: b"owned",
            reconcile: InspectionOutcome::Absent,
            replaced: Vec::new(),
        })))
    }
    fn set(&self, change: impl FnOnce(&mut Inner)) {
        change(&mut self.0.lock().unwrap());
    }
    fn replaced(&self) -> Vec<NotionReplaceRequest> {
        self.0.lock().unwrap().replaced.clone()
    }
}

fn blocks(text: &str) -> Vec<NotionBlockPreview> {
    vec![NotionBlockPreview {
        kind: "paragraph".into(),
        text: text.into(),
        depth: 0,
    }]
}

impl NotionOutputPort for Scripted {
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
            observed_state_digest: Digest::of_bytes(b"empty"),
            blocks: blocks(markdown),
            status: NotionPreflightStatus::Ready,
            reason: None,
        })
    }

    fn write(&self, _: &NotionRequest, _: &AtomicBool) -> WriteOutcome {
        WriteOutcome::Verified {
            receipt_id: "synthetic-month".into(),
            returned_ids: vec!["synthetic-heading".into(), "synthetic-body".into()],
        }
    }

    fn inspect(&self, _: &NotionRequest) -> InspectionOutcome {
        InspectionOutcome::Absent
    }

    fn preflight_replace(
        &self,
        request: &NotionReplaceRequest,
    ) -> Result<NotionReplacePreflight, String> {
        let inner = self.0.lock().unwrap();
        Ok(NotionReplacePreflight {
            preflight: NotionPreflight {
                target_identity: request.expected_target_identity.clone(),
                root_label: "Notion root".into(),
                year_title: "2026".into(),
                month_title: "2026-10".into(),
                year_exists: true,
                month_exists: true,
                renderer_version: 1,
                api_version: "2026-03-11".into(),
                payload_digest: Digest::of_bytes(request.markdown.as_bytes()),
                observed_state_digest: Digest::of_bytes(inner.owned_state),
                blocks: blocks(&request.markdown),
                status: inner.preflight,
                reason: (inner.preflight != NotionPreflightStatus::Ready)
                    .then(|| "notion-section-not-owned".to_owned()),
            },
            current_blocks: blocks("current notion body"),
        })
    }

    fn replace_section(&self, request: &NotionReplaceRequest, _: &AtomicBool) -> WriteOutcome {
        let mut inner = self.0.lock().unwrap();
        inner.replaced.push(request.clone());
        match inner.replace {
            Replace::Verify => WriteOutcome::Verified {
                receipt_id: request.owned_month_id.clone(),
                returned_ids: vec![request.owned_block_ids[0].clone(), "new-body".into()],
            },
            Replace::NoEffect => WriteOutcome::DefiniteNoEffect,
            Replace::Ambiguous => WriteOutcome::AmbiguousWrite,
            Replace::Conflict => WriteOutcome::Conflict,
        }
    }

    fn reconcile_replace(&self, request: &NotionReplaceRequest) -> InspectionOutcome {
        match self.0.lock().unwrap().reconcile.clone() {
            InspectionOutcome::Matching { .. } => InspectionOutcome::Matching {
                receipt_id: request.owned_month_id.clone(),
                returned_ids: vec![request.owned_block_ids[0].clone(), "new-body".into()],
            },
            other => other,
        }
    }
}

struct Fixture {
    _scratch: Scratch,
    root: PathBuf,
    store: Arc<SqliteStore>,
    notion: Arc<Scripted>,
    service: CommandService,
    clock: Arc<AtomicI64>,
    day: DiaryDay,
    mirror: Option<Arc<LocalOutput>>,
    mirror_root: PathBuf,
}

fn fixture() -> Fixture {
    build(false)
}

/// Same as [`fixture`] with an iCloud mirror configured beneath a fake home.
fn fixture_with_mirror() -> Fixture {
    build(true)
}

fn build(with_mirror: bool) -> Fixture {
    let scratch = Scratch::new();
    let store = Arc::new(SqliteStore::open(scratch.0.join("state.sqlite3")).unwrap());
    let root = scratch.0.join("entries");
    let notion = Scripted::new();
    let clock = Arc::new(AtomicI64::new(100));
    let home = scratch.0.join("home");
    let mirror_root = home.join("Library/Mobile Documents/com~apple~CloudDocs/entries");
    fs::create_dir_all(&mirror_root).unwrap();
    let mirror =
        with_mirror.then(|| Arc::new(LocalOutput::new_icloud(mirror_root.clone(), &home).unwrap()));
    let mut service = CommandService::new(
        store.clone(),
        Arc::new(NoSources),
        Arc::new(NoSources),
        Arc::new(TestClock(clock.clone())),
    )
    .with_local_output(Arc::new(LocalOutput::new(root.clone()).unwrap()))
    .with_notion_output(notion.clone());
    if let Some(mirror) = &mirror {
        service = service.with_mirror_output(mirror.clone());
    }
    Fixture {
        mirror,
        mirror_root,
        _scratch: scratch,
        root,
        store,
        notion,
        service,
        clock,
        day: DiaryDay::parse("2026-10-30").unwrap(),
    }
}

impl Fixture {
    fn file(&self) -> PathBuf {
        self.root.join("2026/10/2026-10-30.md")
    }

    fn mirror_file(&self) -> PathBuf {
        self.mirror.as_ref().unwrap().target(&self.day)
    }

    /// Publishes "Today was quiet." to the local file and to Notion.
    fn completed(&self) -> Workflow {
        let service = &self.service;
        let day = &self.day;
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
                WritingLanguage::EnUs,
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
        workflow = service
            .confirm_entry(day, workflow.revision(), workflow.entry().version)
            .unwrap();
        let local = service.preview_local(day, workflow.revision()).unwrap();
        let local = service
            .approve_local(
                day,
                local.workflow.revision(),
                &local.workflow.output_plan().unwrap().plan_digest,
            )
            .unwrap();
        let partial = service.publish_local(day, local.revision()).unwrap();
        let preview = service.preview_notion(day, partial.revision()).unwrap();
        let approved = service
            .approve_notion(
                day,
                preview.workflow.revision(),
                preview.plan_digest.as_ref().unwrap(),
            )
            .unwrap();
        let done = service.publish_notion(day, approved.revision()).unwrap();
        assert_eq!(done.state(), WorkflowState::Completed);
        done
    }

    /// Seeds, edits to `text`, confirms and previews an amendment.
    fn previewed(&self, completed: &Workflow, text: &str) -> (Workflow, Digest) {
        let seeded = self
            .service
            .seed_amendment(&self.day, completed.revision())
            .unwrap();
        let edited = self
            .service
            .save_entry(&self.day, seeded.workflow.entry().version, text.to_owned())
            .unwrap();
        let confirmed = self
            .service
            .confirm_entry(&self.day, edited.revision(), edited.entry().version)
            .unwrap();
        let preview = self
            .service
            .preview_amendment(&self.day, confirmed.revision())
            .unwrap();
        let digest = preview.workflow.output_plan().unwrap().plan_digest.clone();
        (preview.workflow, digest)
    }

    fn status(workflow: &Workflow, destination: &str) -> &'static str {
        workflow
            .destination_progress()
            .iter()
            .find(|item| item.destination == destination)
            .map_or("none", |item| item.status.as_str())
    }
}

#[test]
fn completed_day_amends_locally_and_in_notion_with_a_new_receipt_version() {
    let f = fixture();
    let completed = f.completed();
    let reads = SOURCE_READS.load(Ordering::SeqCst);

    let seeded = f
        .service
        .seed_amendment(&f.day, completed.revision())
        .unwrap();
    assert!(!seeded.hand_edited);
    assert_eq!(seeded.workflow.entry().text, "Today was quiet.");
    assert_eq!(seeded.workflow.state(), WorkflowState::Composing);
    let seed = seeded.workflow.amendment().unwrap();
    assert_eq!(seed.receipt_version, 2);
    let old_file = fs::read_to_string(f.file()).unwrap();
    assert_eq!(seed.file_digest, Digest::of_bytes(old_file.as_bytes()));

    let edited = f
        .service
        .save_entry(
            &f.day,
            seeded.workflow.entry().version,
            "Today was quiet, then it rained.".to_owned(),
        )
        .unwrap();
    let confirmed = f
        .service
        .confirm_entry(&f.day, edited.revision(), edited.entry().version)
        .unwrap();
    let preview = f
        .service
        .preview_amendment(&f.day, confirmed.revision())
        .unwrap();
    assert_eq!(preview.local_current, old_file);
    assert!(preview.local_payload.contains("then it rained"));
    assert_eq!(preview.local_status.as_str(), "pending");
    let notion = preview.notion.as_ref().unwrap();
    assert_eq!(notion.status.as_str(), "pending");
    assert_eq!(notion.current_blocks[0].text, "current notion body");
    // Nothing is written until approval and publication.
    assert_eq!(fs::read_to_string(f.file()).unwrap(), old_file);

    let digest = preview.workflow.output_plan().unwrap().plan_digest.clone();
    let approved = f
        .service
        .approve_amendment(&f.day, preview.workflow.revision(), &digest)
        .unwrap();
    let done = f
        .service
        .publish_amendment(&f.day, approved.revision(), &AtomicBool::new(false))
        .unwrap();
    assert_eq!(done.state(), WorkflowState::Completed);
    assert!(done.amendment().is_none());
    assert!(
        fs::read_to_string(f.file())
            .unwrap()
            .contains("then it rained")
    );
    // The replacement targeted exactly the receipted section.
    let replaced = f.notion.replaced();
    assert_eq!(replaced.len(), 1);
    assert_eq!(replaced[0].owned_month_id, "synthetic-month");
    assert_eq!(
        replaced[0].owned_block_ids,
        vec!["synthetic-heading".to_owned(), "synthetic-body".to_owned()]
    );
    // New receipt version at both destinations; the earlier receipts remain.
    let receipts = f.store.receipts(&f.day).unwrap();
    let versions = |destination: &str| {
        let mut found: Vec<_> = receipts
            .iter()
            .filter(|receipt| receipt.destination == destination)
            .map(|receipt| receipt.version)
            .collect();
        found.sort_unstable();
        found
    };
    assert_eq!(versions("local"), vec![1, 3]);
    assert_eq!(versions("notion"), vec![2, 3]);
    // The seven-day text retention restarts from this completion.
    assert_eq!(done.persisted().all_verified_at, Some(Timestamp(100)));
    assert_eq!(
        SOURCE_READS.load(Ordering::SeqCst),
        reads,
        "no source reads"
    );
}

#[test]
fn a_hand_edited_file_seeds_its_body_and_raises_the_banner_without_a_conflict() {
    let f = fixture();
    let completed = f.completed();
    fs::write(
        f.file(),
        "---\ndate: \"2026-10-30\"\nlanguage: \"en-US\"\n---\n\nHand edited line.\n",
    )
    .unwrap();
    let seeded = f
        .service
        .seed_amendment(&f.day, completed.revision())
        .unwrap();
    assert!(seeded.hand_edited);
    assert!(seeded.workflow.amendment().unwrap().hand_edited);
    assert_eq!(seeded.workflow.entry().text, "Hand edited line.");
    // A hand-edited front matter block is also flagged, and never offered.
    let other = fixture();
    let completed = other.completed();
    fs::write(
        other.file(),
        "---\ndate: \"2026-10-30\"\nlanguage: \"ko-KR\"\n---\n\nSame body.\n",
    )
    .unwrap();
    let seeded = other
        .service
        .seed_amendment(&other.day, completed.revision())
        .unwrap();
    assert!(seeded.hand_edited);
    assert_eq!(seeded.workflow.entry().text, "Same body.");
}

#[test]
fn a_change_after_preview_voids_approval_and_a_race_after_approval_leaves_the_file_alone() {
    let f = fixture();
    let completed = f.completed();
    let (previewed, digest) = f.previewed(&completed, "Changed my mind.");
    fs::write(f.file(), "edited after preview").unwrap();
    assert_eq!(
        f.service
            .approve_amendment(&f.day, previewed.revision(), &digest)
            .unwrap_err(),
        CommandError::PreconditionFailed
    );

    // Reread rebinds to the file as it is now, keeping the person's text.
    let reread = f
        .service
        .reread_amendment(&f.day, previewed.revision())
        .unwrap();
    assert!(reread.hand_edited);
    assert_eq!(reread.workflow.entry().text, "Changed my mind.");
    let confirmed = f
        .service
        .confirm_entry(
            &f.day,
            reread.workflow.revision(),
            reread.workflow.entry().version,
        )
        .unwrap();
    let preview = f
        .service
        .preview_amendment(&f.day, confirmed.revision())
        .unwrap();
    assert_eq!(preview.local_current, "edited after preview");
    let digest = preview.workflow.output_plan().unwrap().plan_digest.clone();
    let approved = f
        .service
        .approve_amendment(&f.day, preview.workflow.revision(), &digest)
        .unwrap();

    // A racing edit between approval and publication wins; nothing is written.
    fs::write(f.file(), "raced edit").unwrap();
    let outcome = f
        .service
        .publish_amendment(&f.day, approved.revision(), &AtomicBool::new(false))
        .unwrap();
    assert_eq!(outcome.state(), WorkflowState::Partial);
    assert_eq!(Fixture::status(&outcome, "local"), "conflict");
    assert_eq!(fs::read_to_string(f.file()).unwrap(), "raced edit");
    assert!(f.notion.replaced().is_empty(), "Notion waits for local");

    // The only choices are reread or abandon; abandon changes nothing.
    let receipts_before = f.store.receipts(&f.day).unwrap();
    let abandoned = f
        .service
        .abandon_amendment(&f.day, outcome.revision())
        .unwrap();
    assert_eq!(abandoned.state(), WorkflowState::Completed);
    assert!(abandoned.amendment().is_none());
    assert_eq!(fs::read_to_string(f.file()).unwrap(), "raced edit");
    assert_eq!(f.store.receipts(&f.day).unwrap(), receipts_before);
    // The day can be amended again afterward.
    assert!(
        f.service
            .seed_amendment(&f.day, abandoned.revision())
            .is_ok()
    );
}

#[test]
fn a_notion_failure_after_the_local_replace_leaves_partial_and_retries_notion_alone() {
    let f = fixture();
    let completed = f.completed();
    let (previewed, digest) = f.previewed(&completed, "Quiet, then rain.");
    let approved = f
        .service
        .approve_amendment(&f.day, previewed.revision(), &digest)
        .unwrap();
    f.notion.set(|inner| inner.replace = Replace::NoEffect);
    let partial = f
        .service
        .publish_amendment(&f.day, approved.revision(), &AtomicBool::new(false))
        .unwrap();
    assert_eq!(partial.state(), WorkflowState::Partial);
    assert_eq!(Fixture::status(&partial, "local"), "verified");
    assert_eq!(Fixture::status(&partial, "notion"), "pending");
    assert!(fs::read_to_string(f.file()).unwrap().contains("then rain"));
    assert!(partial.persisted().all_verified_at.is_none());

    // A changed Notion section voids the reused approval.
    f.notion.set(|inner| {
        inner.replace = Replace::Verify;
        inner.owned_state = b"edited in notion";
    });
    assert_eq!(
        f.service
            .retry_amendment_notion(&f.day, partial.revision(), &AtomicBool::new(false))
            .unwrap_err(),
        CommandError::NotionBindingChanged("notion-state-changed")
    );
    f.notion.set(|inner| inner.owned_state = b"owned");
    let local_before = fs::read_to_string(f.file()).unwrap();
    let done = f
        .service
        .retry_amendment_notion(&f.day, partial.revision(), &AtomicBool::new(false))
        .unwrap();
    assert_eq!(done.state(), WorkflowState::Completed);
    assert_eq!(fs::read_to_string(f.file()).unwrap(), local_before);
    assert_eq!(f.notion.replaced().len(), 2);
}

#[test]
fn an_uncertain_notion_replacement_gates_the_day_until_reconciliation_finishes_it() {
    let f = fixture();
    let completed = f.completed();
    let (previewed, digest) = f.previewed(&completed, "Quiet, then rain.");
    let approved = f
        .service
        .approve_amendment(&f.day, previewed.revision(), &digest)
        .unwrap();
    f.notion.set(|inner| inner.replace = Replace::Ambiguous);
    let publishing = f
        .service
        .publish_amendment(&f.day, approved.revision(), &AtomicBool::new(false))
        .unwrap();
    assert_eq!(publishing.state(), WorkflowState::Publishing);
    assert_eq!(f.store.unresolved_intents(Some(&f.day)).unwrap().len(), 1);
    assert_eq!(
        f.service
            .preview_amendment(&f.day, publishing.revision())
            .unwrap_err(),
        CommandError::Reconciling
    );

    // Undecidable: still gated. Then the destination answers.
    f.notion
        .set(|inner| inner.reconcile = InspectionOutcome::Unavailable);
    f.service.reconcile_amendment_intents().unwrap();
    assert_eq!(f.store.unresolved_intents(Some(&f.day)).unwrap().len(), 1);
    // An undecidable section is resumed once through the adapter, which reads
    // the section before every step; it is not replayed blindly.
    assert_eq!(f.notion.replaced().len(), 2);
    f.notion.set(|inner| {
        inner.reconcile = InspectionOutcome::Matching {
            receipt_id: String::new(),
            returned_ids: Vec::new(),
        }
    });
    f.service.reconcile_amendment_intents().unwrap();
    assert!(f.store.unresolved_intents(Some(&f.day)).unwrap().is_empty());
    let done = f.service.load_workflow(&f.day).unwrap();
    assert_eq!(done.state(), WorkflowState::Completed);
    assert_eq!(
        f.notion.replaced().len(),
        2,
        "a section found complete is adopted without another replacement call"
    );
}

#[test]
fn a_notion_conflict_after_the_local_replace_can_be_reread_then_approved_and_finished() {
    let f = fixture();
    let completed = f.completed();
    let (previewed, digest) = f.previewed(&completed, "Quiet, then rain.");
    let approved = f
        .service
        .approve_amendment(&f.day, previewed.revision(), &digest)
        .unwrap();
    f.notion.set(|inner| inner.replace = Replace::Conflict);
    let partial = f
        .service
        .publish_amendment(&f.day, approved.revision(), &AtomicBool::new(false))
        .unwrap();
    assert_eq!(partial.state(), WorkflowState::Partial);
    assert_eq!(Fixture::status(&partial, "local"), "verified");
    assert_eq!(Fixture::status(&partial, "notion"), "conflict");
    let local_after = fs::read_to_string(f.file()).unwrap();

    // Still not matching the receipt: reread reports it unchanged.
    f.notion
        .set(|inner| inner.preflight = NotionPreflightStatus::Conflict);
    let unchanged = f
        .service
        .reread_amendment(&f.day, partial.revision())
        .unwrap();
    assert_eq!(unchanged.workflow.revision(), partial.revision());
    assert_eq!(Fixture::status(&unchanged.workflow, "notion"), "conflict");

    // The section matches its receipt again: a fresh preview needs a new approval.
    f.notion.set(|inner| {
        inner.preflight = NotionPreflightStatus::Ready;
        inner.replace = Replace::Verify;
    });
    let reread = f
        .service
        .reread_amendment(&f.day, partial.revision())
        .unwrap();
    assert_eq!(reread.workflow.state(), WorkflowState::OutputPlanned);
    assert!(reread.workflow.approval().is_none());
    let preview = f
        .service
        .preview_amendment(&f.day, reread.workflow.revision())
        .unwrap();
    assert_eq!(preview.notion.as_ref().unwrap().status.as_str(), "pending");
    let digest = preview.workflow.output_plan().unwrap().plan_digest.clone();
    let approved = f
        .service
        .approve_amendment(&f.day, preview.workflow.revision(), &digest)
        .unwrap();
    let done = f
        .service
        .publish_amendment(&f.day, approved.revision(), &AtomicBool::new(false))
        .unwrap();
    assert_eq!(done.state(), WorkflowState::Completed);
    assert_eq!(fs::read_to_string(f.file()).unwrap(), local_after);
    assert_eq!(f.notion.replaced().len(), 2);
}

/// Puts an approved amendment into `Publishing` with a recorded local intent,
/// the way a crash after `begin_publication` would leave it.
fn publishing_with_local_intent(f: &Fixture) -> (OperationIntent, String) {
    let completed = f.completed();
    let (previewed, digest) = f.previewed(&completed, "Quiet, then rain.");
    let approved = f
        .service
        .approve_amendment(&f.day, previewed.revision(), &digest)
        .unwrap();
    let mut workflow = f.service.load_workflow(&f.day).unwrap();
    assert_eq!(workflow.revision(), approved.revision());
    let plan = workflow.output_plan().unwrap().clone();
    let local = plan
        .targets
        .iter()
        .find(|target| target.destination == "local")
        .unwrap();
    let intent = OperationIntent::new(
        "attempt_crash",
        plan.plan_id.clone(),
        f.day.clone(),
        local,
        plan.entry_digest.clone(),
        plan.selection_digest.clone(),
        Timestamp(150),
    )
    .unwrap();
    let revision = workflow.revision();
    workflow
        .begin_publication(revision, Timestamp(150))
        .unwrap();
    f.store.compare_and_swap(revision, None, &workflow).unwrap();
    assert!(f.store.record_intent(&intent).unwrap());
    let payload =
        diary_core::render_local_markdown(&f.day, WritingLanguage::EnUs, &workflow.entry().text);
    (intent, payload)
}

#[test]
fn a_crash_after_the_rename_but_before_the_receipt_is_adopted_without_rewriting() {
    let f = fixture();
    let (intent, payload) = publishing_with_local_intent(&f);
    let seed_digest = f
        .service
        .load_workflow(&f.day)
        .unwrap()
        .amendment()
        .unwrap()
        .file_digest
        .clone();
    // The rename landed; the receipt and workflow update did not.
    let output = LocalOutput::new(f.root.clone()).unwrap();
    assert_eq!(
        output.replace(&f.day, &seed_digest, &payload),
        Ok(diary_core::LocalReplaceOutcome::Replaced)
    );
    assert_eq!(f.store.unresolved_intents(Some(&f.day)).unwrap().len(), 1);
    f.service.reconcile_amendment_intents().unwrap();
    assert!(f.store.unresolved_intents(Some(&f.day)).unwrap().is_empty());
    assert_eq!(fs::read_to_string(f.file()).unwrap(), payload);
    let receipts = f.store.receipts(&f.day).unwrap();
    assert!(
        receipts
            .iter()
            .any(|receipt| receipt.attempt_id == intent.attempt_id
                && receipt.destination == "local"
                && receipt.version == 3)
    );
    // Local is done; Notion is the only thing left, retried alone.
    let partial = f.service.load_workflow(&f.day).unwrap();
    assert_eq!(partial.state(), WorkflowState::Partial);
    assert_eq!(Fixture::status(&partial, "local"), "verified");
    assert_eq!(Fixture::status(&partial, "notion"), "pending");
    let done = f
        .service
        .retry_amendment_notion(&f.day, partial.revision(), &AtomicBool::new(false))
        .unwrap();
    assert_eq!(done.state(), WorkflowState::Completed);
}

#[test]
fn a_crash_with_only_a_temp_file_written_leaves_the_file_and_retries_safely() {
    let f = fixture();
    let (_intent, payload) = publishing_with_local_intent(&f);
    let before = fs::read_to_string(f.file()).unwrap();
    // The temp file was written but never renamed.
    let stray =
        f.file()
            .parent()
            .unwrap()
            .join(format!(".nd-{}-{}-0.tmp", std::process::id() + 1, f.day));
    fs::write(&stray, &payload).unwrap();
    f.service.reconcile_amendment_intents().unwrap();
    assert_eq!(fs::read_to_string(f.file()).unwrap(), before, "untouched");
    assert!(f.store.unresolved_intents(Some(&f.day)).unwrap().is_empty());
    let partial = f.service.load_workflow(&f.day).unwrap();
    assert_eq!(partial.state(), WorkflowState::Partial);
    assert_eq!(Fixture::status(&partial, "local"), "pending");
    // The approval is reused for a local retry; the stray temp file is skipped.
    let done = f
        .service
        .publish_amendment(&f.day, partial.revision(), &AtomicBool::new(false))
        .unwrap();
    assert_eq!(done.state(), WorkflowState::Completed);
    assert_eq!(fs::read_to_string(f.file()).unwrap(), payload);
    assert!(!stray.exists(), "an earlier process's orphan temp is swept");
}

#[test]
fn completing_an_amendment_restarts_the_seven_day_text_purge() {
    const WEEK: i64 = 7 * 86_400;
    let f = fixture();
    let completed = f.completed();
    assert_eq!(completed.persisted().all_verified_at, Some(Timestamp(100)));
    f.clock.store(5_000, Ordering::SeqCst);
    let (previewed, digest) = f.previewed(&completed, "Quiet, then rain.");
    let approved = f
        .service
        .approve_amendment(&f.day, previewed.revision(), &digest)
        .unwrap();
    let done = f
        .service
        .publish_amendment(&f.day, approved.revision(), &AtomicBool::new(false))
        .unwrap();
    assert_eq!(done.persisted().all_verified_at, Some(Timestamp(5_000)));
    // Seven days after the first completion nothing expires any more...
    assert!(
        f.store
            .purge_expired(Timestamp(100 + WEEK))
            .unwrap()
            .is_empty()
    );
    assert!(
        f.store
            .purge_expired(Timestamp(5_000 + WEEK - 1))
            .unwrap()
            .is_empty()
    );
    // ...and seven days after the amendment the text is purged.
    assert_eq!(
        f.store.purge_expired(Timestamp(5_000 + WEEK)).unwrap(),
        vec![f.day.clone()]
    );
}

#[test]
fn an_unchanged_amendment_completes_instead_of_sticking_in_publishing() {
    let f = fixture();
    let completed = f.completed();
    let (previewed, digest) = f.previewed(&completed, "Today was quiet.");
    let approved = f
        .service
        .approve_amendment(&f.day, previewed.revision(), &digest)
        .unwrap();
    let done = f
        .service
        .publish_amendment(&f.day, approved.revision(), &AtomicBool::new(false))
        .unwrap();
    assert_eq!(done.state(), WorkflowState::Completed);
    assert!(f.store.unresolved_intents(Some(&f.day)).unwrap().is_empty());
}

fn amend_to(f: &Fixture, completed: &Workflow, text: &str) -> Workflow {
    let (previewed, digest) = f.previewed(completed, text);
    let approved = f
        .service
        .approve_amendment(&f.day, previewed.revision(), &digest)
        .unwrap();
    f.service
        .publish_amendment(&f.day, approved.revision(), &AtomicBool::new(false))
        .unwrap()
}

#[test]
fn the_icloud_mirror_is_replaced_with_the_local_file_guarded_by_its_own_receipt() {
    let f = fixture_with_mirror();
    let completed = f.completed();
    assert_eq!(
        fs::read(f.mirror_file()).unwrap(),
        fs::read(f.file()).unwrap()
    );
    let (previewed, _) = f.previewed(&completed, "Quiet, then rain.");
    let plan = previewed.output_plan().unwrap();
    assert!(
        plan.targets
            .iter()
            .any(|target| target.destination == "icloud")
    );
    assert_eq!(Fixture::status(&previewed, "icloud"), "pending");

    let done = amend_to(&f, &previewed_base(&f), "Quiet, then rain.");
    assert_eq!(done.state(), WorkflowState::Completed);
    let local = fs::read(f.file()).unwrap();
    assert!(String::from_utf8_lossy(&local).contains("then rain"));
    assert_eq!(fs::read(f.mirror_file()).unwrap(), local, "byte-identical");
    let receipts = f.store.receipts(&f.day).unwrap();
    assert!(
        receipts
            .iter()
            .any(|receipt| receipt.destination == "icloud" && receipt.version == 3)
    );
}

/// The workflow as it stands after an earlier preview was abandoned.
fn previewed_base(f: &Fixture) -> Workflow {
    let current = f.service.load_workflow(&f.day).unwrap();
    f.service
        .abandon_amendment(&f.day, current.revision())
        .unwrap()
}

#[test]
fn a_hand_edited_mirror_is_a_contained_conflict_and_never_overwritten() {
    let f = fixture_with_mirror();
    let completed = f.completed();
    fs::write(f.mirror_file(), "my own notes").unwrap();
    let partial = amend_to(&f, &completed, "Quiet, then rain.");
    assert_eq!(partial.state(), WorkflowState::Partial);
    assert_eq!(Fixture::status(&partial, "local"), "verified");
    assert_eq!(Fixture::status(&partial, "notion"), "verified");
    assert_eq!(Fixture::status(&partial, "icloud"), "conflict");
    assert_eq!(fs::read_to_string(f.mirror_file()).unwrap(), "my own notes");
}

#[test]
fn a_blocked_mirror_is_retried_alone_after_the_amendment() {
    let f = fixture_with_mirror();
    let completed = f.completed();
    let (previewed, digest) = f.previewed(&completed, "Quiet, then rain.");
    let approved = f
        .service
        .approve_amendment(&f.day, previewed.revision(), &digest)
        .unwrap();
    let moved = f.mirror_root.with_extension("away");
    fs::rename(&f.mirror_root, &moved).unwrap();
    let partial = f
        .service
        .publish_amendment(&f.day, approved.revision(), &AtomicBool::new(false))
        .unwrap();
    assert_eq!(partial.state(), WorkflowState::Partial);
    assert_eq!(Fixture::status(&partial, "icloud"), "blocked");
    assert_eq!(Fixture::status(&partial, "notion"), "verified");
    fs::rename(&moved, &f.mirror_root).unwrap();
    let done = f
        .service
        .publish_mirror_guarded(&f.day, partial.revision(), || true)
        .unwrap();
    assert_eq!(done.state(), WorkflowState::Completed);
    assert_eq!(
        fs::read(f.mirror_file()).unwrap(),
        fs::read(f.file()).unwrap()
    );
}
