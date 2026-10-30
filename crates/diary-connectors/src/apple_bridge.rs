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

use diary_core::{
    AssistantAction, AssistantBackendDescriptor, AssistantBackendId, AssistantRequest,
    AssistantResponse, CompositionPath, DiaryDay, DraftAssistant, DraftAssistantError,
    DraftEntryResult, MemoryCandidate, MemoryExtractionResult, ParagraphReviewResult,
    ReflectionRange, SourceAccessError, SourceRef, SourceRefinementResult, Timestamp,
    WritingLanguage,
};

use crate::ayumi::MAX_SOURCE_BYTES;

const MAX_FRAME_BYTES: usize = 20 * 1024 * 1024;
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
        let body = format!(
            "{{\"protocol_version\":2,\"request_id\":{},\"command\":{},\"payload\":{payload}}}",
            json_string(&request_id),
            json_string(command)
        );
        if body.len() > MAX_FRAME_BYTES {
            return Err(SourceAccessError::InvalidConfiguration);
        }
        let frame = encode_frame(body.as_bytes())?;
        let output = self.execute(&frame, cancelled)?;
        parse_response(&output, &request_id)
    }

    fn execute(&self, frame: &[u8], cancelled: &AtomicBool) -> Result<Vec<u8>, SourceAccessError> {
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
            .write_all(frame)
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
        if output.stdout.len() > MAX_FRAME_BYTES + 4 || output.stderr.len() > MAX_DIAGNOSTIC_BYTES {
            return Err(SourceAccessError::HelperProtocol);
        }
        if output.stdout.is_empty() && !output.status.success() {
            return Err(SourceAccessError::HelperCrashed);
        }
        Ok(output.stdout)
    }

    fn assistant_call(
        &self,
        command: &str,
        payload: &str,
        cancelled: &AtomicBool,
    ) -> Result<JsonValue, BridgeResponseError> {
        let request_id = format!("nd-{}", REQUEST_SEQUENCE.fetch_add(1, Ordering::Relaxed));
        let body = format!(
            "{{\"protocol_version\":2,\"request_id\":{},\"command\":{},\"payload\":{payload}}}",
            json_string(&request_id),
            json_string(command),
        );
        let frame = encode_frame(body.as_bytes()).map_err(|_| BridgeResponseError::Protocol)?;
        let output = self
            .execute(&frame, cancelled)
            .map_err(BridgeResponseError::Transport)?;
        parse_response_with_code(&output, &request_id)
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

#[derive(Clone, Debug)]
pub struct AppleFoundationAssistant {
    bridge: AppleBridgeClient,
    available: Arc<AtomicBool>,
}

impl AppleFoundationAssistant {
    #[must_use]
    pub fn new(bridge: AppleBridgeClient, helper_available: bool) -> Self {
        let adapter = Self {
            bridge,
            available: Arc::new(AtomicBool::new(false)),
        };
        let available = helper_available
            && adapter.model_available(WritingLanguage::EnUs, &AtomicBool::new(false))
            && adapter.model_available(WritingLanguage::KoKr, &AtomicBool::new(false));
        adapter.available.store(available, Ordering::Release);
        adapter
    }

    fn model_available(&self, locale: WritingLanguage, cancelled: &AtomicBool) -> bool {
        let payload = format!("{{\"locale\":{}}}", json_string(locale.as_str()));
        self.bridge
            .assistant_call("assistant-availability", &payload, cancelled)
            .ok()
            .and_then(|value| object_string(&value, "status").ok().map(str::to_owned))
            .is_some_and(|status| status == "available")
    }
}

impl DraftAssistant for AppleFoundationAssistant {
    fn descriptor(&self) -> AssistantBackendDescriptor {
        let available = self.available.load(Ordering::Acquire);
        AssistantBackendDescriptor::new(
            AssistantBackendId::parse("apple-foundation-models")
                .expect("static Apple backend ID is valid"),
            "Apple On-Device",
            CompositionPath::AppleOnDevice,
            available,
            (!available).then(|| "apple-model-unavailable".to_owned()),
            [WritingLanguage::EnUs, WritingLanguage::KoKr]
                .into_iter()
                .collect(),
            [
                AssistantAction::ExtractMemories,
                AssistantAction::DraftEntry,
                AssistantAction::ReviewParagraph,
                AssistantAction::RefineSource,
            ]
            .into_iter()
            .collect(),
        )
        .expect("static Apple descriptor is valid")
    }

    fn invoke(
        &self,
        request: &AssistantRequest,
        cancelled: &AtomicBool,
    ) -> Result<AssistantResponse, DraftAssistantError> {
        let locale = match request {
            AssistantRequest::ExtractMemories(_) => WritingLanguage::EnUs,
            AssistantRequest::DraftEntry(request) => request.language(),
            AssistantRequest::ReviewParagraph(request) => request.language(),
            AssistantRequest::RefineSource(_) => WritingLanguage::EnUs,
        };
        let available = self.model_available(locale, cancelled);
        self.available.store(available, Ordering::Release);
        if !available {
            return Err(if cancelled.load(Ordering::Acquire) {
                DraftAssistantError::Cancelled
            } else {
                DraftAssistantError::Unavailable
            });
        }
        let templates = apple_templates(request.action())?;
        let (command, payload) = match request {
            AssistantRequest::ExtractMemories(request) => {
                let sources = request
                    .sources()
                    .iter()
                    .map(|source| {
                        format!(
                            "{{\"ordinal\":{},\"body\":{}}}",
                            source.ordinal(),
                            json_string(source.body())
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(",");
                (
                    "assistant-extract-memories",
                    format!("{{\"locale\":\"en-US\",{templates},\"sources\":[{sources}]}}"),
                )
            }
            AssistantRequest::DraftEntry(request) => (
                "assistant-draft-entry",
                format!(
                    "{{\"locale\":{},{templates},\"prior_draft\":{},\"reviewed_memory\":{},\"style\":{}}}",
                    json_string(request.language().as_str()),
                    request
                        .prior_draft()
                        .map(json_string)
                        .unwrap_or_else(|| "null".to_owned()),
                    json_string(request.reviewed_memory()),
                    json_string(request.style().as_str()),
                ),
            ),
            AssistantRequest::ReviewParagraph(request) => (
                "assistant-review-paragraph",
                format!(
                    "{{\"locale\":{},{templates},\"paragraph\":{},\"style\":{}}}",
                    json_string(request.language().as_str()),
                    json_string(request.paragraph()),
                    json_string(request.style().as_str()),
                ),
            ),
            AssistantRequest::RefineSource(request) => (
                "assistant-refine-source",
                format!(
                    "{{\"locale\":\"en-US\",{templates},\"body\":{}}}",
                    json_string(request.body()),
                ),
            ),
        };
        let output = self
            .bridge
            .assistant_call(command, &payload, cancelled)
            .map_err(map_assistant_bridge_error)?;
        parse_assistant_output(request, &output)
    }
}

/// The `"instructions"`/`"prompt"` payload members: raw templates from
/// `assets/prompts/apple-foundation-models/`, rendered by the Swift helper.
fn apple_templates(action: AssistantAction) -> Result<String, DraftAssistantError> {
    let instructions = diary_core::prompt_asset::apple_session_instructions()
        .map_err(|_| DraftAssistantError::Unavailable)?;
    let prompt = diary_core::prompt_asset::apple_prompt(action)
        .map_err(|_| DraftAssistantError::Unavailable)?;
    Ok(format!(
        "\"instructions\":{},\"prompt\":{}",
        json_string(instructions.template()),
        json_string(prompt.template())
    ))
}

fn map_assistant_bridge_error(error: BridgeResponseError) -> DraftAssistantError {
    match error {
        BridgeResponseError::Protocol => DraftAssistantError::InvalidOutput,
        BridgeResponseError::Remote(code) => match code.as_str() {
            "unsupported_locale" => DraftAssistantError::UnsupportedLanguage,
            "refusal" => DraftAssistantError::Refused,
            "context_overflow" | "frame_too_large" => DraftAssistantError::ContextOverflow,
            "invalid_output" | "malformed_input" => DraftAssistantError::InvalidOutput,
            "timed_out" => DraftAssistantError::TimedOut,
            "cancelled" => DraftAssistantError::Cancelled,
            _ => DraftAssistantError::Unavailable,
        },
        BridgeResponseError::Transport(SourceAccessError::TimedOut) => {
            DraftAssistantError::TimedOut
        }
        BridgeResponseError::Transport(SourceAccessError::Cancelled) => {
            DraftAssistantError::Cancelled
        }
        BridgeResponseError::Transport(_) => DraftAssistantError::Unavailable,
    }
}

fn parse_assistant_output(
    request: &AssistantRequest,
    output: &JsonValue,
) -> Result<AssistantResponse, DraftAssistantError> {
    match request {
        AssistantRequest::ExtractMemories(request) => {
            let candidates = object_array(output, "candidates")
                .map_err(|_| DraftAssistantError::InvalidOutput)?
                .iter()
                .map(|candidate| {
                    let text = object_string(candidate, "text")?.to_owned();
                    let ordinals = object_array(candidate, "source_ordinals")?
                        .iter()
                        .map(|value| match value {
                            JsonValue::Number(value) => {
                                u16::try_from(*value).map_err(|_| SourceAccessError::HelperProtocol)
                            }
                            _ => Err(SourceAccessError::HelperProtocol),
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    MemoryCandidate::new(text, ordinals, request.sources().len())
                        .map_err(|_| SourceAccessError::HelperProtocol)
                })
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| DraftAssistantError::InvalidOutput)?;
            MemoryExtractionResult::new(candidates)
                .map(AssistantResponse::Memories)
                .map_err(|_| DraftAssistantError::InvalidOutput)
        }
        AssistantRequest::DraftEntry(_) => {
            let text =
                object_string(output, "text").map_err(|_| DraftAssistantError::InvalidOutput)?;
            let ranges = assistant_ranges(output)?;
            DraftEntryResult::new(text, ranges)
                .map(AssistantResponse::EntryDraft)
                .map_err(|error| {
                    DraftAssistantError::InvalidOutputAt(diary_core::invalid_output_reason(&error))
                })
        }
        AssistantRequest::ReviewParagraph(_) => {
            let replacement = object_string(output, "replacement")
                .map_err(|_| DraftAssistantError::InvalidOutput)?;
            let explanation = match as_object(output)
                .map_err(|_| DraftAssistantError::InvalidOutput)?
                .get("explanation")
            {
                Some(JsonValue::String(value)) => Some(value.clone()),
                Some(JsonValue::Null) => None,
                _ => return Err(DraftAssistantError::InvalidOutput),
            };
            ParagraphReviewResult::new(replacement, assistant_ranges(output)?, explanation)
                .map(AssistantResponse::ParagraphReview)
                .map_err(|error| {
                    DraftAssistantError::InvalidOutputAt(diary_core::invalid_output_reason(&error))
                })
        }
        AssistantRequest::RefineSource(_) => {
            let body =
                object_string(output, "body").map_err(|_| DraftAssistantError::InvalidOutput)?;
            SourceRefinementResult::new(body)
                .map(AssistantResponse::SourceRefinement)
                .map_err(|_| DraftAssistantError::InvalidOutput)
        }
    }
}

fn object_array<'a>(value: &'a JsonValue, key: &str) -> Result<&'a [JsonValue], SourceAccessError> {
    match as_object(value)?.get(key) {
        Some(JsonValue::Array(values)) => Ok(values),
        _ => Err(SourceAccessError::HelperProtocol),
    }
}

fn assistant_ranges(output: &JsonValue) -> Result<Vec<ReflectionRange>, DraftAssistantError> {
    object_array(output, "reflection_ranges")
        .map_err(|_| DraftAssistantError::InvalidOutput)?
        .iter()
        .map(|range| {
            let start = object_i64(range, "start")
                .ok()
                .and_then(|value| usize::try_from(value).ok())
                .ok_or(DraftAssistantError::InvalidOutput)?;
            let end = object_i64(range, "end")
                .ok()
                .and_then(|value| usize::try_from(value).ok())
                .ok_or(DraftAssistantError::InvalidOutput)?;
            Ok(ReflectionRange { start, end })
        })
        .collect()
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
    Array(Vec<JsonValue>),
    Object(BTreeMap<String, JsonValue>),
}

fn parse_response(bytes: &[u8], request_id: &str) -> Result<JsonValue, SourceAccessError> {
    match parse_response_with_code(bytes, request_id) {
        Ok(value) => Ok(value),
        Err(BridgeResponseError::Protocol) => Err(SourceAccessError::HelperProtocol),
        Err(BridgeResponseError::Transport(error)) => Err(error),
        Err(BridgeResponseError::Remote(code)) => match code.as_str() {
            "cancelled" => Err(SourceAccessError::Cancelled),
            "permission_denied" => Err(SourceAccessError::PermissionDenied),
            "needs_download" => Err(SourceAccessError::NeedsDownload),
            "not_found" => Err(SourceAccessError::NotFound),
            "oversize" | "frame_too_large" => Err(SourceAccessError::Oversized),
            "unsupported_version" | "malformed_input" | "unknown_command" => {
                Err(SourceAccessError::HelperProtocol)
            }
            _ => Err(SourceAccessError::Unavailable),
        },
    }
}

#[derive(Debug)]
enum BridgeResponseError {
    Protocol,
    Remote(String),
    Transport(SourceAccessError),
}

fn parse_response_with_code(
    bytes: &[u8],
    request_id: &str,
) -> Result<JsonValue, BridgeResponseError> {
    let body = decode_frame(bytes).map_err(|_| BridgeResponseError::Protocol)?;
    let text = std::str::from_utf8(body).map_err(|_| BridgeResponseError::Protocol)?;
    let root = JsonParser::new(text)
        .parse()
        .map_err(|_| BridgeResponseError::Protocol)?;
    let object = as_object(&root).map_err(|_| BridgeResponseError::Protocol)?;
    if object_i64(&root, "protocol_version").map_err(|_| BridgeResponseError::Protocol)? != 2
        || object_string(&root, "request_id").map_err(|_| BridgeResponseError::Protocol)?
            != request_id
    {
        return Err(BridgeResponseError::Protocol);
    }
    match object.get("ok") {
        Some(JsonValue::Bool(true)) => object
            .get("value")
            .cloned()
            .ok_or(BridgeResponseError::Protocol),
        Some(JsonValue::Bool(false)) => {
            let error = object.get("error").ok_or(BridgeResponseError::Protocol)?;
            let code = object_string(error, "code").map_err(|_| BridgeResponseError::Protocol)?;
            Err(BridgeResponseError::Remote(code.to_owned()))
        }
        _ => Err(BridgeResponseError::Protocol),
    }
}

fn encode_frame(body: &[u8]) -> Result<Vec<u8>, SourceAccessError> {
    if body.is_empty() || body.len() > MAX_FRAME_BYTES {
        return Err(SourceAccessError::HelperProtocol);
    }
    let length = u32::try_from(body.len()).map_err(|_| SourceAccessError::HelperProtocol)?;
    let mut frame = Vec::with_capacity(body.len() + 4);
    frame.extend_from_slice(&length.to_be_bytes());
    frame.extend_from_slice(body);
    Ok(frame)
}

fn decode_frame(frame: &[u8]) -> Result<&[u8], SourceAccessError> {
    let header: [u8; 4] = frame
        .get(..4)
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or(SourceAccessError::HelperProtocol)?;
    let length = usize::try_from(u32::from_be_bytes(header))
        .map_err(|_| SourceAccessError::HelperProtocol)?;
    if length == 0 || length > MAX_FRAME_BYTES || frame.len() != length + 4 {
        return Err(SourceAccessError::HelperProtocol);
    }
    Ok(&frame[4..])
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
            Some(b'[') => self.array(),
            Some(b'"') => self.string().map(JsonValue::String),
            Some(b't') if self.literal(b"true") => Ok(JsonValue::Bool(true)),
            Some(b'f') if self.literal(b"false") => Ok(JsonValue::Bool(false)),
            Some(b'n') if self.literal(b"null") => Ok(JsonValue::Null),
            Some(b'-' | b'0'..=b'9') => self.number().map(JsonValue::Number),
            _ => Err(SourceAccessError::HelperProtocol),
        }
    }

    fn array(&mut self) -> Result<JsonValue, SourceAccessError> {
        self.offset += 1;
        let mut values = Vec::new();
        loop {
            self.whitespace();
            if self.peek() == Some(b']') {
                self.offset += 1;
                return Ok(JsonValue::Array(values));
            }
            values.push(self.value()?);
            self.whitespace();
            match self.peek() {
                Some(b',') => {
                    self.offset += 1;
                    self.whitespace();
                    if self.peek() == Some(b']') {
                        return Err(SourceAccessError::HelperProtocol);
                    }
                }
                Some(b']') => {}
                _ => return Err(SourceAccessError::HelperProtocol),
            }
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
            match byte {
                b'"' => {
                    self.offset += 1;
                    return Ok(output);
                }
                b'\\' => {
                    self.offset += 1;
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
                value if value < 0x20 => {
                    return Err(SourceAccessError::HelperProtocol);
                }
                value if value.is_ascii() => {
                    self.offset += 1;
                    output.push(char::from(value));
                }
                _ => {
                    let remaining = std::str::from_utf8(&self.bytes[self.offset..])
                        .map_err(|_| SourceAccessError::HelperProtocol)?;
                    let character = remaining
                        .chars()
                        .next()
                        .ok_or(SourceAccessError::HelperProtocol)?;
                    self.offset += character.len_utf8();
                    output.push(character);
                }
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
mod tests;
