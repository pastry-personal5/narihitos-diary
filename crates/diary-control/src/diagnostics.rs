use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

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
}

pub struct DiagnosticLog {
    root: PathBuf,
    expected_uid: u32,
    clock: Arc<dyn LogClock>,
    min_level: DiagnosticLevel,
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
        validate_directory_entries(&root, expected_uid)?;
        maintain(&root, expected_uid, clock.now_unix_ms())?;
        let active = root.join(ACTIVE_NAME);
        let mut file = open_private_regular(&active, expected_uid)?;
        let (bytes, last_timestamp) = repair_and_validate_active(&mut file)?;
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
            state: Mutex::new(State {
                file,
                bytes,
                day,
                last_timestamp,
                sequence: 0,
            }),
        })
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
        DiagnosticEvent::Preparation { event } => {
            if !matches!(
                event,
                "issued" | "consumed" | "expired" | "binding-mismatch"
            ) {
                return Err(DiagnosticError::InvalidRecord);
            }
            ("preparation", event, Vec::new())
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
            let _ = validate_closed_segment(&entry.path())?;
        }
    }
    Ok(())
}

fn validate_closed_segment(path: &Path) -> Result<u64, DiagnosticError> {
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
    let mut last_timestamp = 0;
    for line in text.lines() {
        last_timestamp = last_timestamp.max(validate_line(line)?);
    }
    Ok(last_timestamp)
}

fn repair_and_validate_active(file: &mut File) -> Result<(u64, u64), DiagnosticError> {
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
    let mut last = 0;
    for line in text.lines() {
        last = validate_line(line)?;
    }
    Ok((bytes.len() as u64, last))
}

fn validate_line(line: &str) -> Result<u64, DiagnosticError> {
    if line.len() + 1 > MAX_RECORD_BYTES {
        return Err(DiagnosticError::InvalidRecord);
    }
    let fields: Vec<_> = line.split(' ').collect();
    if fields.len() < 5 || fields[0] != "ndlog-v1" {
        return Err(DiagnosticError::InvalidRecord);
    }
    let timestamp = fields[1]
        .strip_prefix("ts_unix_ms=")
        .and_then(|value| value.parse().ok())
        .ok_or(DiagnosticError::InvalidRecord)?;
    if !matches!(
        fields[2],
        "level=TRACE" | "level=DEBUG" | "level=INFO" | "level=WARN" | "level=ERROR"
    ) {
        return Err(DiagnosticError::InvalidRecord);
    }
    for field in fields.iter().skip(3) {
        let (key, value) = field
            .split_once('=')
            .ok_or(DiagnosticError::InvalidRecord)?;
        validate_token(key)?;
        validate_token(value)?;
    }
    let component = fields[3]
        .strip_prefix("component=")
        .ok_or(DiagnosticError::InvalidRecord)?;
    let event = fields[4]
        .strip_prefix("event=")
        .ok_or(DiagnosticError::InvalidRecord)?;
    match (component, event) {
        ("server", "startup" | "ready" | "shutdown") if fields.len() == 5 => {}
        ("http", "request") if fields.len() == 9 => {
            fields[5]
                .strip_prefix("duration_ms=")
                .and_then(|value| value.parse::<u64>().ok())
                .ok_or(DiagnosticError::InvalidRecord)?;
            let outcome = fields[6]
                .strip_prefix("outcome=")
                .filter(|value| valid_http_outcome(value))
                .ok_or(DiagnosticError::InvalidRecord)?;
            let route = fields[7]
                .strip_prefix("route=")
                .filter(|value| valid_http_route(value))
                .ok_or(DiagnosticError::InvalidRecord)?;
            fields[8]
                .strip_prefix("status=")
                .and_then(|value| value.parse::<u16>().ok())
                .filter(|status| (100..=599).contains(status))
                .ok_or(DiagnosticError::InvalidRecord)?;
            let _ = (outcome, route);
        }
        ("jobs" | "events" | "requests" | "diagnostics", "saturated" | "maintenance")
            if fields.len() == 7 =>
        {
            fields[5]
                .strip_prefix("current=")
                .and_then(|value| value.parse::<u64>().ok())
                .ok_or(DiagnosticError::InvalidRecord)?;
            fields[6]
                .strip_prefix("limit=")
                .and_then(|value| value.parse::<u64>().ok())
                .ok_or(DiagnosticError::InvalidRecord)?;
        }
        ("job", "accepted" | "running" | "completed" | "failed" | "cancelled")
            if matches!(fields.len(), 6 | 7) =>
        {
            fields[5]
                .strip_prefix("kind=")
                .filter(|value| {
                    matches!(
                        *value,
                        "discovery"
                            | "source-preparation"
                            | "assistant-memory-extraction"
                            | "assistant-draft"
                            | "assistant-paragraph-review"
                            | "assistant-source-refine"
                    )
                })
                .ok_or(DiagnosticError::InvalidRecord)?;
            if let Some(field) = fields.get(6) {
                if event != "failed" {
                    return Err(DiagnosticError::InvalidRecord);
                }
                field
                    .strip_prefix("reason=")
                    .ok_or(DiagnosticError::InvalidRecord)?;
            }
        }
        ("preparation", "issued" | "consumed" | "expired" | "binding-mismatch")
            if fields.len() == 5 => {}
        _ => return Err(DiagnosticError::InvalidRecord),
    }
    Ok(timestamp)
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
    /// single debug-level run — exactly the `diagnostic-init-failed`
    /// failure this guards against.
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

    #[test]
    fn record_validation_rejects_unknown_or_reordered_fields() {
        assert!(validate_line(
            "ndlog-v1 ts_unix_ms=1 level=INFO component=http event=request duration_ms=2 outcome=ok route=health status=200"
        )
        .is_ok());
        assert_eq!(
            validate_line(
                "ndlog-v1 ts_unix_ms=1 level=INFO component=http event=request outcome=ok duration_ms=2 route=health status=200"
            ),
            Err(DiagnosticError::InvalidRecord)
        );
        assert_eq!(
            validate_line(
                "ndlog-v1 ts_unix_ms=1 level=INFO component=server event=startup private=value"
            ),
            Err(DiagnosticError::InvalidRecord)
        );
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
