//! Versioned wire contract shared by the local Control API Server and clients.

#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};

pub const API_VERSION: &str = "v1";
pub const CONTRACT_VERSION: u32 = 5;
/// Covers one maximum-size source body plus a bounded JSON envelope. Individual
/// routes still enforce their narrower action-specific limits.
pub const MAX_REQUEST_BYTES: usize = 8 * 1024 * 1024 + 64 * 1024;
pub const MAX_ENTRY_BYTES: usize = 512 * 1024;
pub const MAX_STRING_BYTES: usize = 4 * 1024;
pub const MAX_COLLECTION_ITEMS: usize = 256;
pub const MAX_CANDIDATES: usize = 512;
pub const MAX_SELECTED_SOURCES: usize = 64;
pub const MAX_MEMORY_CANDIDATES: usize = 128;
pub const MAX_MEMORY_CANDIDATE_BYTES: usize = MAX_STRING_BYTES;
pub const MAX_REVIEWED_MEMORY_BYTES: usize = MAX_ENTRY_BYTES;
pub const MAX_SOURCE_ORDINALS: usize = MAX_SELECTED_SOURCES;
pub const MAX_DRAFT_RESULT_BYTES: usize = MAX_ENTRY_BYTES;
pub const MAX_PARAGRAPH_BYTES: usize = MAX_STRING_BYTES * 8;
pub const MAX_SUGGESTION_EXPLANATION_BYTES: usize = MAX_STRING_BYTES;
pub const MAX_REFLECTION_RANGES: usize = 32;
pub const MAX_PREPARED_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_SOURCE_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
pub const WORKFLOW_PAGE_SIZE: usize = 50;
pub const SOURCE_PAGE_SIZE: usize = MAX_COLLECTION_ITEMS;
pub const MAX_WORKFLOWS: usize = 256;
pub const JOB_QUEUE_CAPACITY: usize = 32;
pub const JOB_CONCURRENCY: usize = 4;
pub const MAX_JOBS: usize = 128;
pub const MAX_SUBSCRIBERS: usize = 16;
pub const EVENT_REPLAY_CAPACITY: usize = 256;
pub const SUBSCRIBER_QUEUE_CAPACITY: usize = 64;

/// The `JobDto::status` values the server emits. Clients branch on these
/// constants rather than literals so a renamed status fails to compile
/// instead of silently never matching. Timeouts are `failed` with an
/// `error_code`, never a separate status.
pub mod job_status {
    pub const QUEUED: &str = "queued";
    pub const RUNNING: &str = "running";
    pub const CANCELLATION_REQUESTED: &str = "cancellation-requested";
    pub const COMPLETED: &str = "completed";
    pub const FAILED: &str = "failed";
    pub const CANCELLED: &str = "cancelled";

    /// Whether a job in `status` will never change again.
    #[must_use]
    pub fn is_terminal(status: &str) -> bool {
        matches!(status, COMPLETED | FAILED | CANCELLED)
    }

    /// Whether a job in `status` still occupies queue or worker capacity.
    #[must_use]
    pub fn is_active(status: &str) -> bool {
        matches!(status, QUEUED | RUNNING | CANCELLATION_REQUESTED)
    }
}

/// The `ProblemDto::code` for a job ID the server does not (or no longer)
/// knows, e.g. after its terminal result expired.
pub const JOB_NOT_FOUND: &str = "job-not-found";

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LocalPreviewDto {
    pub workflow: WorkflowDto,
    pub target: String,
    pub payload: String,
    pub payload_digest: String,
    pub plan_digest: String,
    pub existing_text: Option<String>,
    pub intended_changes: Vec<String>,
    pub local_status: String,
    pub notion_status: String,
    /// Mirror target label and status; absent when no mirror is configured
    /// or from a server that predates the mirror kind.
    #[serde(default)]
    pub icloud_target: Option<String>,
    #[serde(default)]
    pub icloud_status: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LocalApprovalRequest {
    pub plan_digest: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NotionBlockDto {
    pub kind: String,
    pub text: String,
    pub depth: u8,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NotionPreviewDto {
    pub workflow: WorkflowDto,
    pub path: Vec<String>,
    pub year_exists: bool,
    pub month_exists: bool,
    pub renderer_version: u32,
    pub api_version: String,
    pub payload_digest: String,
    pub plan_digest: Option<String>,
    pub blocks: Vec<NotionBlockDto>,
    pub status: String,
    pub reason: Option<String>,
}

/// Present on a workflow while a completed day is being amended.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AmendmentDto {
    /// The local file no longer matched its receipt when the amendment was
    /// seeded (or reread): the changed-since-publication banner.
    pub hand_edited: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AmendmentNotionDto {
    pub current_blocks: Vec<NotionBlockDto>,
    pub replacement_blocks: Vec<NotionBlockDto>,
    pub status: String,
    pub reason: Option<String>,
}

/// Current against replacement content per destination.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AmendmentPreviewDto {
    pub workflow: WorkflowDto,
    pub plan_digest: String,
    pub local_target: String,
    pub local_current: String,
    pub local_payload: String,
    pub local_status: String,
    pub notion: Option<AmendmentNotionDto>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SaveUnderDayRequest {
    pub diary_day: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RouteSpec {
    pub method: &'static str,
    pub path: &'static str,
    pub capability: Option<&'static str>,
    pub authenticated: bool,
}

pub const ROUTES: &[RouteSpec] = &[
    RouteSpec {
        method: "GET",
        path: "/v1/health",
        capability: None,
        authenticated: false,
    },
    RouteSpec {
        method: "GET",
        path: "/v1/capabilities",
        capability: Some("read"),
        authenticated: true,
    },
    RouteSpec {
        method: "GET",
        path: "/v1/setup-check",
        capability: Some("read"),
        authenticated: true,
    },
    RouteSpec {
        method: "GET",
        path: "/v1/assistant-backends",
        capability: Some("read"),
        authenticated: true,
    },
    RouteSpec {
        method: "GET",
        path: "/v1/preferences",
        capability: Some("read"),
        authenticated: true,
    },
    RouteSpec {
        method: "PUT",
        path: "/v1/preferences",
        capability: Some("workflow:write"),
        authenticated: true,
    },
    RouteSpec {
        method: "GET",
        path: "/v1/workflows",
        capability: Some("read"),
        authenticated: true,
    },
    RouteSpec {
        method: "POST",
        path: "/v1/workflows",
        capability: Some("workflow:write"),
        authenticated: true,
    },
    RouteSpec {
        method: "GET",
        path: "/v1/workflows/{id}",
        capability: Some("read"),
        authenticated: true,
    },
    RouteSpec {
        method: "POST",
        path: "/v1/workflows/{id}/output-previews",
        capability: Some("workflow:write"),
        authenticated: true,
    },
    RouteSpec {
        method: "POST",
        path: "/v1/workflows/{id}/output-approvals",
        capability: Some("workflow:write"),
        authenticated: true,
    },
    RouteSpec {
        method: "POST",
        path: "/v1/workflows/{id}/local-publications",
        capability: Some("workflow:write"),
        authenticated: true,
    },
    RouteSpec {
        method: "POST",
        path: "/v1/workflows/{id}/notion-previews",
        capability: Some("workflow:write"),
        authenticated: true,
    },
    RouteSpec {
        method: "POST",
        path: "/v1/workflows/{id}/notion-approvals",
        capability: Some("workflow:write"),
        authenticated: true,
    },
    RouteSpec {
        method: "POST",
        path: "/v1/workflows/{id}/notion-publications",
        capability: Some("workflow:write"),
        authenticated: true,
    },
    RouteSpec {
        method: "POST",
        path: "/v1/workflows/{id}/amendments",
        capability: Some("workflow:write"),
        authenticated: true,
    },
    RouteSpec {
        method: "POST",
        path: "/v1/workflows/{id}/amendments/previews",
        capability: Some("workflow:write"),
        authenticated: true,
    },
    RouteSpec {
        method: "POST",
        path: "/v1/workflows/{id}/amendments/approvals",
        capability: Some("workflow:write"),
        authenticated: true,
    },
    RouteSpec {
        method: "POST",
        path: "/v1/workflows/{id}/amendments/publications",
        capability: Some("workflow:write"),
        authenticated: true,
    },
    RouteSpec {
        method: "POST",
        path: "/v1/workflows/{id}/amendments/notion-retries",
        capability: Some("workflow:write"),
        authenticated: true,
    },
    RouteSpec {
        method: "POST",
        path: "/v1/workflows/{id}/amendments/rereads",
        capability: Some("workflow:write"),
        authenticated: true,
    },
    RouteSpec {
        method: "POST",
        path: "/v1/workflows/{id}/amendments/abandonments",
        capability: Some("workflow:write"),
        authenticated: true,
    },
    RouteSpec {
        method: "POST",
        path: "/v1/workflows/{id}/save-under-day",
        capability: Some("workflow:write"),
        authenticated: true,
    },
    RouteSpec {
        method: "DELETE",
        path: "/v1/workflows/{id}",
        capability: Some("workflow:write"),
        authenticated: true,
    },
    RouteSpec {
        method: "POST",
        path: "/v1/workflows/{id}/discoveries",
        capability: Some("source:read"),
        authenticated: true,
    },
    RouteSpec {
        method: "GET",
        path: "/v1/workflows/{id}/sources",
        capability: Some("read"),
        authenticated: true,
    },
    RouteSpec {
        method: "GET",
        path: "/v1/workflows/{id}/sources/{source_id}",
        capability: Some("read"),
        authenticated: true,
    },
    RouteSpec {
        method: "POST",
        path: "/v1/workflows/{id}/sources/{source_id}/refine-requests",
        capability: Some("workflow:write"),
        authenticated: true,
    },
    RouteSpec {
        method: "GET",
        path: "/v1/workflows/{id}/sources/{source_id}/refine-suggestion",
        capability: Some("read"),
        authenticated: true,
    },
    RouteSpec {
        method: "PUT",
        path: "/v1/workflows/{id}/sources/{source_id}/refine-suggestion/resolution",
        capability: Some("workflow:write"),
        authenticated: true,
    },
    RouteSpec {
        method: "PUT",
        path: "/v1/workflows/{id}/sources/{source_id}/body",
        capability: Some("workflow:write"),
        authenticated: true,
    },
    RouteSpec {
        method: "DELETE",
        path: "/v1/workflows/{id}/sources/{source_id}",
        capability: Some("workflow:write"),
        authenticated: true,
    },
    RouteSpec {
        method: "POST",
        path: "/v1/workflows/{id}/sources/prepare",
        capability: Some("source:read"),
        authenticated: true,
    },
    RouteSpec {
        method: "PUT",
        path: "/v1/workflows/{id}/selection",
        capability: Some("workflow:write"),
        authenticated: true,
    },
    RouteSpec {
        method: "POST",
        path: "/v1/workflows/{id}/empty-selection-confirmations",
        capability: Some("workflow:write"),
        authenticated: true,
    },
    RouteSpec {
        method: "PUT",
        path: "/v1/workflows/{id}/writing-settings",
        capability: Some("workflow:write"),
        authenticated: true,
    },
    RouteSpec {
        method: "GET",
        path: "/v1/workflows/{id}/assistant-selection",
        capability: Some("read"),
        authenticated: true,
    },
    RouteSpec {
        method: "PUT",
        path: "/v1/workflows/{id}/assistant-selection",
        capability: Some("workflow:write"),
        authenticated: true,
    },
    RouteSpec {
        method: "POST",
        path: "/v1/workflows/{id}/memory-extractions",
        capability: Some("workflow:write"),
        authenticated: true,
    },
    RouteSpec {
        method: "GET",
        path: "/v1/workflows/{id}/memory-review",
        capability: Some("read"),
        authenticated: true,
    },
    RouteSpec {
        method: "PUT",
        path: "/v1/workflows/{id}/memory-review",
        capability: Some("workflow:write"),
        authenticated: true,
    },
    RouteSpec {
        method: "POST",
        path: "/v1/workflows/{id}/draft-requests",
        capability: Some("workflow:write"),
        authenticated: true,
    },
    RouteSpec {
        method: "GET",
        path: "/v1/workflows/{id}/draft-result",
        capability: Some("read"),
        authenticated: true,
    },
    RouteSpec {
        method: "POST",
        path: "/v1/workflows/{id}/paragraph-review-requests",
        capability: Some("workflow:write"),
        authenticated: true,
    },
    RouteSpec {
        method: "GET",
        path: "/v1/workflows/{id}/paragraph-suggestions",
        capability: Some("read"),
        authenticated: true,
    },
    RouteSpec {
        method: "GET",
        path: "/v1/workflows/{id}/paragraph-suggestions/{suggestion_id}",
        capability: Some("read"),
        authenticated: true,
    },
    RouteSpec {
        method: "PUT",
        path: "/v1/workflows/{id}/paragraph-suggestions/{suggestion_id}/resolution",
        capability: Some("workflow:write"),
        authenticated: true,
    },
    RouteSpec {
        method: "POST",
        path: "/v1/workflows/{id}/manual-compositions",
        capability: Some("workflow:write"),
        authenticated: true,
    },
    RouteSpec {
        method: "PUT",
        path: "/v1/workflows/{id}/entry",
        capability: Some("workflow:write"),
        authenticated: true,
    },
    RouteSpec {
        method: "POST",
        path: "/v1/workflows/{id}/entry-confirmations",
        capability: Some("workflow:write"),
        authenticated: true,
    },
    RouteSpec {
        method: "GET",
        path: "/v1/jobs/{id}",
        capability: Some("read"),
        authenticated: true,
    },
    RouteSpec {
        method: "DELETE",
        path: "/v1/jobs/{id}",
        capability: Some("workflow:write"),
        authenticated: true,
    },
    RouteSpec {
        method: "GET",
        path: "/v1/events",
        capability: Some("read"),
        authenticated: true,
    },
];

macro_rules! dto {
    ($(#[$meta:meta])* $visibility:vis struct $name:ident { $($body:tt)* }) => {
        $(#[$meta])*
        #[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
        #[serde(deny_unknown_fields)]
        $visibility struct $name { $($body)* }
    };
}

dto!(
    pub struct HealthDto {
        pub api_version: String,
        pub contract_version: u32,
        pub instance_id: String,
        pub build_version: String,
        pub ready: bool,
    }
);

dto!(
    pub struct CapabilitiesDto {
        pub capabilities: Vec<String>,
        pub manual_workflow_available: bool,
        pub assistant_backends: Vec<AssistantBackendDto>,
        pub disabled_routes: Vec<String>,
    }
);

dto!(
    pub struct AssistantBackendDto {
        pub id: String,
        pub display_label: String,
        pub composition_path: String,
        pub available: bool,
        pub reason_code: Option<String>,
        pub supported_languages: Vec<String>,
        pub supported_actions: Vec<String>,
    }
);

dto!(
    pub struct AssistantBackendListDto {
        pub backends: Vec<AssistantBackendDto>,
    }
);

dto!(
    pub struct AssistantSelectionDto {
        pub backend_id: Option<String>,
        pub composition_path: Option<String>,
    }
);

dto!(
    pub struct PutAssistantSelectionRequest {
        pub backend_id: Option<String>,
    }
);

dto!(
    pub struct SetupCheckDto {
        pub config_safe: bool,
        pub database_ready: bool,
        pub apple_helper_available: bool,
        pub notion_configured: bool,
        pub notion_reason_code: Option<String>,
        pub manual_workflow_available: bool,
    }
);

dto!(
    pub struct PreferencesDto {
        pub last_language: Option<String>,
        pub last_assistant_backend_id: Option<String>,
    }
);

dto!(
    pub struct WorkflowListDto {
        pub workflows: Vec<WorkflowSummaryDto>,
        pub next_cursor: Option<String>,
    }
);

dto!(
    pub struct WorkflowSummaryDto {
        pub id: String,
        pub diary_day: String,
        pub state: String,
        pub revision: u64,
        pub buffer_version: u64,
        pub updated_at_unix_ms: i64,
        pub discarded: bool,
    }
);

dto!(
    struct WorkflowDtoWire {
        pub id: String,
        pub diary_day: String,
        pub state: String,
        pub revision: u64,
        pub buffer_version: u64,
        pub language: Option<String>,
        pub style: String,
        pub assistant_backend_id: Option<String>,
        pub composition_path: Option<String>,
        pub selection_digest: Option<String>,
        pub explicit_empty_selection: bool,
        pub invalidation_code: Option<String>,
        pub entry_text: String,
        pub entry_confirmed: bool,
        #[serde(default)]
        pub local_status: Option<String>,
        #[serde(default)]
        pub notion_status: Option<String>,
        #[serde(default)]
        pub icloud_status: Option<String>,
        #[serde(default)]
        pub amendment: Option<AmendmentDto>,
        pub candidates: Vec<SourceCandidateDto>,
    }
);

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct WorkflowDto {
    pub id: String,
    pub diary_day: String,
    pub state: String,
    pub revision: u64,
    pub buffer_version: u64,
    pub language: Option<String>,
    pub style: String,
    pub assistant_backend_id: Option<String>,
    pub composition_path: Option<String>,
    pub selection_digest: Option<String>,
    pub explicit_empty_selection: bool,
    pub invalidation_code: Option<String>,
    pub entry_text: String,
    pub entry_confirmed: bool,
    pub local_status: Option<String>,
    pub notion_status: Option<String>,
    pub icloud_status: Option<String>,
    pub amendment: Option<AmendmentDto>,
    pub candidates: Vec<SourceCandidateDto>,
}

impl TryFrom<WorkflowDtoWire> for WorkflowDto {
    type Error = &'static str;

    fn try_from(value: WorkflowDtoWire) -> Result<Self, Self::Error> {
        match (&value.assistant_backend_id, &value.composition_path) {
            (None, None) => {}
            (Some(backend_id), Some(path))
                if assistant_backend_id_is_valid(backend_id)
                    && matches!(path.as_str(), "hosted" | "apple-on-device") => {}
            _ => return Err("invalid assistant selection"),
        }
        Ok(Self {
            id: value.id,
            diary_day: value.diary_day,
            state: value.state,
            revision: value.revision,
            buffer_version: value.buffer_version,
            language: value.language,
            style: value.style,
            assistant_backend_id: value.assistant_backend_id,
            composition_path: value.composition_path,
            selection_digest: value.selection_digest,
            explicit_empty_selection: value.explicit_empty_selection,
            invalidation_code: value.invalidation_code,
            entry_text: value.entry_text,
            entry_confirmed: value.entry_confirmed,
            local_status: value.local_status,
            notion_status: value.notion_status,
            icloud_status: value.icloud_status,
            amendment: value.amendment,
            candidates: value.candidates,
        })
    }
}

impl<'de> Deserialize<'de> for WorkflowDto {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = WorkflowDtoWire::deserialize(deserializer)?;
        Self::try_from(wire).map_err(serde::de::Error::custom)
    }
}

fn assistant_backend_id_is_valid(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_' | b'.')
        })
}

dto!(
    pub struct SourceCandidateDto {
        pub id: String,
        pub source_note_name: String,
        /// Local `YYYY-MM-DD HH:MM:SS` derived from the note's UTC file name.
        pub local_time: Option<String>,
        pub directory_day: Option<String>,
        pub header_day: Option<String>,
        pub warning_code: Option<String>,
        pub availability: String,
        pub selected: bool,
    }
);

dto!(
    pub struct SourceListDto {
        pub sources: Vec<SourceCandidateDto>,
        pub next_cursor: Option<String>,
    }
);

dto!(
    pub struct SourceDetailDto {
        pub id: String,
        pub source_note_name: String,
        /// Local `YYYY-MM-DD HH:MM:SS` derived from the note's UTC file name.
        pub local_time: Option<String>,
        pub body: String,
        pub content_digest: String,
        pub directory_day: Option<String>,
        pub header_day: Option<String>,
        pub warning_code: Option<String>,
    }
);

dto!(
    pub struct RefineSourceRequest {
        pub content_digest: String,
    }
);
dto!(
    pub struct SourceRefineSuggestionDto {
        pub source_id: String,
        pub original_body: String,
        pub suggested_body: String,
        pub binding_digest: String,
    }
);
dto!(
    pub struct ResolveSourceRefineRequest {
        pub accept: bool,
        pub binding_digest: String,
    }
);
dto!(
    pub struct EditSourceBodyRequest {
        pub text: String,
        pub expected_content_digest: String,
    }
);
dto!(
    pub struct DeleteSourceRequest {
        pub confirm: bool,
    }
);

dto!(
    pub struct StartWorkflowRequest {
        pub diary_day: Option<String>,
        pub language: Option<String>,
        pub style: Option<String>,
    }
);
dto!(
    pub struct DiscoveryRequest {
        pub explicit_date: Option<String>,
    }
);
dto!(
    pub struct PrepareSourcesRequest {
        pub source_ids: Vec<String>,
    }
);
dto!(
    pub struct ConfirmSelectionRequest {
        pub preparation_handle: String,
        pub source_ids: Vec<String>,
    }
);
dto!(
    pub struct WritingSettingsRequest {
        pub language: String,
        pub style: String,
        pub assistant_backend_id: Option<String>,
    }
);
dto!(
    pub struct SaveEntryRequest {
        pub text: String,
    }
);
dto!(
    pub struct ConfirmEntryRequest {
        pub buffer_etag: String,
    }
);
dto!(
    pub struct StartMemoryExtractionRequest {}
);
dto!(
    pub struct MemoryCandidateDto {
        pub id: String,
        pub text: String,
        pub source_snapshot_digests: Vec<String>,
        pub stale: bool,
    }
);
dto!(
    pub struct MemoryReviewDto {
        pub candidate_set_id: Option<String>,
        pub candidates: Vec<MemoryCandidateDto>,
        pub reviewed_memory: String,
        pub reviewed_memory_revision: u64,
        pub stale: bool,
    }
);
dto!(
    pub struct PutMemoryReviewRequest {
        pub candidate_set_id: String,
        pub reviewed_memory: String,
    }
);
dto!(
    pub struct StartDraftRequest {
        pub include_prior_draft: bool,
    }
);
dto!(
    pub struct ReflectionRangeDto {
        pub start: usize,
        pub end: usize,
    }
);
dto!(
    pub struct DraftResultDto {
        pub id: String,
        pub backend_id: String,
        pub composition_path: String,
        pub text: String,
        pub reflection_ranges: Vec<ReflectionRangeDto>,
        pub stale: bool,
        pub populated_buffer_version: Option<u64>,
    }
);
dto!(
    pub struct StartParagraphReviewRequest {
        pub buffer_etag: String,
        pub range_start: usize,
        pub range_end: usize,
        pub selected_text_digest: String,
    }
);
dto!(
    pub struct ParagraphSuggestionSummaryDto {
        pub id: String,
        pub backend_id: String,
        pub composition_path: String,
        pub stale: bool,
        pub resolution: Option<String>,
    }
);
dto!(
    pub struct ParagraphSuggestionListDto {
        pub suggestions: Vec<ParagraphSuggestionSummaryDto>,
    }
);
dto!(
    pub struct ParagraphSuggestionDto {
        pub id: String,
        pub backend_id: String,
        pub composition_path: String,
        pub replacement: String,
        pub reflection_ranges: Vec<ReflectionRangeDto>,
        pub explanation: Option<String>,
        pub buffer_version: u64,
        pub range_start: usize,
        pub range_end: usize,
        pub selected_text_digest: String,
        pub stale: bool,
        pub resolution: Option<String>,
    }
);
dto!(
    pub struct ResolveParagraphSuggestionRequest {
        pub resolution: String,
        pub buffer_etag: String,
        pub range_start: usize,
        pub range_end: usize,
        pub selected_text_digest: String,
    }
);
dto!(
    pub struct JobAcceptedDto {
        pub job_id: String,
    }
);

dto!(
    pub struct JobDto {
        pub id: String,
        pub kind: String,
        pub status: String,
        pub workflow_id: String,
        pub progress_percent: u8,
        pub result_handle: Option<String>,
        pub error_code: Option<String>,
        pub expires_at_unix_ms: i64,
    }
);

dto!(
    pub struct EventDto {
        pub id: u64,
        pub instance_id: String,
        pub event_type: String,
        pub workflow_id: Option<String>,
        pub revision: Option<u64>,
        pub job_id: Option<String>,
        pub progress_percent: Option<u8>,
    }
);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProblemDto {
    #[serde(rename = "type")]
    pub type_uri: String,
    pub title: String,
    pub status: u16,
    pub code: String,
    pub instance: Option<String>,
}

#[must_use]
pub fn workflow_id(day: &str) -> Option<String> {
    if day.len() == 10
        && day.as_bytes()[4] == b'-'
        && day.as_bytes()[7] == b'-'
        && day
            .bytes()
            .enumerate()
            .all(|(index, byte)| index == 4 || index == 7 || byte.is_ascii_digit())
    {
        Some(format!("wf_{}{}{}", &day[0..4], &day[5..7], &day[8..10]))
    } else {
        None
    }
}

#[must_use]
pub fn workflow_day(id: &str) -> Option<String> {
    let raw = id.strip_prefix("wf_")?;
    if raw.len() != 8 || !raw.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    Some(format!("{}-{}-{}", &raw[0..4], &raw[4..6], &raw[6..8]))
}

#[must_use]
pub fn workflow_etag(revision: u64) -> String {
    format!("\"wf-{revision}\"")
}

#[must_use]
pub fn buffer_etag(version: u64) -> String {
    format!("\"buf-{version}\"")
}

#[must_use]
pub fn parse_workflow_etag(value: &str) -> Option<u64> {
    parse_etag(value, "wf-")
}

#[must_use]
pub fn parse_buffer_etag(value: &str) -> Option<u64> {
    parse_etag(value, "buf-")
}

fn parse_etag(value: &str, prefix: &str) -> Option<u64> {
    let inner = value.strip_prefix('"')?.strip_suffix('"')?;
    inner.strip_prefix(prefix)?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_job_status_is_exactly_one_of_active_or_terminal() {
        use job_status::*;
        for status in [
            QUEUED,
            RUNNING,
            CANCELLATION_REQUESTED,
            COMPLETED,
            FAILED,
            CANCELLED,
        ] {
            assert_ne!(is_active(status), is_terminal(status), "{status}");
        }
        assert!(!is_terminal("timed-out"));
        assert!(!is_active("timed-out"));
    }

    #[test]
    fn ids_and_strong_etags_have_frozen_grammar() {
        assert_eq!(workflow_id("2026-10-29").as_deref(), Some("wf_20261029"));
        assert_eq!(workflow_day("wf_20261029").as_deref(), Some("2026-10-29"));
        assert_eq!(workflow_etag(7), "\"wf-7\"");
        assert_eq!(buffer_etag(8), "\"buf-8\"");
        assert_eq!(parse_workflow_etag("\"wf-7\""), Some(7));
        assert_eq!(parse_buffer_etag("W/\"buf-8\""), None);
    }

    #[test]
    fn request_dtos_reject_unknown_fields() {
        let error =
            serde_json::from_str::<SaveEntryRequest>(r#"{"text":"synthetic","unexpected":true}"#)
                .unwrap_err();
        assert!(error.to_string().contains("unknown field"));
        let settings: WritingSettingsRequest = serde_json::from_str(
            r#"{"language":"en-US","style":"concise","assistant_backend_id":null}"#,
        )
        .unwrap();
        assert_eq!(settings.assistant_backend_id, None);
        let legacy = serde_json::from_str::<WritingSettingsRequest>(
            r#"{"language":"en-US","style":"concise","composition_path":"manual"}"#,
        )
        .unwrap_err();
        assert!(legacy.to_string().contains("unknown field"));
        let legacy = serde_json::from_str::<PreferencesDto>(
            r#"{"last_language":"en-US","last_composition_path":"manual"}"#,
        )
        .unwrap_err();
        assert!(legacy.to_string().contains("unknown field"));
    }

    #[test]
    fn assistant_backend_descriptor_has_collection_shaped_action_contract() {
        let descriptor = AssistantBackendDto {
            id: "synthetic-hosted".to_owned(),
            display_label: "Synthetic hosted".to_owned(),
            composition_path: "hosted".to_owned(),
            available: true,
            reason_code: None,
            supported_languages: vec!["en-US".to_owned(), "ko-KR".to_owned()],
            supported_actions: vec!["extract-memories".to_owned(), "draft-entry".to_owned()],
        };
        assert_eq!(
            serde_json::to_value(descriptor).unwrap(),
            serde_json::json!({
                "id": "synthetic-hosted",
                "display_label": "Synthetic hosted",
                "composition_path": "hosted",
                "available": true,
                "reason_code": null,
                "supported_languages": ["en-US", "ko-KR"],
                "supported_actions": ["extract-memories", "draft-entry"]
            })
        );
    }

    #[test]
    fn workflow_dto_rejects_partial_or_malformed_assistant_selection() {
        let base = serde_json::json!({
            "id": "wf_20261029",
            "diary_day": "2026-10-29",
            "state": "composing",
            "revision": 2,
            "buffer_version": 0,
            "language": "en-US",
            "style": "concise",
            "assistant_backend_id": null,
            "composition_path": null,
            "selection_digest": null,
            "explicit_empty_selection": true,
            "invalidation_code": null,
            "entry_text": "",
            "entry_confirmed": false,
            "candidates": []
        });
        assert!(serde_json::from_value::<WorkflowDto>(base.clone()).is_ok());

        for (backend_id, path) in [
            (serde_json::json!("hosted-default"), serde_json::Value::Null),
            (serde_json::Value::Null, serde_json::json!("hosted")),
            (
                serde_json::json!("Hosted Default"),
                serde_json::json!("hosted"),
            ),
            (
                serde_json::json!("hosted-default"),
                serde_json::json!("manual"),
            ),
        ] {
            let mut invalid = base.clone();
            invalid["assistant_backend_id"] = backend_id;
            invalid["composition_path"] = path;
            assert!(serde_json::from_value::<WorkflowDto>(invalid).is_err());
        }

        let mut valid = base;
        valid["assistant_backend_id"] = serde_json::json!("hosted-default");
        valid["composition_path"] = serde_json::json!("hosted");
        assert!(serde_json::from_value::<WorkflowDto>(valid).is_ok());
    }

    #[test]
    fn route_table_contains_local_and_notion_publication_and_excludes_future_surfaces() {
        assert!(ROUTES.iter().all(|route| !route.path.contains("mcp")));
        for suffix in [
            "/amendments",
            "/amendments/previews",
            "/amendments/approvals",
            "/amendments/publications",
            "/amendments/notion-retries",
            "/amendments/rereads",
            "/amendments/abandonments",
        ] {
            assert!(
                ROUTES.iter().any(|route| route.path.ends_with(suffix)),
                "{suffix}"
            );
        }
        assert_eq!(CONTRACT_VERSION, 5, "amendments bumped the contract");
        assert!(
            ROUTES
                .iter()
                .any(|route| route.path.ends_with("/local-publications"))
        );
        for suffix in [
            "/notion-previews",
            "/notion-approvals",
            "/notion-publications",
        ] {
            assert!(ROUTES.iter().any(|route| route.path.ends_with(suffix)));
        }
        for path in [
            "/v1/assistant-backends",
            "/v1/workflows/{id}/memory-extractions",
            "/v1/workflows/{id}/draft-result",
            "/v1/workflows/{id}/paragraph-suggestions/{suggestion_id}/resolution",
        ] {
            assert!(ROUTES.iter().any(|route| route.path == path));
        }
        assert_eq!(
            ROUTES.iter().filter(|route| !route.authenticated).count(),
            1
        );
    }

    #[test]
    fn m8_request_dtos_reject_unknown_fields() {
        assert!(serde_json::from_str::<StartMemoryExtractionRequest>("{}").is_ok());
        assert!(
            serde_json::from_str::<StartMemoryExtractionRequest>(r#"{"unexpected":true}"#).is_err()
        );
        let request: ResolveParagraphSuggestionRequest = serde_json::from_str(
            r#"{"resolution":"accept","buffer_etag":"\"buf-3\"","range_start":0,"range_end":4,"selected_text_digest":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}"#,
        )
        .unwrap();
        assert_eq!(request.resolution, "accept");
    }
}
