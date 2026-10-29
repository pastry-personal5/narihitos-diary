//! Bounded client for the one-request/one-response Swift helper.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use diary_core::{DiaryDay, SourceAccessError, SourceRef, Timestamp};

use crate::ayumi::MAX_SOURCE_BYTES;

const MAX_FRAME_BYTES: usize = 64 * 1024;
const MAX_DIAGNOSTIC_BYTES: usize = 4 * 1024;
const CONNECTOR: &str = "ayumi";
static REQUEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BridgeAvailability {
    Resident,
    CloudOnly,
    Missing,
    PermissionDenied,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BridgeRead {
    pub bytes: Vec<u8>,
    pub file_identity: String,
    pub size: u64,
    pub modified_at: Timestamp,
}

#[derive(Clone, Debug)]
pub struct AppleBridgeClient {
    executable: PathBuf,
    directory_name: String,
    handoff_directory: PathBuf,
    timeout: Duration,
    concurrency: Arc<Mutex<usize>>,
    maximum_concurrency: usize,
}

impl AppleBridgeClient {
    pub fn locate_executable(
        nd_executable: &Path,
        workspace_root: &Path,
    ) -> Result<PathBuf, SourceAccessError> {
        if let Some(override_path) = std::env::var_os("ND_APPLE_BRIDGE_PATH") {
            if override_path.is_empty() {
                return Err(SourceAccessError::InvalidConfiguration);
            }
            let path = PathBuf::from(override_path);
            return path
                .is_file()
                .then_some(path)
                .ok_or(SourceAccessError::HelperMissing);
        }
        let installed = nd_executable
            .parent()
            .and_then(Path::parent)
            .map(|prefix| prefix.join("libexec/apple-bridge"));
        if let Some(path) = installed.filter(|path| path.is_file()) {
            return Ok(path);
        }
        let development = workspace_root.join("helpers/apple-bridge/.build/debug/apple-bridge");
        development
            .is_file()
            .then_some(development)
            .ok_or(SourceAccessError::HelperMissing)
    }

    pub fn new(
        executable: impl Into<PathBuf>,
        directory_name: impl Into<String>,
        handoff_directory: impl Into<PathBuf>,
        timeout: Duration,
        maximum_concurrency: usize,
    ) -> Result<Self, SourceAccessError> {
        let directory_name = directory_name.into();
        if !valid_directory_name(&directory_name) || maximum_concurrency == 0 {
            return Err(SourceAccessError::InvalidConfiguration);
        }
        let handoff_directory = handoff_directory.into();
        prepare_handoff_directory(&handoff_directory)?;
        let client = Self {
            executable: executable.into(),
            directory_name,
            handoff_directory,
            timeout,
            concurrency: Arc::new(Mutex::new(0)),
            maximum_concurrency,
        };
        client.cleanup_orphans()?;
        Ok(client)
    }

    pub fn availability(
        &self,
        source: &SourceRef,
        cancelled: &AtomicBool,
    ) -> Result<BridgeAvailability, SourceAccessError> {
        let value = self.invoke("icloud-availability", source, None, cancelled)?;
        match object_string(&value, "status")? {
            "resident" => Ok(BridgeAvailability::Resident),
            "cloud-only" => Ok(BridgeAvailability::CloudOnly),
            "missing" => Ok(BridgeAvailability::Missing),
            "permission-denied" => Ok(BridgeAvailability::PermissionDenied),
            _ => Err(SourceAccessError::HelperProtocol),
        }
    }

    pub fn request_download(
        &self,
        source: &SourceRef,
        cancelled: &AtomicBool,
    ) -> Result<(), SourceAccessError> {
        let value = self.invoke("icloud-request-download", source, None, cancelled)?;
        match object_string(&value, "status")? {
            "resident" => Ok(()),
            "cloud-only" => Err(SourceAccessError::NeedsDownload),
            "permission-denied" => Err(SourceAccessError::PermissionDenied),
            _ => Err(SourceAccessError::HelperProtocol),
        }
    }

    pub fn coordinated_read(
        &self,
        source: &SourceRef,
        cancelled: &AtomicBool,
    ) -> Result<BridgeRead, SourceAccessError> {
        let request_directory = self.create_request_directory()?;
        let _directory_cleanup = HandoffDirectoryCleanup(request_directory.clone());
        let value = self.invoke(
            "icloud-coordinated-read",
            source,
            Some(&request_directory),
            cancelled,
        )?;
        let handoff_name = object_string(&value, "handoff_name")?;
        if !valid_handoff_name(handoff_name) {
            return Err(SourceAccessError::HelperProtocol);
        }
        let size = object_u64(&value, "size")?;
        if size > MAX_SOURCE_BYTES {
            return Err(SourceAccessError::Oversized);
        }
        let file_identity = object_string(&value, "file_identity")?.to_owned();
        if file_identity.is_empty() {
            return Err(SourceAccessError::HelperProtocol);
        }
        let modified_at = Timestamp(object_i64(&value, "modified_at")?);
        let path = request_directory.join(handoff_name);
        let _cleanup = HandoffCleanup(path.clone());
        let directory_metadata =
            fs::symlink_metadata(&request_directory).map_err(|_| SourceAccessError::Unavailable)?;
        let root_metadata = fs::symlink_metadata(&self.handoff_directory)
            .map_err(|_| SourceAccessError::Unavailable)?;
        if !directory_metadata.is_dir()
            || directory_metadata.file_type().is_symlink()
            || directory_metadata.uid() != root_metadata.uid()
            || directory_metadata.permissions().mode() & 0o777 != 0o700
        {
            return Err(SourceAccessError::HelperProtocol);
        }
        let metadata =
            fs::symlink_metadata(&path).map_err(|_| SourceAccessError::HelperProtocol)?;
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || metadata.uid() != directory_metadata.uid()
            || metadata.nlink() != 1
            || metadata.permissions().mode() & 0o777 != 0o600
            || metadata.len() != size
        {
            return Err(SourceAccessError::HelperProtocol);
        }
        let mut file = File::open(&path).map_err(|_| SourceAccessError::HelperProtocol)?;
        let opened = file
            .metadata()
            .map_err(|_| SourceAccessError::HelperProtocol)?;
        if handoff_identity(&metadata) != handoff_identity(&opened) {
            return Err(SourceAccessError::HelperProtocol);
        }
        let mut bytes = Vec::with_capacity(size as usize);
        Read::by_ref(&mut file)
            .take(MAX_SOURCE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| SourceAccessError::Unavailable)?;
        let after_opened = file
            .metadata()
            .map_err(|_| SourceAccessError::HelperProtocol)?;
        let after_path =
            fs::symlink_metadata(&path).map_err(|_| SourceAccessError::HelperProtocol)?;
        if bytes.len() as u64 != size
            || bytes.len() as u64 > MAX_SOURCE_BYTES
            || handoff_identity(&metadata) != handoff_identity(&after_opened)
            || handoff_identity(&metadata) != handoff_identity(&after_path)
        {
            return Err(SourceAccessError::HelperProtocol);
        }
        Ok(BridgeRead {
            bytes,
            file_identity,
            size,
            modified_at,
        })
    }

    fn create_request_directory(&self) -> Result<PathBuf, SourceAccessError> {
        for _ in 0..16 {
            let sequence = REQUEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = self
                .handoff_directory
                .join(format!("nd-handoff-{}-{sequence}", std::process::id()));
            let mut builder = fs::DirBuilder::new();
            builder.mode(0o700);
            match builder.create(&path) {
                Ok(()) => return Ok(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(_) => return Err(SourceAccessError::Unavailable),
            }
        }
        Err(SourceAccessError::Unavailable)
    }

    fn invoke(
        &self,
        command: &str,
        source: &SourceRef,
        handoff_directory: Option<&Path>,
        cancelled: &AtomicBool,
    ) -> Result<JsonValue, SourceAccessError> {
        validate_source(source)?;
        let _permit = Permit::acquire(&self.concurrency, self.maximum_concurrency)?;
        let request_id = format!("nd-{}", REQUEST_SEQUENCE.fetch_add(1, Ordering::Relaxed));
        let mut payload = format!(
            "{{\"directory_name\":{},\"source_id\":{}",
            json_string(&self.directory_name),
            json_string(&source.source_id)
        );
        if let Some(directory) = handoff_directory {
            let raw = directory
                .to_str()
                .ok_or(SourceAccessError::InvalidConfiguration)?;
            payload.push_str(&format!(
                ",\"handoff_directory\":{},\"maximum_bytes\":{}",
                json_string(raw),
                MAX_SOURCE_BYTES
            ));
        } else if command == "icloud-request-download" {
            let helper_timeout = self.timeout.saturating_sub(Duration::from_millis(100));
            payload.push_str(&format!(
                ",\"timeout_milliseconds\":{}",
                helper_timeout.as_millis().clamp(1, 60_000)
            ));
        }
        payload.push('}');
        let frame = format!(
            "{{\"protocol_version\":1,\"request_id\":{},\"command\":{},\"payload\":{payload}}}\n",
            json_string(&request_id),
            json_string(command)
        );
        if frame.len() > MAX_FRAME_BYTES {
            return Err(SourceAccessError::InvalidConfiguration);
        }
        let mut child = Command::new(&self.executable)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| {
                if error.kind() == std::io::ErrorKind::NotFound {
                    SourceAccessError::HelperMissing
                } else {
                    SourceAccessError::Unavailable
                }
            })?;
        child
            .stdin
            .take()
            .ok_or(SourceAccessError::HelperCrashed)?
            .write_all(frame.as_bytes())
            .map_err(|_| SourceAccessError::HelperCrashed)?;
        let started = Instant::now();
        loop {
            if cancelled.load(Ordering::Relaxed) {
                let _ = child.kill();
                let _ = child.wait();
                return Err(SourceAccessError::Cancelled);
            }
            if started.elapsed() >= self.timeout {
                let _ = child.kill();
                let _ = child.wait();
                return Err(SourceAccessError::TimedOut);
            }
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) => std::thread::sleep(Duration::from_millis(5)),
                Err(_) => return Err(SourceAccessError::HelperCrashed),
            }
        }
        let output = child
            .wait_with_output()
            .map_err(|_| SourceAccessError::HelperCrashed)?;
        if output.stdout.len() > MAX_FRAME_BYTES || output.stderr.len() > MAX_DIAGNOSTIC_BYTES {
            return Err(SourceAccessError::HelperProtocol);
        }
        if output.stdout.is_empty() && !output.status.success() {
            return Err(SourceAccessError::HelperCrashed);
        }
        parse_response(&output.stdout, &request_id)
    }

    fn cleanup_orphans(&self) -> Result<(), SourceAccessError> {
        let directory = fs::symlink_metadata(&self.handoff_directory)
            .map_err(|_| SourceAccessError::Unavailable)?;
        for entry in
            fs::read_dir(&self.handoff_directory).map_err(|_| SourceAccessError::Unavailable)?
        {
            let Ok(entry) = entry else { continue };
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let Ok(metadata) = fs::symlink_metadata(entry.path()) else {
                continue;
            };
            if valid_handoff_name(&name)
                && metadata.is_file()
                && !metadata.file_type().is_symlink()
                && metadata.uid() == directory.uid()
                && metadata.permissions().mode() & 0o777 == 0o600
            {
                let _ = fs::remove_file(entry.path());
            } else if metadata.is_dir()
                && !metadata.file_type().is_symlink()
                && metadata.uid() == directory.uid()
                && metadata.permissions().mode() & 0o777 == 0o700
                && name.starts_with("nd-handoff-")
            {
                cleanup_handoff_directory(&entry.path());
            }
        }
        Ok(())
    }
}

fn prepare_handoff_directory(path: &Path) -> Result<(), SourceAccessError> {
    if !path.exists() {
        fs::create_dir_all(path).map_err(|_| SourceAccessError::InvalidConfiguration)?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|_| SourceAccessError::InvalidConfiguration)?;
    }
    let metadata =
        fs::symlink_metadata(path).map_err(|_| SourceAccessError::InvalidConfiguration)?;
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || metadata.permissions().mode() & 0o777 != 0o700
    {
        return Err(SourceAccessError::InvalidConfiguration);
    }
    Ok(())
}

fn valid_directory_name(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && !value.contains(['/', '\\', '\0'])
        && Path::new(value).components().count() == 1
}

fn validate_source(source: &SourceRef) -> Result<(), SourceAccessError> {
    if source.connector != CONNECTOR {
        return Err(SourceAccessError::InvalidConfiguration);
    }
    let parts: Vec<_> = Path::new(&source.source_id).components().collect();
    let [
        Component::Normal(year),
        Component::Normal(month),
        Component::Normal(day),
        Component::Normal(file),
    ] = parts.as_slice()
    else {
        return Err(SourceAccessError::InvalidConfiguration);
    };
    let Some(file) = file.to_str() else {
        return Err(SourceAccessError::InvalidConfiguration);
    };
    let valid_date_shape = [year, month, day].iter().all(|part| {
        part.to_str()
            .is_some_and(|value| value.bytes().all(|b| b.is_ascii_digit()))
    });
    let valid_date = valid_date_shape
        && DiaryDay::parse(format!(
            "{}-{}-{}",
            year.to_string_lossy(),
            month.to_string_lossy(),
            day.to_string_lossy()
        ))
        .is_ok();
    if !valid_date || file.starts_with('.') || !file.ends_with(".md") {
        return Err(SourceAccessError::InvalidConfiguration);
    }
    Ok(())
}

fn valid_handoff_name(value: &str) -> bool {
    value.starts_with("nd-source-")
        && value.ends_with(".bin")
        && Path::new(value).components().count() == 1
        && matches!(
            Path::new(value).components().next(),
            Some(Component::Normal(_))
        )
}

fn handoff_identity(metadata: &fs::Metadata) -> (u64, u64, u64, i64, i64) {
    (
        metadata.dev(),
        metadata.ino(),
        metadata.len(),
        metadata.mtime(),
        metadata.mtime_nsec(),
    )
}

struct Permit {
    counter: Arc<Mutex<usize>>,
}

impl Permit {
    fn acquire(counter: &Arc<Mutex<usize>>, maximum: usize) -> Result<Self, SourceAccessError> {
        let mut active = counter.lock().map_err(|_| SourceAccessError::Unavailable)?;
        if *active >= maximum {
            return Err(SourceAccessError::Busy);
        }
        *active += 1;
        Ok(Self {
            counter: Arc::clone(counter),
        })
    }
}

impl Drop for Permit {
    fn drop(&mut self) {
        if let Ok(mut active) = self.counter.lock() {
            *active = active.saturating_sub(1);
        }
    }
}

struct HandoffCleanup(PathBuf);

impl Drop for HandoffCleanup {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

struct HandoffDirectoryCleanup(PathBuf);

impl Drop for HandoffDirectoryCleanup {
    fn drop(&mut self) {
        cleanup_handoff_directory(&self.0);
    }
}

fn cleanup_handoff_directory(path: &Path) {
    let Ok(directory) = fs::symlink_metadata(path) else {
        return;
    };
    if !directory.is_dir() || directory.file_type().is_symlink() {
        return;
    }
    let Ok(entries) = fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let Ok(metadata) = fs::symlink_metadata(entry.path()) else {
            continue;
        };
        if valid_handoff_name(&name)
            && metadata.is_file()
            && !metadata.file_type().is_symlink()
            && metadata.nlink() == 1
            && metadata.uid() == directory.uid()
        {
            let _ = fs::remove_file(entry.path());
        }
    }
    let _ = fs::remove_dir(path);
}

fn json_string(value: &str) -> String {
    let mut output = String::from("\"");
    for character in value.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            value if value.is_control() => output.push_str(&format!("\\u{:04x}", value as u32)),
            value => output.push(value),
        }
    }
    output.push('"');
    output
}

#[derive(Clone, Debug, PartialEq)]
enum JsonValue {
    Null,
    Bool(bool),
    Number(i64),
    String(String),
    Object(BTreeMap<String, JsonValue>),
}

fn parse_response(bytes: &[u8], request_id: &str) -> Result<JsonValue, SourceAccessError> {
    if bytes.last() != Some(&b'\n') || bytes[..bytes.len().saturating_sub(1)].contains(&b'\n') {
        return Err(SourceAccessError::HelperProtocol);
    }
    let text = std::str::from_utf8(&bytes[..bytes.len() - 1])
        .map_err(|_| SourceAccessError::HelperProtocol)?;
    let root = JsonParser::new(text).parse()?;
    let object = as_object(&root)?;
    if object_i64(&root, "protocol_version")? != 1
        || object_string(&root, "request_id")? != request_id
    {
        return Err(SourceAccessError::HelperProtocol);
    }
    match object.get("ok") {
        Some(JsonValue::Bool(true)) => object
            .get("value")
            .cloned()
            .ok_or(SourceAccessError::HelperProtocol),
        Some(JsonValue::Bool(false)) => {
            let error = object
                .get("error")
                .ok_or(SourceAccessError::HelperProtocol)?;
            match object_string(error, "code")? {
                "cancelled" => Err(SourceAccessError::Cancelled),
                "permission_denied" => Err(SourceAccessError::PermissionDenied),
                "needs_download" => Err(SourceAccessError::NeedsDownload),
                "not_found" => Err(SourceAccessError::NotFound),
                "oversize" => Err(SourceAccessError::Oversized),
                "unsupported_version" | "malformed_input" | "unknown_command" => {
                    Err(SourceAccessError::HelperProtocol)
                }
                _ => Err(SourceAccessError::Unavailable),
            }
        }
        _ => Err(SourceAccessError::HelperProtocol),
    }
}

fn as_object(value: &JsonValue) -> Result<&BTreeMap<String, JsonValue>, SourceAccessError> {
    match value {
        JsonValue::Object(value) => Ok(value),
        _ => Err(SourceAccessError::HelperProtocol),
    }
}

fn object_string<'a>(value: &'a JsonValue, key: &str) -> Result<&'a str, SourceAccessError> {
    match as_object(value)?.get(key) {
        Some(JsonValue::String(value)) => Ok(value),
        _ => Err(SourceAccessError::HelperProtocol),
    }
}

fn object_i64(value: &JsonValue, key: &str) -> Result<i64, SourceAccessError> {
    match as_object(value)?.get(key) {
        Some(JsonValue::Number(value)) => Ok(*value),
        _ => Err(SourceAccessError::HelperProtocol),
    }
}

fn object_u64(value: &JsonValue, key: &str) -> Result<u64, SourceAccessError> {
    object_i64(value, key)?
        .try_into()
        .map_err(|_| SourceAccessError::HelperProtocol)
}

struct JsonParser<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> JsonParser<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            bytes: input.as_bytes(),
            offset: 0,
        }
    }

    fn parse(mut self) -> Result<JsonValue, SourceAccessError> {
        let value = self.value()?;
        self.whitespace();
        if self.offset != self.bytes.len() {
            return Err(SourceAccessError::HelperProtocol);
        }
        Ok(value)
    }

    fn value(&mut self) -> Result<JsonValue, SourceAccessError> {
        self.whitespace();
        match self.peek() {
            Some(b'{') => self.object(),
            Some(b'"') => self.string().map(JsonValue::String),
            Some(b't') if self.literal(b"true") => Ok(JsonValue::Bool(true)),
            Some(b'f') if self.literal(b"false") => Ok(JsonValue::Bool(false)),
            Some(b'n') if self.literal(b"null") => Ok(JsonValue::Null),
            Some(b'-' | b'0'..=b'9') => self.number().map(JsonValue::Number),
            _ => Err(SourceAccessError::HelperProtocol),
        }
    }

    fn object(&mut self) -> Result<JsonValue, SourceAccessError> {
        self.offset += 1;
        let mut values = BTreeMap::new();
        loop {
            self.whitespace();
            if self.peek() == Some(b'}') {
                self.offset += 1;
                return Ok(JsonValue::Object(values));
            }
            let key = self.string()?;
            self.whitespace();
            if self.peek() != Some(b':') {
                return Err(SourceAccessError::HelperProtocol);
            }
            self.offset += 1;
            if values.insert(key, self.value()?).is_some() {
                return Err(SourceAccessError::HelperProtocol);
            }
            self.whitespace();
            match self.peek() {
                Some(b',') => {
                    self.offset += 1;
                    self.whitespace();
                    if self.peek() == Some(b'}') {
                        return Err(SourceAccessError::HelperProtocol);
                    }
                }
                Some(b'}') => {}
                _ => return Err(SourceAccessError::HelperProtocol),
            }
        }
    }

    fn string(&mut self) -> Result<String, SourceAccessError> {
        if self.peek() != Some(b'"') {
            return Err(SourceAccessError::HelperProtocol);
        }
        self.offset += 1;
        let mut output = String::new();
        while let Some(byte) = self.peek() {
            self.offset += 1;
            match byte {
                b'"' => return Ok(output),
                b'\\' => {
                    let escaped = self.peek().ok_or(SourceAccessError::HelperProtocol)?;
                    self.offset += 1;
                    output.push(match escaped {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{0008}',
                        b'f' => '\u{000c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        _ => return Err(SourceAccessError::HelperProtocol),
                    });
                }
                value if value < 0x20 || !value.is_ascii() => {
                    return Err(SourceAccessError::HelperProtocol);
                }
                value => output.push(char::from(value)),
            }
        }
        Err(SourceAccessError::HelperProtocol)
    }

    fn number(&mut self) -> Result<i64, SourceAccessError> {
        let start = self.offset;
        if self.peek() == Some(b'-') {
            self.offset += 1;
        }
        match self.peek() {
            Some(b'0') => {
                self.offset += 1;
                if self.peek().is_some_and(|value| value.is_ascii_digit()) {
                    return Err(SourceAccessError::HelperProtocol);
                }
            }
            Some(b'1'..=b'9') => {
                self.offset += 1;
                while self.peek().is_some_and(|value| value.is_ascii_digit()) {
                    self.offset += 1;
                }
            }
            _ => return Err(SourceAccessError::HelperProtocol),
        }
        std::str::from_utf8(&self.bytes[start..self.offset])
            .ok()
            .and_then(|value| value.parse().ok())
            .ok_or(SourceAccessError::HelperProtocol)
    }

    fn literal(&mut self, value: &[u8]) -> bool {
        if self.bytes.get(self.offset..self.offset + value.len()) == Some(value) {
            self.offset += value.len();
            true
        } else {
            false
        }
    }

    fn whitespace(&mut self) {
        while self.peek().is_some_and(|byte| byte.is_ascii_whitespace()) {
            self.offset += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.offset).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::OpenOptionsExt;

    #[test]
    fn strict_response_parser_rejects_extra_frames_and_mismatched_ids() {
        let valid =
            br#"{"ok":true,"protocol_version":1,"request_id":"r","value":{"status":"resident"}}
"#;
        assert_eq!(
            object_string(&parse_response(valid, "r").unwrap(), "status"),
            Ok("resident")
        );
        assert_eq!(
            parse_response(b"{}\n{}\n", "r"),
            Err(SourceAccessError::HelperProtocol)
        );
        assert_eq!(
            parse_response(valid, "other"),
            Err(SourceAccessError::HelperProtocol)
        );
        assert_eq!(
            parse_response(
                br#"{"ok":true,"protocol_version":1,"request_id":"r","value":{"status":"resident",}}
"#,
                "r"
            ),
            Err(SourceAccessError::HelperProtocol)
        );
        assert_eq!(
            parse_response(
                br#"{"error":{"code":"permission_denied","message":"request rejected"},"ok":false,"protocol_version":1,"request_id":"r"}
"#,
                "r"
            ),
            Err(SourceAccessError::PermissionDenied)
        );
        assert_eq!(
            parse_response(
                br#"{"error":{"code":"oversize","message":"request rejected"},"ok":false,"protocol_version":1,"request_id":"r"}
"#,
                "r"
            ),
            Err(SourceAccessError::Oversized)
        );
    }

    #[test]
    fn handoff_directory_rejects_unsafe_mode_and_cleans_owned_orphans() {
        let root = std::env::temp_dir().join(format!(
            "nd-bridge-client-{}-{}",
            std::process::id(),
            REQUEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        let orphan = root.join("nd-source-orphan.bin");
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&orphan)
            .unwrap();
        let client =
            AppleBridgeClient::new("missing", "Ayumi", &root, Duration::from_secs(1), 1).unwrap();
        assert!(!orphan.exists());
        drop(client);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn concurrency_permit_is_bounded_and_released() {
        let counter = Arc::new(Mutex::new(0));
        let permit = Permit::acquire(&counter, 1).unwrap();
        assert!(matches!(
            Permit::acquire(&counter, 1),
            Err(SourceAccessError::Busy)
        ));
        drop(permit);
        assert!(Permit::acquire(&counter, 1).is_ok());
    }

    #[test]
    fn helper_lookup_prefers_installed_layout_then_development_checkout() {
        if std::env::var_os("ND_APPLE_BRIDGE_PATH").is_some() {
            return;
        }
        let scratch = scratch("lookup");
        let nd = scratch.join("bin/nd");
        let installed = scratch.join("libexec/apple-bridge");
        let development = scratch.join("workspace/helpers/apple-bridge/.build/debug/apple-bridge");
        fs::create_dir_all(nd.parent().unwrap()).unwrap();
        fs::create_dir_all(installed.parent().unwrap()).unwrap();
        fs::create_dir_all(development.parent().unwrap()).unwrap();
        fs::write(&installed, "").unwrap();
        fs::write(&development, "").unwrap();
        assert_eq!(
            AppleBridgeClient::locate_executable(&nd, &scratch.join("workspace")).unwrap(),
            installed
        );
        fs::remove_file(&installed).unwrap();
        assert_eq!(
            AppleBridgeClient::locate_executable(&nd, &scratch.join("workspace")).unwrap(),
            development
        );
        let _ = fs::remove_dir_all(scratch);
    }

    #[test]
    fn fake_helper_covers_success_protocol_crash_timeout_cancellation_and_missing() {
        let scratch = scratch("outcomes");
        let handoff = scratch.join("handoff");
        let success = script(
            &scratch,
            "success.sh",
            r#"input=$(cat)
id=$(printf '%s' "$input" | sed -n 's/.*"request_id":"\([^"]*\)".*/\1/p')
printf '{"ok":true,"protocol_version":1,"request_id":"%s","value":{"status":"resident"}}\n' "$id"
"#,
        );
        let client =
            AppleBridgeClient::new(&success, "Ayumi", &handoff, Duration::from_secs(5), 1).unwrap();
        let source = SourceRef::new(CONNECTOR, "2026/10/29/n.md").unwrap();
        assert_eq!(
            client.availability(&source, &AtomicBool::new(false)),
            Ok(BridgeAvailability::Resident)
        );

        let malformed = script(
            &scratch,
            "malformed.sh",
            "cat >/dev/null\nprintf '{}\\n{}\\n'\n",
        );
        let malformed_client =
            AppleBridgeClient::new(malformed, "Ayumi", &handoff, Duration::from_secs(5), 1)
                .unwrap();
        assert_eq!(
            malformed_client.availability(&source, &AtomicBool::new(false)),
            Err(SourceAccessError::HelperProtocol)
        );

        let crash = script(&scratch, "crash.sh", "cat >/dev/null\nexit 7\n");
        let crash_client =
            AppleBridgeClient::new(crash, "Ayumi", &handoff, Duration::from_secs(5), 1).unwrap();
        assert_eq!(
            crash_client.availability(&source, &AtomicBool::new(false)),
            Err(SourceAccessError::HelperCrashed)
        );

        let slow = script(&scratch, "slow.sh", "cat >/dev/null\nsleep 2\n");
        let slow_client =
            AppleBridgeClient::new(slow, "Ayumi", &handoff, Duration::from_millis(20), 1).unwrap();
        assert_eq!(
            slow_client.availability(&source, &AtomicBool::new(false)),
            Err(SourceAccessError::TimedOut)
        );
        assert_eq!(
            slow_client.availability(&source, &AtomicBool::new(true)),
            Err(SourceAccessError::Cancelled)
        );

        let missing_client = AppleBridgeClient::new(
            scratch.join("missing-helper"),
            "Ayumi",
            &handoff,
            Duration::from_secs(1),
            1,
        )
        .unwrap();
        assert_eq!(
            missing_client.availability(&source, &AtomicBool::new(false)),
            Err(SourceAccessError::HelperMissing)
        );
        let _ = fs::remove_dir_all(scratch);
    }

    #[test]
    fn fake_helper_handoff_accepts_large_private_file_and_always_cleans_it() {
        let scratch = scratch("handoff");
        let handoff = scratch.join("private");
        let reader = script(
            &scratch,
            "reader.sh",
            r#"input=$(cat)
id=$(printf '%s' "$input" | sed -n 's/.*"request_id":"\([^"]*\)".*/\1/p')
directory=$(printf '%s' "$input" | sed -n 's/.*"handoff_directory":"\([^"]*\)".*/\1/p')
dd if=/dev/zero of="$directory/nd-source-fake.bin" bs=70000 count=1 2>/dev/null
chmod 600 "$directory/nd-source-fake.bin"
printf '{"ok":true,"protocol_version":1,"request_id":"%s","value":{"file_identity":"9:7","handoff_name":"nd-source-fake.bin","modified_at":123,"size":70000}}\n' "$id"
"#,
        );
        let client =
            AppleBridgeClient::new(reader, "Ayumi", &handoff, Duration::from_secs(2), 1).unwrap();
        let source = SourceRef::new(CONNECTOR, "2026/10/29/n.md").unwrap();
        let read = client
            .coordinated_read(&source, &AtomicBool::new(false))
            .unwrap();
        assert_eq!(read.bytes.len(), 70_000);
        assert_eq!(read.file_identity, "9:7");
        assert!(!handoff.join("nd-source-fake.bin").exists());

        let failing = script(
            &scratch,
            "failing-reader.sh",
            r#"input=$(cat)
directory=$(printf '%s' "$input" | sed -n 's/.*"handoff_directory":"\([^"]*\)".*/\1/p')
printf partial > "$directory/nd-source-partial.bin"
chmod 600 "$directory/nd-source-partial.bin"
printf '{}\n{}\n'
"#,
        );
        let failing_client =
            AppleBridgeClient::new(failing, "Ayumi", &handoff, Duration::from_secs(2), 1).unwrap();
        assert_eq!(
            failing_client.coordinated_read(&source, &AtomicBool::new(false)),
            Err(SourceAccessError::HelperProtocol)
        );
        assert!(!handoff.join("nd-source-partial.bin").exists());
        let _ = fs::remove_dir_all(scratch);
    }

    #[test]
    fn concurrent_reads_use_isolated_handoff_directories() {
        let scratch = scratch("concurrent-handoff");
        let handoff = scratch.join("private");
        let marker = scratch.join("slow-ready");
        let reader = script(
            &scratch,
            "concurrent-reader.sh",
            &format!(
                r#"input=$(cat)
id=$(printf '%s' "$input" | sed -n 's/.*"request_id":"\([^"]*\)".*/\1/p')
directory=$(printf '%s' "$input" | sed -n 's/.*"handoff_directory":"\([^"]*\)".*/\1/p')
file="$directory/nd-source-fake.bin"
printf data > "$file"
chmod 600 "$file"
if printf '%s' "$input" | grep -q 'slow.md'; then
  touch '{}'
  sleep 0.2
fi
printf '{{"ok":true,"protocol_version":1,"request_id":"%s","value":{{"file_identity":"9:7","handoff_name":"nd-source-fake.bin","modified_at":123,"size":4}}}}\n' "$id"
"#,
                marker.display()
            ),
        );
        let client =
            AppleBridgeClient::new(reader, "Ayumi", &handoff, Duration::from_secs(2), 2).unwrap();
        let slow_client = client.clone();
        let slow = std::thread::spawn(move || {
            slow_client.coordinated_read(
                &SourceRef::new(CONNECTOR, "2026/10/29/slow.md").unwrap(),
                &AtomicBool::new(false),
            )
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        while !marker.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(marker.exists());
        let fast = client
            .coordinated_read(
                &SourceRef::new(CONNECTOR, "2026/10/29/fast.md").unwrap(),
                &AtomicBool::new(false),
            )
            .unwrap();
        assert_eq!(fast.bytes, b"data");
        assert_eq!(slow.join().unwrap().unwrap().bytes, b"data");
        assert!(fs::read_dir(&handoff).unwrap().next().is_none());
        let _ = fs::remove_dir_all(scratch);
    }

    #[test]
    fn download_wait_is_capped_to_the_protocol_maximum() {
        let scratch = scratch("download-timeout");
        let helper = script(
            &scratch,
            "download.sh",
            r#"input=$(cat)
id=$(printf '%s' "$input" | sed -n 's/.*"request_id":"\([^"]*\)".*/\1/p')
if ! printf '%s' "$input" | grep -q '"timeout_milliseconds":60000'; then exit 9; fi
printf '{"ok":true,"protocol_version":1,"request_id":"%s","value":{"status":"resident"}}\n' "$id"
"#,
        );
        let client = AppleBridgeClient::new(
            helper,
            "Ayumi",
            scratch.join("handoff"),
            Duration::from_secs(120),
            1,
        )
        .unwrap();
        client
            .request_download(
                &SourceRef::new(CONNECTOR, "2026/10/29/n.md").unwrap(),
                &AtomicBool::new(false),
            )
            .unwrap();
        let _ = fs::remove_dir_all(scratch);
    }

    fn scratch(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "nd-bridge-{label}-{}-{}",
            std::process::id(),
            REQUEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn script(directory: &Path, name: &str, body: &str) -> PathBuf {
        let path = directory.join(name);
        fs::write(&path, format!("#!/bin/sh\n{body}")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        path
    }
}
