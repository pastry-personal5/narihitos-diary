//! Compose-stage ViewModel: the in-memory editor buffer, conditional
//! autosave, and the generation-tracking rule from the architecture doc's
//! "Local presentation state and recovery" section — a successful save
//! advances the canonical buffer version but replaces local text only when
//! no newer local edit exists; a `412` or unknown outcome never silently
//! merges or replays.

use diary_control_contract::WorkflowDto;

use crate::client::{ApiClient, Attachment, ClientOutcome, ProblemView, Transport, TransportError};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ComposeError {
    Problem(ProblemView),
    Transport(TransportError),
}

impl ComposeError {
    #[must_use]
    pub fn safe_label(&self) -> String {
        match self {
            Self::Problem(problem) => problem.safe_label(),
            Self::Transport(error) => error.safe_label().to_owned(),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ComposeViewModel {
    /// The workflow whose entry `text` belongs to.
    workflow_id: Option<String>,
    pub text: String,
    pub buffer_version: u64,
    /// Bumped on every local edit. The only durable text is the server
    /// entry buffer; this counter never persists past the process.
    local_generation: u64,
    /// The generation that was last successfully saved.
    saved_generation: u64,
    pub last_error: Option<ComposeError>,
    /// Canonical text observed while unsaved local text exists. The UI
    /// must ask the person which version to keep; it must never merge or
    /// replace either version implicitly.
    pub conflicting_server_text: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SaveSnapshot {
    pub generation: u64,
    pub buffer_version: u64,
    pub text: String,
}

impl ComposeViewModel {
    /// Seeds the buffer from a canonical workflow response (initial load
    /// or an explicit reload after a conflict). Only replaces local text
    /// when nothing is locally dirty; a reload while dirty must not
    /// silently discard unsent edits.
    pub fn load_from_canonical(&mut self, workflow: &WorkflowDto) {
        if self.workflow_id.as_deref() != Some(workflow.id.as_str()) {
            // Another workflow (day): local text belongs to the previous
            // one, so it must never be kept, let alone autosaved, here.
            *self = Self {
                workflow_id: Some(workflow.id.clone()),
                text: workflow.entry_text.clone(),
                buffer_version: workflow.buffer_version,
                ..Self::default()
            };
            return;
        }
        if workflow.buffer_version < self.buffer_version {
            // A read that raced a newer save we already applied.
            return;
        }
        // Only a buffer newer than our last acknowledged save is someone
        // else's text. At the same version it is our own previous save (e.g.
        // the reload its `entry.changed` triggers, while newer typing is
        // still unsent), which is no conflict.
        let advanced = workflow.buffer_version > self.buffer_version;
        self.buffer_version = workflow.buffer_version;
        if self.is_dirty() {
            if workflow.entry_text == self.text {
                // A refetch after an uncertain write proves that the exact
                // local bytes are canonical, even if the write response was
                // lost.
                self.saved_generation = self.local_generation;
                self.last_error = None;
                self.conflicting_server_text = None;
            } else if advanced {
                self.conflicting_server_text = Some(workflow.entry_text.clone());
            }
            return;
        }
        self.text = workflow.entry_text.clone();
        self.local_generation = 0;
        self.saved_generation = 0;
        self.conflicting_server_text = None;
    }

    #[must_use]
    pub const fn is_dirty(&self) -> bool {
        self.local_generation != self.saved_generation
    }

    /// Replaces the buffer text and marks it dirty. The caller (editor
    /// widget) is the source of the new text; this never mutates it.
    pub fn edit(&mut self, text: String) {
        if text == self.text {
            return;
        }
        self.text = text;
        self.local_generation += 1;
    }

    /// Whether local text may be written. While a canonical conflict is
    /// unresolved, `buffer_version` already names the server's newer text,
    /// so a write would silently replace it; the person must first choose
    /// `keep_local_after_conflict` or `use_server_after_conflict`.
    #[must_use]
    pub const fn can_save(&self) -> bool {
        self.is_dirty() && self.conflicting_server_text.is_none()
    }

    /// Captures the exact conditional write an async worker should make.
    /// The caller owns the one-in-flight guard.
    #[must_use]
    pub fn save_snapshot(&self) -> Option<SaveSnapshot> {
        self.can_save().then(|| SaveSnapshot {
            generation: self.local_generation,
            buffer_version: self.buffer_version,
            text: self.text.clone(),
        })
    }

    /// Explicitly keeps local text after a canonical conflict. The next
    /// snapshot uses the freshly loaded server buffer version.
    pub fn keep_local_after_conflict(&mut self) {
        self.conflicting_server_text = None;
        self.last_error = None;
    }

    /// Explicitly discards local edits in favor of the separately shown
    /// canonical version.
    pub fn use_server_after_conflict(&mut self) {
        let Some(server_text) = self.conflicting_server_text.take() else {
            return;
        };
        self.text = server_text;
        self.local_generation = 0;
        self.saved_generation = 0;
        self.last_error = None;
    }

    /// Attempts a conditional save if the buffer is dirty. No-op if
    /// nothing has changed since the last successful save. There is no
    /// "in flight" guard here because this increment's client calls are
    /// synchronous; the bounded async worker that enforces "at most one
    /// save in flight" is implemented separately.
    pub fn autosave<T: Transport>(
        &mut self,
        client: &ApiClient<T>,
        attachment: &Attachment,
        workflow_id: &str,
    ) {
        if !self.can_save() {
            return;
        }
        let generation_at_send = self.local_generation;
        let buffer_version_sent = self.buffer_version;
        let text_at_send = self.text.clone();
        let outcome =
            client.save_entry(attachment, workflow_id, buffer_version_sent, &text_at_send);
        self.apply_save_outcome(generation_at_send, outcome);
    }

    /// The response-processing half of `autosave`, factored out so tests
    /// can drive the generation comparison directly: a synchronous call
    /// has no seam to honestly simulate an edit landing between send and
    /// response, so tests pass `generation_at_send` explicitly instead of
    /// relying on real timing.
    pub fn apply_save_outcome(
        &mut self,
        generation_at_send: u64,
        outcome: ClientOutcome<WorkflowDto>,
    ) -> Option<WorkflowDto> {
        match outcome {
            ClientOutcome::Ok(workflow) => {
                self.buffer_version = workflow.buffer_version;
                self.last_error = None;
                if self.local_generation == generation_at_send {
                    // No newer local edit arrived while this save was in
                    // flight: the save is fully caught up.
                    self.saved_generation = generation_at_send;
                } else {
                    // A newer edit exists; the text stays dirty so the
                    // next autosave call sends it, rather than being
                    // treated as already saved.
                }
                Some(workflow)
            }
            ClientOutcome::Problem(problem) => {
                // A 412/409 requires canonical refetch and deliberate
                // resolution, never a blind replay with the same version.
                self.last_error = Some(ComposeError::Problem(problem));
                None
            }
            ClientOutcome::Transport(error) => {
                self.last_error = Some(ComposeError::Transport(error));
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

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
                .expect("unexpected request in fake transport: autosave must be a no-op when nothing is dirty");
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

    fn workflow_body(buffer_version: u64, entry_text: &str) -> Vec<u8> {
        serde_json::json!({
            "id": "wf_20261029",
            "diary_day": "2026-10-29",
            "state": "composing",
            "revision": 3,
            "buffer_version": buffer_version,
            "language": "en-US",
            "style": "concise",
            "assistant_backend_id": null,
            "composition_path": null,
            "selection_digest": null,
            "explicit_empty_selection": false,
            "invalidation_code": null,
            "entry_text": entry_text,
            "entry_confirmed": false,
            "candidates": []
        })
        .to_string()
        .into_bytes()
    }

    fn workflow_from(buffer_version: u64, entry_text: &str) -> WorkflowDto {
        serde_json::from_slice(&workflow_body(buffer_version, entry_text)).unwrap()
    }

    #[test]
    fn editing_marks_the_buffer_dirty_and_a_no_op_edit_does_not() {
        let mut view = ComposeViewModel::default();
        assert!(!view.is_dirty());
        view.edit("hello".to_owned());
        assert!(view.is_dirty());
        let generation_after_first_edit = view.local_generation;
        view.edit("hello".to_owned());
        assert_eq!(
            view.local_generation, generation_after_first_edit,
            "an edit with identical text must not bump the generation"
        );
    }

    #[test]
    fn a_successful_save_with_no_newer_edit_clears_dirty() {
        let transport = FakeTransport::new();
        transport.push(
            "PUT",
            "/v1/workflows/wf_20261029/entry",
            RawResponse {
                status: 200,
                body: workflow_body(5, "hello"),
                ..RawResponse::default()
            },
        );
        let client = ApiClient::new(transport);
        let mut view = ComposeViewModel::default();
        view.edit("hello".to_owned());
        view.autosave(&client, &attachment(), "wf_20261029");
        assert!(!view.is_dirty());
        assert_eq!(view.buffer_version, 5);
        assert_eq!(view.last_error, None);
    }

    #[test]
    fn a_newer_local_edit_during_the_save_keeps_the_buffer_dirty() {
        // A synchronous call has no real seam between "request sent" and
        // "response processed", so this drives the response-processing
        // half directly with a `generation_at_send` that is already
        // behind the current `local_generation` — exactly what a
        // concurrent edit landing mid-flight would leave behind.
        let mut view = ComposeViewModel::default();
        view.edit("hello".to_owned()); // local_generation becomes 1
        view.edit("hello, world".to_owned()); // local_generation becomes 2
        let generation_at_send = 1; // the save that was in flight saw generation 1
        view.apply_save_outcome(
            generation_at_send,
            ClientOutcome::Ok(workflow_from(5, "hello")),
        );
        assert!(
            view.is_dirty(),
            "a newer local edit must keep the buffer dirty so it gets saved next"
        );
        assert_eq!(
            view.buffer_version, 5,
            "the canonical buffer version still advances even though local text stays dirty"
        );
    }

    #[test]
    fn a_412_on_save_is_recorded_and_never_silently_retried() {
        let transport = FakeTransport::new();
        transport.push(
            "PUT",
            "/v1/workflows/wf_20261029/entry",
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
        let mut view = ComposeViewModel::default();
        view.edit("hello".to_owned());
        view.autosave(&client, &attachment(), "wf_20261029");
        assert!(view.is_dirty());
        assert_eq!(
            view.buffer_version, 0,
            "an unsuccessful save must not advance the buffer version"
        );
        assert!(matches!(
            view.last_error,
            Some(ComposeError::Problem(ref problem)) if problem.code == "precondition-failed"
        ));
    }

    #[test]
    fn autosave_is_a_no_op_when_nothing_is_dirty() {
        // No response is pushed; `FakeTransport::execute` panics on any
        // request it doesn't expect, so a call here fails the test.
        let transport = FakeTransport::new();
        let client = ApiClient::new(transport);
        let mut view = ComposeViewModel::default();
        view.autosave(&client, &attachment(), "wf_20261029");
    }

    #[test]
    fn load_from_canonical_replaces_text_only_when_not_dirty() {
        let mut view = ComposeViewModel::default();
        view.load_from_canonical(&workflow_from(2, "server text"));
        assert_eq!(view.text, "server text");
        assert_eq!(view.buffer_version, 2);

        view.edit("local edit".to_owned());
        view.load_from_canonical(&workflow_from(3, "newer server text"));
        assert_eq!(
            view.text, "local edit",
            "a reload while dirty must not discard unsent local text"
        );
        assert_eq!(
            view.buffer_version, 3,
            "the canonical buffer version still updates for the next save attempt"
        );
    }

    #[test]
    fn save_snapshot_binds_generation_version_and_exact_text() {
        let mut view = ComposeViewModel::default();
        view.load_from_canonical(&workflow_from(7, "old"));
        view.edit("local".to_owned());
        assert_eq!(
            view.save_snapshot(),
            Some(SaveSnapshot {
                generation: 1,
                buffer_version: 7,
                text: "local".to_owned(),
            })
        );
    }

    #[test]
    fn canonical_reload_keeps_both_sides_of_a_conflict_until_resolution() {
        let mut view = ComposeViewModel::default();
        view.load_from_canonical(&workflow_from(2, "old"));
        view.edit("local".to_owned());
        view.load_from_canonical(&workflow_from(3, "server"));
        assert_eq!(view.text, "local");
        assert_eq!(view.conflicting_server_text.as_deref(), Some("server"));

        view.use_server_after_conflict();
        assert_eq!(view.text, "server");
        assert!(!view.is_dirty());
    }

    #[test]
    fn the_reload_echoing_our_own_save_is_no_conflict_while_typing_continues() {
        let mut view = ComposeViewModel::default();
        view.load_from_canonical(&workflow_from(2, "old"));
        view.edit("a".to_owned());
        let sent = view.save_snapshot().unwrap();
        view.edit("ab".to_owned());
        assert!(
            view.apply_save_outcome(sent.generation, ClientOutcome::Ok(workflow_from(3, "a")))
                .is_some()
        );
        // The save's own `entry.changed` reload arrives after more typing.
        view.load_from_canonical(&workflow_from(3, "a"));
        assert_eq!(view.conflicting_server_text, None);
        assert_eq!(
            view.save_snapshot(),
            Some(SaveSnapshot {
                generation: 2,
                buffer_version: 3,
                text: "ab".to_owned(),
            })
        );
        // A read that raced that save never rolls the version back.
        view.load_from_canonical(&workflow_from(2, "old"));
        assert_eq!(view.buffer_version, 3);
        assert_eq!(view.text, "ab");
    }

    #[test]
    fn loading_another_workflow_never_carries_this_ones_text_into_it() {
        let mut view = ComposeViewModel::default();
        view.load_from_canonical(&workflow_from(2, "day one"));
        view.edit("unsent day one".to_owned());
        let mut other_day = workflow_from(5, "day two");
        other_day.id = "wf_20261030".to_owned();
        view.load_from_canonical(&other_day);
        assert_eq!(view.text, "day two");
        assert_eq!(view.buffer_version, 5);
        assert!(!view.is_dirty());
        assert_eq!(view.conflicting_server_text, None);
    }

    #[test]
    fn an_unresolved_conflict_never_autosaves_over_the_server_text() {
        let mut view = ComposeViewModel::default();
        view.load_from_canonical(&workflow_from(2, "old"));
        view.edit("local".to_owned());
        view.load_from_canonical(&workflow_from(3, "server"));
        // Buffer version 3 now names "server": a save would replace it.
        assert_eq!(view.save_snapshot(), None);

        view.keep_local_after_conflict();
        assert_eq!(
            view.save_snapshot(),
            Some(SaveSnapshot {
                generation: 1,
                buffer_version: 3,
                text: "local".to_owned(),
            })
        );
    }

    #[test]
    fn canonical_reload_proves_an_uncertain_save_when_text_matches() {
        let mut view = ComposeViewModel::default();
        view.load_from_canonical(&workflow_from(2, "old"));
        view.edit("possibly saved".to_owned());
        view.load_from_canonical(&workflow_from(3, "possibly saved"));
        assert!(!view.is_dirty());
        assert_eq!(view.buffer_version, 3);
        assert_eq!(view.conflicting_server_text, None);
    }
}
