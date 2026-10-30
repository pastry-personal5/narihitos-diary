use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use diary_core::Workflow;
use diary_core::workflow::{INFERRED_REFLECTION_CLOSE, INFERRED_REFLECTION_OPEN};

use crate::diagnostic_tags::{DiagnosticTag, DiagnosticTagger};

const ACTIVE_NAME: &str = "control-api.log";
const MAX_RECORD_BYTES: usize = 1024;
const SEGMENT_BYTES: u64 = 8 * 1024 * 1024;
const DIRECTORY_BYTES: u64 = 24 * 1024 * 1024;
const RETENTION_MS: u64 = 72 * 60 * 60 * 1000;
const DAY_MS: u64 = 24 * 60 * 60 * 1000;

pub trait LogClock: Send + Sync {
    fn now_unix_ms(&self) -> u64;
}

/// Ordered Trace..Error so `<`/`>=` compare severity; `DiagnosticLog` uses
/// this to filter records below its configured minimum level. Declaration
/// order is the severity order — keep them in sync.
#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord)]
pub enum DiagnosticLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl DiagnosticLevel {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Trace => "TRACE",
            Self::Debug => "DEBUG",
            Self::Info => "INFO",
            Self::Warn => "WARN",
            Self::Error => "ERROR",
        }
    }

    /// Parses the `ND_LOG_LEVEL` value (case-insensitive); unrecognized
    /// input is the caller's problem, not a silent fallback here.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "trace" => Some(Self::Trace),
            "debug" => Some(Self::Debug),
            "info" => Some(Self::Info),
            "warn" => Some(Self::Warn),
            "error" => Some(Self::Error),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiagnosticEvent {
    Lifecycle {
        component: &'static str,
        event: &'static str,
    },
    Http {
        route: &'static str,
        outcome: &'static str,
        status: u16,
        duration_ms: u64,
    },
    Capacity {
        component: &'static str,
        event: &'static str,
        current: u64,
        limit: u64,
    },
    /// A job's lifecycle transition. `kind` is the job type (e.g.
    /// `source-preparation`); never the workflow day, source, or job ID.
    /// `reason` is set only on `event: "failed"`, and only to a fixed
    /// `&'static str` error code such as `CommandError::code()` or a literal
    /// defined alongside `fail_job` — never request data, free-form error
    /// text, or anything else that didn't originate as a closed, compiled-in
    /// token.
    Job {
        kind: &'static str,
        event: &'static str,
        reason: Option<&'static str>,
    },
    /// A preparation handle's lifecycle: issued when a prepare job completes,
    /// consumed when `confirm_selection` redeems it, and `expired` or
    /// `binding-mismatch` when a later use fails — exactly the detail needed
    /// to diagnose a `preparation-expired`/`preparation-binding-changed`
    /// rejection. Never the handle value itself, workflow day, or source IDs.
    Preparation { event: &'static str },
    /// The outcome of a Notion-only preview: `status` is the destination
    /// status and `reason` a closed `notion-*` token (or `none`/`other`),
    /// never a title, identifier, or response text.
    NotionPreview {
        status: &'static str,
        reason: &'static str,
    },
    /// A content-free picture of a workflow at an assistant job's start or
    /// failure, so a rejection such as `reviewed-memory-required` can be
    /// matched to the state that caused it. Only closed tokens, booleans,
    /// counts, and keyed [`DiagnosticTag`]s — never the day, a source, a
    /// backend alias, or any text.
    AssistantState(AssistantSnapshot),
    /// A content-free picture of a day's publication state when a publication
    /// job ends, so a workflow stuck in `publishing` can be matched to its
    /// per-destination statuses, receipts, and unresolved intents. Closed
    /// tokens, counts, and a keyed [`DiagnosticTag`] only.
    PublicationState(PublicationSnapshot),
    /// Private-state schema migration progress at startup: `current`,
    /// `started`, `step`, `finished`, or `failed`. `from`/`to` are schema
    /// versions; `reason` is set only on `failed`, and only to a compiled-in
    /// token (never SQL text, a path, or stored data).
    /// Startup's tally of the retained segments it validated; counts only.
    Validation(ValidationReport),
    Migration {
        event: &'static str,
        from: u32,
        to: u32,
        duration_ms: u64,
        reason: Option<&'static str>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AssistantSnapshot {
    /// One of the four `assistant-*` job kinds.
    pub kind: &'static str,
    /// `start` or `failed`.
    pub event_status: &'static str,
    pub workflow: DiagnosticTag,
    /// `None` when no assistant is selected.
    pub backend: Option<DiagnosticTag>,
    /// `WorkflowState::as_str`.
    pub state: &'static str,
    pub revision: u64,
    pub selected: bool,
    pub selection_digest: bool,
    /// `none`, `en-US`, or `ko-KR`.
    pub language: &'static str,
    /// `absent`, `current`, or `stale`.
    pub reviewed_memory: &'static str,
    pub memory_sets: u64,
    pub drafts: u64,
    /// Reflection ranges in the latest draft: nonzero means its stored text
    /// carries markers.
    pub draft_ranges: u64,
    /// Reflection-marker characters in the entry buffer: nonzero means a
    /// paragraph review target can contain markers.
    pub entry_markers: u64,
    pub sources: u64,
    pub snapshots: u64,
}

impl AssistantSnapshot {
    /// Summarizes `workflow` for `kind`/`event_status`. `None` when `log` has no tag
    /// key, because an untagged snapshot could not be correlated.
    #[must_use]
    pub fn capture(
        log: &DiagnosticLog,
        workflow: &Workflow,
        kind: &'static str,
        event_status: &'static str,
    ) -> Option<Self> {
        let selection = workflow.assistant_selection();
        Some(Self {
            kind,
            event_status,
            workflow: log.tag("workflow", workflow.day().as_str().as_bytes())?,
            backend: match selection {
                Some(selection) => {
                    Some(log.tag("backend", selection.backend_id.as_str().as_bytes())?)
                }
                None => None,
            },
            state: workflow.state().as_str(),
            revision: workflow.revision(),
            selected: selection.is_some(),
            selection_digest: workflow.selection_digest().is_some(),
            language: workflow
                .language()
                .map_or("none", |language| language.as_str()),
            reviewed_memory: match workflow.reviewed_memory_artifact() {
                None => "absent",
                Some(artifact) if artifact.stale => "stale",
                Some(_) => "current",
            },
            memory_sets: workflow.memory_candidate_sets().len() as u64,
            drafts: workflow.draft_proposals().len() as u64,
            draft_ranges: workflow
                .draft_proposals()
                .last()
                .map_or(0, |draft| draft.reflection_ranges.len() as u64),
            entry_markers: workflow
                .entry()
                .text
                .chars()
                .filter(|c| matches!(*c, INFERRED_REFLECTION_OPEN | INFERRED_REFLECTION_CLOSE))
                .count() as u64,
            sources: workflow.candidates().len() as u64,
            snapshots: workflow.snapshots().len() as u64,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PublicationSnapshot {
    /// One of the publication job kinds.
    pub kind: &'static str,
    /// `finished` or `failed`.
    pub event_status: &'static str,
    pub workflow: DiagnosticTag,
    /// `WorkflowState::as_str`.
    pub state: &'static str,
    pub revision: u64,
    /// `DestinationStatus::as_str`, or `absent` when the plan has no such
    /// destination.
    pub local: &'static str,
    pub notion: &'static str,
    pub icloud: &'static str,
    pub receipts: u64,
    /// Operation intents without a terminal outcome: nonzero gates the day.
    pub unresolved: u64,
}

impl PublicationSnapshot {
    /// `None` when `log` has no tag key, because an untagged snapshot could
    /// not be correlated.
    #[must_use]
    pub fn capture(
        log: &DiagnosticLog,
        workflow: &Workflow,
        kind: &'static str,
        event_status: &'static str,
        receipts: u64,
        unresolved: u64,
    ) -> Option<Self> {
        let status = |destination: &str| {
            workflow
                .destination_progress()
                .iter()
                .find(|item| item.destination == destination)
                .map_or("absent", |item| item.status.as_str())
        };
        Some(Self {
            kind,
            event_status,
            workflow: log.tag("workflow", workflow.day().as_str().as_bytes())?,
            state: workflow.state().as_str(),
            revision: workflow.revision(),
            local: status("local"),
            notion: status("notion"),
            icloud: status("icloud"),
            receipts,
            unresolved,
        })
    }
}

const ASSISTANT_KINDS: [&str; 4] = [
    "assistant-memory-extraction",
    "assistant-draft",
    "assistant-paragraph-review",
    "assistant-source-refine",
];
const WORKFLOW_STATES: [&str; 10] = [
    "started",
    "discovered",
    "sources-confirmed",
    "composing",
    "entry-confirmed",
    "output-planned",
    "approved",
    "publishing",
    "completed",
    "partial",
];
/// The closed `reason` vocabulary of a `migration-failed` record, mirrored
/// from the store's `MigrationEvent::Failed`.
const MIGRATION_FAILURE_REASONS: [&str; 4] = [
    "unsupported-schema",
    "step-failed",
    "migration-gap",
    "schema-validation-failed",
];

pub struct DiagnosticLog {
    root: PathBuf,
    expected_uid: u32,
    clock: Arc<dyn LogClock>,
    min_level: DiagnosticLevel,
    tagger: Option<DiagnosticTagger>,
    report: ValidationReport,
    state: Mutex<State>,
}

#[derive(Debug)]
struct State {
    file: File,
    bytes: u64,
    day: u64,
    last_timestamp: u64,
    sequence: u32,
}

impl DiagnosticLog {
    pub fn open(
        root: impl AsRef<Path>,
        expected_uid: u32,
        clock: Arc<dyn LogClock>,
    ) -> Result<Self, DiagnosticError> {
        Self::open_with_min_level(root, expected_uid, clock, DiagnosticLevel::Info)
    }

    /// Like [`Self::open`], but records below `min_level` are dropped before
    /// they're written (e.g. pass `DiagnosticLevel::Debug` to see job and
    /// preparation lifecycle events; the default via `open` is `Info`, which
    /// matches every level this log previously ever wrote).
    pub fn open_with_min_level(
        root: impl AsRef<Path>,
        expected_uid: u32,
        clock: Arc<dyn LogClock>,
        min_level: DiagnosticLevel,
    ) -> Result<Self, DiagnosticError> {
        let root = root.as_ref().to_path_buf();
        ensure_private_directory(&root, expected_uid)?;
        let mut report = ValidationReport::default();
        scan_directory_entries(&root, expected_uid, &mut report)?;
        maintain(&root, expected_uid, clock.now_unix_ms())?;
        let active = root.join(ACTIVE_NAME);
        let mut file = open_private_regular(&active, expected_uid)?;
        let (bytes, last_timestamp) = repair_and_validate_active(&mut file, &mut report)?;
        let now = clock.now_unix_ms();
        let day = if bytes == 0 {
            now / DAY_MS
        } else {
            last_timestamp / DAY_MS
        };
        file.seek(SeekFrom::End(0))
            .map_err(|_| DiagnosticError::Io)?;
        Ok(Self {
            root,
            expected_uid,
            clock,
            min_level,
            tagger: None,
            report,
            state: Mutex::new(State {
                file,
                bytes,
                day,
                last_timestamp,
                sequence: 0,
            }),
        })
    }

    /// Enables [`Self::tag`] with the persistent private key.
    #[must_use]
    pub fn with_tag_key(mut self, key: [u8; 32]) -> Self {
        self.tagger = Some(DiagnosticTagger::new(key));
        self
    }

    /// What startup found while validating the retained segments: how many
    /// were scanned, and how many records were `legacy` (older shape) or
    /// `unrecognized` (newer or unknown to this build). A nonzero
    /// `unrecognized` after a downgrade is expected and harmless.
    #[must_use]
    pub const fn validation_report(&self) -> ValidationReport {
        self.report
    }

    /// A keyed correlation tag for `value`, or `None` when no key was
    /// installed (tags are then simply omitted, never faked).
    #[must_use]
    pub fn tag(&self, domain: &'static str, value: &[u8]) -> Option<DiagnosticTag> {
        self.tagger.as_ref().map(|tagger| tagger.tag(domain, value))
    }

    /// Whether a record at `level` would be written, letting callers skip the
    /// work of building one.
    #[must_use]
    pub fn enabled(&self, level: DiagnosticLevel) -> bool {
        level >= self.min_level
    }

    pub fn write(
        &self,
        level: DiagnosticLevel,
        event: DiagnosticEvent,
    ) -> Result<(), DiagnosticError> {
        if level < self.min_level {
            return Ok(());
        }
        let now = self.clock.now_unix_ms();
        let record = encode(now, level, event)?;
        let mut state = self.state.lock().map_err(|_| DiagnosticError::Io)?;
        let new_day = now / DAY_MS;
        if state.bytes > 0
            && (new_day != state.day || state.bytes + record.len() as u64 > SEGMENT_BYTES)
        {
            rotate(&self.root, self.expected_uid, &mut state, now)?;
        }
        state
            .file
            .write_all(record.as_bytes())
            .map_err(|_| DiagnosticError::Io)?;
        state.file.flush().map_err(|_| DiagnosticError::Io)?;
        state.bytes += record.len() as u64;
        state.day = new_day;
        state.last_timestamp = now;
        Ok(())
    }

    pub fn maintain(&self) -> Result<(), DiagnosticError> {
        let now = self.clock.now_unix_ms();
        let mut state = self.state.lock().map_err(|_| DiagnosticError::Io)?;
        if state.bytes > 0 && state.day != now / DAY_MS {
            rotate(&self.root, self.expected_uid, &mut state, now)?;
        }
        maintain(&self.root, self.expected_uid, now)
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }
}

fn encode(
    now: u64,
    level: DiagnosticLevel,
    event: DiagnosticEvent,
) -> Result<String, DiagnosticError> {
    let (component, event_code, fields) = match event {
        DiagnosticEvent::Lifecycle { component, event } => {
            if component != "server" || !matches!(event, "startup" | "ready" | "shutdown") {
                return Err(DiagnosticError::InvalidRecord);
            }
            (component, event, Vec::new())
        }
        DiagnosticEvent::Http {
            route,
            outcome,
            status,
            duration_ms,
        } => {
            if !valid_http_route(route)
                || !valid_http_outcome(outcome)
                || !(100..=599).contains(&status)
            {
                return Err(DiagnosticError::InvalidRecord);
            }
            (
                "http",
                "request",
                vec![
                    ("duration_ms", duration_ms.to_string()),
                    ("outcome", outcome.to_owned()),
                    ("route", route.to_owned()),
                    ("status", status.to_string()),
                ],
            )
        }
        DiagnosticEvent::Capacity {
            component,
            event,
            current,
            limit,
        } => {
            if !matches!(component, "jobs" | "events" | "requests" | "diagnostics")
                || !matches!(event, "saturated" | "maintenance")
            {
                return Err(DiagnosticError::InvalidRecord);
            }
            (
                component,
                event,
                vec![
                    ("current", current.to_string()),
                    ("limit", limit.to_string()),
                ],
            )
        }
        DiagnosticEvent::Job {
            kind,
            event,
            reason,
        } => {
            if !matches!(
                kind,
                "discovery"
                    | "source-preparation"
                    | "assistant-memory-extraction"
                    | "assistant-draft"
                    | "assistant-paragraph-review"
                    | "assistant-source-refine"
                    | "local-publication"
                    | "notion-publication"
                    | "amendment-publication"
                    | "amendment-notion-retry"
                    | "startup-recovery-local"
                    | "startup-recovery-mirror"
                    | "startup-recovery-notion"
            ) || !matches!(
                event,
                "accepted" | "running" | "completed" | "failed" | "cancelled"
            ) || (reason.is_some() && event != "failed")
            {
                return Err(DiagnosticError::InvalidRecord);
            }
            let mut fields = vec![("kind", kind.to_owned())];
            if let Some(reason) = reason {
                fields.push(("reason", reason.to_owned()));
            }
            ("job", event, fields)
        }
        DiagnosticEvent::Validation(report) => (
            "diagnostics",
            "validated",
            vec![
                ("legacy", report.legacy.to_string()),
                ("records", report.records.to_string()),
                ("segments", report.segments.to_string()),
                ("unrecognized", report.unrecognized.to_string()),
            ],
        ),
        DiagnosticEvent::Migration {
            event,
            from,
            to,
            duration_ms,
            reason,
        } => {
            if !matches!(
                event,
                "current" | "started" | "step" | "finished" | "failed"
            ) || reason.is_some() != (event == "failed")
                || reason.is_some_and(|reason| !MIGRATION_FAILURE_REASONS.contains(&reason))
            {
                return Err(DiagnosticError::InvalidRecord);
            }
            let mut fields = vec![
                ("duration_ms", duration_ms.to_string()),
                ("from", from.to_string()),
            ];
            if let Some(reason) = reason {
                fields.push(("reason", reason.to_owned()));
            }
            fields.push(("to", to.to_string()));
            (
                "store",
                match event {
                    "current" => "migration-current",
                    "started" => "migration-started",
                    "step" => "migration-step",
                    "finished" => "migration-finished",
                    _ => "migration-failed",
                },
                fields,
            )
        }
        DiagnosticEvent::Preparation { event } => {
            if !matches!(
                event,
                "issued" | "consumed" | "expired" | "binding-mismatch"
            ) {
                return Err(DiagnosticError::InvalidRecord);
            }
            ("preparation", event, Vec::new())
        }
        DiagnosticEvent::NotionPreview { status, reason } => {
            if !NOTION_PREVIEW_STATUSES.contains(&status)
                || !NOTION_PREVIEW_REASONS.contains(&reason)
            {
                return Err(DiagnosticError::InvalidRecord);
            }
            (
                "notion",
                "preview",
                vec![("reason", reason.to_owned()), ("status", status.to_owned())],
            )
        }
        DiagnosticEvent::PublicationState(snapshot) => {
            if !vocab_publication_kind(snapshot.kind)
                || !matches!(snapshot.event_status, "finished" | "failed")
                || !WORKFLOW_STATES.contains(&snapshot.state)
                || ![snapshot.local, snapshot.notion, snapshot.icloud]
                    .into_iter()
                    .all(vocab_destination_status)
            {
                return Err(DiagnosticError::InvalidRecord);
            }
            (
                "publication",
                "snapshot",
                vec![
                    ("icloud", snapshot.icloud.to_owned()),
                    ("kind", snapshot.kind.to_owned()),
                    ("local", snapshot.local.to_owned()),
                    ("notion", snapshot.notion.to_owned()),
                    ("phase", snapshot.event_status.to_owned()),
                    ("receipts", snapshot.receipts.to_string()),
                    ("revision", snapshot.revision.to_string()),
                    ("state", snapshot.state.to_owned()),
                    ("unresolved", snapshot.unresolved.to_string()),
                    ("workflow", snapshot.workflow.as_str().to_owned()),
                ],
            )
        }
        DiagnosticEvent::AssistantState(snapshot) => {
            if !ASSISTANT_KINDS.contains(&snapshot.kind)
                || !matches!(snapshot.event_status, "start" | "failed")
                || !WORKFLOW_STATES.contains(&snapshot.state)
                || !matches!(snapshot.language, "none" | "en-US" | "ko-KR")
                || !matches!(snapshot.reviewed_memory, "absent" | "current" | "stale")
            {
                return Err(DiagnosticError::InvalidRecord);
            }
            let flag = |value: bool| u8::from(value).to_string();
            (
                "assistant",
                "snapshot",
                vec![
                    (
                        "backend",
                        snapshot
                            .backend
                            .map_or_else(|| "none".to_owned(), |tag| tag.as_str().to_owned()),
                    ),
                    ("draft_ranges", snapshot.draft_ranges.to_string()),
                    ("drafts", snapshot.drafts.to_string()),
                    ("entry_markers", snapshot.entry_markers.to_string()),
                    ("kind", snapshot.kind.to_owned()),
                    ("language", snapshot.language.to_owned()),
                    ("memory_sets", snapshot.memory_sets.to_string()),
                    ("phase", snapshot.event_status.to_owned()),
                    ("reviewed_memory", snapshot.reviewed_memory.to_owned()),
                    ("revision", snapshot.revision.to_string()),
                    ("selected", flag(snapshot.selected)),
                    ("selection_digest", flag(snapshot.selection_digest)),
                    ("snapshots", snapshot.snapshots.to_string()),
                    ("sources", snapshot.sources.to_string()),
                    ("state", snapshot.state.to_owned()),
                    ("workflow", snapshot.workflow.as_str().to_owned()),
                ],
            )
        }
    };
    for token in [component, event_code] {
        validate_token(token)?;
    }
    let mut line = format!(
        "ndlog-v1 ts_unix_ms={now} level={} component={component} event={event_code}",
        level.as_str()
    );
    for (key, value) in fields {
        validate_token(key)?;
        validate_token(&value)?;
        line.push(' ');
        line.push_str(key);
        line.push('=');
        line.push_str(&value);
    }
    line.push('\n');
    if line.len() > MAX_RECORD_BYTES {
        return Err(DiagnosticError::InvalidRecord);
    }
    Ok(line)
}

fn validate_token(value: &str) -> Result<(), DiagnosticError> {
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:-".contains(&byte))
    {
        Err(DiagnosticError::InvalidRecord)
    } else {
        Ok(())
    }
}

fn ensure_private_directory(path: &Path, expected_uid: u32) -> Result<(), DiagnosticError> {
    if !path.exists() {
        fs::create_dir(path).map_err(|_| DiagnosticError::Io)?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|_| DiagnosticError::Io)?;
    }
    let metadata = fs::symlink_metadata(path).map_err(|_| DiagnosticError::Io)?;
    if !metadata.file_type().is_dir()
        || metadata.file_type().is_symlink()
        || metadata.uid() != expected_uid
        || metadata.mode() & 0o077 != 0
    {
        return Err(DiagnosticError::UnsafePath);
    }
    Ok(())
}

fn open_private_regular(path: &Path, expected_uid: u32) -> Result<File, DiagnosticError> {
    if path.exists()
        && fs::symlink_metadata(path)
            .map_err(|_| DiagnosticError::Io)?
            .file_type()
            .is_symlink()
    {
        return Err(DiagnosticError::UnsafePath);
    }
    let file = OpenOptions::new()
        .read(true)
        .append(true)
        .create(true)
        .mode(0o600)
        .open(path)
        .map_err(|_| DiagnosticError::Io)?;
    let metadata = file.metadata().map_err(|_| DiagnosticError::Io)?;
    let path_metadata = fs::symlink_metadata(path).map_err(|_| DiagnosticError::Io)?;
    if !metadata.file_type().is_file()
        || metadata.uid() != expected_uid
        || metadata.nlink() != 1
        || metadata.mode() & 0o077 != 0
        || path_metadata.file_type().is_symlink()
        || path_metadata.dev() != metadata.dev()
        || path_metadata.ino() != metadata.ino()
    {
        return Err(DiagnosticError::UnsafePath);
    }
    Ok(file)
}

fn validate_directory_entries(root: &Path, expected_uid: u32) -> Result<(), DiagnosticError> {
    scan_directory_entries(root, expected_uid, &mut ValidationReport::default())
}

fn scan_directory_entries(
    root: &Path,
    expected_uid: u32,
    report: &mut ValidationReport,
) -> Result<(), DiagnosticError> {
    for entry in fs::read_dir(root).map_err(|_| DiagnosticError::Io)? {
        let entry = entry.map_err(|_| DiagnosticError::Io)?;
        let name = entry.file_name();
        let name = name.to_str().ok_or(DiagnosticError::UnsafePath)?;
        if name != ACTIVE_NAME && parse_rotated(name).is_none() {
            return Err(DiagnosticError::UnrecognizedEntry);
        }
        let metadata = fs::symlink_metadata(entry.path()).map_err(|_| DiagnosticError::Io)?;
        if !metadata.file_type().is_file()
            || metadata.file_type().is_symlink()
            || metadata.uid() != expected_uid
            || metadata.nlink() != 1
            || metadata.mode() & 0o077 != 0
        {
            return Err(DiagnosticError::UnsafePath);
        }
        if name != ACTIVE_NAME {
            let _ = scan_closed_segment(&entry.path(), report)?;
        }
    }
    Ok(())
}

fn validate_closed_segment(path: &Path) -> Result<u64, DiagnosticError> {
    scan_closed_segment(path, &mut ValidationReport::default())
}

fn scan_closed_segment(path: &Path, report: &mut ValidationReport) -> Result<u64, DiagnosticError> {
    let mut file = File::open(path).map_err(|_| DiagnosticError::Io)?;
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(SEGMENT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| DiagnosticError::Io)?;
    if bytes.is_empty() || bytes.len() as u64 > SEGMENT_BYTES || bytes.last() != Some(&b'\n') {
        return Err(DiagnosticError::InvalidRecord);
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| DiagnosticError::InvalidRecord)?;
    report.segments += 1;
    let mut last_timestamp = 0;
    for line in text.lines() {
        let info = validate_line(line)?;
        report.note(info.class);
        last_timestamp = last_timestamp.max(info.timestamp);
    }
    Ok(last_timestamp)
}

fn repair_and_validate_active(
    file: &mut File,
    report: &mut ValidationReport,
) -> Result<(u64, u64), DiagnosticError> {
    file.seek(SeekFrom::Start(0))
        .map_err(|_| DiagnosticError::Io)?;
    let mut bytes = Vec::new();
    Read::by_ref(file)
        .take(SEGMENT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| DiagnosticError::Io)?;
    if bytes.len() as u64 > SEGMENT_BYTES {
        return Err(DiagnosticError::InvalidRecord);
    }
    if let Some(last_newline) = bytes.iter().rposition(|byte| *byte == b'\n') {
        if last_newline + 1 < bytes.len() {
            bytes.truncate(last_newline + 1);
            file.set_len(bytes.len() as u64)
                .map_err(|_| DiagnosticError::Io)?;
        }
    } else if !bytes.is_empty() {
        bytes.clear();
        file.set_len(0).map_err(|_| DiagnosticError::Io)?;
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| DiagnosticError::InvalidRecord)?;
    report.segments += 1;
    let mut last = 0;
    for line in text.lines() {
        let info = validate_line(line)?;
        report.note(info.class);
        last = info.timestamp;
    }
    Ok((bytes.len() as u64, last))
}

/// How a record that passed validation relates to what this build writes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RecordClass {
    /// Matches a record this build writes.
    Current,
    /// A known record that predates one or more of its optional keys.
    Legacy,
    /// Well-formed but not fully understood by this build: a newer `ndlog`
    /// version, an unknown component or event, an unknown key, or a value
    /// outside a vocabulary this build knows.
    Unrecognized,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct LineInfo {
    timestamp: u64,
    class: RecordClass,
}

/// Counts gathered while startup validates every retained segment; see
/// [`DiagnosticLog::validation_report`].
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ValidationReport {
    pub segments: u64,
    pub records: u64,
    pub legacy: u64,
    pub unrecognized: u64,
}

impl ValidationReport {
    fn note(&mut self, class: RecordClass) {
        self.records += 1;
        match class {
            RecordClass::Current => {}
            RecordClass::Legacy => self.legacy += 1,
            RecordClass::Unrecognized => self.unrecognized += 1,
        }
    }
}

/// What a key's value must look like. Types are part of the format contract
/// (changing one needs a new `ndlog` version); vocabularies are not, so a
/// value outside a [`ValueKind::Vocab`] is tolerated and counted as
/// unrecognized rather than rejected.
#[derive(Clone, Copy)]
enum ValueKind {
    Number,
    Flag,
    Tag,
    TagOrNone,
    Token,
    Vocab(fn(&str) -> bool),
}

enum ValueCheck {
    Valid,
    Unknown,
    Invalid,
}

impl ValueKind {
    fn check(self, value: &str) -> ValueCheck {
        let valid = match self {
            Self::Number => value.parse::<u64>().is_ok(),
            Self::Flag => matches!(value, "0" | "1"),
            Self::Tag => valid_tag(value),
            Self::TagOrNone => value == "none" || valid_tag(value),
            Self::Token => true,
            Self::Vocab(known) => {
                return if known(value) {
                    ValueCheck::Valid
                } else {
                    ValueCheck::Unknown
                };
            }
        };
        if valid {
            ValueCheck::Valid
        } else {
            ValueCheck::Invalid
        }
    }
}

struct KeySpec {
    name: &'static str,
    kind: ValueKind,
    required: bool,
}

const fn required(name: &'static str, kind: ValueKind) -> KeySpec {
    KeySpec {
        name,
        kind,
        required: true,
    }
}

const fn optional(name: &'static str, kind: ValueKind) -> KeySpec {
    KeySpec {
        name,
        kind,
        required: false,
    }
}

/// Destination statuses a Notion preview can report.
pub const NOTION_PREVIEW_STATUSES: &[&str] = &["pending", "blocked", "conflict"];

/// Closed reason tokens for a Notion preview record; `other` absorbs any
/// token this build does not list so free text can never reach the log.
pub const NOTION_PREVIEW_REASONS: &[&str] = &[
    "none",
    "other",
    "notion-config-invalid",
    "notion-credential-invalid",
    "notion-hierarchy-ambiguous",
    "notion-not-configured",
    "notion-permission-denied",
    "notion-remote-unavailable",
    "notion-render-invalid",
    "notion-response-invalid",
    "notion-retry-deferred",
    "notion-root-unverifiable",
    "notion-same-day-conflict",
    "notion-unavailable",
    "notion-write-ambiguous",
];

fn vocab_notion_status(value: &str) -> bool {
    NOTION_PREVIEW_STATUSES.contains(&value)
}

fn vocab_notion_reason(value: &str) -> bool {
    NOTION_PREVIEW_REASONS.contains(&value)
}

fn vocab_job_kind(value: &str) -> bool {
    matches!(
        value,
        "discovery"
            | "source-preparation"
            | "local-publication"
            | "notion-publication"
            | "amendment-publication"
            | "amendment-notion-retry"
            | "startup-recovery-local"
            | "startup-recovery-mirror"
            | "startup-recovery-notion"
    ) || ASSISTANT_KINDS.contains(&value)
}

fn vocab_publication_kind(value: &str) -> bool {
    matches!(
        value,
        "local-publication"
            | "notion-publication"
            | "amendment-publication"
            | "amendment-notion-retry"
    )
}

fn vocab_destination_status(value: &str) -> bool {
    matches!(
        value,
        "absent" | "pending" | "blocked" | "conflict" | "verified" | "reconciling"
    )
}

fn vocab_publication_status(value: &str) -> bool {
    matches!(value, "finished" | "failed")
}

fn vocab_assistant_kind(value: &str) -> bool {
    ASSISTANT_KINDS.contains(&value)
}

fn vocab_event_status(value: &str) -> bool {
    matches!(value, "start" | "failed")
}

fn vocab_state(value: &str) -> bool {
    WORKFLOW_STATES.contains(&value)
}

fn vocab_language(value: &str) -> bool {
    matches!(value, "none" | "en-US" | "ko-KR")
}

fn vocab_reviewed_memory(value: &str) -> bool {
    matches!(value, "absent" | "current" | "stale")
}

fn vocab_migration_reason(value: &str) -> bool {
    MIGRATION_FAILURE_REASONS.contains(&value)
}

const HTTP_SPEC: &[KeySpec] = &[
    required("duration_ms", ValueKind::Number),
    required("outcome", ValueKind::Vocab(valid_http_outcome)),
    required("route", ValueKind::Vocab(valid_http_route)),
    required("status", ValueKind::Number),
];
const CAPACITY_SPEC: &[KeySpec] = &[
    required("current", ValueKind::Number),
    required("limit", ValueKind::Number),
];
const JOB_SPEC: &[KeySpec] = &[
    required("kind", ValueKind::Vocab(vocab_job_kind)),
    optional("reason", ValueKind::Token),
];
const SNAPSHOT_SPEC: &[KeySpec] = &[
    required("backend", ValueKind::TagOrNone),
    required("drafts", ValueKind::Number),
    required("kind", ValueKind::Vocab(vocab_assistant_kind)),
    required("language", ValueKind::Vocab(vocab_language)),
    required("memory_sets", ValueKind::Number),
    required("phase", ValueKind::Vocab(vocab_event_status)),
    required("reviewed_memory", ValueKind::Vocab(vocab_reviewed_memory)),
    required("revision", ValueKind::Number),
    required("selected", ValueKind::Flag),
    required("selection_digest", ValueKind::Flag),
    required("snapshots", ValueKind::Number),
    required("sources", ValueKind::Number),
    required("state", ValueKind::Vocab(vocab_state)),
    required("workflow", ValueKind::Tag),
    // Added after the record first shipped; older segments omit them.
    optional("draft_ranges", ValueKind::Number),
    optional("entry_markers", ValueKind::Number),
];
const PUBLICATION_SPEC: &[KeySpec] = &[
    required("icloud", ValueKind::Vocab(vocab_destination_status)),
    required("kind", ValueKind::Vocab(vocab_publication_kind)),
    required("local", ValueKind::Vocab(vocab_destination_status)),
    required("notion", ValueKind::Vocab(vocab_destination_status)),
    required("phase", ValueKind::Vocab(vocab_publication_status)),
    required("receipts", ValueKind::Number),
    required("revision", ValueKind::Number),
    required("state", ValueKind::Vocab(vocab_state)),
    required("unresolved", ValueKind::Number),
    required("workflow", ValueKind::Tag),
];
const NOTION_PREVIEW_SPEC: &[KeySpec] = &[
    required("reason", ValueKind::Vocab(vocab_notion_reason)),
    required("status", ValueKind::Vocab(vocab_notion_status)),
];
const MIGRATION_SPEC: &[KeySpec] = &[
    required("duration_ms", ValueKind::Number),
    required("from", ValueKind::Number),
    required("to", ValueKind::Number),
    optional("reason", ValueKind::Vocab(vocab_migration_reason)),
];
const VALIDATION_SPEC: &[KeySpec] = &[
    required("legacy", ValueKind::Number),
    required("records", ValueKind::Number),
    required("segments", ValueKind::Number),
    required("unrecognized", ValueKind::Number),
];

/// The keys of every record this build writes, by `(component, event)`.
fn record_spec(component: &str, event: &str) -> Option<&'static [KeySpec]> {
    match (component, event) {
        ("server", "startup" | "ready" | "shutdown")
        | ("preparation", "issued" | "consumed" | "expired" | "binding-mismatch") => Some(&[]),
        ("http", "request") => Some(HTTP_SPEC),
        ("jobs" | "events" | "requests" | "diagnostics", "saturated" | "maintenance") => {
            Some(CAPACITY_SPEC)
        }
        ("job", "accepted" | "running" | "completed" | "failed" | "cancelled") => Some(JOB_SPEC),
        ("assistant", "snapshot") => Some(SNAPSHOT_SPEC),
        ("publication", "snapshot") => Some(PUBLICATION_SPEC),
        ("notion", "preview") => Some(NOTION_PREVIEW_SPEC),
        (
            "store",
            "migration-current" | "migration-started" | "migration-step" | "migration-finished"
            | "migration-failed",
        ) => Some(MIGRATION_SPEC),
        ("diagnostics", "validated") => Some(VALIDATION_SPEC),
        _ => None,
    }
}

/// Validates one stored record. The writer ([`encode`]) is strict and only
/// emits the closed vocabulary above; this reader runs at every startup over
/// segments that an older or newer build may have written, so it is tolerant:
///
/// * Always rejected (corruption): over-long lines, a malformed header, a
///   non-token key or value, duplicate keys, and, for a record this build
///   knows, a missing required key or a value of the wrong type.
/// * Tolerated and counted as `unrecognized`: a newer `ndlog` version, an
///   unknown component, event, or level, unknown extra keys, and values
///   outside a known vocabulary. Key order is not significant.
/// * Tolerated and counted as `legacy`: a known record missing optional keys.
///
/// Token charset and size limits still hold for everything accepted, so the
/// no-free-text guarantee does not depend on this reader recognizing a record.
fn validate_line(line: &str) -> Result<LineInfo, DiagnosticError> {
    if line.len() + 1 > MAX_RECORD_BYTES {
        return Err(DiagnosticError::InvalidRecord);
    }
    let fields: Vec<_> = line.split(' ').collect();
    if fields.len() < 5 {
        return Err(DiagnosticError::InvalidRecord);
    }
    let version = fields[0]
        .strip_prefix("ndlog-v")
        .filter(|digits| (1..=4).contains(&digits.len()))
        .filter(|digits| digits.bytes().all(|byte| byte.is_ascii_digit()))
        .and_then(|digits| digits.parse::<u32>().ok())
        .filter(|version| *version >= 1)
        .ok_or(DiagnosticError::InvalidRecord)?;
    let timestamp = fields[1]
        .strip_prefix("ts_unix_ms=")
        .and_then(|value| value.parse().ok())
        .ok_or(DiagnosticError::InvalidRecord)?;
    let level = fields[2]
        .strip_prefix("level=")
        .ok_or(DiagnosticError::InvalidRecord)?;
    let component = fields[3]
        .strip_prefix("component=")
        .ok_or(DiagnosticError::InvalidRecord)?;
    let event = fields[4]
        .strip_prefix("event=")
        .ok_or(DiagnosticError::InvalidRecord)?;
    for value in [level, component, event] {
        validate_token(value)?;
    }
    let mut pairs: Vec<(&str, &str)> = Vec::with_capacity(fields.len() - 5);
    for field in &fields[5..] {
        let (key, value) = field
            .split_once('=')
            .ok_or(DiagnosticError::InvalidRecord)?;
        validate_token(key)?;
        validate_token(value)?;
        if matches!(key, "ts_unix_ms" | "level" | "component" | "event")
            || pairs.iter().any(|(seen, _)| *seen == key)
        {
            return Err(DiagnosticError::InvalidRecord);
        }
        pairs.push((key, value));
    }

    let mut unrecognized =
        version != 1 || !matches!(level, "TRACE" | "DEBUG" | "INFO" | "WARN" | "ERROR");
    let mut legacy = false;
    if version == 1 {
        match record_spec(component, event) {
            None => unrecognized = true,
            Some(spec) => {
                for key in spec {
                    match pairs.iter().find(|(name, _)| *name == key.name) {
                        Some((_, value)) => match key.kind.check(value) {
                            ValueCheck::Valid => {}
                            ValueCheck::Unknown => unrecognized = true,
                            ValueCheck::Invalid => return Err(DiagnosticError::InvalidRecord),
                        },
                        None if key.required => return Err(DiagnosticError::InvalidRecord),
                        None => legacy = true,
                    }
                }
                if pairs
                    .iter()
                    .any(|(name, _)| !spec.iter().any(|key| key.name == *name))
                {
                    unrecognized = true;
                }
            }
        }
    }
    let class = if unrecognized {
        RecordClass::Unrecognized
    } else if legacy {
        RecordClass::Legacy
    } else {
        RecordClass::Current
    };
    Ok(LineInfo { timestamp, class })
}

fn valid_http_route(route: &str) -> bool {
    matches!(
        route,
        "health"
            | "capabilities"
            | "setup"
            | "preferences"
            | "workflows"
            | "workflow"
            | "discovery"
            | "sources"
            | "source"
            | "preparation"
            | "selection"
            | "settings"
            | "draft-editor"
            | "entry"
            | "confirmation"
            | "job"
            | "events"
            | "unknown"
    )
}

fn valid_tag(value: &str) -> bool {
    value.len() == 8
        && value
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

fn valid_http_outcome(outcome: &str) -> bool {
    matches!(
        outcome,
        "ok" | "client-error" | "denied" | "busy" | "failed"
    )
}

fn rotate(
    root: &Path,
    expected_uid: u32,
    state: &mut State,
    now: u64,
) -> Result<(), DiagnosticError> {
    state.file.flush().map_err(|_| DiagnosticError::Io)?;
    state.file.sync_all().map_err(|_| DiagnosticError::Io)?;
    loop {
        let name = format!("control-api.{now}.{}.log", state.sequence);
        state.sequence = state.sequence.saturating_add(1);
        let target = root.join(name);
        match fs::hard_link(root.join(ACTIVE_NAME), &target) {
            Ok(()) => {
                fs::remove_file(root.join(ACTIVE_NAME)).map_err(|_| DiagnosticError::Io)?;
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => return Err(DiagnosticError::Io),
        }
    }
    state.file = open_private_regular(&root.join(ACTIVE_NAME), expected_uid)?;
    state.bytes = 0;
    state.day = now / DAY_MS;
    state.last_timestamp = 0;
    maintain(root, expected_uid, now)
}

fn maintain(root: &Path, expected_uid: u32, now: u64) -> Result<(), DiagnosticError> {
    maintain_with_policy(root, expected_uid, now, RETENTION_MS, DIRECTORY_BYTES)
}

fn maintain_with_policy(
    root: &Path,
    expected_uid: u32,
    now: u64,
    retention_ms: u64,
    directory_bytes: u64,
) -> Result<(), DiagnosticError> {
    validate_directory_entries(root, expected_uid)?;
    let cutoff = now.saturating_sub(retention_ms);
    let mut closed = Vec::new();
    let mut total = 0_u64;
    for entry in fs::read_dir(root).map_err(|_| DiagnosticError::Io)? {
        let entry = entry.map_err(|_| DiagnosticError::Io)?;
        let metadata = entry.metadata().map_err(|_| DiagnosticError::Io)?;
        total = total.saturating_add(metadata.len());
        let name = entry.file_name();
        let name = name.to_str().ok_or(DiagnosticError::UnsafePath)?;
        if let Some(timestamp) = parse_rotated(name) {
            let last_record = validate_closed_segment(&entry.path())?;
            closed.push((timestamp.max(last_record), entry.path(), metadata.len()));
        }
    }
    closed.sort_by_key(|(timestamp, path, _)| (*timestamp, path.clone()));
    for (timestamp, path, size) in &closed {
        if *timestamp <= cutoff {
            fs::remove_file(path).map_err(|_| DiagnosticError::Io)?;
            total = total.saturating_sub(*size);
        }
    }
    for (timestamp, path, size) in closed {
        if timestamp > cutoff && total > directory_bytes && path.exists() {
            fs::remove_file(path).map_err(|_| DiagnosticError::Io)?;
            total = total.saturating_sub(size);
        }
    }
    if total > directory_bytes {
        return Err(DiagnosticError::Capacity);
    }
    Ok(())
}

fn parse_rotated(name: &str) -> Option<u64> {
    let middle = name.strip_prefix("control-api.")?.strip_suffix(".log")?;
    let (timestamp, sequence) = middle.split_once('.')?;
    if sequence.is_empty() || !sequence.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    timestamp.parse().ok()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiagnosticError {
    UnsafePath,
    UnrecognizedEntry,
    InvalidRecord,
    Capacity,
    Io,
}

impl DiagnosticError {
    /// The startup failure token for this error. Opening the log has several
    /// distinct ways to fail; a single catch-all token hid which one (for
    /// example a stray file in the log directory) from whoever read it.
    #[must_use]
    pub const fn startup_code(self) -> &'static str {
        match self {
            Self::UnsafePath => "diagnostic-path-unsafe",
            Self::UnrecognizedEntry => "diagnostic-unrecognized-entry",
            Self::InvalidRecord => "diagnostic-record-invalid",
            Self::Capacity => "diagnostic-capacity-exceeded",
            Self::Io => "diagnostic-io-failed",
        }
    }
}

#[cfg(test)]
mod tests {
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
            class_of(
                "ndlog-v1 ts_unix_ms=1 level=INFO component=server event=startup private=value"
            ),
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
            AssistantBackendId, AssistantSelection, CompositionPath, DiaryDay, Preferences,
            Timestamp,
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
            PublicationSnapshot::capture(&log, &workflow, "local-publication", "failed", 1, 2)
                .unwrap();
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
}
