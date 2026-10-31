#![forbid(unsafe_code)]

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use diary_connectors::{LocalOutput, SqliteStore};
use diary_control::{CommandError, CommandService, ExistingEntryMode, Readiness, RemoteStatus};
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
                "nd-existing-notion-{}-{}",
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

use diary_core::{
    NotionExistingSection, NotionExtendRequest, NotionReplacePreflight, NotionReplaceRequest,
    OperationLog,
};

fn texts_digest(texts: &[String]) -> Digest {
    Digest::of_bytes(texts.join("\n").as_bytes())
}

fn preview_of(texts: &[String]) -> Vec<NotionBlockPreview> {
    texts
        .iter()
        .map(|text| NotionBlockPreview {
            kind: "paragraph".into(),
            text: text.clone(),
            depth: 0,
        })
        .collect()
}

/// A one-section Notion page: `(block id, text)` for the day's section.
#[derive(Default)]
struct FakeNotion {
    section: Mutex<Option<Vec<(String, String)>>>,
    next: Mutex<u64>,
}

impl FakeNotion {
    fn seed(&self, texts: &[&str]) {
        let section = texts
            .iter()
            .enumerate()
            .map(|(index, text)| (format!("seed-{index}"), (*text).to_owned()))
            .collect();
        *self.section.lock().unwrap() = Some(section);
    }

    fn edit(&self, extra: &str) {
        let mut guard = self.section.lock().unwrap();
        guard
            .as_mut()
            .unwrap()
            .push(("edited".into(), extra.to_owned()));
    }

    fn texts(&self) -> Vec<String> {
        self.section
            .lock()
            .unwrap()
            .as_ref()
            .map(|section| section.iter().map(|(_, text)| text.clone()).collect())
            .unwrap_or_default()
    }

    fn fresh_id(&self) -> String {
        let mut next = self.next.lock().unwrap();
        *next += 1;
        format!("new-{next}")
    }

    fn matches(&self, ids: &[String], digest: &Digest) -> bool {
        // One lock only: `texts` takes the same mutex, which is not reentrant.
        let guard = self.section.lock().unwrap();
        guard.as_ref().is_some_and(|section| {
            let texts: Vec<_> = section.iter().map(|(_, text)| text.clone()).collect();
            section.iter().map(|(id, _)| id.clone()).collect::<Vec<_>>() == ids
                && &texts_digest(&texts) == digest
        })
    }

    fn base(&self, day: &DiaryDay, payload: Digest, observed: Digest) -> NotionPreflight {
        NotionPreflight {
            target_identity: "root".into(),
            root_label: "Notion root".into(),
            year_title: day.as_str()[..4].to_owned(),
            month_title: day.as_str()[..7].to_owned(),
            year_exists: true,
            month_exists: true,
            placement: "end".into(),
            renderer_version: 1,
            api_version: "2026-03-11".into(),
            payload_digest: payload,
            observed_state_digest: observed,
            blocks: Vec::new(),
            status: NotionPreflightStatus::Ready,
            reason: None,
        }
    }
}

impl NotionOutputPort for FakeNotion {
    fn preflight(&self, day: &DiaryDay, markdown: &str) -> Result<NotionPreflight, String> {
        let payload = Digest::of_bytes(format!("plain:{markdown}").as_bytes());
        let exists = self.section.lock().unwrap().is_some();
        let mut preflight = match exists {
            false => self.base(day, payload, Digest::of_bytes(b"empty")),
            true => {
                let mut conflict = self.base(day, payload, texts_digest(&self.texts()));
                conflict.status = NotionPreflightStatus::Existing;
                conflict.reason = Some("notion-same-day-conflict".into());
                conflict
            }
        };
        preflight.blocks = preview_of(&[markdown.to_owned()]);
        Ok(preflight)
    }

    fn write(&self, request: &NotionRequest, _: &AtomicBool) -> WriteOutcome {
        let mut section = self.section.lock().unwrap();
        if section.is_some() {
            return WriteOutcome::Conflict;
        }
        *section = Some(vec![
            ("h".into(), "heading".into()),
            ("b".into(), request.markdown.clone()),
        ]);
        WriteOutcome::Verified {
            receipt_id: "m".into(),
            returned_ids: vec!["h".into(), "b".into()],
        }
    }

    fn inspect(&self, request: &NotionRequest) -> InspectionOutcome {
        if self.texts() == ["heading".to_owned(), request.markdown.clone()] {
            InspectionOutcome::Matching {
                receipt_id: "m".into(),
                returned_ids: vec!["h".into(), "b".into()],
            }
        } else {
            InspectionOutcome::Absent
        }
    }

    fn observe_existing(&self, _: &DiaryDay) -> Result<Option<NotionExistingSection>, String> {
        Ok(self.section.lock().unwrap().as_ref().map(|section| {
            let texts: Vec<_> = section.iter().map(|(_, text)| text.clone()).collect();
            NotionExistingSection {
                month_id: "m".into(),
                block_ids: section.iter().map(|(id, _)| id.clone()).collect(),
                section_digest: texts_digest(&texts),
                blocks: preview_of(&texts),
            }
        }))
    }

    fn preflight_replace(
        &self,
        request: &NotionReplaceRequest,
    ) -> Result<NotionReplacePreflight, String> {
        let mut preflight = self.base(
            &request.day,
            request.expected_payload_digest.clone(),
            request.owned_payload_digest.clone(),
        );
        if !self.matches(&request.owned_block_ids, &request.owned_payload_digest) {
            preflight.status = NotionPreflightStatus::Existing;
            preflight.reason = Some("notion-section-not-owned".into());
        }
        preflight.blocks = preview_of(std::slice::from_ref(&request.markdown));
        Ok(NotionReplacePreflight {
            preflight,
            current_blocks: preview_of(&self.texts()),
        })
    }

    fn replace_section(&self, request: &NotionReplaceRequest, _: &AtomicBool) -> WriteOutcome {
        if !self.matches(&request.owned_block_ids, &request.owned_payload_digest) {
            return WriteOutcome::Conflict;
        }
        let ids = vec![request.owned_block_ids[0].clone(), self.fresh_id()];
        *self.section.lock().unwrap() = Some(vec![
            (ids[0].clone(), "heading".into()),
            (ids[1].clone(), request.markdown.clone()),
        ]);
        WriteOutcome::Verified {
            receipt_id: "m".into(),
            returned_ids: ids,
        }
    }

    fn preflight_extend(
        &self,
        request: &NotionExtendRequest,
    ) -> Result<NotionReplacePreflight, String> {
        let added = vec![
            "---".to_owned(),
            format!("Appended {}", request.appended_at),
            request.markdown.clone(),
        ];
        let mut result = self.texts();
        result.extend(added.clone());
        let mut preflight = self.base(
            &request.day,
            texts_digest(&result),
            request.owned_section_digest.clone(),
        );
        if !self.matches(&request.owned_block_ids, &request.owned_section_digest) {
            preflight.status = NotionPreflightStatus::Existing;
            preflight.reason = Some("notion-section-not-owned".into());
        }
        preflight.blocks = preview_of(&added);
        Ok(NotionReplacePreflight {
            preflight,
            current_blocks: preview_of(&self.texts()),
        })
    }

    fn extend_section(&self, request: &NotionExtendRequest, _: &AtomicBool) -> WriteOutcome {
        if !self.matches(&request.owned_block_ids, &request.owned_section_digest) {
            return WriteOutcome::Conflict;
        }
        let mut guard = self.section.lock().unwrap();
        let section = guard.as_mut().unwrap();
        for text in [
            "---".to_owned(),
            format!("Appended {}", request.appended_at),
            request.markdown.clone(),
        ] {
            let id = {
                let mut next = self.next.lock().unwrap();
                *next += 1;
                format!("new-{next}")
            };
            section.push((id, text));
        }
        let ids: Vec<_> = section.iter().map(|(id, _)| id.clone()).collect();
        let texts: Vec<_> = section.iter().map(|(_, text)| text.clone()).collect();
        if request.expected_result_digest.as_ref() != Some(&texts_digest(&texts)) {
            return WriteOutcome::Conflict;
        }
        WriteOutcome::Verified {
            receipt_id: "m".into(),
            returned_ids: ids,
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
    notion: Arc<FakeNotion>,
    service: CommandService,
    day: DiaryDay,
}

impl Fixture {
    fn new() -> Self {
        let scratch = Scratch::new();
        let store = Arc::new(SqliteStore::open(scratch.0.join("state.sqlite3")).unwrap());
        let local = Arc::new(LocalOutput::new(scratch.0.join("entries")).unwrap());
        let notion = Arc::new(FakeNotion::default());
        let service = CommandService::new(
            store.clone(),
            Arc::new(EmptySources),
            Arc::new(EmptySources),
            Arc::new(FixedClock),
        )
        .with_local_output(local)
        .with_notion_output(notion.clone());
        Self {
            _scratch: scratch,
            store,
            notion,
            service,
            day: DiaryDay::parse("2026-10-30").unwrap(),
        }
    }

    /// Publishes the local file so the Notion phase can start.
    fn local_published(&self) -> Workflow {
        let confirmed = confirmed(&self.service, &self.day);
        let preview = self
            .service
            .preview_local(&self.day, confirmed.revision())
            .unwrap();
        let approved = self
            .service
            .approve_local(
                &self.day,
                preview.workflow.revision(),
                &preview.workflow.output_plan().unwrap().plan_digest,
            )
            .unwrap();
        self.service
            .publish_local(&self.day, approved.revision())
            .unwrap()
    }
}

fn notion_status(workflow: &Workflow) -> DestinationStatus {
    workflow
        .destination_progress()
        .iter()
        .find(|item| item.destination == "notion")
        .unwrap()
        .status
}

#[test]
fn a_foreign_section_is_a_conflict_with_no_commands_taken_until_a_mode_is_chosen() {
    let fx = Fixture::new();
    fx.notion.seed(&["heading", "theirs"]);
    let local = fx.local_published();
    let preview = fx
        .service
        .preview_notion(&fx.day, local.revision())
        .unwrap();
    assert_eq!(preview.status, DestinationStatus::Changed);
    assert_eq!(preview.reason.as_deref(), Some("notion-same-day-conflict"));
    assert!(preview.plan_digest.is_none());
    assert_eq!(fx.notion.texts(), ["heading", "theirs"]);
    assert_eq!(
        fx.service.remotes(&fx.day).unwrap()[1].status,
        RemoteStatus::Exists
    );
    let readiness = fx
        .service
        .check_publish_readiness(&fx.day, local.revision())
        .unwrap();
    assert_eq!(readiness.destinations[0].readiness, Readiness::Ready);
}

#[test]
fn append_adds_the_dated_block_after_the_existing_section() {
    let fx = Fixture::new();
    fx.notion.seed(&["heading", "theirs"]);
    let local = fx.local_published();
    let preview = fx
        .service
        .preview_notion_with(&fx.day, local.revision(), ExistingEntryMode::Append)
        .unwrap();
    assert_eq!(preview.status, DestinationStatus::Pending);
    assert_eq!(preview.mode, ExistingEntryMode::Append);
    assert_eq!(preview.current_blocks.len(), 2);
    assert_eq!(preview.blocks.len(), 3);
    assert_eq!(preview.blocks[0].text, "---");
    let digest = preview.plan_digest.clone().unwrap();
    let approved = fx
        .service
        .approve_notion(&fx.day, preview.workflow.revision(), &digest)
        .unwrap();
    let published = fx
        .service
        .publish_notion(&fx.day, approved.revision())
        .unwrap();
    assert_eq!(notion_status(&published), DestinationStatus::Verified);
    let texts = fx.notion.texts();
    assert_eq!(&texts[..2], ["heading", "theirs"]);
    assert_eq!(texts[2], "---");
    assert!(texts[3].starts_with("Appended "));
    assert_eq!(texts[4], "Today was quiet.");
    // The receipt vouches for the whole resulting section.
    assert!(
        fx.store
            .receipts(&fx.day)
            .unwrap()
            .iter()
            .any(|receipt| receipt.destination == "notion"
                && receipt.payload_digest == texts_digest(&texts))
    );
}

#[test]
fn overwrite_replaces_the_body_and_keeps_the_heading() {
    let fx = Fixture::new();
    fx.notion.seed(&["heading", "theirs"]);
    let local = fx.local_published();
    let preview = fx
        .service
        .preview_notion_with(&fx.day, local.revision(), ExistingEntryMode::Overwrite)
        .unwrap();
    assert_eq!(preview.mode, ExistingEntryMode::Overwrite);
    let digest = preview.plan_digest.clone().unwrap();
    let approved = fx
        .service
        .approve_notion(&fx.day, preview.workflow.revision(), &digest)
        .unwrap();
    let published = fx
        .service
        .publish_notion(&fx.day, approved.revision())
        .unwrap();
    assert_eq!(notion_status(&published), DestinationStatus::Verified);
    assert_eq!(fx.notion.texts(), ["heading", "Today was quiet."]);
}

#[test]
fn a_section_this_app_wrote_is_recognised_until_it_is_edited() {
    let fx = Fixture::new();
    fx.notion.seed(&["heading", "theirs"]);
    let local = fx.local_published();
    // Someone else's section is never treated as ours.
    assert!(!fx.service.notion_section_is_ours(&fx.day));
    let preview = fx
        .service
        .preview_notion_with(&fx.day, local.revision(), ExistingEntryMode::Append)
        .unwrap();
    let digest = preview.plan_digest.clone().unwrap();
    let approved = fx
        .service
        .approve_notion(&fx.day, preview.workflow.revision(), &digest)
        .unwrap();
    fx.service
        .publish_notion(&fx.day, approved.revision())
        .unwrap();
    // After this app's own write the section is ours, so a later push updates
    // it in place without asking.
    assert!(fx.service.notion_section_is_ours(&fx.day));
    assert_eq!(
        fx.service.remotes(&fx.day).unwrap()[1].status,
        RemoteStatus::Pushed
    );
    // Anything added by hand makes it someone else's again.
    fx.notion.edit("added by hand");
    assert!(!fx.service.notion_section_is_ours(&fx.day));
    assert_eq!(
        fx.service.remotes(&fx.day).unwrap()[1].status,
        RemoteStatus::Exists
    );
}

#[test]
fn an_amended_local_entry_marks_the_earlier_notion_copy_not_pushed() {
    let fx = Fixture::new();
    fx.notion.seed(&["heading", "theirs"]);
    let local = fx.local_published();
    let preview = fx
        .service
        .preview_notion_with(&fx.day, local.revision(), ExistingEntryMode::Append)
        .unwrap();
    let approved = fx
        .service
        .approve_notion(
            &fx.day,
            preview.workflow.revision(),
            preview.plan_digest.as_ref().unwrap(),
        )
        .unwrap();
    let published = fx
        .service
        .publish_notion(&fx.day, approved.revision())
        .unwrap();
    assert_eq!(
        fx.service.remotes(&fx.day).unwrap()[1].status,
        RemoteStatus::Pushed
    );

    let seeded = fx
        .service
        .seed_amendment(&fx.day, published.revision())
        .unwrap();
    let edited = fx
        .service
        .save_entry(
            &fx.day,
            seeded.workflow.entry().version,
            "A different day.".to_owned(),
        )
        .unwrap();
    let confirmed = fx
        .service
        .confirm_entry(&fx.day, edited.revision(), edited.entry().version)
        .unwrap();
    let plan = fx
        .service
        .preview_amendment(&fx.day, confirmed.revision())
        .unwrap();
    let approved = fx
        .service
        .approve_amendment(
            &fx.day,
            plan.workflow.revision(),
            &plan.workflow.output_plan().unwrap().plan_digest,
        )
        .unwrap();
    let amended = fx
        .service
        .publish_amendment(&fx.day, approved.revision(), &AtomicBool::new(false))
        .unwrap();
    assert!(fx.service.notion_section_is_ours(&fx.day));
    assert_eq!(
        fx.service.remotes(&fx.day).unwrap()[1].status,
        RemoteStatus::NotPushed
    );
    let readiness = fx
        .service
        .check_publish_readiness(&fx.day, amended.revision())
        .unwrap();
    assert_eq!(readiness.destinations[0].readiness, Readiness::Ready);
}

#[test]
fn a_section_edited_after_the_preview_voids_the_plan_for_both_modes() {
    for mode in [ExistingEntryMode::Append, ExistingEntryMode::Overwrite] {
        let fx = Fixture::new();
        fx.notion.seed(&["heading", "theirs"]);
        let local = fx.local_published();
        let preview = fx
            .service
            .preview_notion_with(&fx.day, local.revision(), mode)
            .unwrap();
        fx.notion.edit("added by hand");
        let digest = preview.plan_digest.clone().unwrap();
        assert!(
            matches!(
                fx.service
                    .approve_notion(&fx.day, preview.workflow.revision(), &digest),
                Err(CommandError::NotionBindingChanged(_))
            ),
            "{mode:?}"
        );
        assert_eq!(fx.notion.texts(), ["heading", "theirs", "added by hand"]);
    }
}

#[test]
fn a_section_edited_after_approval_blocks_publication() {
    let fx = Fixture::new();
    fx.notion.seed(&["heading", "theirs"]);
    let local = fx.local_published();
    let preview = fx
        .service
        .preview_notion_with(&fx.day, local.revision(), ExistingEntryMode::Overwrite)
        .unwrap();
    let digest = preview.plan_digest.clone().unwrap();
    let approved = fx
        .service
        .approve_notion(&fx.day, preview.workflow.revision(), &digest)
        .unwrap();
    fx.notion.edit("added by hand");
    assert!(matches!(
        fx.service.publish_notion(&fx.day, approved.revision()),
        Err(CommandError::NotionBindingChanged(_))
    ));
    assert_eq!(fx.notion.texts(), ["heading", "theirs", "added by hand"]);
}

#[test]
fn a_chosen_mode_does_nothing_when_the_day_has_no_section() {
    let fx = Fixture::new();
    let local = fx.local_published();
    let preview = fx
        .service
        .preview_notion_with(&fx.day, local.revision(), ExistingEntryMode::Overwrite)
        .unwrap();
    assert_eq!(preview.status, DestinationStatus::Pending);
    assert_eq!(preview.mode, ExistingEntryMode::Refuse);
    assert!(preview.current_blocks.is_empty());
    let digest = preview.plan_digest.clone().unwrap();
    let approved = fx
        .service
        .approve_notion(&fx.day, preview.workflow.revision(), &digest)
        .unwrap();
    fx.service
        .publish_notion(&fx.day, approved.revision())
        .unwrap();
    assert_eq!(fx.notion.texts(), ["heading", "Today was quiet."]);
}
