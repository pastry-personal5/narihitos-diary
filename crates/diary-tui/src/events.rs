//! Dedicated bounded SSE worker. Events are notifications only; callers feed
//! them to `SessionAuthority` and perform canonical REST reads before mutation.

use std::io::Read;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::thread;
use std::time::Duration;

use diary_control_contract::EventDto;
use ureq::Agent;
use ureq::config::Config;

use crate::client::{Attachment, SseParser};

const EVENT_RESULT_CAPACITY: usize = 32;
const MAX_EVENT_BYTES: usize = 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const SEND_TIMEOUT: Duration = Duration::from_secs(5);
const HEADER_TIMEOUT: Duration = Duration::from_secs(5);
const BODY_WINDOW: Duration = Duration::from_secs(45);
const INITIAL_BACKOFF: Duration = Duration::from_millis(250);
const MAX_BACKOFF: Duration = Duration::from_secs(4);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EventError {
    ConnectFailed,
    Timeout,
    Rejected,
    WrongContentType,
    DecodeFailed,
}

pub trait EventSource: Send + 'static {
    fn open(
        &mut self,
        attachment: &Attachment,
        workflow_id: Option<&str>,
        last_event_id: Option<u64>,
    ) -> Result<Box<dyn Read + Send>, EventError>;
}

pub struct UreqEventSource {
    agent: Agent,
}

impl UreqEventSource {
    #[must_use]
    pub fn new() -> Self {
        let config: Config = Agent::config_builder()
            .proxy(None)
            .https_only(false)
            .http_status_as_error(false)
            .max_redirects(0)
            .timeout_connect(Some(CONNECT_TIMEOUT))
            .timeout_send_request(Some(SEND_TIMEOUT))
            .timeout_recv_response(Some(HEADER_TIMEOUT))
            .timeout_recv_body(Some(BODY_WINDOW))
            .build();
        Self {
            agent: Agent::new_with_config(config),
        }
    }
}

impl Default for UreqEventSource {
    fn default() -> Self {
        Self::new()
    }
}

impl EventSource for UreqEventSource {
    fn open(
        &mut self,
        attachment: &Attachment,
        workflow_id: Option<&str>,
        last_event_id: Option<u64>,
    ) -> Result<Box<dyn Read + Send>, EventError> {
        let path = workflow_id.map_or_else(
            || "/v1/events".to_owned(),
            |id| format!("/v1/events?workflow_id={}", crate::client::urlencode(id)),
        );
        let url = format!("{}{}", attachment.base_url(), path);
        let mut request = self
            .agent
            .get(&url)
            .header(
                "Authorization",
                format!("Bearer {}", attachment.control_token()),
            )
            .header("Accept", "text/event-stream");
        if let Some(last_event_id) = last_event_id {
            request = request.header("Last-Event-ID", last_event_id.to_string());
        }
        let response = request.call().map_err(classify_ureq_error)?;
        if !(200..300).contains(&response.status().as_u16()) {
            return Err(EventError::Rejected);
        }
        let is_event_stream = response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.eq_ignore_ascii_case("text/event-stream"));
        if !is_event_stream {
            return Err(EventError::WrongContentType);
        }
        let (_, body) = response.into_parts();
        Ok(Box::new(body.into_reader()))
    }
}

fn classify_ureq_error(error: ureq::Error) -> EventError {
    match error {
        ureq::Error::Timeout(_) => EventError::Timeout,
        ureq::Error::ConnectionFailed | ureq::Error::Io(_) => EventError::ConnectFailed,
        _ => EventError::Rejected,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EventWorkerResult {
    Connected,
    Event(EventDto),
    Disconnected(EventError),
    ResultOverflow,
}

pub struct EventWorker {
    results: Receiver<EventWorkerResult>,
    overflowed: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
}

impl EventWorker {
    #[must_use]
    pub fn spawn<S: EventSource>(
        mut source: S,
        attachment: Attachment,
        workflow_id: Option<String>,
    ) -> Self {
        let (results_tx, results_rx) = sync_channel(EVENT_RESULT_CAPACITY);
        let overflowed = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let thread_overflowed = Arc::clone(&overflowed);
        let thread_stop = Arc::clone(&stop);
        thread::spawn(move || {
            run_event_loop(
                &mut source,
                &attachment,
                workflow_id.as_deref(),
                &results_tx,
                &thread_overflowed,
                &thread_stop,
            );
        });
        Self {
            results: results_rx,
            overflowed,
            stop,
        }
    }

    pub fn try_recv(&self) -> Option<EventWorkerResult> {
        if self.overflowed.swap(false, Ordering::AcqRel) {
            return Some(EventWorkerResult::ResultOverflow);
        }
        self.results.try_recv().ok()
    }
}

impl Drop for EventWorker {
    fn drop(&mut self) {
        // A production reader can be inside its bounded 45-second body window.
        // Signal it to stop at the next read boundary but never freeze terminal
        // restoration by joining a blocking network read on the render thread.
        self.stop.store(true, Ordering::Release);
    }
}

fn run_event_loop<S: EventSource>(
    source: &mut S,
    attachment: &Attachment,
    workflow_id: Option<&str>,
    results: &SyncSender<EventWorkerResult>,
    overflowed: &AtomicBool,
    stop: &AtomicBool,
) {
    let mut last_event_id = None;
    let mut backoff = INITIAL_BACKOFF;
    while !stop.load(Ordering::Acquire) {
        let mut reader = match source.open(attachment, workflow_id, last_event_id) {
            Ok(reader) => {
                backoff = INITIAL_BACKOFF;
                reader
            }
            Err(error) => {
                if !send_result(results, EventWorkerResult::Disconnected(error), overflowed) {
                    return;
                }
                interruptible_wait(backoff, stop);
                backoff = (backoff * 2).min(MAX_BACKOFF);
                continue;
            }
        };
        let mut parser = SseParser::new(MAX_EVENT_BYTES);
        // Ask the streaming body for one decoded byte at a time. Ureq retains
        // its own socket buffer, while the bounded SSE parser accumulates the
        // frame; the one-byte destination prevents a small chunked event from
        // waiting for a later heartbeat before Read returns under load.
        let mut chunk = [0_u8; 1];
        let mut body_confirmed = false;
        loop {
            if stop.load(Ordering::Acquire) {
                return;
            }
            match reader.read(&mut chunk) {
                Ok(0) => break,
                Ok(count) => {
                    if !body_confirmed {
                        if !send_result(results, EventWorkerResult::Connected, overflowed) {
                            return;
                        }
                        body_confirmed = true;
                    }
                    match parser.feed(&chunk[..count]) {
                        Ok(events) => {
                            for event in events {
                                last_event_id = Some(event.id);
                                if !send_result(
                                    results,
                                    EventWorkerResult::Event(event),
                                    overflowed,
                                ) {
                                    return;
                                }
                            }
                        }
                        Err(_) => {
                            let _ = send_result(
                                results,
                                EventWorkerResult::Disconnected(EventError::DecodeFailed),
                                overflowed,
                            );
                            break;
                        }
                    }
                }
                Err(_) => {
                    let _ = send_result(
                        results,
                        EventWorkerResult::Disconnected(EventError::ConnectFailed),
                        overflowed,
                    );
                    break;
                }
            }
        }
        interruptible_wait(backoff, stop);
        backoff = (backoff * 2).min(MAX_BACKOFF);
    }
}

fn send_result(
    results: &SyncSender<EventWorkerResult>,
    result: EventWorkerResult,
    overflowed: &AtomicBool,
) -> bool {
    match results.try_send(result) {
        Ok(()) => true,
        Err(TrySendError::Full(_)) => {
            overflowed.store(true, Ordering::Release);
            true
        }
        Err(TrySendError::Disconnected(_)) => false,
    }
}

fn interruptible_wait(duration: Duration, stop: &AtomicBool) {
    let mut remaining = duration;
    let quantum = Duration::from_millis(50);
    while !remaining.is_zero() && !stop.load(Ordering::Acquire) {
        let wait = remaining.min(quantum);
        thread::sleep(wait);
        remaining = remaining.saturating_sub(wait);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::io::Cursor;
    use std::sync::Mutex;
    use std::time::Instant;

    use super::*;

    struct FakeEventSource {
        streams: Mutex<VecDeque<Result<Vec<u8>, EventError>>>,
        seen_last_ids: Arc<Mutex<Vec<Option<u64>>>>,
    }

    impl EventSource for FakeEventSource {
        fn open(
            &mut self,
            _attachment: &Attachment,
            _workflow_id: Option<&str>,
            last_event_id: Option<u64>,
        ) -> Result<Box<dyn Read + Send>, EventError> {
            self.seen_last_ids.lock().unwrap().push(last_event_id);
            match self.streams.lock().unwrap().pop_front() {
                Some(Ok(bytes)) => Ok(Box::new(Cursor::new(bytes))),
                Some(Err(error)) => Err(error),
                None => Err(EventError::ConnectFailed),
            }
        }
    }

    fn attachment() -> Attachment {
        Attachment::new(
            "instance_a".to_owned(),
            "127.0.0.1".to_owned(),
            4000,
            "token_synthetic".to_owned(),
            2,
        )
    }

    fn frame(id: u64) -> Vec<u8> {
        let event = EventDto {
            id,
            instance_id: "instance_a".to_owned(),
            event_type: "workflow.changed".to_owned(),
            workflow_id: Some("wf_20261029".to_owned()),
            revision: Some(2),
            job_id: None,
            progress_percent: None,
        };
        format!(
            "id: {id}\nevent: workflow.changed\ndata: {}\n\n",
            serde_json::to_string(&event).unwrap()
        )
        .into_bytes()
    }

    fn recv_until(
        worker: &EventWorker,
        predicate: impl Fn(&EventWorkerResult) -> bool,
    ) -> EventWorkerResult {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(result) = worker.try_recv()
                && predicate(&result)
            {
                return result;
            }
            assert!(Instant::now() < deadline, "event worker did not reply");
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn decoded_events_are_forwarded() {
        let worker = EventWorker::spawn(
            FakeEventSource {
                streams: Mutex::new(VecDeque::from([Ok(frame(1))])),
                seen_last_ids: Arc::new(Mutex::new(Vec::new())),
            },
            attachment(),
            Some("wf_20261029".to_owned()),
        );
        let result = recv_until(&worker, |result| {
            matches!(result, EventWorkerResult::Event(_))
        });
        assert!(matches!(result, EventWorkerResult::Event(event) if event.id == 1));
    }

    #[test]
    fn connected_is_announced_only_after_the_stream_delivers_body_bytes() {
        let worker = EventWorker::spawn(
            FakeEventSource {
                streams: Mutex::new(VecDeque::from([Ok(frame(1))])),
                seen_last_ids: Arc::new(Mutex::new(Vec::new())),
            },
            attachment(),
            None,
        );
        assert_eq!(recv_until(&worker, |_| true), EventWorkerResult::Connected);
        assert!(
            matches!(recv_until(&worker, |_| true), EventWorkerResult::Event(event) if event.id == 1)
        );
    }

    #[test]
    fn reconnect_uses_the_last_completely_decoded_event_id() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let worker = EventWorker::spawn(
            FakeEventSource {
                streams: Mutex::new(VecDeque::from([
                    Ok(frame(7)),
                    Err(EventError::ConnectFailed),
                ])),
                seen_last_ids: Arc::clone(&seen),
            },
            attachment(),
            None,
        );
        let _ = recv_until(&worker, |result| {
            matches!(result, EventWorkerResult::Event(_))
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        while seen.lock().unwrap().len() < 2 {
            assert!(Instant::now() < deadline, "worker did not reconnect");
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(seen.lock().unwrap()[..2], [None, Some(7)]);
    }

    #[test]
    fn malformed_streams_report_a_content_free_decode_failure() {
        let worker = EventWorker::spawn(
            FakeEventSource {
                streams: Mutex::new(VecDeque::from([Ok(b"data: nope\n\n".to_vec())])),
                seen_last_ids: Arc::new(Mutex::new(Vec::new())),
            },
            attachment(),
            None,
        );
        let result = recv_until(&worker, |result| {
            matches!(
                result,
                EventWorkerResult::Disconnected(EventError::DecodeFailed)
            )
        });
        assert_eq!(
            result,
            EventWorkerResult::Disconnected(EventError::DecodeFailed)
        );
    }
}
