use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::sync::atomic::{AtomicU64, Ordering};

use super::*;

static SCRATCH_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct FixedClock(AtomicU64);

impl FixedClock {
    fn new(now: u64) -> Self {
        Self(AtomicU64::new(now))
    }
    fn set(&self, now: u64) {
        self.0.store(now, Ordering::Relaxed);
    }
}

impl LogClock for FixedClock {
    fn now_unix_ms(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }
}

#[test]
fn typed_records_rotate_and_repair_only_a_torn_tail() {
    let scratch = Scratch::new("rotate");
    let clock = Arc::new(FixedClock::new(1));
    let uid = fs::metadata(scratch.path()).unwrap().uid();
    let log = DiagnosticLog::open(scratch.path().join("logs"), uid, clock.clone()).unwrap();
    log.write(
        DiagnosticLevel::Info,
        DiagnosticEvent::Lifecycle {
            component: "server",
            event: "startup",
        },
    )
    .unwrap();
    clock.set(DAY_MS + 1);
    log.write(
        DiagnosticLevel::Warn,
        DiagnosticEvent::Capacity {
            component: "jobs",
            event: "saturated",
            current: 32,
            limit: 32,
        },
    )
    .unwrap();
    let entries = fs::read_dir(log.root()).unwrap().count();
    assert_eq!(entries, 2);
    let active = log.root().join(ACTIVE_NAME);
    drop(log);
    OpenOptions::new()
        .append(true)
        .open(&active)
        .unwrap()
        .write_all(b"torn-private-tail")
        .unwrap();
    let reopened = DiagnosticLog::open(scratch.path().join("logs"), uid, clock).unwrap();
    let text = fs::read_to_string(reopened.root().join(ACTIVE_NAME)).unwrap();
    assert!(!text.contains("private"));
    assert!(text.ends_with('\n'));
}

#[test]
fn level_parse_accepts_known_names_case_insensitively_and_rejects_others() {
    assert_eq!(
        DiagnosticLevel::parse("debug"),
        Some(DiagnosticLevel::Debug)
    );
    assert_eq!(
        DiagnosticLevel::parse("DEBUG"),
        Some(DiagnosticLevel::Debug)
    );
    assert_eq!(DiagnosticLevel::parse("Info"), Some(DiagnosticLevel::Info));
    assert_eq!(DiagnosticLevel::parse("verbose"), None);
}

#[test]
fn min_level_drops_records_below_it_and_keeps_records_at_or_above_it() {
    let scratch = Scratch::new("min-level");
    let clock = Arc::new(FixedClock::new(1));
    let uid = fs::metadata(scratch.path()).unwrap().uid();
    let log = DiagnosticLog::open_with_min_level(
        scratch.path().join("logs"),
        uid,
        clock,
        DiagnosticLevel::Debug,
    )
    .unwrap();
    log.write(
        DiagnosticLevel::Trace,
        DiagnosticEvent::Job {
            kind: "discovery",
            event: "accepted",
            reason: None,
        },
    )
    .unwrap();
    log.write(
        DiagnosticLevel::Debug,
        DiagnosticEvent::Job {
            kind: "discovery",
            event: "completed",
            reason: None,
        },
    )
    .unwrap();
    let text = fs::read_to_string(log.root().join(ACTIVE_NAME)).unwrap();
    assert!(!text.contains("event=accepted"));
    assert!(text.contains("event=completed"));
}

#[test]
fn default_min_level_is_info_and_silently_drops_debug_records() {
    let scratch = Scratch::new("default-min-level");
    let clock = Arc::new(FixedClock::new(1));
    let uid = fs::metadata(scratch.path()).unwrap().uid();
    let log = DiagnosticLog::open(scratch.path().join("logs"), uid, clock).unwrap();
    log.write(
        DiagnosticLevel::Debug,
        DiagnosticEvent::Preparation { event: "issued" },
    )
    .unwrap();
    log.write(
        DiagnosticLevel::Info,
        DiagnosticEvent::Lifecycle {
            component: "server",
            event: "ready",
        },
    )
    .unwrap();
    let text = fs::read_to_string(log.root().join(ACTIVE_NAME)).unwrap();
    assert!(!text.contains("preparation"));
    assert!(text.contains("component=server event=ready"));
}

#[test]
fn job_and_preparation_events_reject_outside_their_closed_vocabulary() {
    let scratch = Scratch::new("closed-vocab");
    let clock = Arc::new(FixedClock::new(1));
    let uid = fs::metadata(scratch.path()).unwrap().uid();
    let log = DiagnosticLog::open_with_min_level(
        scratch.path().join("logs"),
        uid,
        clock,
        DiagnosticLevel::Debug,
    )
    .unwrap();
    assert_eq!(
        log.write(
            DiagnosticLevel::Debug,
            DiagnosticEvent::Job {
                kind: "unknown-kind",
                event: "accepted",
                reason: None,
            },
        ),
        Err(DiagnosticError::InvalidRecord)
    );
    assert_eq!(
        log.write(
            DiagnosticLevel::Debug,
            DiagnosticEvent::Preparation {
                event: "not-a-real-event",
            },
        ),
        Err(DiagnosticError::InvalidRecord)
    );
}

#[test]
fn notion_preview_records_a_closed_reason_and_rejects_free_text() {
    let scratch = Scratch::new("notion-preview");
    let clock = Arc::new(FixedClock::new(1));
    let uid = fs::metadata(scratch.path()).unwrap().uid();
    let log = DiagnosticLog::open_with_min_level(
        scratch.path().join("logs"),
        uid,
        clock,
        DiagnosticLevel::Debug,
    )
    .unwrap();
    log.write(
        DiagnosticLevel::Debug,
        DiagnosticEvent::NotionPreview {
            status: "blocked",
            reason: "notion-root-unverifiable",
        },
    )
    .unwrap();
    let text = fs::read_to_string(log.root().join(ACTIVE_NAME)).unwrap();
    assert!(text.contains("component=notion event=preview"));
    assert!(text.contains("reason=notion-root-unverifiable status=blocked"));
    assert_eq!(
        log.write(
            DiagnosticLevel::Debug,
            DiagnosticEvent::NotionPreview {
                status: "blocked",
                reason: "page abc123 not found",
            },
        ),
        Err(DiagnosticError::InvalidRecord)
    );
}

#[test]
fn job_failed_records_the_reason_code_and_other_events_reject_one() {
    let scratch = Scratch::new("job-reason");
    let clock = Arc::new(FixedClock::new(1));
    let uid = fs::metadata(scratch.path()).unwrap().uid();
    let log = DiagnosticLog::open_with_min_level(
        scratch.path().join("logs"),
        uid,
        clock,
        DiagnosticLevel::Debug,
    )
    .unwrap();
    log.write(
        DiagnosticLevel::Debug,
        DiagnosticEvent::Job {
            kind: "assistant-draft",
            event: "failed",
            reason: Some("assistant-backend-unavailable"),
        },
    )
    .unwrap();
    let text = fs::read_to_string(log.root().join(ACTIVE_NAME)).unwrap();
    assert!(text.contains("event=failed"));
    assert!(text.contains("reason=assistant-backend-unavailable"));
    assert_eq!(
        log.write(
            DiagnosticLevel::Debug,
            DiagnosticEvent::Job {
                kind: "assistant-draft",
                event: "running",
                reason: Some("assistant-backend-unavailable"),
            },
        ),
        Err(DiagnosticError::InvalidRecord)
    );
}

/// A server started with `ND_LOG_LEVEL=debug` writes `job`/`preparation`
/// records; the *next* startup re-validates the active segment it left
/// behind. If that validation doesn't recognize its own `job`/
/// `preparation` records, the server can never start again after a
/// single debug-level run — exactly the startup failure (then reported as
/// `diagnostic-init-failed`, now `diagnostic-record-invalid`) this guards
/// against.
#[test]
fn job_and_preparation_records_survive_a_reopen() {
    let scratch = Scratch::new("job-reopen");
    let clock = Arc::new(FixedClock::new(1));
    let uid = fs::metadata(scratch.path()).unwrap().uid();
    let log = DiagnosticLog::open_with_min_level(
        scratch.path().join("logs"),
        uid,
        Arc::clone(&clock) as Arc<dyn LogClock>,
        DiagnosticLevel::Debug,
    )
    .unwrap();
    log.write(
        DiagnosticLevel::Debug,
        DiagnosticEvent::Job {
            kind: "source-preparation",
            event: "accepted",
            reason: None,
        },
    )
    .unwrap();
    log.write(
        DiagnosticLevel::Debug,
        DiagnosticEvent::Job {
            kind: "assistant-draft",
            event: "failed",
            reason: Some("assistant-backend-unavailable"),
        },
    )
    .unwrap();
    log.write(
        DiagnosticLevel::Debug,
        DiagnosticEvent::Preparation { event: "issued" },
    )
    .unwrap();
    drop(log);
    DiagnosticLog::open_with_min_level(
        scratch.path().join("logs"),
        uid,
        clock,
        DiagnosticLevel::Debug,
    )
    .expect("a restart must be able to re-validate its own job/preparation records");
}

#[test]
fn unsafe_entries_and_unregistered_event_codes_fail_closed() {
    let scratch = Scratch::new("unsafe");
    let uid = fs::metadata(scratch.path()).unwrap().uid();
    let clock = Arc::new(FixedClock::new(1));
    let log = DiagnosticLog::open(scratch.path().join("logs"), uid, clock.clone()).unwrap();
    assert_eq!(
        log.write(
            DiagnosticLevel::Info,
            DiagnosticEvent::Lifecycle {
                component: "diary",
                event: "private"
            },
        ),
        Err(DiagnosticError::InvalidRecord)
    );
    let root = log.root().to_path_buf();
    drop(log);
    fs::write(root.join("unexpected.txt"), b"private").unwrap();
    fs::set_permissions(
        root.join("unexpected.txt"),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    assert!(matches!(
        DiagnosticLog::open(root, uid, clock),
        Err(DiagnosticError::UnrecognizedEntry)
    ));
}

fn class_of(line: &str) -> Result<RecordClass, DiagnosticError> {
    validate_line(line).map(|info| info.class)
}

const HTTP_LINE: &str = "ndlog-v1 ts_unix_ms=1 level=INFO component=http event=request duration_ms=2 outcome=ok route=health status=200";
const SNAPSHOT_LINE: &str = "ndlog-v1 ts_unix_ms=2 level=DEBUG component=assistant event=snapshot backend=none drafts=1 kind=assistant-draft language=en-US memory_sets=0 phase=start reviewed_memory=absent revision=1 selected=0 selection_digest=0 snapshots=0 sources=0 state=composing workflow=ba279216";

#[test]
fn reader_tolerates_what_a_different_build_may_have_written() {
    assert_eq!(class_of(HTTP_LINE), Ok(RecordClass::Current));
    // Key order is not significant.
    assert_eq!(
        class_of(
            "ndlog-v1 ts_unix_ms=1 level=INFO component=http event=request status=200 route=health outcome=ok duration_ms=2"
        ),
        Ok(RecordClass::Current)
    );
    // A newer build's extra key, event, component, vocabulary value, level,
    // or whole format version is accepted and only counted.
    for line in [
        format!("{HTTP_LINE} cache=hit"),
        HTTP_LINE.replace("route=health", "route=a-new-route"),
        HTTP_LINE.replace("event=request", "event=a-new-event"),
        HTTP_LINE.replace("component=http", "component=a-new-component"),
        HTTP_LINE.replace("level=INFO", "level=NOTICE"),
        HTTP_LINE.replace("ndlog-v1", "ndlog-v2"),
        "ndlog-v7 ts_unix_ms=9 level=INFO component=x event=y anything=goes".to_owned(),
    ] {
        assert_eq!(
            class_of(&line),
            Ok(RecordClass::Unrecognized),
            "rejected {line}"
        );
    }
    // A server record gaining a key is still readable.
    assert_eq!(
        class_of("ndlog-v1 ts_unix_ms=1 level=INFO component=server event=startup private=value"),
        Ok(RecordClass::Unrecognized)
    );
}

#[test]
fn every_open_failure_has_its_own_startup_token() {
    let codes = [
        DiagnosticError::UnsafePath,
        DiagnosticError::UnrecognizedEntry,
        DiagnosticError::InvalidRecord,
        DiagnosticError::Capacity,
        DiagnosticError::Io,
    ]
    .map(DiagnosticError::startup_code);
    for (index, code) in codes.iter().enumerate() {
        assert!(code.starts_with("diagnostic-"));
        assert!(validate_token(code).is_ok());
        assert!(!codes[..index].contains(code), "duplicate token {code}");
    }
}

#[test]
fn reader_still_rejects_corruption() {
    for (why, line) in [
        ("missing required key", HTTP_LINE.replace(" status=200", "")),
        ("wrong type", HTTP_LINE.replace("status=200", "status=ok")),
        (
            "negative number",
            HTTP_LINE.replace("status=200", "status=-1"),
        ),
        ("duplicate key", format!("{HTTP_LINE} status=201")),
        ("header key reused", format!("{HTTP_LINE} level=ERROR")),
        ("no equals sign", format!("{HTTP_LINE} stray")),
        (
            "free text value",
            format!("{HTTP_LINE} note=/Users/me/diary"),
        ),
        ("non-ascii value", format!("{HTTP_LINE} note=caf\u{e9}")),
        (
            "oversize token",
            format!("{HTTP_LINE} note={}", "a".repeat(65)),
        ),
        (
            "oversize record",
            format!("{HTTP_LINE} {}", "k=v ".repeat(300)),
        ),
        ("version zero", HTTP_LINE.replace("ndlog-v1", "ndlog-v0")),
        (
            "non-numeric version",
            HTTP_LINE.replace("ndlog-v1", "ndlog-vx"),
        ),
        ("unversioned", HTTP_LINE.replace("ndlog-v1", "log")),
        (
            "bad timestamp",
            HTTP_LINE.replace("ts_unix_ms=1", "ts_unix_ms=soon"),
        ),
        ("short", "ndlog-v1 ts_unix_ms=1".to_owned()),
        (
            "bad flag",
            SNAPSHOT_LINE.replace("selected=0", "selected=2"),
        ),
    ] {
        assert_eq!(
            validate_line(&line),
            Err(DiagnosticError::InvalidRecord),
            "accepted ({why}) {line}"
        );
    }
}

#[test]
fn startup_reports_what_it_validated_and_what_it_did_not_recognize() {
    let scratch = Scratch::new("validation-report");
    let uid = fs::metadata(scratch.path()).unwrap().uid();
    let root = scratch.path().join("logs");
    fs::create_dir(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    // The snapshot is a legacy shape (no draft_ranges / entry_markers).
    let lines = [
        HTTP_LINE.to_owned(),
        SNAPSHOT_LINE.to_owned(),
        format!("{HTTP_LINE} cache=hit"),
    ];
    let rotated = root.join("control-api.1.0.log");
    fs::write(&rotated, format!("{}\n", lines.join("\n"))).unwrap();
    fs::write(root.join(ACTIVE_NAME), format!("{HTTP_LINE}\n")).unwrap();
    for path in [&rotated, &root.join(ACTIVE_NAME)] {
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    }

    let log = DiagnosticLog::open_with_min_level(
        &root,
        uid,
        Arc::new(FixedClock::new(5)),
        DiagnosticLevel::Debug,
    )
    .unwrap();
    let report = log.validation_report();
    assert_eq!(
        report,
        ValidationReport {
            segments: 2,
            records: 4,
            legacy: 1,
            unrecognized: 1,
        }
    );
    log.write(DiagnosticLevel::Debug, DiagnosticEvent::Validation(report))
        .unwrap();
    let text = fs::read_to_string(root.join(ACTIVE_NAME)).unwrap();
    assert!(text.contains(
        "component=diagnostics event=validated legacy=1 records=4 segments=2 unrecognized=1"
    ));
    drop(log);
    // The summary record written above is itself valid on the next start.
    DiagnosticLog::open_with_min_level(
        &root,
        uid,
        Arc::new(FixedClock::new(6)),
        DiagnosticLevel::Debug,
    )
    .unwrap();
}

#[test]
fn rolling_retention_removes_segments_at_the_exact_cutoff() {
    let scratch = Scratch::new("retention");
    let uid = fs::metadata(scratch.path()).unwrap().uid();
    let clock = Arc::new(FixedClock::new(1));
    let log = DiagnosticLog::open(scratch.path().join("logs"), uid, clock.clone()).unwrap();
    log.write(
        DiagnosticLevel::Info,
        DiagnosticEvent::Lifecycle {
            component: "server",
            event: "startup",
        },
    )
    .unwrap();
    clock.set(DAY_MS + 1);
    log.maintain().unwrap();
    assert_eq!(fs::read_dir(log.root()).unwrap().count(), 2);
    clock.set(DAY_MS + 1 + RETENTION_MS);
    log.maintain().unwrap();
    assert_eq!(fs::read_dir(log.root()).unwrap().count(), 1);
}

#[test]
fn retention_uses_the_last_record_when_the_clock_rolled_back_at_rotation() {
    let scratch = Scratch::new("rollback-retention");
    let uid = fs::metadata(scratch.path()).unwrap().uid();
    let root = scratch.path().join("logs");
    fs::create_dir(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    let active = root.join(ACTIVE_NAME);
    fs::write(&active, b"").unwrap();
    fs::set_permissions(&active, fs::Permissions::from_mode(0o600)).unwrap();
    let segment = root.join(format!("control-api.{DAY_MS}.0.log"));
    fs::write(
        &segment,
        format!(
            "ndlog-v1 ts_unix_ms={} level=INFO component=server event=startup\n",
            2 * DAY_MS
        ),
    )
    .unwrap();
    fs::set_permissions(&segment, fs::Permissions::from_mode(0o600)).unwrap();

    maintain_with_policy(
        &root,
        uid,
        DAY_MS + RETENTION_MS,
        RETENTION_MS,
        DIRECTORY_BYTES,
    )
    .unwrap();
    assert!(segment.exists());
    maintain_with_policy(
        &root,
        uid,
        2 * DAY_MS + RETENTION_MS,
        RETENTION_MS,
        DIRECTORY_BYTES,
    )
    .unwrap();
    assert!(!segment.exists());
}

#[test]
fn directory_cap_evicts_oldest_recognized_segment_only() {
    let scratch = Scratch::new("capacity");
    let uid = fs::metadata(scratch.path()).unwrap().uid();
    let root = scratch.path().join("logs");
    fs::create_dir(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    let active = root.join(ACTIVE_NAME);
    fs::write(&active, b"").unwrap();
    fs::set_permissions(&active, fs::Permissions::from_mode(0o600)).unwrap();
    let record = b"ndlog-v1 ts_unix_ms=100 level=INFO component=server event=startup\n";
    for timestamp in [100_u64, 200, 300] {
        let path = root.join(format!("control-api.{timestamp}.0.log"));
        fs::write(&path, record).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    }
    let cap = u64::try_from(record.len() * 2).unwrap();
    maintain_with_policy(&root, uid, 400, 10_000, cap).unwrap();
    assert!(!root.join("control-api.100.0.log").exists());
    assert!(root.join("control-api.200.0.log").exists());
    assert!(root.join("control-api.300.0.log").exists());
    assert!(active.exists());
}

#[test]
fn size_rotation_clock_rollback_and_name_collision_are_safe() {
    let scratch = Scratch::new("rollback");
    let uid = fs::metadata(scratch.path()).unwrap().uid();
    let clock = Arc::new(FixedClock::new(2 * DAY_MS));
    let log = DiagnosticLog::open(scratch.path().join("logs"), uid, clock.clone()).unwrap();
    log.write(
        DiagnosticLevel::Info,
        DiagnosticEvent::Lifecycle {
            component: "server",
            event: "startup",
        },
    )
    .unwrap();
    let collision = log.root().join(format!("control-api.{}.0.log", 2 * DAY_MS));
    fs::write(
        &collision,
        b"ndlog-v1 ts_unix_ms=1 level=INFO component=server event=startup\n",
    )
    .unwrap();
    fs::set_permissions(&collision, fs::Permissions::from_mode(0o600)).unwrap();
    log.state.lock().unwrap().bytes = SEGMENT_BYTES;
    log.write(
        DiagnosticLevel::Info,
        DiagnosticEvent::Lifecycle {
            component: "server",
            event: "ready",
        },
    )
    .unwrap();
    assert!(
        log.root()
            .join(format!("control-api.{}.1.log", 2 * DAY_MS))
            .exists()
    );
    clock.set(DAY_MS);
    log.write(
        DiagnosticLevel::Info,
        DiagnosticEvent::Lifecycle {
            component: "server",
            event: "shutdown",
        },
    )
    .unwrap();
    assert!(
        fs::read_dir(log.root())
            .unwrap()
            .filter_map(Result::ok)
            .any(|entry| entry
                .file_name()
                .to_string_lossy()
                .starts_with(&format!("control-api.{DAY_MS}.")))
    );
}

#[test]
fn malformed_closed_segments_and_file_failures_stop_logging() {
    let scratch = Scratch::new("failures");
    let uid = fs::metadata(scratch.path()).unwrap().uid();
    let clock = Arc::new(FixedClock::new(100_000));
    let log = DiagnosticLog::open(scratch.path().join("logs"), uid, clock.clone()).unwrap();
    let active = log.root().join(ACTIVE_NAME);
    log.state.lock().unwrap().file = File::open(&active).unwrap();
    assert_eq!(
        log.write(
            DiagnosticLevel::Info,
            DiagnosticEvent::Lifecycle {
                component: "server",
                event: "startup"
            },
        ),
        Err(DiagnosticError::Io)
    );
    let root = log.root().to_path_buf();
    drop(log);
    let malformed = root.join("control-api.1.0.log");
    fs::write(&malformed, b"not-a-log-record\n").unwrap();
    fs::set_permissions(&malformed, fs::Permissions::from_mode(0o600)).unwrap();
    assert!(matches!(
        DiagnosticLog::open(&root, uid, clock),
        Err(DiagnosticError::InvalidRecord)
    ));
}

#[test]
fn retention_deletion_failure_is_not_silently_ignored() {
    let scratch = Scratch::new("delete-failure");
    let uid = fs::metadata(scratch.path()).unwrap().uid();
    let root = scratch.path().join("logs");
    fs::create_dir(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    for (name, contents) in [
        (ACTIVE_NAME, b"".as_slice()),
        (
            "control-api.1.0.log",
            b"ndlog-v1 ts_unix_ms=1 level=INFO component=server event=startup\n".as_slice(),
        ),
    ] {
        let path = root.join(name);
        fs::write(&path, contents).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    }
    fs::set_permissions(&root, fs::Permissions::from_mode(0o500)).unwrap();
    let result = maintain_with_policy(&root, uid, 2, 1, DIRECTORY_BYTES);
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(result, Err(DiagnosticError::Io));
}

#[test]
fn assistant_snapshot_is_content_free_tagged_and_survives_reopen() {
    use diary_core::{
        AssistantBackendId, AssistantSelection, CompositionPath, DiaryDay, Preferences, Timestamp,
    };

    let scratch = Scratch::new("assistant-snapshot");
    let uid = fs::metadata(scratch.path()).unwrap().uid();
    let root = scratch.path().join("logs");
    let log = DiagnosticLog::open_with_min_level(
        &root,
        uid,
        Arc::new(FixedClock::new(1)),
        DiagnosticLevel::Debug,
    )
    .unwrap()
    .with_tag_key([9; 32]);
    let mut workflow = Workflow::new(
        DiaryDay::parse("2026-10-30").unwrap(),
        Timestamp(0),
        &Preferences::default(),
    );
    let bare = AssistantSnapshot::capture(&log, &workflow, "assistant-draft", "start").unwrap();
    assert!(bare.backend.is_none());
    assert!(!bare.selected);
    assert_eq!(bare.reviewed_memory, "absent");
    workflow
        .discover(workflow.revision(), Vec::new(), Timestamp(1))
        .unwrap();
    workflow
        .confirm_sources(workflow.revision(), Vec::new(), true, Timestamp(2))
        .unwrap();
    workflow
        .set_assistant_selection(
            workflow.revision(),
            Some(AssistantSelection::new(
                AssistantBackendId::parse("work-profile").unwrap(),
                CompositionPath::Hosted,
            )),
            Timestamp(3),
        )
        .unwrap();
    let selected =
        AssistantSnapshot::capture(&log, &workflow, "assistant-draft", "failed").unwrap();
    assert!(selected.selected);
    assert_eq!(selected.workflow, bare.workflow);
    log.write(
        DiagnosticLevel::Debug,
        DiagnosticEvent::AssistantState(bare),
    )
    .unwrap();
    log.write(
        DiagnosticLevel::Debug,
        DiagnosticEvent::AssistantState(selected),
    )
    .unwrap();
    drop(log);

    let text = fs::read_to_string(root.join(ACTIVE_NAME)).unwrap();
    for private in ["2026-10-30", "work-profile"] {
        assert!(!text.contains(private), "{private} leaked");
    }
    assert!(text.contains("component=assistant event=snapshot"));
    assert!(text.contains("backend=none"));
    assert!(text.contains("reviewed_memory=absent"));
    assert!(text.contains("selected=1"));
    // Reopening re-validates every record, including the new shape.
    DiagnosticLog::open(&root, uid, Arc::new(FixedClock::new(2))).unwrap();
}

#[test]
fn publication_snapshots_are_content_free_and_survive_a_reopen() {
    use diary_core::{DiaryDay, Preferences, Timestamp};

    let scratch = Scratch::new("publication-snapshot");
    let uid = fs::metadata(scratch.path()).unwrap().uid();
    let root = scratch.path().join("logs");
    let log = DiagnosticLog::open_with_min_level(
        &root,
        uid,
        Arc::new(FixedClock::new(1)),
        DiagnosticLevel::Debug,
    )
    .unwrap()
    .with_tag_key([9; 32]);
    let workflow = Workflow::new(
        DiaryDay::parse("2026-10-30").unwrap(),
        Timestamp(0),
        &Preferences::default(),
    );
    let snapshot =
        PublicationSnapshot::capture(&log, &workflow, "local-publication", "failed", 1, 2).unwrap();
    assert_eq!(snapshot.local, "absent");
    log.write(
        DiagnosticLevel::Debug,
        DiagnosticEvent::PublicationState(snapshot),
    )
    .unwrap();
    let unknown = PublicationSnapshot {
        kind: "discovery",
        ..snapshot
    };
    assert!(
        log.write(
            DiagnosticLevel::Debug,
            DiagnosticEvent::PublicationState(unknown),
        )
        .is_err()
    );
    drop(log);

    let text = fs::read_to_string(root.join(ACTIVE_NAME)).unwrap();
    assert!(!text.contains("2026-10-30"));
    assert!(text.contains("component=publication event=snapshot"));
    assert!(text.contains("kind=local-publication"));
    assert!(text.contains("phase=failed"));
    assert!(text.contains("receipts=1"));
    assert!(text.contains("unresolved=2"));
    DiagnosticLog::open(&root, uid, Arc::new(FixedClock::new(2))).unwrap();
}

/// Segments written before `draft_ranges`/`entry_markers` existed stay on
/// disk and are re-validated at every startup; rejecting them was
/// the startup failure then reported as `diagnostic-init-failed`.
#[test]
fn legacy_snapshot_records_without_late_keys_still_validate() {
    assert_eq!(class_of(SNAPSHOT_LINE), Ok(RecordClass::Legacy));
    assert_eq!(validate_line(SNAPSHOT_LINE).unwrap().timestamp, 2);

    let current = SNAPSHOT_LINE.replace("drafts=1", "draft_ranges=0 drafts=1 entry_markers=0");
    assert_eq!(class_of(&current), Ok(RecordClass::Current));

    // A half-migrated shape is just another legacy shape.
    let partial = SNAPSHOT_LINE.replace("drafts=1", "draft_ranges=0 drafts=1");
    assert_eq!(class_of(&partial), Ok(RecordClass::Legacy));

    // But a missing required key is corruption, not history.
    let broken = SNAPSHOT_LINE.replace("revision=1 ", "");
    assert_eq!(class_of(&broken), Err(DiagnosticError::InvalidRecord));
}

#[test]
fn migration_records_are_closed_vocabulary_and_survive_a_reopen() {
    let scratch = Scratch::new("migration-records");
    let uid = fs::metadata(scratch.path()).unwrap().uid();
    let root = scratch.path().join("logs");
    let log = DiagnosticLog::open_with_min_level(
        &root,
        uid,
        Arc::new(FixedClock::new(1)),
        DiagnosticLevel::Debug,
    )
    .unwrap();
    for (event, reason) in [
        ("current", None),
        ("started", None),
        ("step", None),
        ("finished", None),
        ("failed", Some("step-failed")),
    ] {
        log.write(
            DiagnosticLevel::Debug,
            DiagnosticEvent::Migration {
                event,
                from: 3,
                to: 4,
                duration_ms: 7,
                reason,
            },
        )
        .unwrap();
    }
    for bad in [
        ("step", Some("step-failed")),
        ("failed", None),
        ("failed", Some("free text")),
        ("failed", Some("disk /Users/me is full")),
        ("rolled-back", None),
    ] {
        assert_eq!(
            log.write(
                DiagnosticLevel::Debug,
                DiagnosticEvent::Migration {
                    event: bad.0,
                    from: 1,
                    to: 2,
                    duration_ms: 0,
                    reason: bad.1,
                },
            ),
            Err(DiagnosticError::InvalidRecord),
            "accepted {bad:?}"
        );
    }
    let text = fs::read_to_string(root.join(ACTIVE_NAME)).unwrap();
    assert!(text.contains("component=store event=migration-step duration_ms=7 from=3 to=4"));
    assert!(text.contains(
        "component=store event=migration-failed duration_ms=7 from=3 reason=step-failed to=4"
    ));
    drop(log);

    // Every record just written must pass the next startup's validation.
    DiagnosticLog::open_with_min_level(
        &root,
        uid,
        Arc::new(FixedClock::new(2)),
        DiagnosticLevel::Debug,
    )
    .unwrap();
}

#[test]
fn assistant_snapshot_needs_a_tag_key_and_rejects_open_vocabulary() {
    use diary_core::{DiaryDay, Preferences, Timestamp};

    let scratch = Scratch::new("assistant-snapshot-reject");
    let uid = fs::metadata(scratch.path()).unwrap().uid();
    let log = DiagnosticLog::open_with_min_level(
        scratch.path().join("logs"),
        uid,
        Arc::new(FixedClock::new(1)),
        DiagnosticLevel::Debug,
    )
    .unwrap();
    let workflow = Workflow::new(
        DiaryDay::parse("2026-10-30").unwrap(),
        Timestamp(0),
        &Preferences::default(),
    );
    assert!(AssistantSnapshot::capture(&log, &workflow, "assistant-draft", "start").is_none());
    let log = log.with_tag_key([1; 32]);
    let mut snapshot =
        AssistantSnapshot::capture(&log, &workflow, "assistant-draft", "start").unwrap();
    snapshot.kind = "discovery";
    assert!(
        log.write(
            DiagnosticLevel::Debug,
            DiagnosticEvent::AssistantState(snapshot)
        )
        .is_err()
    );
    snapshot.kind = "assistant-draft";
    snapshot.state = "2026-10-30";
    assert!(
        log.write(
            DiagnosticLevel::Debug,
            DiagnosticEvent::AssistantState(snapshot)
        )
        .is_err()
    );
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "nd-diagnostics-{label}-{}-{}",
            std::process::id(),
            SCRATCH_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
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
