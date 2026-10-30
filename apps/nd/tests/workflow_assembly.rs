#![forbid(unsafe_code)]

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use diary_connectors::SqliteStore;
use diary_core::{
    DateEvidence, DiaryDay, Preferences, SourceAvailability, SourceCandidate, SourceRef,
    StartOutcome, Timestamp, WorkflowStore,
};

static SCRATCH_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[test]
fn core_and_sqlite_assemble_without_starting_a_production_service() {
    let scratch = Scratch::new();
    let store = SqliteStore::open(scratch.path().join("private.sqlite3")).unwrap();
    let day = DiaryDay::parse("2026-10-28").unwrap();
    let mut workflow = match store
        .start_or_resume(day.clone(), Timestamp(1), &Preferences::default())
        .unwrap()
    {
        StartOutcome::Created(workflow) => workflow,
        _ => panic!("synthetic workflow must be new"),
    };
    workflow
        .discover(
            workflow.revision(),
            vec![SourceCandidate {
                source: SourceRef::new("fake", "note-1").unwrap(),
                source_note_name: "note-1.md".to_owned(),
                date_evidence: DateEvidence {
                    directory_day: Some(day.clone()),
                    header_day: Some(day.clone()),
                    warning: None,
                },
                availability: SourceAvailability::Resident,
            }],
            Timestamp(2),
        )
        .unwrap();
    store.compare_and_swap(0, None, &workflow).unwrap();
    drop(store);

    let reopened = SqliteStore::open(scratch.path().join("private.sqlite3"))
        .unwrap()
        .load(&day)
        .unwrap()
        .unwrap();
    assert_eq!(reopened, workflow);
    assert_eq!(fs::read_dir(scratch.path()).unwrap().count(), 1);
}

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        loop {
            let sequence = SCRATCH_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "nd-workflow-assembly-{}-{sequence}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("scratch directory should be created: {error}"),
            }
        }
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.0)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            eprintln!("failed to remove test scratch directory: {error}");
        }
    }
}
