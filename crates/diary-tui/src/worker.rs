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

use diary_control_contract::{JobAcceptedDto, JobDto, SourceListDto, WorkflowDto, WorkflowListDto};

use crate::client::{ApiClient, Attachment, ClientOutcome, SourceDetailResponse, Transport};

const COMMAND_QUEUE_CAPACITY: usize = 16;
const RESULT_QUEUE_CAPACITY: usize = 32;

/// A unit of work the render loop wants done off-thread. Each variant
/// names the operation, not a raw request; the worker is the only thing
/// that knows the route/header details, same as `ApiClient`.
#[derive(Clone, Debug)]
pub enum Intent {
    RefreshWorkflows,
    StartOrResumeWorkflow {
        diary_day: Option<String>,
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
        cursor: Option<usize>,
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
}

/// The worker's reply, paired with the intent that produced it so the
/// render loop can tell which pending action just completed.
pub enum WorkerResult {
    Workflows(ClientOutcome<WorkflowListDto>),
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
    SelectionConfirmed(Box<ClientOutcome<WorkflowDto>>),
    EmptySelectionConfirmed(Box<ClientOutcome<WorkflowDto>>),
    WritingSettingsSet(Box<ClientOutcome<WorkflowDto>>),
    ManualCompositionBegun(Box<ClientOutcome<WorkflowDto>>),
    EntrySaved(Box<ClientOutcome<WorkflowDto>>),
    EntryConfirmed(Box<ClientOutcome<WorkflowDto>>),
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
            Intent::RefreshWorkflows => {
                WorkerResult::Workflows(client.list_workflows(attachment, None))
            }
            Intent::StartOrResumeWorkflow { diary_day } => WorkerResult::StartedWorkflow(Box::new(
                client.start_workflow(attachment, diary_day.as_deref()),
            )),
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
            } => WorkerResult::Sources(client.list_sources(attachment, &workflow_id, cursor)),
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
            } => WorkerResult::SourceDetail(Box::new(client.source_detail_with_handle(
                attachment,
                &workflow_id,
                &source_id,
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
            } => WorkerResult::WritingSettingsSet(Box::new(client.set_writing_settings(
                attachment,
                &workflow_id,
                revision,
                &language,
                &style,
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
        release: Receiver<()>,
    }

    impl Transport for BlockingFakeTransport {
        fn execute(
            &self,
            _attachment: &Attachment,
            _request: &RawRequest,
        ) -> Result<RawResponse, TransportError> {
            self.release.recv().unwrap();
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
        let worker = Worker::spawn(
            BlockingFakeTransport {
                release: release_rx,
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
            worker
                .try_send(Intent::PollJob {
                    job_id: format!("job_{index}"),
                })
                .unwrap();
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
