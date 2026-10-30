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
    /// The outcome of one destination's read-only readiness check, as closed
    /// tokens only: never a path, identifier, or response text.
    PublishReadiness {
        destination: &'static str,
        readiness: &'static str,
        reason: &'static str,
        next_step: &'static str,
        /// The check that settled the answer, so a surprising result can be
        /// traced to a gate rather than guessed at.
        decided_by: &'static str,
        duration_ms: u64,
    },
    /// One readiness request as a whole: which prerequisites held and how
    /// long the destinations took. The workflow is a keyed tag, never a day.
    ReadinessCheck {
        workflow: DiagnosticTag,
        revision: u64,
        local_verified: bool,
        notion_configured: bool,
        icloud_configured: bool,
        duration_ms: u64,
    },
    /// A readiness request that was refused before any destination ran.
    ReadinessRefused { reason: &'static str },
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
        DiagnosticEvent::PublishReadiness {
            destination,
            readiness,
            reason,
            next_step,
            decided_by,
            duration_ms,
        } => {
            if !READINESS_DESTINATIONS.contains(&destination)
                || !READINESS_STATES.contains(&readiness)
                || !READINESS_REASONS.contains(&reason)
                || !READINESS_NEXT_STEPS.contains(&next_step)
                || !READINESS_DECIDERS.contains(&decided_by)
            {
                return Err(DiagnosticError::InvalidRecord);
            }
            (
                "publish",
                "readiness",
                vec![
                    ("decided_by", decided_by.to_owned()),
                    ("destination", destination.to_owned()),
                    ("duration_ms", duration_ms.to_string()),
                    ("next_step", next_step.to_owned()),
                    ("readiness", readiness.to_owned()),
                    ("reason", reason.to_owned()),
                ],
            )
        }
        DiagnosticEvent::ReadinessCheck {
            workflow,
            revision,
            local_verified,
            notion_configured,
            icloud_configured,
            duration_ms,
        } => {
            let flag = |value: bool| u8::from(value).to_string();
            (
                "publish",
                "readiness-check",
                vec![
                    ("duration_ms", duration_ms.to_string()),
                    ("icloud_configured", flag(icloud_configured)),
                    ("local_verified", flag(local_verified)),
                    ("notion_configured", flag(notion_configured)),
                    ("revision", revision.to_string()),
                    ("workflow", workflow.as_str().to_owned()),
                ],
            )
        }
        DiagnosticEvent::ReadinessRefused { reason } => {
            if !READINESS_REFUSALS.contains(&reason) {
                return Err(DiagnosticError::InvalidRecord);
            }
            (
                "publish",
                "readiness-refused",
                vec![("reason", reason.to_owned())],
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

/// Destinations a readiness check reports on.
pub const READINESS_DESTINATIONS: &[&str] = &["notion", "icloud"];

/// Readiness states a destination can report.
pub const READINESS_STATES: &[&str] = &["ready", "not-ready", "conflict", "published"];

/// Closed reason tokens for a readiness record and response; free text can
/// never reach the log or the client.
pub const READINESS_REASONS: &[&str] = &[
    "none",
    "already-published",
    "icloud-different-file-exists",
    "icloud-folder-unavailable",
    "icloud-not-configured",
    "local-file-not-verified",
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

/// Closed next-step tokens a readiness record and response can carry.
pub const READINESS_NEXT_STEPS: &[&str] = &[
    "none",
    "publish-local-first",
    "configure-notion",
    "fix-notion-access",
    "check-notion-page",
    "retry-later",
    "configure-icloud",
    "check-icloud-folder",
    "resolve-icloud-file",
    "preview-and-approve",
    "publish-approved",
];

/// The check that settled a destination's answer.
pub const READINESS_DECIDERS: &[&str] = &[
    "local-gate",
    "published",
    "not-configured",
    "preflight-error",
    "preflight-status",
    "mirror-inspect",
];

/// Why a readiness request was refused before any destination ran.
pub const READINESS_REFUSALS: &[&str] = &[
    "stale-revision",
    "reconciling",
    "not-found",
    "state-unavailable",
];

fn vocab_readiness_next_step(value: &str) -> bool {
    READINESS_NEXT_STEPS.contains(&value)
}

fn vocab_readiness_decider(value: &str) -> bool {
    READINESS_DECIDERS.contains(&value)
}

fn vocab_readiness_refusal(value: &str) -> bool {
    READINESS_REFUSALS.contains(&value)
}

fn vocab_readiness_destination(value: &str) -> bool {
    READINESS_DESTINATIONS.contains(&value)
}

fn vocab_readiness_state(value: &str) -> bool {
    READINESS_STATES.contains(&value)
}

fn vocab_readiness_reason(value: &str) -> bool {
    READINESS_REASONS.contains(&value)
}

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
const READINESS_SPEC: &[KeySpec] = &[
    required("decided_by", ValueKind::Vocab(vocab_readiness_decider)),
    required("destination", ValueKind::Vocab(vocab_readiness_destination)),
    required("duration_ms", ValueKind::Number),
    required("next_step", ValueKind::Vocab(vocab_readiness_next_step)),
    required("readiness", ValueKind::Vocab(vocab_readiness_state)),
    required("reason", ValueKind::Vocab(vocab_readiness_reason)),
];
const READINESS_CHECK_SPEC: &[KeySpec] = &[
    required("duration_ms", ValueKind::Number),
    required("icloud_configured", ValueKind::Flag),
    required("local_verified", ValueKind::Flag),
    required("notion_configured", ValueKind::Flag),
    required("revision", ValueKind::Number),
    required("workflow", ValueKind::Tag),
];
const READINESS_REFUSED_SPEC: &[KeySpec] = &[required(
    "reason",
    ValueKind::Vocab(vocab_readiness_refusal),
)];
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
        ("publish", "readiness") => Some(READINESS_SPEC),
        ("publish", "readiness-check") => Some(READINESS_CHECK_SPEC),
        ("publish", "readiness-refused") => Some(READINESS_REFUSED_SPEC),
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
mod tests;
