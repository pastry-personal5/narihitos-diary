//! Review-stage ViewModel: inspect and confirm the complete edited entry.
//! Confirmation is disabled for blank text or an unresolved
//! inferred-reflection delimiter — the same two constants the server
//! enforces authoritatively (`diary_core::workflow::INFERRED_REFLECTION_*`);
//! this client-side check is presentation only (what to show/disable), not
//! a second source of truth, since the server rejects confirmation
//! regardless.

use diary_control_contract::WorkflowDto;

use crate::client::{ApiClient, Attachment, ClientOutcome, Transport};
use crate::review_view::ReviewError::{Problem, TransportFailure};

/// Mirrors `diary_core::workflow::INFERRED_REFLECTION_OPEN`/`_CLOSE`.
/// Diary-tui has no dependency on diary-core (see the architecture doc's
/// "Affected modules and interfaces"), so these are the same two fixed
/// characters, not a derived or guessed value.
pub const INFERRED_REFLECTION_OPEN: char = '⟪';
pub const INFERRED_REFLECTION_CLOSE: char = '⟫';

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReviewError {
    Problem(crate::client::ProblemView),
    TransportFailure(crate::client::TransportError),
}

impl ReviewError {
    #[must_use]
    pub fn safe_label(&self) -> String {
        match self {
            Problem(problem) => problem.safe_label(),
            TransportFailure(error) => error.safe_label().to_owned(),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ReviewViewModel {
    pub last_error: Option<ReviewError>,
}

/// Finds the first unresolved reflection delimiter in `text`, if any.
/// "Unresolved" means the delimiter character itself is still present;
/// this does not try to pair open/close or interpret nesting, matching
/// the server's own simple `contains` check.
#[must_use]
pub fn first_unresolved_delimiter(text: &str) -> Option<char> {
    text.chars()
        .find(|&ch| ch == INFERRED_REFLECTION_OPEN || ch == INFERRED_REFLECTION_CLOSE)
}

/// Whether the entry text can be confirmed: non-blank, and free of any
/// reflection delimiter. Purely advisory for the client's own Confirm
/// control; the server re-checks and is authoritative.
#[must_use]
pub fn can_confirm(text: &str) -> bool {
    !text.trim().is_empty() && first_unresolved_delimiter(text).is_none()
}

impl ReviewViewModel {
    /// Confirms the entry. Returns `None` without calling the server at
    /// all if `can_confirm` would refuse — the client never sends a
    /// confirmation it already knows will be rejected, but still never
    /// trusts its own check as the actual authority.
    pub fn confirm<T: Transport>(
        &mut self,
        client: &ApiClient<T>,
        attachment: &Attachment,
        workflow_id: &str,
        revision: u64,
        buffer_version: u64,
        text: &str,
    ) -> Option<WorkflowDto> {
        if !can_confirm(text) {
            return None;
        }
        match client.confirm_entry(attachment, workflow_id, revision, buffer_version) {
            ClientOutcome::Ok(workflow) => {
                self.last_error = None;
                Some(workflow)
            }
            ClientOutcome::Problem(problem) => {
                self.last_error = Some(Problem(problem));
                None
            }
            ClientOutcome::Transport(error) => {
                self.last_error = Some(TransportFailure(error));
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;
    use crate::client::{RawRequest, RawResponse, TransportError};

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
                .expect(
                    "unexpected request in fake transport: confirm must be refused client-side",
                );
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

    fn workflow_body(state: &str) -> Vec<u8> {
        serde_json::json!({
            "id": "wf_20261029",
            "diary_day": "2026-10-29",
            "state": state,
            "revision": 4,
            "buffer_version": 2,
            "language": "en-US",
            "style": "concise",
            "assistant_backend_id": null,
            "composition_path": null,
            "selection_digest": null,
            "explicit_empty_selection": false,
            "invalidation_code": null,
            "entry_text": "Reviewed and confirmed.",
            "entry_confirmed": true,
            "candidates": []
        })
        .to_string()
        .into_bytes()
    }

    #[test]
    fn blank_text_cannot_be_confirmed() {
        assert!(!can_confirm(""));
        assert!(!can_confirm("   \n\t "));
    }

    #[test]
    fn text_with_an_open_or_close_delimiter_cannot_be_confirmed() {
        assert!(!can_confirm(
            "Maybe \u{27EA}this reflects something\u{27EB}."
        ));
        assert!(!can_confirm("Dangling close \u{27EB} only."));
    }

    #[test]
    fn ordinary_non_blank_text_can_be_confirmed() {
        assert!(can_confirm("A perfectly ordinary diary entry."));
    }

    #[test]
    fn confirm_never_calls_the_server_when_the_client_side_check_already_refuses() {
        let transport = FakeTransport::new();
        let client = ApiClient::new(transport);
        let mut view = ReviewViewModel::default();
        let result = view.confirm(&client, &attachment(), "wf_20261029", 4, 2, "");
        assert_eq!(result, None);
        assert_eq!(view.last_error, None);
    }

    #[test]
    fn a_successful_confirm_returns_the_confirmed_workflow() {
        let transport = FakeTransport::new();
        transport.push(
            "POST",
            "/v1/workflows/wf_20261029/entry-confirmations",
            RawResponse {
                status: 200,
                body: workflow_body("entry-confirmed"),
                ..RawResponse::default()
            },
        );
        let client = ApiClient::new(transport);
        let mut view = ReviewViewModel::default();
        let workflow = view.confirm(
            &client,
            &attachment(),
            "wf_20261029",
            4,
            2,
            "Reviewed and confirmed.",
        );
        assert_eq!(workflow.unwrap().state, "entry-confirmed");
        assert_eq!(view.last_error, None);
    }

    #[test]
    fn a_412_from_the_server_is_recorded_with_a_safe_label() {
        let transport = FakeTransport::new();
        transport.push(
            "POST",
            "/v1/workflows/wf_20261029/entry-confirmations",
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
        let mut view = ReviewViewModel::default();
        let result = view.confirm(
            &client,
            &attachment(),
            "wf_20261029",
            4,
            2,
            "Reviewed and confirmed.",
        );
        assert_eq!(result, None);
        assert_eq!(
            view.last_error.as_ref().map(ReviewError::safe_label),
            Some("Request rejected: precondition-failed".to_owned())
        );
    }
}
