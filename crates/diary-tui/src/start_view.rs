//! Start-stage ViewModel: list, start, resume, or discard a workflow. The
//! typed `ApiClient` is the only route to persisted state; this module
//! holds no filesystem or SQLite access and never synthesizes workflow
//! state the server did not return.

use diary_control_contract::{WorkflowDto, WorkflowSummaryDto, workflow_etag};

use crate::client::{ApiClient, Attachment, ClientOutcome, ProblemView, Transport, TransportError};
use crate::resources::{ResourceRow, ResourcesSidebarModel};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StartError {
    Problem(ProblemView),
    Transport(TransportError),
}

impl StartError {
    #[must_use]
    pub fn safe_label(&self) -> String {
        match self {
            Self::Problem(problem) => problem.safe_label(),
            Self::Transport(error) => error.safe_label().to_owned(),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct StartViewModel {
    pub workflows: Vec<WorkflowSummaryDto>,
    pub next_cursor: Option<String>,
    pub last_error: Option<StartError>,
}

impl StartViewModel {
    /// Replaces `workflows` with one page from the server, in the server's
    /// own order (unfinished drafts first). Does not page through further
    /// results itself; a caller wanting more calls again with
    /// `next_cursor`.
    pub fn refresh<T: Transport>(&mut self, client: &ApiClient<T>, attachment: &Attachment) {
        self.apply_refresh(client.list_workflows(attachment, None));
    }

    /// Applies a `RefreshWorkflows` outcome, however it was obtained — a
    /// direct synchronous call in tests, or a `Worker::try_recv` result in
    /// the real render loop, which never performs HTTP itself.
    pub fn apply_refresh(
        &mut self,
        outcome: ClientOutcome<diary_control_contract::WorkflowListDto>,
    ) {
        match outcome {
            ClientOutcome::Ok(list) => {
                self.workflows = list.workflows;
                self.next_cursor = list.next_cursor;
                self.last_error = None;
            }
            ClientOutcome::Problem(problem) => self.last_error = Some(StartError::Problem(problem)),
            ClientOutcome::Transport(error) => self.last_error = Some(StartError::Transport(error)),
        }
    }

    /// Starts a fresh workflow, or resumes the server-chosen one, for
    /// `diary_day`. `None` lets the server apply its own Mac-local "today"
    /// default; this ViewModel never computes "today" itself.
    pub fn start_or_resume<T: Transport>(
        &mut self,
        client: &ApiClient<T>,
        attachment: &Attachment,
        diary_day: Option<&str>,
    ) -> Option<WorkflowDto> {
        self.apply_start_or_resume(client.start_workflow(attachment, diary_day))
    }

    /// Applies a `StartOrResumeWorkflow` outcome, same split as
    /// [`Self::apply_refresh`].
    pub fn apply_start_or_resume(
        &mut self,
        outcome: ClientOutcome<WorkflowDto>,
    ) -> Option<WorkflowDto> {
        match outcome {
            ClientOutcome::Ok(workflow) => {
                self.last_error = None;
                Some(workflow)
            }
            ClientOutcome::Problem(problem) => {
                self.last_error = Some(StartError::Problem(problem));
                None
            }
            ClientOutcome::Transport(error) => {
                self.last_error = Some(StartError::Transport(error));
                None
            }
        }
    }

    /// Discards the listed workflow at `index` using its last-known
    /// revision as the `If-Match` precondition. On a `412`, the caller
    /// must refetch and retry deliberately rather than replaying blindly
    /// (the ViewModel does not retry on its own).
    pub fn discard<T: Transport>(
        &mut self,
        client: &ApiClient<T>,
        attachment: &Attachment,
        index: usize,
    ) -> bool {
        let Some(summary) = self.workflows.get(index) else {
            return false;
        };
        let outcome = client.discard_workflow(attachment, &summary.id, summary.revision);
        self.apply_discard(index, outcome)
    }

    /// Applies a `DiscardWorkflow` outcome for the workflow previously at
    /// `index`. The caller (synchronous test or the real worker-driven
    /// loop) is responsible for matching the result back to the request
    /// it sent; this only updates local state once that pairing is known.
    pub fn apply_discard(&mut self, index: usize, outcome: ClientOutcome<()>) -> bool {
        match outcome {
            ClientOutcome::Ok(()) => {
                if index < self.workflows.len() {
                    self.workflows.remove(index);
                }
                self.last_error = None;
                true
            }
            ClientOutcome::Problem(problem) => {
                self.last_error = Some(StartError::Problem(problem));
                false
            }
            ClientOutcome::Transport(error) => {
                self.last_error = Some(StartError::Transport(error));
                false
            }
        }
    }

    /// Renders the listed workflows as Resources Sidebar rows. Discarded
    /// workflows are marked explicitly rather than hidden, since the
    /// server, not this presentation layer, decides what is listed.
    #[must_use]
    pub fn resources(&self) -> ResourcesSidebarModel {
        ResourcesSidebarModel {
            rows: self
                .workflows
                .iter()
                .map(|workflow| ResourceRow {
                    label: workflow.diary_day.clone(),
                    state: if workflow.discarded {
                        "discarded".to_owned()
                    } else {
                        workflow.state.clone()
                    },
                    indent: 0,
                })
                .collect(),
        }
    }
}

/// Used only to prove the `If-Match` header is built from the exact
/// revision the list reported, not a recomputed or guessed value.
#[must_use]
pub fn expected_discard_if_match(summary: &WorkflowSummaryDto) -> String {
    workflow_etag(summary.revision)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use diary_control_contract::WorkflowListDto;

    use super::*;
    use crate::client::{RawRequest, RawResponse};

    struct FakeTransport {
        responses: RefCell<Vec<(&'static str, String, RawResponse)>>,
        seen_if_match: std::rc::Rc<RefCell<Vec<Option<String>>>>,
    }

    impl FakeTransport {
        fn new() -> Self {
            Self {
                responses: RefCell::new(Vec::new()),
                seen_if_match: std::rc::Rc::new(RefCell::new(Vec::new())),
            }
        }

        fn push(&self, method: &'static str, path: &str, response: RawResponse) {
            self.responses
                .borrow_mut()
                .push((method, path.to_owned(), response));
        }

        /// A handle to inspect `If-Match` headers after the `FakeTransport`
        /// has been moved into an `ApiClient`.
        fn if_match_handle(&self) -> std::rc::Rc<RefCell<Vec<Option<String>>>> {
            self.seen_if_match.clone()
        }
    }

    impl Transport for FakeTransport {
        fn execute(
            &self,
            _attachment: &Attachment,
            request: &RawRequest,
        ) -> Result<RawResponse, TransportError> {
            self.seen_if_match
                .borrow_mut()
                .push(request.if_match.clone());
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

    fn summary(
        id: &str,
        day: &str,
        state: &str,
        revision: u64,
        discarded: bool,
    ) -> WorkflowSummaryDto {
        WorkflowSummaryDto {
            id: id.to_owned(),
            diary_day: day.to_owned(),
            state: state.to_owned(),
            revision,
            buffer_version: 0,
            updated_at_unix_ms: 0,
            discarded,
        }
    }

    #[test]
    fn refresh_replaces_the_workflow_list_in_server_order() {
        let transport = FakeTransport::new();
        transport.push(
            "GET",
            "/v1/workflows",
            RawResponse {
                status: 200,
                body: serde_json::to_vec(&WorkflowListDto {
                    workflows: vec![summary("wf_20261029", "2026-10-29", "composing", 3, false)],
                    next_cursor: None,
                })
                .unwrap(),
                ..RawResponse::default()
            },
        );
        let client = ApiClient::new(transport);
        let mut view = StartViewModel::default();
        view.refresh(&client, &attachment());
        assert_eq!(view.workflows.len(), 1);
        assert_eq!(view.workflows[0].id, "wf_20261029");
        assert_eq!(view.last_error, None);
    }

    #[test]
    fn a_failed_refresh_records_the_error_without_clearing_an_existing_list() {
        let transport = FakeTransport::new();
        transport.push(
            "GET",
            "/v1/workflows",
            RawResponse {
                status: 503,
                body: b"not json".to_vec(),
                ..RawResponse::default()
            },
        );
        let client = ApiClient::new(transport);
        let mut view = StartViewModel {
            workflows: vec![summary("wf_20261028", "2026-10-28", "composing", 1, false)],
            ..StartViewModel::default()
        };
        view.refresh(&client, &attachment());
        assert_eq!(
            view.workflows.len(),
            1,
            "a failed refresh must not clear stale data it cannot replace"
        );
        // A non-2xx status is always decoded as a typed problem, even with
        // a garbage body (falling back to the fixed "malformed-problem"
        // code) — never silently reinterpreted as a transport failure.
        assert!(matches!(
            view.last_error,
            Some(StartError::Problem(ref problem)) if problem.code == "malformed-problem"
        ));
    }

    #[test]
    fn discard_sends_the_summarys_own_revision_as_if_match_and_removes_it_on_success() {
        let transport = FakeTransport::new();
        transport.push(
            "DELETE",
            "/v1/workflows/wf_20261029",
            RawResponse {
                status: 204,
                ..RawResponse::default()
            },
        );
        let if_match_handle = transport.if_match_handle();
        let client = ApiClient::new(transport);
        let mut view = StartViewModel {
            workflows: vec![summary("wf_20261029", "2026-10-29", "composing", 7, false)],
            ..StartViewModel::default()
        };
        assert!(view.discard(&client, &attachment(), 0));
        assert!(view.workflows.is_empty());
        assert_eq!(
            if_match_handle.borrow()[0],
            Some(expected_discard_if_match(&summary(
                "wf_20261029",
                "2026-10-29",
                "composing",
                7,
                false
            )))
        );
    }

    #[test]
    fn a_412_on_discard_keeps_the_entry_for_deliberate_resolution_rather_than_retrying() {
        let transport = FakeTransport::new();
        transport.push(
            "DELETE",
            "/v1/workflows/wf_20261029",
            RawResponse {
                status: 412,
                body: serde_json::json!({
                    "type": "https://nd.local/problems/precondition-failed",
                    "title": "Precondition failed",
                    "status": 412,
                    "code": "precondition-failed",
                    "instance": null
                })
                .to_string()
                .into_bytes(),
                ..RawResponse::default()
            },
        );
        let client = ApiClient::new(transport);
        let mut view = StartViewModel {
            workflows: vec![summary("wf_20261029", "2026-10-29", "composing", 7, false)],
            ..StartViewModel::default()
        };
        assert!(!view.discard(&client, &attachment(), 0));
        assert_eq!(
            view.workflows.len(),
            1,
            "a conflict must never be silently retried or treated as success"
        );
        assert!(matches!(
            view.last_error,
            Some(StartError::Problem(ref problem)) if problem.code == "precondition-failed"
        ));
    }

    #[test]
    fn resources_marks_a_discarded_workflow_explicitly_rather_than_hiding_it() {
        let view = StartViewModel {
            workflows: vec![summary("wf_20261028", "2026-10-28", "composing", 1, true)],
            ..StartViewModel::default()
        };
        let resources = view.resources();
        assert_eq!(resources.rows[0].state, "discarded");
    }

    #[test]
    fn start_or_resume_sends_no_day_for_the_server_owned_today_default() {
        let transport = FakeTransport::new();
        transport.push(
            "POST",
            "/v1/workflows",
            RawResponse {
                status: 201,
                body: workflow_dto_body("wf_20261029", "2026-10-29", "started"),
                ..RawResponse::default()
            },
        );
        let client = ApiClient::new(transport);
        let mut view = StartViewModel::default();
        let workflow = view.start_or_resume(&client, &attachment(), None);
        assert_eq!(workflow.unwrap().diary_day, "2026-10-29");
    }

    fn workflow_dto_body(id: &str, day: &str, state: &str) -> Vec<u8> {
        serde_json::json!({
            "id": id,
            "diary_day": day,
            "state": state,
            "revision": 1,
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
            "candidates": []
        })
        .to_string()
        .into_bytes()
    }
}
