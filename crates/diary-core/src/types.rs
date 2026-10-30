//! Connector-neutral values shared by workflow use cases and adapters.

use std::collections::BTreeSet;
use std::fmt;

use crate::Digest;
use crate::digest::push_framed;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Timestamp(pub i64);

impl Timestamp {
    pub const SECONDS_PER_DAY: i64 = 86_400;

    #[must_use]
    pub const fn days_before(self, days: i64) -> Self {
        Self(
            self.0
                .saturating_sub(days.saturating_mul(Self::SECONDS_PER_DAY)),
        )
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct DiaryDay(String);

impl DiaryDay {
    pub fn parse(value: impl Into<String>) -> Result<Self, ValueError> {
        let value = value.into();
        let bytes = value.as_bytes();
        if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
            return Err(ValueError::new("diary day must use YYYY-MM-DD"));
        }
        let year = parse_component(&value[0..4])?;
        let month = parse_component(&value[5..7])?;
        let day = parse_component(&value[8..10])?;
        let max_day = match month {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 if is_leap_year(year) => 29,
            2 => 28,
            _ => return Err(ValueError::new("diary day month is out of range")),
        };
        if day == 0 || day > max_day {
            return Err(ValueError::new("diary day is out of range"));
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DiaryDay {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

fn parse_component(value: &str) -> Result<u32, ValueError> {
    if !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(ValueError::new("diary day contains a non-digit"));
    }
    value
        .parse()
        .map_err(|_| ValueError::new("diary day component is invalid"))
}

const fn is_leap_year(year: u32) -> bool {
    year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400))
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SourceRef {
    pub connector: String,
    pub source_id: String,
}

impl SourceRef {
    pub fn new(
        connector: impl Into<String>,
        source_id: impl Into<String>,
    ) -> Result<Self, ValueError> {
        Ok(Self {
            connector: nonempty(connector.into(), "source connector")?,
            source_id: nonempty(source_id.into(), "source ID")?,
        })
    }

    #[must_use]
    pub fn stable_key(&self) -> String {
        format!(
            "{}:{}{}:{}",
            self.connector.len(),
            self.connector,
            self.source_id.len(),
            self.source_id
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DateEvidence {
    pub directory_day: Option<DiaryDay>,
    pub header_day: Option<DiaryDay>,
    pub warning: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceCandidate {
    pub source: SourceRef,
    /// Safe local presentation name. This never enters assistant requests or
    /// content-free diagnostics.
    pub source_note_name: String,
    pub date_evidence: DateEvidence,
    pub availability: SourceAvailability,
}

/// Returns whether a connector-supplied source-note name is safe to present as
/// a local relative label. Names are not filesystem authority: absolute,
/// traversal-like, empty-component, and control-character forms are rejected.
#[must_use]
pub fn source_note_name_is_valid(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 4 * 1024
        && !value.chars().any(char::is_control)
        && value
            .split('/')
            .all(|component| !component.is_empty() && !matches!(component, "." | ".."))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceAvailability {
    Resident,
    CloudOnly,
    PermissionDenied,
    Unknown,
}

impl SourceAvailability {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Resident => "resident",
            Self::CloudOnly => "cloud-only",
            Self::PermissionDenied => "permission-denied",
            Self::Unknown => "unknown",
        }
    }

    pub fn parse(value: &str) -> Result<Self, ValueError> {
        match value {
            "resident" => Ok(Self::Resident),
            "cloud-only" => Ok(Self::CloudOnly),
            "permission-denied" => Ok(Self::PermissionDenied),
            "unknown" => Ok(Self::Unknown),
            _ => Err(ValueError::new("unknown source availability")),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FreshnessStamp {
    pub file_identity: String,
    pub size: u64,
    pub modified_at: Timestamp,
    pub content_digest: Digest,
}

impl FreshnessStamp {
    pub fn new(
        file_identity: impl Into<String>,
        size: u64,
        modified_at: Timestamp,
        content_digest: Digest,
    ) -> Result<Self, ValueError> {
        Ok(Self {
            file_identity: nonempty(file_identity.into(), "file identity")?,
            size,
            modified_at,
            content_digest,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceSnapshot {
    pub source: SourceRef,
    pub stamp: FreshnessStamp,
    /// Sanitized readable Markdown body. No front matter or attachment data.
    pub body: String,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum WritingLanguage {
    EnUs,
    KoKr,
}

impl WritingLanguage {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::EnUs => "en-US",
            Self::KoKr => "ko-KR",
        }
    }

    pub fn parse(value: &str) -> Result<Self, ValueError> {
        match value {
            "en-US" => Ok(Self::EnUs),
            "ko-KR" => Ok(Self::KoKr),
            _ => Err(ValueError::new("unknown writing language")),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WritingStyle {
    Concise,
    Standard,
    Reflective,
}

impl WritingStyle {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Concise => "concise",
            Self::Standard => "standard",
            Self::Reflective => "reflective",
        }
    }

    pub fn parse(value: &str) -> Result<Self, ValueError> {
        match value {
            "concise" => Ok(Self::Concise),
            "standard" => Ok(Self::Standard),
            "reflective" => Ok(Self::Reflective),
            _ => Err(ValueError::new("unknown writing style")),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompositionPath {
    Hosted,
    AppleOnDevice,
}

impl CompositionPath {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Hosted => "hosted",
            Self::AppleOnDevice => "apple-on-device",
        }
    }

    pub fn parse(value: &str) -> Result<Self, ValueError> {
        match value {
            "hosted" => Ok(Self::Hosted),
            "apple-on-device" => Ok(Self::AppleOnDevice),
            _ => Err(ValueError::new("unknown composition path")),
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct AssistantBackendId(String);

impl AssistantBackendId {
    pub fn parse(value: impl Into<String>) -> Result<Self, ValueError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 128
            || !value.bytes().all(|byte| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || matches!(byte, b'-' | b'_' | b'.')
            })
        {
            return Err(ValueError::new("invalid assistant backend ID"));
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AssistantBackendId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssistantSelection {
    pub backend_id: AssistantBackendId,
    pub composition_path: CompositionPath,
}

impl AssistantSelection {
    #[must_use]
    pub const fn new(backend_id: AssistantBackendId, composition_path: CompositionPath) -> Self {
        Self {
            backend_id,
            composition_path,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Preferences {
    pub last_language: Option<WritingLanguage>,
    pub last_assistant_backend_id: Option<AssistantBackendId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkflowSummary {
    pub day: DiaryDay,
    pub state: WorkflowState,
    pub revision: u64,
    pub buffer_version: u64,
    pub updated_at: Timestamp,
    pub discarded_at: Option<Timestamp>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum AssistantAction {
    ExtractMemories,
    DraftEntry,
    ReviewParagraph,
    RefineSource,
}

impl AssistantAction {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ExtractMemories => "extract-memories",
            Self::DraftEntry => "draft-entry",
            Self::ReviewParagraph => "review-paragraph",
            Self::RefineSource => "refine-source",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssistantBackendDescriptor {
    pub id: AssistantBackendId,
    pub display_label: String,
    pub composition_path: CompositionPath,
    pub available: bool,
    pub reason_code: Option<String>,
    pub supported_languages: BTreeSet<WritingLanguage>,
    pub supported_actions: BTreeSet<AssistantAction>,
}

impl AssistantBackendDescriptor {
    pub fn new(
        id: AssistantBackendId,
        display_label: impl Into<String>,
        composition_path: CompositionPath,
        available: bool,
        reason_code: Option<String>,
        supported_languages: BTreeSet<WritingLanguage>,
        supported_actions: BTreeSet<AssistantAction>,
    ) -> Result<Self, ValueError> {
        let display_label = nonempty(display_label.into(), "assistant display label")?;
        if display_label.len() > 256
            || reason_code
                .as_ref()
                .is_some_and(|reason| reason.is_empty() || reason.len() > 128 || !reason.is_ascii())
            || (available && reason_code.is_some())
            || (!available && reason_code.is_none())
        {
            return Err(ValueError::new("invalid assistant backend descriptor"));
        }
        Ok(Self {
            id,
            display_label,
            composition_path,
            available,
            reason_code,
            supported_languages,
            supported_actions,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DraftAssistantError {
    Unavailable,
    UnsupportedLanguage,
    UnsupportedAction,
    Refused,
    ContextOverflow,
    InvalidOutput,
    TimedOut,
    Cancelled,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum WorkflowState {
    Started,
    Discovered,
    SourcesConfirmed,
    Composing,
    EntryConfirmed,
    OutputPlanned,
    Approved,
    Publishing,
    Completed,
    Partial,
}

impl WorkflowState {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Started => "started",
            Self::Discovered => "discovered",
            Self::SourcesConfirmed => "sources-confirmed",
            Self::Composing => "composing",
            Self::EntryConfirmed => "entry-confirmed",
            Self::OutputPlanned => "output-planned",
            Self::Approved => "approved",
            Self::Publishing => "publishing",
            Self::Completed => "completed",
            Self::Partial => "partial",
        }
    }

    pub fn parse(value: &str) -> Result<Self, ValueError> {
        match value {
            "started" => Ok(Self::Started),
            "discovered" => Ok(Self::Discovered),
            "sources-confirmed" => Ok(Self::SourcesConfirmed),
            "composing" => Ok(Self::Composing),
            "entry-confirmed" => Ok(Self::EntryConfirmed),
            "output-planned" => Ok(Self::OutputPlanned),
            "approved" => Ok(Self::Approved),
            "publishing" => Ok(Self::Publishing),
            "completed" => Ok(Self::Completed),
            "partial" => Ok(Self::Partial),
            _ => Err(ValueError::new("unknown workflow state")),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FreshnessCheckpoint {
    PostRead,
    BeforeExtraction,
    BeforeDrafting,
    BeforeEntryConfirmation,
    BeforePreview,
    BeforeFirstWrite,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FreshnessOutcome {
    Fresh,
    Changed { source: SourceRef },
    Unverifiable { source: SourceRef },
    NeedsDownload { source: SourceRef },
    PermissionDenied { source: SourceRef },
    Unreadable { source: SourceRef },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SelectedSource {
    pub snapshot: SourceSnapshot,
    pub date_evidence: DateEvidence,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SourceReadOutcome {
    Read(SelectedSource),
    NeedsDownload { source: SourceRef },
    PermissionDenied { source: SourceRef },
    Unreadable { source: SourceRef },
    Unverifiable { source: SourceRef },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceDiscovery {
    pub candidates: Vec<SourceCandidate>,
    pub diagnostics: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CatalogRequest {
    Automatic(DiaryDay),
    ExplicitDate(DiaryDay),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceAccessError {
    InvalidConfiguration,
    Unavailable,
    HelperMissing,
    HelperProtocol,
    HelperCrashed,
    TimedOut,
    Cancelled,
    Busy,
    PermissionDenied,
    NeedsDownload,
    NotFound,
    Oversized,
}

impl fmt::Display for SourceAccessError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidConfiguration => "source configuration is invalid",
            Self::Unavailable => "source service is unavailable",
            Self::HelperMissing => "Apple helper is missing",
            Self::HelperProtocol => "Apple helper protocol failed",
            Self::HelperCrashed => "Apple helper exited unexpectedly",
            Self::TimedOut => "Apple helper timed out",
            Self::Cancelled => "source operation was cancelled",
            Self::Busy => "source helper capacity is busy",
            Self::PermissionDenied => "source permission was denied",
            Self::NeedsDownload => "source needs download",
            Self::NotFound => "source is missing",
            Self::Oversized => "source exceeds the size limit",
        })
    }
}

impl std::error::Error for SourceAccessError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InvalidationReason {
    SelectionChanged,
    SourceChanged(SourceRef),
    FreshnessUnverifiable(SourceRef),
    EntryEdited,
    CompositionChoiceChanged,
    OutputBindingChanged,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct EntryBuffer {
    pub text: String,
    pub version: u64,
    pub confirmed: Option<EntryConfirmation>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EntryConfirmation {
    pub buffer_version: u64,
    pub entry_digest: Digest,
    pub confirmed_at: Timestamp,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DestinationPlan {
    pub kind: DestinationKind,
    pub destination: String,
    pub target_identity: String,
    pub intended_state_digest: Digest,
    pub payload_digest: Digest,
    pub inspected_state_digest: Digest,
}

impl DestinationPlan {
    pub fn new(
        kind: DestinationKind,
        destination: impl Into<String>,
        target_identity: impl Into<String>,
        intended_state_digest: Digest,
        payload_digest: Digest,
        inspected_state_digest: Digest,
    ) -> Result<Self, ValueError> {
        Ok(Self {
            kind,
            destination: nonempty(destination.into(), "destination")?,
            target_identity: nonempty(target_identity.into(), "target identity")?,
            intended_state_digest,
            payload_digest,
            inspected_state_digest,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum DestinationKind {
    Local,
    Mirror,
    Remote,
}

impl DestinationKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Mirror => "mirror",
            Self::Remote => "remote",
        }
    }

    pub fn parse(value: &str) -> Result<Self, ValueError> {
        match value {
            "local" => Ok(Self::Local),
            "mirror" => Ok(Self::Mirror),
            "remote" => Ok(Self::Remote),
            _ => Err(ValueError::new("unknown destination kind")),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OutputPlan {
    pub plan_id: String,
    pub entry_digest: Digest,
    pub selection_digest: Digest,
    pub targets: Vec<DestinationPlan>,
    pub plan_digest: Digest,
    pub created_at: Timestamp,
}

impl OutputPlan {
    pub fn new(
        plan_id: impl Into<String>,
        entry_digest: Digest,
        selection_digest: Digest,
        mut targets: Vec<DestinationPlan>,
        created_at: Timestamp,
    ) -> Result<Self, ValueError> {
        let plan_id = nonempty(plan_id.into(), "plan ID")?;
        if targets.is_empty() {
            return Err(ValueError::new("an output plan needs at least one target"));
        }
        targets.sort_by(|left, right| {
            (left.kind, &left.destination, &left.target_identity).cmp(&(
                right.kind,
                &right.destination,
                &right.target_identity,
            ))
        });
        let mut destinations = BTreeSet::new();
        if targets
            .iter()
            .any(|target| !destinations.insert(target.destination.as_str()))
        {
            return Err(ValueError::new(
                "an output plan cannot repeat a destination",
            ));
        }
        let mut binding = Vec::new();
        push_framed(&mut binding, plan_id.as_bytes());
        push_framed(&mut binding, entry_digest.as_str().as_bytes());
        push_framed(&mut binding, selection_digest.as_str().as_bytes());
        for target in &targets {
            push_framed(&mut binding, target.kind.as_str().as_bytes());
            push_framed(&mut binding, target.destination.as_bytes());
            push_framed(&mut binding, target.target_identity.as_bytes());
            push_framed(
                &mut binding,
                target.intended_state_digest.as_str().as_bytes(),
            );
            push_framed(&mut binding, target.payload_digest.as_str().as_bytes());
            push_framed(
                &mut binding,
                target.inspected_state_digest.as_str().as_bytes(),
            );
        }
        let plan_digest = Digest::of_bytes(&binding);
        Ok(Self {
            plan_id,
            entry_digest,
            selection_digest,
            targets,
            plan_digest,
            created_at,
        })
    }

    pub fn validate(&self) -> Result<(), ValueError> {
        let rebuilt = Self::new(
            self.plan_id.clone(),
            self.entry_digest.clone(),
            self.selection_digest.clone(),
            self.targets.clone(),
            self.created_at,
        )?;
        if &rebuilt != self {
            return Err(ValueError::new(
                "output plan fields do not match its canonical binding",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Approval {
    pub plan_digest: Digest,
    pub workflow_revision: u64,
    pub approved_at: Timestamp,
    pub destination_only: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DestinationStatus {
    Pending,
    Blocked,
    Conflict,
    Verified,
    Reconciling,
}

impl DestinationStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Blocked => "blocked",
            Self::Conflict => "conflict",
            Self::Verified => "verified",
            Self::Reconciling => "reconciling",
        }
    }

    pub fn parse(value: &str) -> Result<Self, ValueError> {
        match value {
            "pending" => Ok(Self::Pending),
            "blocked" => Ok(Self::Blocked),
            "conflict" => Ok(Self::Conflict),
            "verified" => Ok(Self::Verified),
            "reconciling" => Ok(Self::Reconciling),
            _ => Err(ValueError::new("unknown destination status")),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DestinationProgress {
    pub destination: String,
    pub status: DestinationStatus,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JobStatus {
    Queued,
    Running,
    CancellationRequested,
    Reconciling,
    Completed,
    Failed,
}

impl JobStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::CancellationRequested => "cancellation-requested",
            Self::Reconciling => "reconciling",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }

    pub fn parse(value: &str) -> Result<Self, ValueError> {
        match value {
            "queued" => Ok(Self::Queued),
            "running" => Ok(Self::Running),
            "cancellation-requested" => Ok(Self::CancellationRequested),
            "reconciling" => Ok(Self::Reconciling),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            _ => Err(ValueError::new("unknown job status")),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperationIntent {
    pub attempt_id: String,
    pub plan_id: String,
    pub day: DiaryDay,
    pub destination: String,
    pub target_identity: String,
    pub entry_digest: Digest,
    pub selection_digest: Digest,
    pub payload_digest: Digest,
    pub created_at: Timestamp,
    pub cancellation_requested: bool,
}

impl OperationIntent {
    pub fn new(
        attempt_id: impl Into<String>,
        plan_id: impl Into<String>,
        day: DiaryDay,
        target: &DestinationPlan,
        entry_digest: Digest,
        selection_digest: Digest,
        created_at: Timestamp,
    ) -> Result<Self, ValueError> {
        Ok(Self {
            attempt_id: nonempty(attempt_id.into(), "attempt ID")?,
            plan_id: nonempty(plan_id.into(), "plan ID")?,
            day,
            destination: target.destination.clone(),
            target_identity: target.target_identity.clone(),
            entry_digest,
            selection_digest,
            payload_digest: target.payload_digest.clone(),
            created_at,
            cancellation_requested: false,
        })
    }

    pub fn validate(&self) -> Result<(), ValueError> {
        nonempty(self.attempt_id.clone(), "attempt ID")?;
        nonempty(self.plan_id.clone(), "plan ID")?;
        nonempty(self.destination.clone(), "destination")?;
        nonempty(self.target_identity.clone(), "target identity")?;
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Receipt {
    pub receipt_id: String,
    pub attempt_id: String,
    pub day: DiaryDay,
    pub destination: String,
    pub target_identity: String,
    pub version: u32,
    pub entry_digest: Digest,
    pub selection_digest: Digest,
    pub payload_digest: Digest,
    pub returned_ids: Vec<String>,
    pub verified_at: Timestamp,
}

impl Receipt {
    pub fn validate_against(&self, intent: &OperationIntent) -> Result<(), ValueError> {
        nonempty(self.receipt_id.clone(), "receipt ID")?;
        if self.version == 0 {
            return Err(ValueError::new("receipt version must be positive"));
        }
        for returned_id in &self.returned_ids {
            nonempty(returned_id.clone(), "returned ID")?;
        }
        if self.attempt_id != intent.attempt_id
            || self.day != intent.day
            || self.destination != intent.destination
            || self.target_identity != intent.target_identity
            || self.entry_digest != intent.entry_digest
            || self.selection_digest != intent.selection_digest
            || self.payload_digest != intent.payload_digest
        {
            return Err(ValueError::new(
                "receipt does not match its operation intent",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WriteOutcome {
    Verified {
        receipt_id: String,
        returned_ids: Vec<String>,
    },
    DefiniteNoEffect,
    Conflict,
    AmbiguousWrite,
    Unavailable,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InspectionOutcome {
    Matching {
        receipt_id: String,
        returned_ids: Vec<String>,
    },
    Absent,
    Conflict,
    Unavailable,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReconciliationGate {
    Clear,
    Reconciling,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValueError {
    message: &'static str,
}

impl ValueError {
    #[must_use]
    pub const fn new(message: &'static str) -> Self {
        Self { message }
    }
}

impl fmt::Display for ValueError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.message)
    }
}

impl std::error::Error for ValueError {}

fn nonempty(value: String, name: &'static str) -> Result<String, ValueError> {
    if value.is_empty() || value.chars().any(char::is_control) {
        return Err(ValueError::new(match name {
            "source connector" => {
                "source connector must be nonempty and contain no control characters"
            }
            "source ID" => "source ID must be nonempty and contain no control characters",
            "file identity" => "file identity must be nonempty and contain no control characters",
            "destination" => "destination must be nonempty and contain no control characters",
            "target identity" => {
                "target identity must be nonempty and contain no control characters"
            }
            "plan ID" => "plan ID must be nonempty and contain no control characters",
            "attempt ID" => "attempt ID must be nonempty and contain no control characters",
            "receipt ID" => "receipt ID must be nonempty and contain no control characters",
            "returned ID" => "returned ID must be nonempty and contain no control characters",
            _ => "value must be nonempty and contain no control characters",
        }));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refine_source_action_as_str_is_refine_source() {
        assert_eq!(AssistantAction::RefineSource.as_str(), "refine-source");
    }
}
