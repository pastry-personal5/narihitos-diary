//! Sources-stage ViewModel: discover, inspect, include/exclude, prepare,
//! and confirm a source selection (or the explicit empty selection). The
//! active resource and the prospective included set are independent
//! presentation state; neither is synthesized from local data, and both
//! are reset only by an explicit confirmation or a fresh discovery.

use std::collections::BTreeSet;

use diary_control_contract::{SourceCandidateDto, WorkflowDto};

use crate::client::{ApiClient, Attachment, ClientOutcome, ProblemView, Transport, TransportError};
use crate::resources::{ResourceRow, ResourcesSidebarModel};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SourcesError {
    Problem(ProblemView),
    Transport(TransportError),
}

impl SourcesError {
    #[must_use]
    pub fn safe_label(&self) -> String {
        match self {
            Self::Problem(problem) => problem.safe_label(),
            Self::Transport(error) => error.safe_label().to_owned(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JobPhase {
    Running,
    Succeeded,
    Failed,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SourcesViewModel {
    pub candidates: Vec<SourceCandidateDto>,
    /// The prospective included source IDs. Independent of which source is
    /// the active (inspected) resource.
    pub included: BTreeSet<String>,
    pub pending_discovery_job: Option<String>,
    pub pending_preparation_job: Option<String>,
    /// The handle from the most recent `source_detail` read's
    /// `X-ND-Preparation-Handle` response header. `confirm_selection`
    /// takes a handle explicitly rather than reading this directly, so a
    /// caller can never confirm with a stale handle by accident.
    pub latest_preparation_handle: Option<String>,
    pub last_error: Option<SourcesError>,
}

impl SourcesViewModel {
    /// Starts discovery as a job. Does not create or guess candidates;
    /// the caller polls `poll_job` and then calls `load_sources` once the
    /// job succeeds.
    pub fn start_discovery<T: Transport>(
        &mut self,
        client: &ApiClient<T>,
        attachment: &Attachment,
        workflow_id: &str,
        revision: u64,
        explicit_date: Option<&str>,
    ) {
        match client.discover(attachment, workflow_id, revision, explicit_date) {
            ClientOutcome::Ok(job) => {
                self.pending_discovery_job = Some(job.job_id);
                self.last_error = None;
            }
            ClientOutcome::Problem(problem) => {
                self.last_error = Some(SourcesError::Problem(problem));
            }
            ClientOutcome::Transport(error) => {
                self.last_error = Some(SourcesError::Transport(error));
            }
        }
    }

    /// Starts selected-only preparation for the current `included` set as
    /// a job. The server, not this ViewModel, decides what is cloud-only
    /// and needs preparation.
    pub fn start_preparation<T: Transport>(
        &mut self,
        client: &ApiClient<T>,
        attachment: &Attachment,
        workflow_id: &str,
        revision: u64,
    ) {
        let source_ids: Vec<String> = self.included.iter().cloned().collect();
        match client.prepare_sources(attachment, workflow_id, revision, &source_ids) {
            ClientOutcome::Ok(job) => {
                self.pending_preparation_job = Some(job.job_id);
                self.last_error = None;
            }
            ClientOutcome::Problem(problem) => {
                self.last_error = Some(SourcesError::Problem(problem));
            }
            ClientOutcome::Transport(error) => {
                self.last_error = Some(SourcesError::Transport(error));
            }
        }
    }

    /// Polls a pending job by ID. Returns its phase on success; a
    /// transport/typed-problem failure records `last_error` and returns
    /// `None` rather than guessing the phase.
    pub fn poll_job<T: Transport>(
        &mut self,
        client: &ApiClient<T>,
        attachment: &Attachment,
        job_id: &str,
    ) -> Option<JobPhase> {
        match client.job(attachment, job_id) {
            ClientOutcome::Ok(job) => {
                self.last_error = None;
                match job.status.as_str() {
                    "succeeded" => Some(JobPhase::Succeeded),
                    "failed" | "cancelled" | "expired" => Some(JobPhase::Failed),
                    _ => Some(JobPhase::Running),
                }
            }
            ClientOutcome::Problem(problem) => {
                self.last_error = Some(SourcesError::Problem(problem));
                None
            }
            ClientOutcome::Transport(error) => {
                self.last_error = Some(SourcesError::Transport(error));
                None
            }
        }
    }

    /// Replaces `candidates` from the server-returned workflow. A fresh
    /// discovery must call this with the newly confirmed workflow, never
    /// merge local guesses into the candidate list.
    pub fn load_candidates_from(&mut self, workflow: &WorkflowDto) {
        self.candidates = workflow.candidates.clone();
        self.included
            .retain(|id| self.candidates.iter().any(|candidate| &candidate.id == id));
    }

    /// Toggles a candidate's prospective inclusion. Returns `false` (and
    /// does nothing) for an ID that is not a currently known candidate,
    /// since inclusion is never speculative.
    pub fn toggle_included(&mut self, source_id: &str) -> bool {
        if !self.candidates.iter().any(|c| c.id == source_id) {
            return false;
        }
        if !self.included.remove(source_id) {
            self.included.insert(source_id.to_owned());
        }
        true
    }

    /// Reads a prepared source body and records its preparation handle
    /// from the response header for a later `confirm_selection` call.
    /// Before confirmation this returns the prepared sanitized body;
    /// afterward it returns only the persisted confirmed snapshot (per
    /// the architecture doc) and the handle is absent.
    pub fn fetch_preparation_handle<T: Transport>(
        &mut self,
        client: &ApiClient<T>,
        attachment: &Attachment,
        workflow_id: &str,
        source_id: &str,
    ) -> Option<String> {
        match client.source_detail_with_handle(attachment, workflow_id, source_id) {
            ClientOutcome::Ok(response) => {
                self.last_error = None;
                self.latest_preparation_handle = response.preparation_handle.clone();
                response.preparation_handle
            }
            ClientOutcome::Problem(problem) => {
                self.last_error = Some(SourcesError::Problem(problem));
                None
            }
            ClientOutcome::Transport(error) => {
                self.last_error = Some(SourcesError::Transport(error));
                None
            }
        }
    }

    /// Confirms the current prospective set using a preparation handle
    /// obtained from a prior `fetch_preparation_handle` call. Never
    /// confirms by echoing a body or digest.
    pub fn confirm_selection<T: Transport>(
        &mut self,
        client: &ApiClient<T>,
        attachment: &Attachment,
        workflow_id: &str,
        revision: u64,
        preparation_handle: &str,
    ) -> Option<WorkflowDto> {
        let source_ids: Vec<String> = self.included.iter().cloned().collect();
        match client.confirm_selection(
            attachment,
            workflow_id,
            revision,
            preparation_handle,
            &source_ids,
        ) {
            ClientOutcome::Ok(workflow) => {
                self.last_error = None;
                Some(workflow)
            }
            ClientOutcome::Problem(problem) => {
                self.last_error = Some(SourcesError::Problem(problem));
                None
            }
            ClientOutcome::Transport(error) => {
                self.last_error = Some(SourcesError::Transport(error));
                None
            }
        }
    }

    /// The only way to advance a workflow with zero included sources.
    /// Requires an explicit human action; never triggered by an empty
    /// `included` set on its own.
    pub fn confirm_empty_selection<T: Transport>(
        &mut self,
        client: &ApiClient<T>,
        attachment: &Attachment,
        workflow_id: &str,
        revision: u64,
    ) -> Option<WorkflowDto> {
        match client.confirm_empty_selection(attachment, workflow_id, revision) {
            ClientOutcome::Ok(workflow) => {
                self.last_error = None;
                Some(workflow)
            }
            ClientOutcome::Problem(problem) => {
                self.last_error = Some(SourcesError::Problem(problem));
                None
            }
            ClientOutcome::Transport(error) => {
                self.last_error = Some(SourcesError::Transport(error));
                None
            }
        }
    }

    /// Renders candidates as Resources Sidebar rows. Active-resource
    /// focus is separate presentation state the caller tracks itself;
    /// this only reflects the prospective included/excluded state.
    #[must_use]
    pub fn resources(&self) -> ResourcesSidebarModel {
        ResourcesSidebarModel {
            rows: self
                .candidates
                .iter()
                .map(|candidate| ResourceRow {
                    label: candidate.source_note_name.clone(),
                    state: if self.included.contains(&candidate.id) {
                        "included".to_owned()
                    } else {
                        "excluded".to_owned()
                    },
                    indent: 0,
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use diary_control_contract::{JobAcceptedDto, JobDto};

    use super::*;
    use crate::client::{RawRequest, RawResponse};

    struct FakeTransport {
        responses: RefCell<Vec<(&'static str, String, RawResponse)>>,
    }

    impl FakeTransport {
        fn new() -> Self {
            Self {
                responses: RefCell::new(Vec::new()),
            }
        }

        fn push(&self, method: &'static str, path: &str, response: RawResponse) {
            self.responses
                .borrow_mut()
                .push((method, path.to_owned(), response));
        }
    }

    impl Transport for FakeTransport {
        fn execute(
            &self,
            _attachment: &Attachment,
            request: &RawRequest,
        ) -> Result<RawResponse, TransportError> {
            let mut responses = self.responses.borrow_mut();
            let index = responses
                .iter()
                .position(|(method, path, _)| *method == request.method && *path == request.path)
                .expect("unexpected request in fake transport");
            Ok(responses.remove(index).2)
        }
    }

    fn attachment() -> Attachment {
        Attachment::new(
            "instance_synthetic".to_owned(),
            "127.0.0.1".to_owned(),
            4000,
            "token_synthetic".to_owned(),
            2,
        )
    }

    fn candidate(id: &str, name: &str) -> SourceCandidateDto {
        SourceCandidateDto {
            id: id.to_owned(),
            source_note_name: name.to_owned(),
            directory_day: Some("2026-10-29".to_owned()),
            header_day: Some("2026-10-29".to_owned()),
            warning_code: None,
            availability: "local".to_owned(),
            selected: false,
        }
    }

    fn workflow_with_candidates(candidates: Vec<SourceCandidateDto>) -> WorkflowDto {
        serde_json::from_value(serde_json::json!({
            "id": "wf_20261029",
            "diary_day": "2026-10-29",
            "state": "discovered",
            "revision": 2,
            "buffer_version": 0,
            "language": null,
            "style": "concise",
            "assistant_backend_id": null,
            "composition_path": null,
            "selection_digest": null,
            "explicit_empty_selection": false,
            "invalidation_code": null,
            "entry_text": "",
            "entry_confirmed": false,
            "candidates": candidates
        }))
        .unwrap()
    }

    #[test]
    fn toggle_included_ignores_an_unknown_source_id() {
        let mut view = SourcesViewModel::default();
        assert!(!view.toggle_included("unknown"));
        assert!(view.included.is_empty());
    }

    #[test]
    fn toggle_included_flips_membership_for_a_known_candidate() {
        let mut view = SourcesViewModel {
            candidates: vec![candidate("src_1", "2026-10-28 note")],
            ..SourcesViewModel::default()
        };
        assert!(view.toggle_included("src_1"));
        assert!(view.included.contains("src_1"));
        assert!(view.toggle_included("src_1"));
        assert!(!view.included.contains("src_1"));
    }

    #[test]
    fn loading_candidates_drops_an_included_id_no_longer_present() {
        let mut view = SourcesViewModel {
            candidates: vec![candidate("src_1", "old note")],
            included: BTreeSet::from(["src_1".to_owned(), "src_stale".to_owned()]),
            ..SourcesViewModel::default()
        };
        view.load_candidates_from(&workflow_with_candidates(vec![candidate(
            "src_1", "old note",
        )]));
        assert!(view.included.contains("src_1"));
        assert!(!view.included.contains("src_stale"));
    }

    #[test]
    fn resources_reflects_prospective_inclusion_independent_of_server_selected_flag() {
        let mut view = SourcesViewModel {
            candidates: vec![candidate("src_1", "note a"), candidate("src_2", "note b")],
            ..SourcesViewModel::default()
        };
        view.toggle_included("src_1");
        let rows = view.resources();
        assert_eq!(rows.rows[0].state, "included");
        assert_eq!(rows.rows[1].state, "excluded");
    }

    #[test]
    fn start_discovery_records_the_accepted_job_id() {
        let transport = FakeTransport::new();
        transport.push(
            "POST",
            "/v1/workflows/wf_20261029/discoveries",
            RawResponse {
                status: 202,
                body: serde_json::to_vec(&JobAcceptedDto {
                    job_id: "job_1".to_owned(),
                })
                .unwrap(),
                ..RawResponse::default()
            },
        );
        let client = ApiClient::new(transport);
        let mut view = SourcesViewModel::default();
        view.start_discovery(&client, &attachment(), "wf_20261029", 1, None);
        assert_eq!(view.pending_discovery_job.as_deref(), Some("job_1"));
        assert_eq!(view.last_error, None);
    }

    #[test]
    fn poll_job_maps_server_statuses_to_phases() {
        for (status, expected) in [
            ("succeeded", JobPhase::Succeeded),
            ("failed", JobPhase::Failed),
            ("running", JobPhase::Running),
        ] {
            let transport = FakeTransport::new();
            transport.push(
                "GET",
                "/v1/jobs/job_1",
                RawResponse {
                    status: 200,
                    body: serde_json::to_vec(&JobDto {
                        id: "job_1".to_owned(),
                        kind: "discovery".to_owned(),
                        status: status.to_owned(),
                        workflow_id: "wf_20261029".to_owned(),
                        progress_percent: 0,
                        result_handle: None,
                        error_code: None,
                        expires_at_unix_ms: 0,
                    })
                    .unwrap(),
                    ..RawResponse::default()
                },
            );
            let client = ApiClient::new(transport);
            let mut view = SourcesViewModel::default();
            assert_eq!(
                view.poll_job(&client, &attachment(), "job_1"),
                Some(expected)
            );
        }
    }

    #[test]
    fn fetch_preparation_handle_records_the_header_from_source_detail() {
        let transport = FakeTransport::new();
        transport.push(
            "GET",
            "/v1/workflows/wf_20261029/sources/src_1",
            RawResponse {
                status: 200,
                body: serde_json::json!({
                    "id": "src_1",
                    "source_note_name": "2026-10-28 note",
                    "body": "synthetic body",
                    "content_digest": "digest",
                    "directory_day": "2026-10-28",
                    "header_day": "2026-10-28",
                    "warning_code": null
                })
                .to_string()
                .into_bytes(),
                preparation_handle: Some("handle_abc".to_owned()),
                ..RawResponse::default()
            },
        );
        let client = ApiClient::new(transport);
        let mut view = SourcesViewModel::default();
        let handle = view.fetch_preparation_handle(&client, &attachment(), "wf_20261029", "src_1");
        assert_eq!(handle.as_deref(), Some("handle_abc"));
        assert_eq!(
            view.latest_preparation_handle.as_deref(),
            Some("handle_abc")
        );
    }

    #[test]
    fn confirm_empty_selection_is_the_only_zero_note_continuation() {
        let transport = FakeTransport::new();
        transport.push(
            "POST",
            "/v1/workflows/wf_20261029/empty-selection-confirmations",
            RawResponse {
                status: 200,
                body: workflow_body_bytes("sources-confirmed"),
                ..RawResponse::default()
            },
        );
        let client = ApiClient::new(transport);
        let mut view = SourcesViewModel::default();
        let workflow = view.confirm_empty_selection(&client, &attachment(), "wf_20261029", 1);
        assert_eq!(workflow.unwrap().state, "sources-confirmed");
    }

    fn workflow_body_bytes(state: &str) -> Vec<u8> {
        serde_json::json!({
            "id": "wf_20261029",
            "diary_day": "2026-10-29",
            "state": state,
            "revision": 2,
            "buffer_version": 0,
            "language": null,
            "style": "concise",
            "assistant_backend_id": null,
            "composition_path": null,
            "selection_digest": null,
            "explicit_empty_selection": true,
            "invalidation_code": null,
            "entry_text": "",
            "entry_confirmed": false,
            "candidates": []
        })
        .to_string()
        .into_bytes()
    }
}
