#![forbid(unsafe_code)]

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use diary_connectors::{LocalOutput, SqliteStore};
use diary_control::{CommandError, CommandService, ExistingEntryMode};
use diary_core::{
    CatalogRequest, Clock, DestinationStatus, DiaryDay, Digest, FreshnessOutcome,
    InspectionOutcome, NotionBlockPreview, NotionOutputPort, NotionPreflight,
    NotionPreflightStatus, NotionRequest, SourceAccessError, SourceCatalog, SourceDiscovery,
    SourceReadOutcome, SourceReader, SourceRef, SourceSnapshot, StartOutcome, Timestamp, Workflow,
    WriteOutcome, WritingLanguage, WritingStyle,
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
                "nd-existing-entry-{}-{}",
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
    day: DiaryDay,
}

impl Fixture {
    fn new() -> Self {
        let scratch = Scratch::new();
        let store = Arc::new(SqliteStore::open(scratch.0.join("state.sqlite3")).unwrap());
        let local = Arc::new(LocalOutput::new(scratch.0.join("entries")).unwrap());
        Self {
            _scratch: scratch,
            store,
            local,
            day: DiaryDay::parse("2026-10-30").unwrap(),
        }
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
}

fn status_of(workflow: &Workflow, destination: &str) -> DestinationStatus {
    workflow
        .destination_progress()
        .iter()
        .find(|item| item.destination == destination)
        .unwrap()
        .status
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

const EXISTING: &str = "hand written, no front matter";

fn local_target(fx: &Fixture) -> PathBuf {
    fx.local.target(&fx.day)
}

fn seed_local(fx: &Fixture, text: &str) {
    let target = local_target(fx);
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::write(target, text).unwrap();
}

fn preview_with(
    service: &CommandService,
    day: &DiaryDay,
    mode: ExistingEntryMode,
) -> diary_control::LocalPreview {
    let confirmed = confirmed(service, day);
    service
        .preview_local_with(day, confirmed.revision(), mode)
        .unwrap()
}

fn approve_and_publish(
    service: &CommandService,
    day: &DiaryDay,
    preview: &diary_control::LocalPreview,
) -> Workflow {
    let approved = service
        .approve_local(
            day,
            preview.workflow.revision(),
            &preview.workflow.output_plan().unwrap().plan_digest,
        )
        .unwrap();
    service.publish_local(day, approved.revision()).unwrap()
}

fn backups(path: &std::path::Path) -> Vec<PathBuf> {
    let mut found: Vec<_> = fs::read_dir(path.parent().unwrap())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|candidate| candidate.to_string_lossy().contains(".bak-"))
        .collect();
    found.sort();
    found
}

#[test]
fn a_differing_file_defaults_to_append_and_is_never_a_conflict() {
    let fx = Fixture::new();
    let service = fx.base();
    seed_local(&fx, EXISTING);
    let preview = preview_with(&service, &fx.day, ExistingEntryMode::Refuse);
    assert_eq!(
        status_of(&preview.workflow, "local"),
        DestinationStatus::Pending
    );
    assert_eq!(preview.mode, ExistingEntryMode::Append);
    assert_eq!(preview.existing_text.as_deref(), Some(EXISTING));
    assert!(preview.payload.starts_with(EXISTING));
    // The plan can be approved as it stands, and nothing is written before
    // publication.
    service
        .approve_local(
            &fx.day,
            preview.workflow.revision(),
            &preview.workflow.output_plan().unwrap().plan_digest,
        )
        .unwrap();
    assert_eq!(fs::read_to_string(local_target(&fx)).unwrap(), EXISTING);
}

#[test]
fn append_keeps_the_existing_bytes_and_adds_the_dated_block() {
    let fx = Fixture::new();
    let service = fx.base();
    seed_local(&fx, EXISTING);
    let preview = preview_with(&service, &fx.day, ExistingEntryMode::Append);
    assert_eq!(
        status_of(&preview.workflow, "local"),
        DestinationStatus::Pending
    );
    let added = preview
        .payload
        .strip_prefix(&format!("{EXISTING}\n\n---\n\n*Appended "))
        .expect("existing text, a newline, a blank line, then the rule");
    let (stamp, rest) = added.split_once("*\n\n").expect("closing italic line");
    assert_eq!(stamp.len(), 16, "{stamp}");
    assert_eq!(rest, "Today was quiet.\n");
    let published = approve_and_publish(&service, &fx.day, &preview);
    assert_eq!(status_of(&published, "local"), DestinationStatus::Verified);
    assert_eq!(
        fs::read_to_string(local_target(&fx)).unwrap(),
        preview.payload
    );
    assert!(
        backups(&local_target(&fx)).is_empty(),
        "append needs no backup"
    );
    // The verified, appended file lets the Notion phase proceed.
    let completed = publish_notion(&service, &fx.day, &published);
    assert_eq!(completed.state().as_str(), "completed");
}

#[test]
fn overwrite_backs_up_then_replaces_with_the_plain_rendering() {
    let fx = Fixture::new();
    let service = fx.base();
    seed_local(&fx, EXISTING);
    let preview = preview_with(&service, &fx.day, ExistingEntryMode::Overwrite);
    assert!(preview.payload.starts_with("---\ndate: \"2026-10-30\""));
    assert!(
        preview
            .intended_changes
            .iter()
            .any(|change| change.starts_with("Back up the existing file to "))
    );
    let published = approve_and_publish(&service, &fx.day, &preview);
    assert_eq!(status_of(&published, "local"), DestinationStatus::Verified);
    assert_eq!(
        fs::read_to_string(local_target(&fx)).unwrap(),
        preview.payload
    );
    let backup = backups(&local_target(&fx));
    assert_eq!(backup.len(), 1);
    assert_eq!(
        backup[0],
        PathBuf::from(diary_core::backup_file_name(
            &local_target(&fx).to_string_lossy(),
            &Digest::of_bytes(EXISTING.as_bytes())
        ))
    );
    assert_eq!(fs::read_to_string(&backup[0]).unwrap(), EXISTING);
    assert!(!backup[0].to_string_lossy().ends_with(".md"));
}

#[test]
fn a_chosen_mode_is_ignored_when_nothing_exists() {
    let fx = Fixture::new();
    let service = fx.base();
    let preview = preview_with(&service, &fx.day, ExistingEntryMode::Append);
    assert!(preview.existing_text.is_none());
    assert!(preview.payload.starts_with("---\ndate: \"2026-10-30\""));
    assert!(!preview.payload.contains("Appended"));
}

#[test]
fn a_file_edited_after_the_preview_voids_the_plan_and_nothing_is_written() {
    let fx = Fixture::new();
    let service = fx.base();
    seed_local(&fx, EXISTING);
    let preview = preview_with(&service, &fx.day, ExistingEntryMode::Overwrite);
    fs::write(local_target(&fx), "edited since the preview").unwrap();
    assert!(matches!(
        service.approve_local(
            &fx.day,
            preview.workflow.revision(),
            &preview.workflow.output_plan().unwrap().plan_digest
        ),
        Err(CommandError::PreconditionFailed)
    ));
    assert_eq!(
        fs::read_to_string(local_target(&fx)).unwrap(),
        "edited since the preview"
    );
    assert!(backups(&local_target(&fx)).is_empty());
}

#[test]
fn a_file_edited_after_approval_blocks_publication() {
    let fx = Fixture::new();
    let service = fx.base();
    seed_local(&fx, EXISTING);
    let preview = preview_with(&service, &fx.day, ExistingEntryMode::Append);
    let approved = service
        .approve_local(
            &fx.day,
            preview.workflow.revision(),
            &preview.workflow.output_plan().unwrap().plan_digest,
        )
        .unwrap();
    fs::write(local_target(&fx), "edited after approval").unwrap();
    assert!(matches!(
        service.publish_local(&fx.day, approved.revision()),
        Err(CommandError::PreconditionFailed)
    ));
    assert_eq!(
        fs::read_to_string(local_target(&fx)).unwrap(),
        "edited after approval"
    );
}

#[test]
fn a_backup_that_cannot_be_written_leaves_the_file_untouched() {
    let fx = Fixture::new();
    let service = fx.base();
    seed_local(&fx, EXISTING);
    let preview = preview_with(&service, &fx.day, ExistingEntryMode::Overwrite);
    // A directory squatting on the backup name makes the copy impossible.
    let blocker = PathBuf::from(diary_core::backup_file_name(
        &local_target(&fx).to_string_lossy(),
        &Digest::of_bytes(EXISTING.as_bytes()),
    ));
    fs::create_dir(&blocker).unwrap();
    let published = approve_and_publish(&service, &fx.day, &preview);
    assert_ne!(status_of(&published, "local"), DestinationStatus::Verified);
    assert_eq!(fs::read_to_string(local_target(&fx)).unwrap(), EXISTING);
}
