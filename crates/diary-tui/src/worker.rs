//! The bounded HTTP worker: the one thread that touches the network, so
//! the render loop never blocks on HTTP. Per the architecture doc, this is
//! the "one ordinary HTTP worker with a 16-command queue" from the
//! owner-approved packet; the render loop sends `Intent`s with `try_send`
//! (never blocking) and drains `WorkerResult`s with `try_recv` each tick.
//!
//! This is the prerequisite for autosave, the IME poll, and SSE reload all
//! coexisting with a responsive render loop: without it, every one of
//! those would share the same blocking call stack as typing.

use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::thread::{self, JoinHandle};

use diary_control_contract::{
    AmendmentPreviewDto, AssistantBackendListDto, CapabilitiesDto, DraftResultDto, JobAcceptedDto,
    JobDto, LocalPreviewDto, MemoryReviewDto, NotionPreviewDto, ParagraphSuggestionDto,
    ParagraphSuggestionListDto, PreferencesDto, ResolveParagraphSuggestionRequest, SetupCheckDto,
    SourceListDto, SourceRefineSuggestionDto, StartParagraphReviewRequest, WorkflowDto,
    WorkflowListDto,
};

use crate::client::{ApiClient, Attachment, ClientOutcome, SourceDetailResponse, Transport};

const COMMAND_QUEUE_CAPACITY: usize = 16;
const RESULT_QUEUE_CAPACITY: usize = 32;

/// A unit of work the render loop wants done off-thread. Each variant
/// names the operation, not a raw request; the worker is the only thing
/// that knows the route/header details, same as `ApiClient`.
#[derive(Clone, Debug)]
pub enum Intent {
    GetCapabilities,
    GetSetupCheck,
    GetPreferences,
    SavePreferences {
        last_language: Option<String>,
        last_assistant_backend_id: Option<String>,
    },
    RefreshWorkflows,
    LoadMoreWorkflows {
        cursor: String,
    },
    StartOrResumeWorkflow {
        diary_day: Option<String>,
        language: Option<String>,
        style: Option<String>,
    },
    DiscardWorkflow {
        id: String,
        revision: u64,
    },
    GetWorkflow {
        workflow_id: String,
    },
    Discover {
        workflow_id: String,
        revision: u64,
        explicit_date: Option<String>,
    },
    ListSources {
        workflow_id: String,
        cursor: Option<String>,
    },
    PollJob {
        job_id: String,
    },
    PrepareSources {
        workflow_id: String,
        revision: u64,
        source_ids: Vec<String>,
    },
    SourceDetail {
        workflow_id: String,
        source_id: String,
        preparation_handle: String,
    },
    ConfirmedSourceDetail {
        workflow_id: String,
        source_id: String,
    },
    RefineSource {
        workflow_id: String,
        source_id: String,
        revision: u64,
        digest: String,
    },
    GetSourceRefineSuggestion {
        workflow_id: String,
        source_id: String,
    },
    ResolveSourceRefine {
        workflow_id: String,
        source_id: String,
        revision: u64,
        accept: bool,
        digest: String,
    },
    EditSourceBody {
        workflow_id: String,
        source_id: String,
        revision: u64,
        text: String,
        digest: String,
    },
    DeleteSource {
        workflow_id: String,
        source_id: String,
        revision: u64,
    },
    ConfirmSelection {
        workflow_id: String,
        revision: u64,
        preparation_handle: String,
        source_ids: Vec<String>,
    },
    ConfirmEmptySelection {
        workflow_id: String,
        revision: u64,
    },
    SetWritingSettings {
        workflow_id: String,
        revision: u64,
        language: String,
        style: String,
        assistant_backend_id: Option<String>,
    },
    BeginManualComposition {
        workflow_id: String,
        revision: u64,
    },
    SaveEntry {
        workflow_id: String,
        buffer_version: u64,
        text: String,
    },
    ConfirmEntry {
        workflow_id: String,
        revision: u64,
        buffer_version: u64,
    },
    PreviewLocal {
        workflow_id: String,
        revision: u64,
    },
    ApproveLocal {
        workflow_id: String,
        revision: u64,
        plan_digest: String,
    },
    PublishLocal {
        workflow_id: String,
        revision: u64,
    },
    PreviewNotion {
        workflow_id: String,
        revision: u64,
    },
    ApproveNotion {
        workflow_id: String,
        revision: u64,
        plan_digest: String,
    },
    PublishNotion {
        workflow_id: String,
        revision: u64,
    },
    SaveUnderDay {
        workflow_id: String,
        revision: u64,
        new_day: String,
    },
    SeedAmendment {
        workflow_id: String,
        revision: u64,
    },
    PreviewAmendment {
        workflow_id: String,
        revision: u64,
    },
    ApproveAmendment {
        workflow_id: String,
        revision: u64,
        plan_digest: String,
    },
    PublishAmendment {
        workflow_id: String,
        revision: u64,
    },
    RetryAmendmentNotion {
        workflow_id: String,
        revision: u64,
    },
    RereadAmendment {
        workflow_id: String,
        revision: u64,
    },
    AbandonAmendment {
        workflow_id: String,
        revision: u64,
    },
    GetAssistantBackends,
    SelectAssistant {
        workflow_id: String,
        revision: u64,
        backend_id: Option<String>,
    },
    ExtractMemories {
        workflow_id: String,
        revision: u64,
    },
    GetMemoryReview {
        workflow_id: String,
    },
    SaveMemoryReview {
        workflow_id: String,
        revision: u64,
        candidate_set_id: String,
        reviewed_memory: String,
    },
    RequestDraft {
        workflow_id: String,
        revision: u64,
        include_prior_draft: bool,
    },
    GetDraftResult {
        workflow_id: String,
    },
    ReviewParagraph {
        workflow_id: String,
        revision: u64,
        request: StartParagraphReviewRequest,
    },
    GetParagraphSuggestions {
        workflow_id: String,
    },
    GetParagraphSuggestion {
        workflow_id: String,
        suggestion_id: String,
    },
    ResolveParagraphSuggestion {
        workflow_id: String,
        suggestion_id: String,
        revision: u64,
        request: ResolveParagraphSuggestionRequest,
    },
    CancelJob {
        job_id: String,
    },
}

/// The worker's reply, paired with the intent that produced it so the
/// render loop can tell which pending action just completed.
pub enum WorkerResult {
    Capabilities(ClientOutcome<CapabilitiesDto>),
    SetupCheck(ClientOutcome<SetupCheckDto>),
    Preferences(ClientOutcome<PreferencesDto>),
    PreferencesSaved(ClientOutcome<PreferencesDto>),
    Workflows(ClientOutcome<WorkflowListDto>),
    MoreWorkflows(ClientOutcome<WorkflowListDto>),
    StartedWorkflow(Box<ClientOutcome<WorkflowDto>>),
    Discarded(ClientOutcome<()>),
    WorkflowFetched(Box<ClientOutcome<WorkflowDto>>),
    /// Shared by [`Intent::Discover`] and [`Intent::PrepareSources`]: both
    /// only accept work as a job, never return candidates/handles
    /// directly, so one result shape covers both per the architecture
    /// doc's "Client-to-server boundary" table.
    JobAccepted(ClientOutcome<JobAcceptedDto>),
    Sources(ClientOutcome<SourceListDto>),
    JobStatus(ClientOutcome<JobDto>),
    SourceDetail(Box<ClientOutcome<SourceDetailResponse>>),
    SourceRefineSuggestion(ClientOutcome<SourceRefineSuggestionDto>),
    SourceMutation(Box<ClientOutcome<WorkflowDto>>),
    SelectionConfirmed(Box<ClientOutcome<WorkflowDto>>),
    EmptySelectionConfirmed(Box<ClientOutcome<WorkflowDto>>),
    WritingSettingsSet(Box<ClientOutcome<WorkflowDto>>),
    ManualCompositionBegun(Box<ClientOutcome<WorkflowDto>>),
    EntrySaved(Box<ClientOutcome<WorkflowDto>>),
    EntryConfirmed(Box<ClientOutcome<WorkflowDto>>),
    LocalPreviewed(Box<ClientOutcome<LocalPreviewDto>>),
    LocalApproved(Box<ClientOutcome<WorkflowDto>>),
    LocalPublished(ClientOutcome<JobAcceptedDto>),
    NotionPreviewed(Box<ClientOutcome<NotionPreviewDto>>),
    NotionApproved(Box<ClientOutcome<WorkflowDto>>),
    NotionPublished(ClientOutcome<JobAcceptedDto>),
    SavedUnderDay(Box<ClientOutcome<WorkflowDto>>),
    AmendmentSeeded(Box<ClientOutcome<WorkflowDto>>),
    AmendmentPreviewed(Box<ClientOutcome<AmendmentPreviewDto>>),
    AmendmentApproved(Box<ClientOutcome<WorkflowDto>>),
    AmendmentPublished(ClientOutcome<JobAcceptedDto>),
    AmendmentNotionRetried(ClientOutcome<JobAcceptedDto>),
    AmendmentReread(Box<ClientOutcome<WorkflowDto>>),
    AmendmentAbandoned(Box<ClientOutcome<WorkflowDto>>),
    AssistantBackends(ClientOutcome<AssistantBackendListDto>),
    AssistantSelected(Box<ClientOutcome<WorkflowDto>>),
    MemoryReview(ClientOutcome<MemoryReviewDto>),
    MemoryReviewSaved(Box<ClientOutcome<WorkflowDto>>),
    DraftResult(ClientOutcome<DraftResultDto>),
    ParagraphSuggestions(ClientOutcome<ParagraphSuggestionListDto>),
    ParagraphSuggestion(ClientOutcome<ParagraphSuggestionDto>),
    ParagraphSuggestionResolved(Box<ClientOutcome<WorkflowDto>>),
    JobCancelled(ClientOutcome<()>),
}

/// Owns the single `ApiClient` and the one background thread that serves
/// it. Dropping the `Worker` drops the command sender, which ends the
/// thread's loop and lets it join.
pub struct Worker {
    // `Option` so `Drop` can explicitly drop each channel half before
    // `join`ing: dropping `intents` wakes a thread blocked in
    // `intents_rx.recv()`, and dropping `results` wakes one blocked in
    // `results_tx.send()` on a full queue. Leaving them as plain fields
    // would not help — a struct's own `Drop::drop` runs *before* its
    // fields are dropped, not after, so `join()` inside it would wait on
    // a thread that cannot see either channel half go away yet.
    intents: Option<SyncSender<Intent>>,
    results: Option<Receiver<WorkerResult>>,
    handle: Option<JoinHandle<()>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SendError {
    /// The 16-command queue is full; the caller should show the pending
    /// intent as busy rather than block or drop it silently.
    QueueFull,
    /// The worker thread has already exited.
    WorkerGone,
}

impl Worker {
    /// Spawns the worker thread over any [`Transport`]; production code
    /// uses `UreqTransport`, fakes let tests exercise every intent without
    /// a real server.
    #[must_use]
    pub fn spawn<T: Transport + Send + 'static>(transport: T, attachment: Attachment) -> Self {
        let (intents_tx, intents_rx) = sync_channel(COMMAND_QUEUE_CAPACITY);
        let (results_tx, results_rx) = sync_channel(RESULT_QUEUE_CAPACITY);
        let handle = thread::spawn(move || {
            let client = ApiClient::new(transport);
            while let Ok(intent) = intents_rx.recv() {
                let result = Self::handle_intent(&client, &attachment, intent);
                if results_tx.send(result).is_err() {
                    // The render loop dropped its receiver (shutting
                    // down); nothing left to do but stop.
                    break;
                }
            }
        });
        Self {
            intents: Some(intents_tx),
            results: Some(results_rx),
            handle: Some(handle),
        }
    }

    fn handle_intent<T: Transport>(
        client: &ApiClient<T>,
        attachment: &Attachment,
        intent: Intent,
    ) -> WorkerResult {
        match intent {
            Intent::GetCapabilities => WorkerResult::Capabilities(client.capabilities(attachment)),
            Intent::GetAssistantBackends => {
                WorkerResult::AssistantBackends(client.assistant_backends(attachment))
            }
            Intent::GetSetupCheck => WorkerResult::SetupCheck(client.setup_check(attachment)),
            Intent::GetPreferences => WorkerResult::Preferences(client.preferences(attachment)),
            Intent::SavePreferences {
                last_language,
                last_assistant_backend_id,
            } => WorkerResult::PreferencesSaved(client.save_preferences(
                attachment,
                &PreferencesDto {
                    last_language,
                    last_assistant_backend_id,
                },
            )),
            Intent::RefreshWorkflows => {
                WorkerResult::Workflows(client.list_workflows(attachment, None))
            }
            Intent::LoadMoreWorkflows { cursor } => {
                WorkerResult::MoreWorkflows(client.list_workflows(attachment, Some(&cursor)))
            }
            Intent::StartOrResumeWorkflow {
                diary_day,
                language,
                style,
            } => WorkerResult::StartedWorkflow(Box::new(client.start_workflow(
                attachment,
                diary_day.as_deref(),
                language.as_deref(),
                style.as_deref(),
            ))),
            Intent::DiscardWorkflow { id, revision } => {
                WorkerResult::Discarded(client.discard_workflow(attachment, &id, revision))
            }
            Intent::GetWorkflow { workflow_id } => WorkerResult::WorkflowFetched(Box::new(
                client.get_workflow(attachment, &workflow_id),
            )),
            Intent::Discover {
                workflow_id,
                revision,
                explicit_date,
            } => WorkerResult::JobAccepted(client.discover(
                attachment,
                &workflow_id,
                revision,
                explicit_date.as_deref(),
            )),
            Intent::ListSources {
                workflow_id,
                cursor,
            } => WorkerResult::Sources(client.list_sources(
                attachment,
                &workflow_id,
                cursor.as_deref(),
            )),
            Intent::PollJob { job_id } => WorkerResult::JobStatus(client.job(attachment, &job_id)),
            Intent::PrepareSources {
                workflow_id,
                revision,
                source_ids,
            } => WorkerResult::JobAccepted(client.prepare_sources(
                attachment,
                &workflow_id,
                revision,
                &source_ids,
            )),
            Intent::SourceDetail {
                workflow_id,
                source_id,
                preparation_handle,
            } => WorkerResult::SourceDetail(Box::new(client.source_detail_with_handle(
                attachment,
                &workflow_id,
                &source_id,
                &preparation_handle,
            ))),
            Intent::ConfirmedSourceDetail {
                workflow_id,
                source_id,
            } => WorkerResult::SourceDetail(Box::new(
                match client.source_detail(attachment, &workflow_id, &source_id) {
                    ClientOutcome::Ok(detail) => ClientOutcome::Ok(SourceDetailResponse {
                        detail,
                        preparation_handle: None,
                    }),
                    ClientOutcome::Problem(problem) => ClientOutcome::Problem(problem),
                    ClientOutcome::Transport(error) => ClientOutcome::Transport(error),
                },
            )),
            Intent::RefineSource {
                workflow_id,
                source_id,
                revision,
                digest,
            } => WorkerResult::JobAccepted(client.start_source_refine(
                attachment,
                &workflow_id,
                &source_id,
                revision,
                &digest,
            )),
            Intent::GetSourceRefineSuggestion {
                workflow_id,
                source_id,
            } => WorkerResult::SourceRefineSuggestion(client.source_refine_suggestion(
                attachment,
                &workflow_id,
                &source_id,
            )),
            Intent::ResolveSourceRefine {
                workflow_id,
                source_id,
                revision,
                accept,
                digest,
            } => WorkerResult::SourceMutation(Box::new(client.resolve_source_refine(
                attachment,
                &workflow_id,
                &source_id,
                revision,
                accept,
                &digest,
            ))),
            Intent::EditSourceBody {
                workflow_id,
                source_id,
                revision,
                text,
                digest,
            } => WorkerResult::SourceMutation(Box::new(client.edit_source_body(
                attachment,
                &workflow_id,
                &source_id,
                revision,
                &text,
                &digest,
            ))),
            Intent::DeleteSource {
                workflow_id,
                source_id,
                revision,
            } => WorkerResult::SourceMutation(Box::new(client.delete_source(
                attachment,
                &workflow_id,
                &source_id,
                revision,
            ))),
            Intent::ConfirmSelection {
                workflow_id,
                revision,
                preparation_handle,
                source_ids,
            } => WorkerResult::SelectionConfirmed(Box::new(client.confirm_selection(
                attachment,
                &workflow_id,
                revision,
                &preparation_handle,
                &source_ids,
            ))),
            Intent::ConfirmEmptySelection {
                workflow_id,
                revision,
            } => WorkerResult::EmptySelectionConfirmed(Box::new(client.confirm_empty_selection(
                attachment,
                &workflow_id,
                revision,
            ))),
            Intent::SetWritingSettings {
                workflow_id,
                revision,
                language,
                style,
                assistant_backend_id,
            } => WorkerResult::WritingSettingsSet(Box::new(client.set_writing_settings(
                attachment,
                &workflow_id,
                revision,
                &language,
                &style,
                assistant_backend_id.as_deref(),
            ))),
            Intent::BeginManualComposition {
                workflow_id,
                revision,
            } => WorkerResult::ManualCompositionBegun(Box::new(client.begin_manual_composition(
                attachment,
                &workflow_id,
                revision,
            ))),
            Intent::SaveEntry {
                workflow_id,
                buffer_version,
                text,
            } => WorkerResult::EntrySaved(Box::new(client.save_entry(
                attachment,
                &workflow_id,
                buffer_version,
                &text,
            ))),
            Intent::ConfirmEntry {
                workflow_id,
                revision,
                buffer_version,
            } => WorkerResult::EntryConfirmed(Box::new(client.confirm_entry(
                attachment,
                &workflow_id,
                revision,
                buffer_version,
            ))),
            Intent::PreviewLocal {
                workflow_id,
                revision,
            } => WorkerResult::LocalPreviewed(Box::new(client.preview_local(
                attachment,
                &workflow_id,
                revision,
            ))),
            Intent::ApproveLocal {
                workflow_id,
                revision,
                plan_digest,
            } => WorkerResult::LocalApproved(Box::new(client.approve_local(
                attachment,
                &workflow_id,
                revision,
                &plan_digest,
            ))),
            Intent::PublishLocal {
                workflow_id,
                revision,
            } => WorkerResult::LocalPublished(client.publish_local(
                attachment,
                &workflow_id,
                revision,
            )),
            Intent::PreviewNotion {
                workflow_id,
                revision,
            } => WorkerResult::NotionPreviewed(Box::new(client.preview_notion(
                attachment,
                &workflow_id,
                revision,
            ))),
            Intent::ApproveNotion {
                workflow_id,
                revision,
                plan_digest,
            } => WorkerResult::NotionApproved(Box::new(client.approve_notion(
                attachment,
                &workflow_id,
                revision,
                &plan_digest,
            ))),
            Intent::PublishNotion {
                workflow_id,
                revision,
            } => WorkerResult::NotionPublished(client.publish_notion(
                attachment,
                &workflow_id,
                revision,
            )),
            Intent::SeedAmendment {
                workflow_id,
                revision,
            } => WorkerResult::AmendmentSeeded(Box::new(client.seed_amendment(
                attachment,
                &workflow_id,
                revision,
            ))),
            Intent::PreviewAmendment {
                workflow_id,
                revision,
            } => WorkerResult::AmendmentPreviewed(Box::new(client.preview_amendment(
                attachment,
                &workflow_id,
                revision,
            ))),
            Intent::ApproveAmendment {
                workflow_id,
                revision,
                plan_digest,
            } => WorkerResult::AmendmentApproved(Box::new(client.approve_amendment(
                attachment,
                &workflow_id,
                revision,
                &plan_digest,
            ))),
            Intent::PublishAmendment {
                workflow_id,
                revision,
            } => WorkerResult::AmendmentPublished(client.publish_amendment(
                attachment,
                &workflow_id,
                revision,
            )),
            Intent::RetryAmendmentNotion {
                workflow_id,
                revision,
            } => WorkerResult::AmendmentNotionRetried(client.retry_amendment_notion(
                attachment,
                &workflow_id,
                revision,
            )),
            Intent::RereadAmendment {
                workflow_id,
                revision,
            } => WorkerResult::AmendmentReread(Box::new(client.reread_amendment(
                attachment,
                &workflow_id,
                revision,
            ))),
            Intent::AbandonAmendment {
                workflow_id,
                revision,
            } => WorkerResult::AmendmentAbandoned(Box::new(client.abandon_amendment(
                attachment,
                &workflow_id,
                revision,
            ))),
            Intent::SaveUnderDay {
                workflow_id,
                revision,
                new_day,
            } => WorkerResult::SavedUnderDay(Box::new(client.save_under_day(
                attachment,
                &workflow_id,
                revision,
                &new_day,
            ))),
            Intent::SelectAssistant {
                workflow_id,
                revision,
                backend_id,
            } => WorkerResult::AssistantSelected(Box::new(client.set_assistant_selection(
                attachment,
                &workflow_id,
                revision,
                backend_id,
            ))),
            Intent::ExtractMemories {
                workflow_id,
                revision,
            } => WorkerResult::JobAccepted(client.start_memory_extraction(
                attachment,
                &workflow_id,
                revision,
            )),
            Intent::GetMemoryReview { workflow_id } => {
                WorkerResult::MemoryReview(client.memory_review(attachment, &workflow_id))
            }
            Intent::SaveMemoryReview {
                workflow_id,
                revision,
                candidate_set_id,
                reviewed_memory,
            } => WorkerResult::MemoryReviewSaved(Box::new(client.save_memory_review(
                attachment,
                &workflow_id,
                revision,
                candidate_set_id,
                reviewed_memory,
            ))),
            Intent::RequestDraft {
                workflow_id,
                revision,
                include_prior_draft,
            } => WorkerResult::JobAccepted(client.start_draft(
                attachment,
                &workflow_id,
                revision,
                include_prior_draft,
            )),
            Intent::GetDraftResult { workflow_id } => {
                WorkerResult::DraftResult(client.draft_result(attachment, &workflow_id))
            }
            Intent::ReviewParagraph {
                workflow_id,
                revision,
                request,
            } => WorkerResult::JobAccepted(client.start_paragraph_review(
                attachment,
                &workflow_id,
                revision,
                &request,
            )),
            Intent::GetParagraphSuggestions { workflow_id } => WorkerResult::ParagraphSuggestions(
                client.paragraph_suggestions(attachment, &workflow_id),
            ),
            Intent::GetParagraphSuggestion {
                workflow_id,
                suggestion_id,
            } => WorkerResult::ParagraphSuggestion(client.paragraph_suggestion(
                attachment,
                &workflow_id,
                &suggestion_id,
            )),
            Intent::ResolveParagraphSuggestion {
                workflow_id,
                suggestion_id,
                revision,
                request,
            } => WorkerResult::ParagraphSuggestionResolved(Box::new(
                client.resolve_paragraph_suggestion(
                    attachment,
                    &workflow_id,
                    &suggestion_id,
                    revision,
                    &request,
                ),
            )),
            Intent::CancelJob { job_id } => {
                WorkerResult::JobCancelled(client.cancel_job(attachment, &job_id))
            }
        }
    }

    /// Never blocks. A full queue means a duplicate intent is already
    /// pending; the caller should disable the UI action rather than
    /// queue another one or block the render loop.
    pub fn try_send(&self, intent: Intent) -> Result<(), SendError> {
        let Some(intents) = &self.intents else {
            return Err(SendError::WorkerGone);
        };
        intents.try_send(intent).map_err(|error| match error {
            TrySendError::Full(_) => SendError::QueueFull,
            TrySendError::Disconnected(_) => SendError::WorkerGone,
        })
    }

    /// Drains at most one result per call; the render loop calls this
    /// every tick until it returns `None`.
    pub fn try_recv(&self) -> Option<WorkerResult> {
        self.results.as_ref()?.try_recv().ok()
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        // Drop both channel halves *before* joining: a struct's `Drop`
        // runs before its fields are dropped, so without this the thread
        // would stay blocked forever in `intents_rx.recv()` (nothing
        // dropped `intents_tx` yet) or in `results_tx.send()` on a full
        // queue (nothing dropped `results_rx` yet), and `join()` below
        // would hang. See the field doc comment for the full reasoning.
        self.intents.take();
        self.results.take();
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    use super::*;
    use crate::client::{RawRequest, RawResponse, TransportError};

    /// A `Send`-safe fake transport (unlike the `RefCell`-based fakes used
    /// by synchronous ViewModel tests elsewhere): the worker thread owns
    /// it, so it must cross the `thread::spawn` boundary.
    struct FakeTransport {
        responses: Mutex<VecDeque<(&'static str, String, RawResponse)>>,
    }

    impl FakeTransport {
        fn new(responses: Vec<(&'static str, String, RawResponse)>) -> Self {
            Self {
                responses: Mutex::new(responses.into()),
            }
        }
    }

    impl Transport for FakeTransport {
        fn execute(
            &self,
            _attachment: &Attachment,
            request: &RawRequest,
        ) -> Result<RawResponse, TransportError> {
            let mut responses = self.responses.lock().unwrap();
            let index = responses
                .iter()
                .position(|(method, path, _)| *method == request.method && *path == request.path)
                .expect("unexpected request in fake transport");
            Ok(responses.remove(index).unwrap().2)
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

    fn recv_with_timeout(worker: &Worker) -> WorkerResult {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(result) = worker.try_recv() {
                return result;
            }
            assert!(Instant::now() < deadline, "worker did not reply in time");
            thread::sleep(Duration::from_millis(5));
        }
    }

    fn workflow_dto_body() -> Vec<u8> {
        serde_json::json!({
            "id": "wf_20261029",
            "diary_day": "2026-10-29",
            "state": "composing",
            "revision": 2,
            "buffer_version": 1,
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

    #[test]
    fn session_metadata_and_workflow_pagination_round_trip_through_the_worker() {
        let preferences_body = serde_json::to_vec(&PreferencesDto {
            last_language: Some("ko-KR".to_owned()),
            last_assistant_backend_id: None,
        })
        .unwrap();
        let transport = FakeTransport::new(vec![
            (
                "GET",
                "/v1/capabilities".to_owned(),
                RawResponse {
                    status: 200,
                    body: serde_json::json!({
                        "capabilities": ["read", "workflow:write"],
                        "manual_workflow_available": true,
                        "assistant_backends": [],
                        "disabled_routes": []
                    })
                    .to_string()
                    .into_bytes(),
                    ..RawResponse::default()
                },
            ),
            (
                "GET",
                "/v1/setup-check".to_owned(),
                RawResponse {
                    status: 200,
                    body: serde_json::json!({
                        "config_safe": true,
                        "database_ready": true,
                        "apple_helper_available": false,
                        "notion_configured": false,
                        "notion_reason_code": "notion-not-configured",
                        "manual_workflow_available": true
                    })
                    .to_string()
                    .into_bytes(),
                    ..RawResponse::default()
                },
            ),
            (
                "GET",
                "/v1/preferences".to_owned(),
                RawResponse {
                    status: 200,
                    body: preferences_body.clone(),
                    ..RawResponse::default()
                },
            ),
            (
                "PUT",
                "/v1/preferences".to_owned(),
                RawResponse {
                    status: 200,
                    body: preferences_body,
                    ..RawResponse::default()
                },
            ),
            (
                "GET",
                "/v1/workflows?cursor=next".to_owned(),
                RawResponse {
                    status: 200,
                    body: br#"{"workflows":[],"next_cursor":null}"#.to_vec(),
                    ..RawResponse::default()
                },
            ),
        ]);
        let worker = Worker::spawn(transport, attachment());
        worker.try_send(Intent::GetCapabilities).unwrap();
        assert!(matches!(
            recv_with_timeout(&worker),
            WorkerResult::Capabilities(ClientOutcome::Ok(_))
        ));
        worker.try_send(Intent::GetSetupCheck).unwrap();
        assert!(matches!(
            recv_with_timeout(&worker),
            WorkerResult::SetupCheck(ClientOutcome::Ok(_))
        ));
        worker.try_send(Intent::GetPreferences).unwrap();
        assert!(matches!(
            recv_with_timeout(&worker),
            WorkerResult::Preferences(ClientOutcome::Ok(_))
        ));
        worker
            .try_send(Intent::SavePreferences {
                last_language: Some("ko-KR".to_owned()),
                last_assistant_backend_id: None,
            })
            .unwrap();
        assert!(matches!(
            recv_with_timeout(&worker),
            WorkerResult::PreferencesSaved(ClientOutcome::Ok(_))
        ));
        worker
            .try_send(Intent::LoadMoreWorkflows {
                cursor: "next".to_owned(),
            })
            .unwrap();
        assert!(matches!(
            recv_with_timeout(&worker),
            WorkerResult::MoreWorkflows(ClientOutcome::Ok(_))
        ));
    }

    #[test]
    fn discard_workflow_sends_the_given_revision_and_reports_success() {
        let transport = FakeTransport::new(vec![(
            "DELETE",
            "/v1/workflows/wf_20261029".to_owned(),
            RawResponse {
                status: 204,
                ..RawResponse::default()
            },
        )]);
        let worker = Worker::spawn(transport, attachment());
        worker
            .try_send(Intent::DiscardWorkflow {
                id: "wf_20261029".to_owned(),
                revision: 7,
            })
            .unwrap();
        match recv_with_timeout(&worker) {
            WorkerResult::Discarded(ClientOutcome::Ok(())) => {}
            _ => panic!("expected a successful Discarded result"),
        }
    }

    #[test]
    fn confirm_empty_selection_round_trips_through_the_worker_thread() {
        let transport = FakeTransport::new(vec![(
            "POST",
            "/v1/workflows/wf_20261029/empty-selection-confirmations".to_owned(),
            RawResponse {
                status: 200,
                body: workflow_dto_body(),
                ..RawResponse::default()
            },
        )]);
        let worker = Worker::spawn(transport, attachment());
        worker
            .try_send(Intent::ConfirmEmptySelection {
                workflow_id: "wf_20261029".to_owned(),
                revision: 2,
            })
            .unwrap();
        match recv_with_timeout(&worker) {
            WorkerResult::EmptySelectionConfirmed(outcome) => match *outcome {
                ClientOutcome::Ok(workflow) => assert_eq!(workflow.id, "wf_20261029"),
                _ => panic!("expected a successful EmptySelectionConfirmed result"),
            },
            _ => panic!("expected an EmptySelectionConfirmed result"),
        }
    }

    #[test]
    fn save_entry_and_confirm_entry_each_round_trip_distinct_results() {
        let transport = FakeTransport::new(vec![
            (
                "PUT",
                "/v1/workflows/wf_20261029/entry".to_owned(),
                RawResponse {
                    status: 200,
                    body: workflow_dto_body(),
                    ..RawResponse::default()
                },
            ),
            (
                "POST",
                "/v1/workflows/wf_20261029/entry-confirmations".to_owned(),
                RawResponse {
                    status: 200,
                    body: workflow_dto_body(),
                    ..RawResponse::default()
                },
            ),
        ]);
        let worker = Worker::spawn(transport, attachment());
        worker
            .try_send(Intent::SaveEntry {
                workflow_id: "wf_20261029".to_owned(),
                buffer_version: 1,
                text: "hello".to_owned(),
            })
            .unwrap();
        match recv_with_timeout(&worker) {
            WorkerResult::EntrySaved(outcome) => {
                assert!(matches!(*outcome, ClientOutcome::Ok(_)));
            }
            _ => panic!("expected an EntrySaved result"),
        }
        worker
            .try_send(Intent::ConfirmEntry {
                workflow_id: "wf_20261029".to_owned(),
                revision: 2,
                buffer_version: 1,
            })
            .unwrap();
        match recv_with_timeout(&worker) {
            WorkerResult::EntryConfirmed(outcome) => {
                assert!(matches!(*outcome, ClientOutcome::Ok(_)));
            }
            _ => panic!("expected an EntryConfirmed result"),
        }
    }

    /// Blocks its first `execute` call until released, so a test can pin
    /// the worker thread mid-request and fill the queue deterministically
    /// behind it — unlike a fake that answers instantly, where whether the
    /// queue is actually full when `try_send` runs is a race.
    struct BlockingFakeTransport {
        first_release: Mutex<Option<Receiver<()>>>,
        first_started: Mutex<Option<SyncSender<()>>>,
    }

    impl Transport for BlockingFakeTransport {
        fn execute(
            &self,
            _attachment: &Attachment,
            _request: &RawRequest,
        ) -> Result<RawResponse, TransportError> {
            if let Some(release) = self.first_release.lock().unwrap().take() {
                if let Some(started) = self.first_started.lock().unwrap().take() {
                    let _ = started.send(());
                }
                release.recv().unwrap();
            }
            Ok(RawResponse {
                status: 200,
                body: workflow_dto_body(),
                ..RawResponse::default()
            })
        }
    }

    #[test]
    fn a_full_queue_is_reported_rather_than_blocking_the_caller() {
        let (release_tx, release_rx) = sync_channel::<()>(0);
        let (started_tx, started_rx) = sync_channel::<()>(0);
        let worker = Worker::spawn(
            BlockingFakeTransport {
                first_release: Mutex::new(Some(release_rx)),
                first_started: Mutex::new(Some(started_tx)),
            },
            attachment(),
        );

        // The first intent pins the worker thread inside `execute`
        // (blocked on `release`), so it never drains the queue. The
        // queue then holds exactly `COMMAND_QUEUE_CAPACITY` more before
        // a `try_send` must report `QueueFull` rather than block.
        worker
            .try_send(Intent::GetWorkflow {
                workflow_id: "wf_pinned".to_owned(),
            })
            .unwrap();
        started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        for _ in 0..COMMAND_QUEUE_CAPACITY {
            worker
                .try_send(Intent::PollJob {
                    job_id: "job_x".to_owned(),
                })
                .unwrap();
        }
        assert_eq!(
            worker.try_send(Intent::PollJob {
                job_id: "job_overflow".to_owned(),
            }),
            Err(SendError::QueueFull)
        );

        // Release the pinned call so the worker thread can drain the rest
        // and the `Worker` can drop cleanly at the end of the test.
        release_tx.send(()).unwrap();
    }

    #[test]
    fn dropping_the_worker_never_hangs_even_with_a_result_the_render_loop_never_drained() {
        // Fill the 32-slot result queue past capacity so the worker
        // thread would block in `results_tx.send` on the next reply —
        // exactly the condition the `Drop` fix must still unblock.
        let mut responses = Vec::new();
        for index in 0..(RESULT_QUEUE_CAPACITY + 1) {
            responses.push((
                "GET",
                format!("/v1/jobs/job_{index}"),
                RawResponse {
                    status: 200,
                    body: serde_json::to_vec(&JobDto {
                        id: format!("job_{index}"),
                        kind: "discovery".to_owned(),
                        status: "running".to_owned(),
                        workflow_id: "wf_20261029".to_owned(),
                        progress_percent: 0,
                        result_handle: None,
                        error_code: None,
                        expires_at_unix_ms: 0,
                    })
                    .unwrap(),
                    ..RawResponse::default()
                },
            ));
        }
        let worker = Worker::spawn(FakeTransport::new(responses), attachment());
        for index in 0..(RESULT_QUEUE_CAPACITY + 1) {
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut intent = Intent::PollJob {
                job_id: format!("job_{index}"),
            };
            loop {
                match worker.try_send(intent) {
                    Ok(()) => break,
                    Err(SendError::QueueFull) if Instant::now() < deadline => {
                        thread::yield_now();
                        intent = Intent::PollJob {
                            job_id: format!("job_{index}"),
                        };
                    }
                    error => panic!("failed to queue result-saturation intent: {error:?}"),
                }
            }
        }
        // Deliberately never drain `try_recv`: drop the `Worker` with
        // results still backed up, on a helper thread, and fail fast
        // (rather than hanging the test run) if `drop` does not return.
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        thread::spawn(move || {
            drop(worker);
            let _ = done_tx.send(());
        });
        assert!(
            done_rx.recv_timeout(Duration::from_secs(5)).is_ok(),
            "Worker::drop must not hang even when the result queue was never drained"
        );
    }
}
