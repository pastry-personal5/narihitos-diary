//! Typed API client boundary. ViewModels call purpose-named operations here;
//! nothing above this module constructs a request path, reads a header name,
//! or parses JSON by hand.

mod attach;
mod problem;
mod sse;
#[cfg(test)]
mod tests;
mod transport;

pub use attach::{AttachmentState, verify_health};
pub use problem::{ClientOutcome, ProblemView};
pub use sse::{SseError, SseParser};
pub use transport::{
    Attachment, RawRequest, RawResponse, Transport, TransportError, UreqTransport,
};

use diary_control_contract::{
    CapabilitiesDto, ConfirmEntryRequest, ConfirmSelectionRequest, DiscoveryRequest, HealthDto,
    JobAcceptedDto, JobDto, MAX_RESPONSE_BYTES, MAX_SOURCE_BYTES, PreferencesDto,
    PrepareSourcesRequest, SaveEntryRequest, SetupCheckDto, SourceDetailDto, SourceListDto,
    WorkflowDto, WorkflowListDto, WritingSettingsRequest, buffer_etag, workflow_etag,
};

const SOURCE_DETAIL_RESPONSE_BYTES: usize = MAX_SOURCE_BYTES + 64 * 1024;

/// A source-detail body plus the server-issued preparation handle that
/// authorized its read. The handle originates in the completed preparation
/// job's `result_handle`, is sent as `X-ND-Preparation-Handle`, and is retained
/// only after the server accepts that bound detail read.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceDetailResponse {
    pub detail: SourceDetailDto,
    pub preparation_handle: Option<String>,
}

/// The only route from a ViewModel to persisted workflow state.
pub struct ApiClient<T: Transport> {
    transport: T,
}

impl<T: Transport> ApiClient<T> {
    #[must_use]
    pub const fn new(transport: T) -> Self {
        Self { transport }
    }

    /// Unauthenticated: must be safe to call before the bearer token is
    /// trusted, and before any mutation is attempted.
    pub fn health(&self, attachment: &Attachment) -> ClientOutcome<HealthDto> {
        let raw = RawRequest {
            method: "GET",
            path: "/v1/health".to_owned(),
            authenticated: false,
            body: None,
            if_match: None,
            preparation_handle: None,
            max_response_bytes: MAX_RESPONSE_BYTES,
        };
        self.dispatch(attachment, raw)
    }

    pub fn preferences(&self, attachment: &Attachment) -> ClientOutcome<PreferencesDto> {
        self.get_json("/v1/preferences", MAX_RESPONSE_BYTES, attachment)
    }

    pub fn capabilities(&self, attachment: &Attachment) -> ClientOutcome<CapabilitiesDto> {
        self.get_json("/v1/capabilities", MAX_RESPONSE_BYTES, attachment)
    }

    pub fn setup_check(&self, attachment: &Attachment) -> ClientOutcome<SetupCheckDto> {
        self.get_json("/v1/setup-check", MAX_RESPONSE_BYTES, attachment)
    }

    pub fn save_preferences(
        &self,
        attachment: &Attachment,
        preferences: &PreferencesDto,
    ) -> ClientOutcome<PreferencesDto> {
        let body = serde_json::to_vec(preferences).unwrap_or_default();
        self.send_json(
            "PUT",
            "/v1/preferences",
            Some(body),
            None,
            MAX_RESPONSE_BYTES,
            attachment,
        )
    }

    pub fn list_workflows(
        &self,
        attachment: &Attachment,
        cursor: Option<&str>,
    ) -> ClientOutcome<WorkflowListDto> {
        let path = match cursor {
            Some(cursor) => format!("/v1/workflows?cursor={}", urlencode(cursor)),
            None => "/v1/workflows".to_owned(),
        };
        self.get_json(&path, MAX_RESPONSE_BYTES, attachment)
    }

    pub fn start_workflow(
        &self,
        attachment: &Attachment,
        diary_day: Option<&str>,
    ) -> ClientOutcome<WorkflowDto> {
        let body = serde_json::to_vec(&diary_control_contract::StartWorkflowRequest {
            diary_day: diary_day.map(ToOwned::to_owned),
        })
        .unwrap_or_default();
        self.send_json(
            "POST",
            "/v1/workflows",
            Some(body),
            None,
            MAX_RESPONSE_BYTES,
            attachment,
        )
    }

    pub fn get_workflow(&self, attachment: &Attachment, id: &str) -> ClientOutcome<WorkflowDto> {
        self.get_json(
            &format!("/v1/workflows/{id}"),
            MAX_RESPONSE_BYTES,
            attachment,
        )
    }

    pub fn discard_workflow(
        &self,
        attachment: &Attachment,
        id: &str,
        revision: u64,
    ) -> ClientOutcome<()> {
        self.send_empty(
            "DELETE",
            &format!("/v1/workflows/{id}"),
            Some(workflow_etag(revision)),
            attachment,
        )
    }

    /// Before confirmation, a source-detail read carries the active
    /// preparation handle in its `X-ND-Preparation-Handle` response
    /// header (see the architecture doc's "Client-to-server boundary"
    /// table). `source_detail` alone would drop this header; callers that
    /// need to confirm a selection must use this instead.
    pub fn source_detail_with_handle(
        &self,
        attachment: &Attachment,
        workflow_id: &str,
        source_id: &str,
        preparation_handle: &str,
    ) -> ClientOutcome<SourceDetailResponse> {
        let raw = RawRequest {
            method: "GET",
            path: format!("/v1/workflows/{workflow_id}/sources/{source_id}"),
            authenticated: true,
            body: None,
            if_match: None,
            preparation_handle: Some(preparation_handle.to_owned()),
            max_response_bytes: SOURCE_DETAIL_RESPONSE_BYTES,
        };
        match self.transport.execute(attachment, &raw) {
            Ok(response) if (200..300).contains(&response.status) => {
                match serde_json::from_slice(&response.body) {
                    Ok(detail) => ClientOutcome::Ok(SourceDetailResponse {
                        detail,
                        preparation_handle: Some(preparation_handle.to_owned()),
                    }),
                    Err(_) => ClientOutcome::Transport(TransportError::MalformedResponse),
                }
            }
            Ok(response) => ClientOutcome::Problem(problem::parse(&response)),
            Err(error) => ClientOutcome::Transport(error),
        }
    }

    pub fn source_detail(
        &self,
        attachment: &Attachment,
        workflow_id: &str,
        source_id: &str,
    ) -> ClientOutcome<SourceDetailDto> {
        self.get_json(
            &format!("/v1/workflows/{workflow_id}/sources/{source_id}"),
            SOURCE_DETAIL_RESPONSE_BYTES,
            attachment,
        )
    }

    pub fn set_writing_settings(
        &self,
        attachment: &Attachment,
        workflow_id: &str,
        revision: u64,
        language: &str,
        style: &str,
    ) -> ClientOutcome<WorkflowDto> {
        let body = serde_json::to_vec(&WritingSettingsRequest {
            language: language.to_owned(),
            style: style.to_owned(),
            assistant_backend_id: None,
        })
        .unwrap_or_default();
        self.send_json(
            "PUT",
            &format!("/v1/workflows/{workflow_id}/writing-settings"),
            Some(body),
            Some(workflow_etag(revision)),
            MAX_RESPONSE_BYTES,
            attachment,
        )
    }

    /// Switches the workflow into manual composition. Manual editing is
    /// not itself a backend/composition path; this only advances state so
    /// the Draft editor becomes available.
    pub fn begin_manual_composition(
        &self,
        attachment: &Attachment,
        workflow_id: &str,
        revision: u64,
    ) -> ClientOutcome<WorkflowDto> {
        self.send_json(
            "POST",
            &format!("/v1/workflows/{workflow_id}/manual-compositions"),
            None,
            Some(workflow_etag(revision)),
            MAX_RESPONSE_BYTES,
            attachment,
        )
    }

    /// Autosave uses only the entry-buffer ETag as `If-Match` — never the
    /// workflow ETag — per the architecture doc's "Client-to-server
    /// boundary" table.
    pub fn save_entry(
        &self,
        attachment: &Attachment,
        workflow_id: &str,
        buffer_version: u64,
        text: &str,
    ) -> ClientOutcome<WorkflowDto> {
        let body = serde_json::to_vec(&SaveEntryRequest {
            text: text.to_owned(),
        })
        .unwrap_or_default();
        self.send_json(
            "PUT",
            &format!("/v1/workflows/{workflow_id}/entry"),
            Some(body),
            Some(buffer_etag(buffer_version)),
            MAX_RESPONSE_BYTES,
            attachment,
        )
    }

    /// Confirmation supplies both current versions: the workflow ETag as
    /// `If-Match` and the entry-buffer ETag in the request body.
    pub fn confirm_entry(
        &self,
        attachment: &Attachment,
        workflow_id: &str,
        revision: u64,
        buffer_version: u64,
    ) -> ClientOutcome<WorkflowDto> {
        let body = serde_json::to_vec(&ConfirmEntryRequest {
            buffer_etag: buffer_etag(buffer_version),
        })
        .unwrap_or_default();
        self.send_json(
            "POST",
            &format!("/v1/workflows/{workflow_id}/entry-confirmations"),
            Some(body),
            Some(workflow_etag(revision)),
            MAX_RESPONSE_BYTES,
            attachment,
        )
    }

    /// Accepts work as a job; the caller tracks completion with
    /// [`ApiClient::job`]. Never returns candidates directly or pretends
    /// discovery already finished.
    pub fn discover(
        &self,
        attachment: &Attachment,
        workflow_id: &str,
        revision: u64,
        explicit_date: Option<&str>,
    ) -> ClientOutcome<JobAcceptedDto> {
        let body = serde_json::to_vec(&DiscoveryRequest {
            explicit_date: explicit_date.map(ToOwned::to_owned),
        })
        .unwrap_or_default();
        self.send_json(
            "POST",
            &format!("/v1/workflows/{workflow_id}/discoveries"),
            Some(body),
            Some(workflow_etag(revision)),
            MAX_RESPONSE_BYTES,
            attachment,
        )
    }

    pub fn list_sources(
        &self,
        attachment: &Attachment,
        workflow_id: &str,
        cursor: Option<&str>,
    ) -> ClientOutcome<SourceListDto> {
        let path = match cursor {
            Some(cursor) => format!(
                "/v1/workflows/{workflow_id}/sources?cursor={}",
                urlencode(cursor)
            ),
            None => format!("/v1/workflows/{workflow_id}/sources"),
        };
        self.get_json(&path, MAX_RESPONSE_BYTES, attachment)
    }

    pub fn job(&self, attachment: &Attachment, job_id: &str) -> ClientOutcome<JobDto> {
        self.get_json(
            &format!("/v1/jobs/{job_id}"),
            MAX_RESPONSE_BYTES,
            attachment,
        )
    }

    /// Accepts the selected-only cloud preparation as a job; track it with
    /// [`ApiClient::job`] and read the resulting handle from the
    /// `X-ND-Preparation-Handle` header on a subsequent
    /// [`ApiClient::source_detail`] call, per the architecture doc.
    pub fn prepare_sources(
        &self,
        attachment: &Attachment,
        workflow_id: &str,
        revision: u64,
        source_ids: &[String],
    ) -> ClientOutcome<JobAcceptedDto> {
        let body = serde_json::to_vec(&PrepareSourcesRequest {
            source_ids: source_ids.to_vec(),
        })
        .unwrap_or_default();
        self.send_json(
            "POST",
            &format!("/v1/workflows/{workflow_id}/sources/prepare"),
            Some(body),
            Some(workflow_etag(revision)),
            MAX_RESPONSE_BYTES,
            attachment,
        )
    }

    pub fn confirm_selection(
        &self,
        attachment: &Attachment,
        workflow_id: &str,
        revision: u64,
        preparation_handle: &str,
        source_ids: &[String],
    ) -> ClientOutcome<WorkflowDto> {
        let body = serde_json::to_vec(&ConfirmSelectionRequest {
            preparation_handle: preparation_handle.to_owned(),
            source_ids: source_ids.to_vec(),
        })
        .unwrap_or_default();
        self.send_json(
            "PUT",
            &format!("/v1/workflows/{workflow_id}/selection"),
            Some(body),
            Some(workflow_etag(revision)),
            MAX_RESPONSE_BYTES,
            attachment,
        )
    }

    /// The only command that can advance a newly started workflow with
    /// zero included sources; never synthesized client-side from an empty
    /// candidate list.
    pub fn confirm_empty_selection(
        &self,
        attachment: &Attachment,
        workflow_id: &str,
        revision: u64,
    ) -> ClientOutcome<WorkflowDto> {
        self.send_json(
            "POST",
            &format!("/v1/workflows/{workflow_id}/empty-selection-confirmations"),
            None,
            Some(workflow_etag(revision)),
            MAX_RESPONSE_BYTES,
            attachment,
        )
    }

    fn get_json<D: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        max_response_bytes: usize,
        attachment: &Attachment,
    ) -> ClientOutcome<D> {
        let raw = RawRequest {
            method: "GET",
            path: path.to_owned(),
            authenticated: true,
            body: None,
            if_match: None,
            preparation_handle: None,
            max_response_bytes,
        };
        self.dispatch(attachment, raw)
    }

    fn send_json<D: serde::de::DeserializeOwned>(
        &self,
        method: &'static str,
        path: &str,
        body: Option<Vec<u8>>,
        if_match: Option<String>,
        max_response_bytes: usize,
        attachment: &Attachment,
    ) -> ClientOutcome<D> {
        let raw = RawRequest {
            method,
            path: path.to_owned(),
            authenticated: true,
            body,
            if_match,
            preparation_handle: None,
            max_response_bytes,
        };
        self.dispatch(attachment, raw)
    }

    fn send_empty(
        &self,
        method: &'static str,
        path: &str,
        if_match: Option<String>,
        attachment: &Attachment,
    ) -> ClientOutcome<()> {
        let raw = RawRequest {
            method,
            path: path.to_owned(),
            authenticated: true,
            body: None,
            if_match,
            preparation_handle: None,
            max_response_bytes: MAX_RESPONSE_BYTES,
        };
        match self.transport.execute(attachment, &raw) {
            Ok(response) if (200..300).contains(&response.status) => ClientOutcome::Ok(()),
            Ok(response) => ClientOutcome::Problem(problem::parse(&response)),
            Err(error) => ClientOutcome::Transport(error),
        }
    }

    fn dispatch<D: serde::de::DeserializeOwned>(
        &self,
        attachment: &Attachment,
        raw: RawRequest,
    ) -> ClientOutcome<D> {
        match self.transport.execute(attachment, &raw) {
            Ok(response) if (200..300).contains(&response.status) => {
                match serde_json::from_slice(&response.body) {
                    Ok(value) => ClientOutcome::Ok(value),
                    Err(_) => ClientOutcome::Transport(TransportError::MalformedResponse),
                }
            }
            Ok(response) => ClientOutcome::Problem(problem::parse(&response)),
            Err(error) => ClientOutcome::Transport(error),
        }
    }
}

pub(crate) fn urlencode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(byte as char);
        } else {
            out.push('%');
            out.push_str(&format!("{byte:02X}"));
        }
    }
    out
}
