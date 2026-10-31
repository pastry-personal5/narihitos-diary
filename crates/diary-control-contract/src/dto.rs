//! Strict wire representations grouped under the shared contract.

use super::*;

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
    /// `append` or `overwrite` when this plan replaces existing content;
    /// absent for an ordinary plan or a server that predates the choice.
    #[serde(default)]
    pub existing_mode: Option<String>,
}

/// Optional body of an output preview. Without `existing_mode`, differing
/// existing content is reported as a conflict.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalPreviewRequest {
    #[serde(default)]
    pub existing_mode: Option<String>,
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

/// One remote a completed day can be pushed to.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RemoteDto {
    /// Stable remote id: `icloud` or `notion`.
    pub id: String,
    pub label: String,
    /// `not-pushed`, `exists`, `pushed`, or `blocked`.
    pub status: String,
    /// Closed content-free token when the remote is blocked.
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RemoteListDto {
    pub remotes: Vec<RemoteDto>,
}

/// Optional body of a remote preview. Without `existing_mode`, content already
/// at the remote is shown but no Append or Overwrite plan is made.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemotePreviewRequest {
    #[serde(default)]
    pub existing_mode: Option<String>,
}

/// What a push would do, and what the remote holds now. The push is the
/// approval of exactly this plan.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RemotePreviewDto {
    pub id: String,
    pub label: String,
    pub status: String,
    #[serde(default)]
    pub reason: Option<String>,
    pub workflow: WorkflowDto,
    /// iCloud file path, or the Notion root / year / month path.
    pub target: Vec<String>,
    /// The exact iCloud text that would be written.
    #[serde(default)]
    pub payload: Option<String>,
    /// The iCloud text already present, shown automatically.
    #[serde(default)]
    pub existing_text: Option<String>,
    #[serde(default)]
    pub intended_changes: Vec<String>,
    /// Where the Notion section goes (`end`, `after YYYY-MM-DD`, `before ...`).
    #[serde(default)]
    pub placement: String,
    #[serde(default)]
    pub blocks: Vec<NotionBlockDto>,
    /// The Notion section already present, shown automatically.
    #[serde(default)]
    pub current_blocks: Vec<NotionBlockDto>,
    /// `append` or `overwrite` when this plan acts on existing content.
    #[serde(default)]
    pub existing_mode: Option<String>,
    /// Absent while there is nothing the person can push yet.
    #[serde(default)]
    pub plan_digest: Option<String>,
    /// Only for a Notion month page that cannot be changed safely: a URL to
    /// fix it by hand in the Notion app or on Notion Web.
    #[serde(default)]
    pub page_url: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemotePushRequest {
    pub plan_digest: String,
    #[serde(default)]
    pub existing_mode: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RemoteReadinessDto {
    /// The workflow revision the answer describes.
    pub revision: u64,
    pub readiness: DestinationReadinessDto,
}

/// One destination's read-only readiness. `reason` and `next_step` are closed
/// content-free tokens; the client owns the wording.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DestinationReadinessDto {
    pub destination: String,
    pub readiness: String,
    pub reason: String,
    pub next_step: String,
}

/// Present on a workflow while a completed day is being amended.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AmendmentDto {
    /// The local file no longer matched its receipt when the amendment was
    /// seeded (or reread): the changed-since-publication banner.
    pub hand_edited: bool,
}

/// The current file against its replacement.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AmendmentPreviewDto {
    pub workflow: WorkflowDto,
    pub plan_digest: String,
    pub local_target: String,
    pub local_current: String,
    pub local_payload: String,
    pub local_status: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SaveUnderDayRequest {
    pub diary_day: String,
}

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
