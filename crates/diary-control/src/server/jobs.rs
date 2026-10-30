//! Bounded job lifecycle, preparation handles, and cancellation fences.

use super::*;

pub(super) fn new_job(
    state: &AppState,
    kind: &'static str,
    workflow_id: &str,
) -> Result<String, ApiError> {
    cleanup_jobs(state);
    let mut jobs = state.0.jobs.lock().map_err(|_| ApiError::internal())?;
    let active = jobs
        .jobs
        .values()
        .filter(|job| job_status::is_active(job.status))
        .count();
    if is_assistant_job(kind)
        && jobs.jobs.values().any(|job| {
            job.workflow_id == workflow_id
                && is_assistant_job(job.kind)
                && job_status::is_active(job.status)
        })
    {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "assistant-job-already-active",
        ));
    }
    if active >= JOB_QUEUE_CAPACITY + JOB_CONCURRENCY || jobs.jobs.len() >= MAX_JOBS {
        return Err(ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "job-capacity",
        ));
    }
    let id = random_id("job_").map_err(|_| ApiError::internal())?;
    jobs.jobs.insert(
        id.clone(),
        JobRecord {
            id: id.clone(),
            kind,
            status: job_status::QUEUED,
            workflow_id: workflow_id.to_owned(),
            progress: 0,
            result_handle: None,
            error_code: None,
            expires_at_ms: now_ms().saturating_add(JOB_EXPIRY_MS),
            commit_state: Arc::new(AtomicU8::new(COMMIT_ACTIVE)),
            cancelled: Arc::new(AtomicBool::new(false)),
        },
    );
    drop(jobs);
    log_debug(
        state,
        DiagnosticEvent::Job {
            kind,
            event: "accepted",
            reason: None,
        },
    );
    Ok(id)
}

pub(super) fn is_assistant_job(kind: &str) -> bool {
    matches!(
        kind,
        "assistant-memory-extraction"
            | "assistant-draft"
            | "assistant-paragraph-review"
            | "assistant-source-refine"
    )
}

pub(super) fn ensure_no_assistant_job(state: &AppState, workflow_id: &str) -> Result<(), ApiError> {
    let jobs = state.0.jobs.lock().map_err(|_| ApiError::internal())?;
    if jobs.jobs.values().any(|job| {
        job.workflow_id == workflow_id
            && is_assistant_job(job.kind)
            && job_status::is_active(job.status)
    }) {
        Err(ApiError::new(
            StatusCode::CONFLICT,
            "assistant-job-already-active",
        ))
    } else {
        Ok(())
    }
}

pub(super) async fn run_discovery_job(
    state: AppState,
    job_id: String,
    workflow_id: String,
    day: DiaryDay,
    revision: u64,
    explicit: Option<DiaryDay>,
) {
    let Ok(permit) = Arc::clone(&state.0.workers).acquire_owned().await else {
        return;
    };
    if !mark_running(&state, &job_id) {
        return;
    }
    let Some(commit_state) = job_commit_state(&state, &job_id) else {
        fail_job(&state, &job_id, "internal-failure");
        return;
    };
    let service = state.0.config.commands.clone();
    let worker_commit_state = Arc::clone(&commit_state);
    let mut task = tokio::task::spawn_blocking(move || {
        service.discover_guarded(&day, revision, explicit, || {
            worker_commit_state
                .compare_exchange(
                    COMMIT_ACTIVE,
                    COMMITTING,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_ok()
        })
    });
    let result = match tokio::time::timeout(Duration::from_secs(120), &mut task).await {
        Ok(result) => result,
        Err(_) => {
            match commit_state.compare_exchange(
                COMMIT_ACTIVE,
                COMMIT_CANCELLED,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    drop(permit);
                    fail_job(&state, &job_id, "job-timeout");
                    return;
                }
                Err(COMMIT_CANCELLED) => {
                    drop(permit);
                    fail_job(&state, &job_id, "operation-cancelled");
                    return;
                }
                Err(_) => {}
            }
            task.await
        }
    };
    drop(permit);
    match result {
        Ok(Ok(workflow)) => {
            finish_job(&state, &job_id, None, None);
            publish_event(
                &state,
                "workflow.changed",
                Some(workflow_id),
                Some(workflow.revision()),
                Some(job_id),
                Some(100),
            );
        }
        Ok(Err(error)) => fail_job(&state, &job_id, error.diagnostic_code()),
        Err(_) => fail_job(&state, &job_id, "internal-failure"),
    }
}

pub(super) enum AssistantJob {
    Extract {
        revision: u64,
        artifact_id: String,
    },
    Draft {
        revision: u64,
        artifact_id: String,
        include_prior: bool,
    },
    Paragraph {
        revision: u64,
        suggestion_id: String,
        range: ReflectionRange,
    },
    Refine {
        revision: u64,
        source: SourceRef,
        digest: Digest,
    },
}

impl AssistantJob {
    /// The `Job.kind` token, shared with the job's own lifecycle records.
    const fn kind(&self) -> &'static str {
        match self {
            Self::Extract { .. } => "assistant-memory-extraction",
            Self::Draft { .. } => "assistant-draft",
            Self::Paragraph { .. } => "assistant-paragraph-review",
            Self::Refine { .. } => "assistant-source-refine",
        }
    }
}

pub(super) async fn run_assistant_job(
    state: AppState,
    job_id: String,
    workflow_id: String,
    day: DiaryDay,
    operation: AssistantJob,
) {
    let Ok(permit) = Arc::clone(&state.0.workers).acquire_owned().await else {
        return;
    };
    if !mark_running(&state, &job_id) {
        return;
    }
    let Some((commit_state, cancelled)) = assistant_job_state(&state, &job_id) else {
        fail_job(&state, &job_id, "internal-failure");
        return;
    };
    let kind = operation.kind();
    log_assistant_snapshot(&state, kind, "start", &day).await;
    let snapshot_day = day.clone();
    let service = state.0.config.commands.clone();
    let registry = Arc::clone(&state.0.config.assistant_registry);
    let commit_for_worker = Arc::clone(&commit_state);
    let cancelled_for_worker = Arc::clone(&cancelled);
    let result_handle = match &operation {
        AssistantJob::Extract { artifact_id, .. } | AssistantJob::Draft { artifact_id, .. } => {
            artifact_id.clone()
        }
        AssistantJob::Paragraph { suggestion_id, .. } => suggestion_id.clone(),
        AssistantJob::Refine { source, .. } => opaque_source_id(source),
    };
    let mut task = tokio::task::spawn_blocking(move || {
        let commit = || {
            commit_for_worker
                .compare_exchange(
                    COMMIT_ACTIVE,
                    COMMITTING,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_ok()
        };
        match operation {
            AssistantJob::Extract {
                revision,
                artifact_id,
            } => service.extract_memories(
                &day,
                revision,
                artifact_id,
                &registry,
                &cancelled_for_worker,
                commit,
            ),
            AssistantJob::Draft {
                revision,
                artifact_id,
                include_prior,
            } => service.draft_entry(
                &day,
                revision,
                artifact_id,
                include_prior,
                &registry,
                &cancelled_for_worker,
                commit,
            ),
            AssistantJob::Paragraph {
                revision,
                suggestion_id,
                range,
            } => service.review_paragraph(
                &day,
                revision,
                suggestion_id,
                range,
                &registry,
                &cancelled_for_worker,
                commit,
            ),
            AssistantJob::Refine {
                revision,
                source,
                digest,
            } => service.refine_source(
                &day,
                revision,
                &source,
                &digest,
                &registry,
                &cancelled_for_worker,
                commit,
            ),
        }
    });
    let result = match tokio::time::timeout(Duration::from_secs(120), &mut task).await {
        Ok(result) => result,
        Err(_) => {
            cancelled.store(true, Ordering::Release);
            if commit_state
                .compare_exchange(
                    COMMIT_ACTIVE,
                    COMMIT_CANCELLED,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_ok()
            {
                drop(permit);
                fail_job(&state, &job_id, "assistant-timeout");
                return;
            }
            task.await
        }
    };
    drop(permit);
    match result {
        Ok(Ok(workflow)) => {
            finish_job(&state, &job_id, Some(result_handle), None);
            publish_event(
                &state,
                "workflow.changed",
                Some(workflow_id),
                Some(workflow.revision()),
                Some(job_id),
                Some(100),
            );
        }
        Ok(Err(error)) => {
            log_assistant_snapshot(&state, kind, "failed", &snapshot_day).await;
            fail_job(&state, &job_id, error.diagnostic_code());
        }
        Err(_) => fail_job(&state, &job_id, "internal-failure"),
    }
}

/// Best-effort Debug-level snapshot of the workflow an assistant job runs
/// against. Reads on a blocking thread because the store is synchronous
/// SQLite.
pub(super) async fn log_assistant_snapshot(
    state: &AppState,
    kind: &'static str,
    event_status: &'static str,
    day: &DiaryDay,
) {
    let Some(log) = state.0.config.diagnostics.clone() else {
        return;
    };
    if !log.enabled(DiagnosticLevel::Debug) {
        return;
    }
    let service = state.0.config.commands.clone();
    let day = day.clone();
    let _ = tokio::task::spawn_blocking(move || {
        let Ok(workflow) = service.load_workflow(&day) else {
            return;
        };
        if let Some(snapshot) = AssistantSnapshot::capture(&log, &workflow, kind, event_status) {
            let _ = log.write(
                DiagnosticLevel::Debug,
                DiagnosticEvent::AssistantState(snapshot),
            );
        }
    })
    .await;
}

/// Best-effort Debug-level snapshot of a day's publication state when a
/// publication job ends. Reads on a blocking thread because the store is
/// synchronous SQLite.
pub(super) async fn log_publication_snapshot(
    state: &AppState,
    kind: &'static str,
    event_status: &'static str,
    day: &DiaryDay,
) {
    let Some(log) = state.0.config.diagnostics.clone() else {
        return;
    };
    if !log.enabled(DiagnosticLevel::Debug) {
        return;
    }
    let service = state.0.config.commands.clone();
    let day = day.clone();
    let _ = tokio::task::spawn_blocking(move || {
        let Ok(workflow) = service.load_workflow(&day) else {
            return;
        };
        let (receipts, unresolved) = service.publication_counts(&day).unwrap_or((0, 0));
        if let Some(snapshot) =
            PublicationSnapshot::capture(&log, &workflow, kind, event_status, receipts, unresolved)
        {
            let _ = log.write(
                DiagnosticLevel::Debug,
                DiagnosticEvent::PublicationState(snapshot),
            );
        }
    })
    .await;
}

pub(super) fn assistant_job_state(
    state: &AppState,
    id: &str,
) -> Option<(Arc<AtomicU8>, Arc<AtomicBool>)> {
    let jobs = state.0.jobs.lock().ok()?;
    let job = jobs.jobs.get(id)?;
    Some((Arc::clone(&job.commit_state), Arc::clone(&job.cancelled)))
}

pub(super) async fn run_prepare_job(
    state: AppState,
    job_id: String,
    workflow_id: String,
    day: DiaryDay,
    revision: u64,
    source_ids: Vec<String>,
    sources: Vec<SourceRef>,
) {
    let Ok(permit) = Arc::clone(&state.0.workers).acquire_owned().await else {
        return;
    };
    if !mark_running(&state, &job_id) {
        return;
    }
    let service = state.0.config.commands.clone();
    let result = tokio::time::timeout(
        Duration::from_secs(120),
        tokio::task::spawn_blocking(move || service.prepare_sources(&day, revision, &sources)),
    )
    .await;
    drop(permit);
    match result {
        Ok(Ok(Ok(selected))) => {
            let handle = match random_id("prep_") {
                Ok(handle) => handle,
                Err(_) => {
                    fail_job(&state, &job_id, "internal-failure");
                    return;
                }
            };
            let expires = now_ms().saturating_add(JOB_EXPIRY_MS);
            let mut jobs = lock_job_transitions(&state);
            if jobs
                .jobs
                .get(&job_id)
                .is_some_and(|job| job.status == job_status::CANCELLATION_REQUESTED)
            {
                if let Some(job) = jobs.jobs.get_mut(&job_id) {
                    job.status = job_status::CANCELLED;
                    job.expires_at_ms = now_ms().saturating_add(JOB_EXPIRY_MS);
                }
                drop(jobs);
                log_debug(
                    &state,
                    DiagnosticEvent::Job {
                        kind: "source-preparation",
                        event: "cancelled",
                        reason: None,
                    },
                );
                return;
            }
            jobs.preparations.insert(
                handle.clone(),
                PreparedRecord {
                    workflow_id: workflow_id.clone(),
                    workflow_revision: revision,
                    source_ids,
                    selected,
                    expires_at_ms: expires,
                },
            );
            if let Some(job) = jobs.jobs.get_mut(&job_id) {
                job.status = job_status::COMPLETED;
                job.progress = 100;
                job.result_handle = Some(handle);
                job.expires_at_ms = expires;
            }
            drop(jobs);
            log_debug(&state, DiagnosticEvent::Preparation { event: "issued" });
            log_debug(
                &state,
                DiagnosticEvent::Job {
                    kind: "source-preparation",
                    event: "completed",
                    reason: None,
                },
            );
            publish_event(
                &state,
                "job.changed",
                Some(workflow_id),
                Some(revision),
                Some(job_id),
                Some(100),
            );
        }
        Ok(Ok(Err(error))) => fail_job(&state, &job_id, error.diagnostic_code()),
        // The blocking task panicked or was aborted: not a timeout.
        Ok(Err(_)) => fail_job(&state, &job_id, "internal-failure"),
        Err(_) => fail_job(&state, &job_id, "job-timeout"),
    }
}

pub(super) fn mark_running(state: &AppState, id: &str) -> bool {
    let kind = {
        let mut jobs = lock_job_transitions(state);
        let Some(job) = jobs.jobs.get_mut(id) else {
            return false;
        };
        if job.status != job_status::QUEUED {
            return false;
        }
        job.status = job_status::RUNNING;
        job.progress = 1;
        job.kind
    };
    log_debug(
        state,
        DiagnosticEvent::Job {
            kind,
            event: "running",
            reason: None,
        },
    );
    true
}

pub(super) fn job_commit_state(state: &AppState, id: &str) -> Option<Arc<AtomicU8>> {
    state
        .0
        .jobs
        .lock()
        .ok()?
        .jobs
        .get(id)
        .map(|job| Arc::clone(&job.commit_state))
}

pub(super) fn finish_job(
    state: &AppState,
    id: &str,
    handle: Option<String>,
    error: Option<&'static str>,
) {
    let outcome = {
        let mut jobs = lock_job_transitions(state);
        let Some(job) = jobs.jobs.get_mut(id) else {
            return;
        };
        if job.status == job_status::CANCELLATION_REQUESTED {
            job.status = job_status::CANCELLED;
            job.result_handle = None;
            job.expires_at_ms = now_ms().saturating_add(JOB_EXPIRY_MS);
            (job.kind, "cancelled")
        } else {
            let event = if error.is_some() {
                job_status::FAILED
            } else {
                job_status::COMPLETED
            };
            job.status = event;
            job.progress = 100;
            job.result_handle = handle;
            job.error_code = error.map(str::to_owned);
            job.expires_at_ms = now_ms().saturating_add(JOB_EXPIRY_MS);
            (job.kind, event)
        }
    };
    log_debug(
        state,
        DiagnosticEvent::Job {
            kind: outcome.0,
            event: outcome.1,
            reason: (outcome.1 == "failed").then_some(error).flatten(),
        },
    );
}

pub(super) fn fail_job(state: &AppState, id: &str, code: &'static str) {
    finish_job(state, id, None, Some(code));
}

/// Locks the job table for a lifecycle transition, recovering a poisoned
/// guard. Giving up instead would strand the job `queued`/`running`, states
/// `cleanup_job_state` never evicts, so it would hold capacity forever.
pub(super) fn lock_job_transitions(state: &AppState) -> std::sync::MutexGuard<'_, JobState> {
    state
        .0
        .jobs
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub(super) fn cleanup_jobs(state: &AppState) {
    let now = now_ms();
    let Ok(mut jobs) = state.0.jobs.lock() else {
        return;
    };
    cleanup_job_state(&mut jobs, now);
}

pub(super) fn cleanup_job_state(jobs: &mut JobState, now: i64) {
    let expired_handles: Vec<_> = jobs
        .preparations
        .iter()
        .filter(|(_, prepared)| prepared.expires_at_ms <= now)
        .map(|(handle, _)| handle.clone())
        .collect();
    for handle in expired_handles {
        jobs.preparations.remove(&handle);
    }
    jobs.jobs
        .retain(|_, job| job.expires_at_ms > now || job_status::is_active(job.status));
}

pub(super) fn take_preparation(
    state: &AppState,
    handle: &str,
    workflow_id: &str,
    revision: u64,
    source_ids: &[String],
) -> Result<Vec<SelectedSource>, ApiError> {
    cleanup_jobs(state);
    let mut jobs = state.0.jobs.lock().map_err(|_| ApiError::internal())?;
    let Some(prepared) = jobs.preparations.remove(handle) else {
        drop(jobs);
        log_debug(state, DiagnosticEvent::Preparation { event: "expired" });
        return Err(ApiError::new(
            StatusCode::PRECONDITION_FAILED,
            "preparation-expired",
        ));
    };
    drop(jobs);
    if prepared.workflow_id != workflow_id
        || prepared.workflow_revision != revision
        || prepared.source_ids != source_ids
    {
        log_debug(
            state,
            DiagnosticEvent::Preparation {
                event: "binding-mismatch",
            },
        );
        return Err(ApiError::new(
            StatusCode::PRECONDITION_FAILED,
            "preparation-binding-changed",
        ));
    }
    log_debug(state, DiagnosticEvent::Preparation { event: "consumed" });
    Ok(prepared.selected)
}

pub(super) fn prepared_source(
    state: &AppState,
    handle: &str,
    workflow_id: &str,
    source_id: &str,
) -> Result<SelectedSource, ApiError> {
    cleanup_jobs(state);
    let jobs = state.0.jobs.lock().map_err(|_| ApiError::internal())?;
    let Some(prepared) = jobs.preparations.get(handle) else {
        log_debug(state, DiagnosticEvent::Preparation { event: "expired" });
        return Err(ApiError::new(
            StatusCode::PRECONDITION_FAILED,
            "preparation-expired",
        ));
    };
    if prepared.workflow_id != workflow_id {
        log_debug(
            state,
            DiagnosticEvent::Preparation {
                event: "binding-mismatch",
            },
        );
        return Err(ApiError::new(
            StatusCode::PRECONDITION_FAILED,
            "preparation-binding-changed",
        ));
    }
    let Some(position) = prepared
        .source_ids
        .iter()
        .position(|prepared_id| prepared_id == source_id)
    else {
        log_debug(
            state,
            DiagnosticEvent::Preparation {
                event: "binding-mismatch",
            },
        );
        return Err(ApiError::new(
            StatusCode::PRECONDITION_FAILED,
            "preparation-binding-changed",
        ));
    };
    prepared
        .selected
        .get(position)
        .cloned()
        .ok_or_else(ApiError::internal)
}
