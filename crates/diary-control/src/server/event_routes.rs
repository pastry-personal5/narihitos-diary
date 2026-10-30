//! Event routes at the HTTP boundary.

use super::*;

pub(super) async fn get_job(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<JobDto>, ApiError> {
    cleanup_jobs(&state);
    let jobs = state.0.jobs.lock().map_err(|_| ApiError::internal())?;
    let record = jobs
        .jobs
        .get(&id)
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, JOB_NOT_FOUND))?;
    Ok(Json(job_dto(record)))
}

pub(super) async fn cancel_job(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let mut jobs = state.0.jobs.lock().map_err(|_| ApiError::internal())?;
    let record = jobs
        .jobs
        .get_mut(&id)
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, JOB_NOT_FOUND))?;
    if record.kind == "local-publication"
        && record.commit_state.load(Ordering::Acquire) == COMMITTING
    {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "publication-commit-started",
        ));
    }
    if record.status == job_status::QUEUED {
        record.cancelled.store(true, Ordering::Release);
        record
            .commit_state
            .store(COMMIT_CANCELLED, Ordering::Release);
        record.status = job_status::CANCELLED;
        record.expires_at_ms = now_ms().saturating_add(JOB_EXPIRY_MS);
    } else if record.status == job_status::RUNNING
        && (record.kind == "notion-publication"
            || record.kind == "amendment-publication"
            || record.kind == "amendment-notion-retry"
            || record
                .commit_state
                .compare_exchange(
                    COMMIT_ACTIVE,
                    COMMIT_CANCELLED,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_ok())
    {
        record.cancelled.store(true, Ordering::Release);
        record.status = job_status::CANCELLATION_REQUESTED;
    }
    if let Some(handle) = record.result_handle.take() {
        jobs.preparations.remove(&handle);
    }
    Ok(StatusCode::ACCEPTED)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct EventsQuery {
    workflow_id: Option<String>,
}

pub(super) async fn events(
    State(state): State<AppState>,
    Query(query): Query<EventsQuery>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    if let Some(workflow_id) = &query.workflow_id {
        day_from_id(workflow_id)?;
    }
    if state.0.subscriber_count.fetch_add(1, Ordering::AcqRel) >= MAX_SUBSCRIBERS {
        state.0.subscriber_count.fetch_sub(1, Ordering::AcqRel);
        return Err(ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "subscriber-capacity",
        ));
    }
    let last_headers = headers.get_all("last-event-id").iter().collect::<Vec<_>>();
    let (last_id, invalid_last_id) = match last_headers.as_slice() {
        [] => (None, false),
        [value] => match value
            .to_str()
            .ok()
            .filter(|value| value.len() <= 20)
            .and_then(|value| value.parse::<u64>().ok())
        {
            Some(value) => (Some(value), false),
            None => (None, true),
        },
        _ => (None, true),
    };
    let (tx, rx) = mpsc::channel(SUBSCRIBER_QUEUE_CAPACITY);
    let (overflow_tx, mut overflow_rx) = oneshot::channel();
    let subscriber_id = {
        let mut events = match state.0.events.lock() {
            Ok(events) => events,
            Err(_) => {
                state.0.subscriber_count.fetch_sub(1, Ordering::AcqRel);
                return Err(ApiError::internal());
            }
        };
        let subscriber_id = events.next_subscriber_id;
        events.next_subscriber_id = events.next_subscriber_id.saturating_add(1);
        match replay_decision(
            &events,
            query.workflow_id.as_deref(),
            last_id,
            invalid_last_id,
        ) {
            ReplayDecision::Reset => {
                let reset = EventDto {
                    id: events.next_id.saturating_sub(1),
                    instance_id: state.0.config.instance_id.clone(),
                    event_type: "reset".to_owned(),
                    workflow_id: query.workflow_id.clone(),
                    revision: None,
                    job_id: None,
                    progress_percent: None,
                };
                let _ = tx.try_send(Ok(Bytes::from(sse_bytes(&reset))));
            }
            ReplayDecision::Events(replay) => {
                for event in replay {
                    let _ = tx.try_send(Ok(Bytes::from(sse_bytes(&event))));
                }
            }
        }
        events.subscribers.push(Subscriber {
            id: subscriber_id,
            sender: tx.clone(),
            workflow_id: query.workflow_id.clone(),
            overflow: Some(overflow_tx),
        });
        subscriber_id
    };
    // Put one body frame on every newly established stream immediately.
    // Besides giving HTTP intermediaries something to flush, this means the
    // client can prove the SSE body is flowing before the first mutation;
    // comments consume no event ID and are ignored by the typed parser.
    let _ = tx.try_send(Ok(Bytes::from_static(b": connected\n\n")));
    let count = Arc::clone(&state.0.subscriber_count);
    let timer_state = state.clone();
    let mut shutdown_rx = state.0.shutdown.subscribe();
    tokio::spawn(async move {
        let deadline = tokio::time::sleep(STREAM_LIFETIME);
        tokio::pin!(deadline);
        loop {
            tokio::select! {
                _ = tokio::time::sleep(HEARTBEAT) => {
                    if tx.try_send(Ok(Bytes::from_static(b": keep-alive\n\n"))).is_err() { break; }
                }
                _ = &mut overflow_rx => break,
                _ = &mut deadline => break,
                _ = shutdown_rx.wait_for(|stopping| *stopping) => break,
            }
        }
        remove_subscriber(&timer_state, subscriber_id);
    });
    let stream = EventStream {
        receiver: rx,
        count,
        state,
        subscriber_id,
    };
    let mut response = Response::new(Body::from_stream(stream));
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("text/event-stream"));
    Ok(response)
}

pub(super) struct EventStream {
    receiver: mpsc::Receiver<Result<Bytes, Infallible>>,
    count: Arc<AtomicUsize>,
    state: AppState,
    subscriber_id: u64,
}

impl Stream for EventStream {
    type Item = Result<Bytes, Infallible>;
    fn poll_next(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.receiver.poll_recv(context)
    }
}

impl Drop for EventStream {
    fn drop(&mut self) {
        remove_subscriber(&self.state, self.subscriber_id);
        self.count.fetch_sub(1, Ordering::AcqRel);
    }
}

pub(super) fn remove_subscriber(state: &AppState, subscriber_id: u64) {
    if let Ok(mut events) = state.0.events.lock() {
        events
            .subscribers
            .retain(|subscriber| subscriber.id != subscriber_id);
    }
}

pub(super) enum ReplayDecision {
    Reset,
    Events(Vec<EventDto>),
}

pub(super) fn replay_decision(
    events: &EventState,
    workflow_id: Option<&str>,
    last_id: Option<u64>,
    invalid_last_id: bool,
) -> ReplayDecision {
    if invalid_last_id {
        return ReplayDecision::Reset;
    }
    let Some(last) = last_id else {
        return ReplayDecision::Events(Vec::new());
    };
    let first = events.replay.front().map(|event| event.id);
    if first.is_some_and(|first| last.saturating_add(1) < first) || last >= events.next_id {
        return ReplayDecision::Reset;
    }
    let replay = events
        .replay
        .iter()
        .filter(|event| {
            event.id > last && workflow_id.is_none_or(|id| event.workflow_id.as_deref() == Some(id))
        })
        .cloned()
        .collect::<Vec<_>>();
    if replay.len() > SUBSCRIBER_QUEUE_CAPACITY {
        ReplayDecision::Reset
    } else {
        ReplayDecision::Events(replay)
    }
}
