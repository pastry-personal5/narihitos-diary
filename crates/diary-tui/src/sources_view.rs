//! Sources-stage ViewModel: discover, inspect, include/exclude, prepare,
//! and confirm a source selection (or the explicit empty selection). The
//! active resource and the prospective included set are independent
//! presentation state; neither is synthesized from local data, and both
//! are reset only by an explicit confirmation or a fresh discovery.
//! Confirm Source List removes the per-note "reviewed" gate: it prepares
//! the included notes and confirms them in one sequence.

use std::collections::BTreeSet;

use diary_control_contract::{
    JobAcceptedDto, JobDto, SourceCandidateDto, SourceDetailDto, SourceListDto,
    SourceRefineSuggestionDto, WorkflowDto, job_status,
};

use crate::client::{
    ApiClient, Attachment, ClientOutcome, ProblemView, SourceDetailResponse, Transport,
    TransportError,
};
use crate::resources::{ResourceRow, ResourcesSidebarModel};
use crate::sources_menu::SourcesMenuState;

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
pub enum JobOutcome {
    Running,
    Succeeded,
    Failed,
}

/// Which bookkeeping an accepted source job belongs to. `Preview` is the ad
/// hoc, single-source job used to inspect any candidate's text regardless of
/// inclusion; it never shares state with the bulk `Preparation` job that the
/// included set uses to obtain the handle `confirm_selection` requires.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceJobKind {
    Discovery,
    Preparation,
    Preview,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SourcesViewModel {
    pub candidates: Vec<SourceCandidateDto>,
    /// The prospective included source IDs. Independent of which source is
    /// the active (inspected) resource.
    pub included: BTreeSet<String>,
    /// Session-only, client-side filter (Hide). A hidden note leaves the
    /// list and is forced excluded. Never written anywhere and cleared when the
    /// open workflow changes; the Ayumi note is untouched.
    pub hidden: BTreeSet<String>,
    /// The workflow `candidates` belong to, so a different workflow resets
    /// per-workflow presentation state such as `hidden`.
    pub workflow_id: Option<String>,
    /// The ordered source IDs of a Confirm Source List in progress. The same
    /// list is used to prepare and to confirm, because the server binds the
    /// single-use handle to the exact ordered IDs.
    pub confirm_ids: Option<Vec<String>>,
    pub next_cursor: Option<String>,
    pub active_detail: Option<SourceDetailDto>,
    pub active_refine_suggestion: Option<SourceRefineSuggestionDto>,
    pub pending_delete_confirmation: Option<String>,
    pub pending_discovery_job: Option<String>,
    pub pending_preparation_job: Option<String>,
    pub job_progress_percent: Option<u8>,
    /// The completed preparation job's result handle, retained after a
    /// source-detail read succeeds with it in `X-ND-Preparation-Handle`.
    /// `confirm_selection` still takes a handle explicitly so its caller must
    /// bind confirmation to the preparation it just awaited.
    pub latest_preparation_handle: Option<String>,
    /// The ad hoc preview job's in-flight ID, independent of
    /// `pending_discovery_job`/`pending_preparation_job`. Previewing a
    /// candidate's text never touches the bulk preparation bookkeeping that
    /// `confirm_selection` depends on.
    pub pending_preview_job: Option<String>,
    /// The handle a succeeded preview job produced, scoped only to
    /// `preview_source_id`. Consumed by the follow-up source-detail fetch.
    pub preview_handle: Option<String>,
    /// The source a preview job targets or just resolved, so the caller can
    /// bind the follow-up source-detail fetch.
    pub preview_source_id: Option<String>,
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
        let source_ids = self.ordered_included();
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

    /// Polls a pending job by ID. Returns its outcome on success; a
    /// transport/typed-problem failure records `last_error` and returns
    /// `None` rather than guessing the outcome.
    pub fn poll_job<T: Transport>(
        &mut self,
        client: &ApiClient<T>,
        attachment: &Attachment,
        job_id: &str,
    ) -> Option<JobOutcome> {
        match client.job(attachment, job_id) {
            ClientOutcome::Ok(job) => {
                self.last_error = None;
                self.job_progress_percent = Some(job.progress_percent);
                Some(job_outcome(&job.status))
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
        if self.workflow_id.as_deref() != Some(workflow.id.as_str()) {
            self.workflow_id = Some(workflow.id.clone());
            self.hidden.clear();
            self.confirm_ids = None;
        }
        if matches!(
            workflow.invalidation_code.as_deref(),
            Some("selection-changed" | "source-changed" | "freshness-unverifiable")
        ) {
            self.latest_preparation_handle = None;
            self.active_detail = None;
        }
        self.candidates = workflow.candidates.clone();
        self.included.extend(
            self.candidates
                .iter()
                .filter(|candidate| candidate.selected)
                .map(|candidate| candidate.id.clone()),
        );
        self.included.retain(|id| {
            !self.hidden.contains(id) && self.candidates.iter().any(|candidate| &candidate.id == id)
        });
    }

    pub fn accept_fresh_discovery(&mut self) {
        self.candidates.clear();
        self.included.clear();
        self.next_cursor = None;
        self.active_detail = None;
        self.latest_preparation_handle = None;
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
        self.invalidate_preparation();
        true
    }

    pub fn invalidate_preparation(&mut self) {
        self.latest_preparation_handle = None;
    }

    /// The included IDs in the one order used to prepare and to confirm.
    #[must_use]
    pub fn ordered_included(&self) -> Vec<String> {
        self.included.iter().cloned().collect()
    }

    pub fn apply_job_accepted(
        &mut self,
        kind: SourceJobKind,
        outcome: ClientOutcome<JobAcceptedDto>,
    ) {
        match outcome {
            ClientOutcome::Ok(job) => {
                match kind {
                    SourceJobKind::Preparation => self.pending_preparation_job = Some(job.job_id),
                    SourceJobKind::Discovery => self.pending_discovery_job = Some(job.job_id),
                    SourceJobKind::Preview => self.pending_preview_job = Some(job.job_id),
                }
                self.last_error = None;
            }
            ClientOutcome::Problem(problem) => {
                self.last_error = Some(SourcesError::Problem(problem));
                self.abort_confirm_if(kind);
            }
            ClientOutcome::Transport(error) => {
                self.last_error = Some(SourcesError::Transport(error));
                self.abort_confirm_if(kind);
            }
        }
    }

    fn abort_confirm_if(&mut self, kind: SourceJobKind) {
        if kind == SourceJobKind::Preparation {
            self.confirm_ids = None;
        }
    }

    pub fn apply_job_status(&mut self, outcome: ClientOutcome<JobDto>) -> Option<JobOutcome> {
        match outcome {
            ClientOutcome::Ok(job) => {
                self.last_error = None;
                let outcome = job_outcome(&job.status);
                if outcome != JobOutcome::Running {
                    self.job_progress_percent = None;
                    if self.pending_preparation_job.as_deref() == Some(job.id.as_str()) {
                        self.pending_preparation_job = None;
                        if outcome == JobOutcome::Succeeded {
                            self.latest_preparation_handle = job.result_handle.clone();
                        }
                    }
                    if self.pending_discovery_job.as_deref() == Some(job.id.as_str()) {
                        self.pending_discovery_job = None;
                    }
                    if self.pending_preview_job.as_deref() == Some(job.id.as_str()) {
                        self.pending_preview_job = None;
                        if outcome == JobOutcome::Succeeded {
                            self.preview_handle = job.result_handle;
                        } else {
                            self.preview_source_id = None;
                        }
                    }
                }
                Some(outcome)
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

    /// Whether `job_id` is one of this ViewModel's in-flight source jobs.
    #[must_use]
    pub fn is_pending_job(&self, job_id: &str) -> bool {
        [
            &self.pending_discovery_job,
            &self.pending_preparation_job,
            &self.pending_preview_job,
        ]
        .into_iter()
        .any(|pending| pending.as_deref() == Some(job_id))
    }

    /// Drops a pending job the server no longer knows (`job-not-found`, e.g.
    /// its terminal result expired before it was observed). Without this the
    /// job would stay pending forever, keeping every Sources action busy and
    /// re-polling a 404 each tick. It produced no handle, so none is kept.
    pub fn forget_job(&mut self, job_id: &str) {
        let mut forgot = false;
        if self.pending_preparation_job.as_deref() == Some(job_id) {
            self.pending_preparation_job = None;
            self.confirm_ids = None;
            forgot = true;
        }
        if self.pending_discovery_job.as_deref() == Some(job_id) {
            self.pending_discovery_job = None;
            forgot = true;
        }
        if self.pending_preview_job.as_deref() == Some(job_id) {
            self.pending_preview_job = None;
            self.preview_source_id = None;
            self.preview_handle = None;
            forgot = true;
        }
        if forgot {
            self.job_progress_percent = None;
        }
    }

    pub fn apply_source_page(&mut self, append: bool, outcome: ClientOutcome<SourceListDto>) {
        match outcome {
            ClientOutcome::Ok(page) => {
                if !append {
                    self.candidates.clear();
                }
                for candidate in page.sources {
                    if let Some(existing) = self
                        .candidates
                        .iter_mut()
                        .find(|existing| existing.id == candidate.id)
                    {
                        *existing = candidate;
                    } else {
                        self.candidates.push(candidate);
                    }
                }
                self.next_cursor = page.next_cursor;
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

    pub fn apply_source_detail(&mut self, outcome: ClientOutcome<SourceDetailResponse>) {
        match outcome {
            ClientOutcome::Ok(response) => {
                self.latest_preparation_handle = response.preparation_handle;
                self.active_detail = Some(response.detail);
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

    /// Applies the source-detail fetch that follows a succeeded preview job.
    /// Shows the body like `apply_source_detail`, but never touches
    /// `latest_preparation_handle`: viewing a candidate is independent of the
    /// preparation Confirm Source List performs. A note hidden while the read
    /// was in flight is not shown.
    pub fn apply_preview_detail(&mut self, outcome: ClientOutcome<SourceDetailResponse>) {
        self.preview_source_id = None;
        match outcome {
            ClientOutcome::Ok(response) => {
                if !self.hidden.contains(&response.detail.id) {
                    self.active_detail = Some(response.detail);
                }
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

    /// Reads a prepared source body with the completed job's result handle and
    /// records that handle after the server accepts the bound read.
    /// Before confirmation this returns the prepared sanitized body;
    /// afterward it returns only the persisted confirmed snapshot (per
    /// the architecture doc) and the handle is absent.
    pub fn fetch_preparation_handle<T: Transport>(
        &mut self,
        client: &ApiClient<T>,
        attachment: &Attachment,
        workflow_id: &str,
        source_id: &str,
        preparation_handle: &str,
    ) -> Option<String> {
        match client.source_detail_with_handle(
            attachment,
            workflow_id,
            source_id,
            preparation_handle,
        ) {
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
        let source_ids = self.ordered_included();
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

    /// Renders the visible (not hidden) candidates as Resources Sidebar rows.
    /// Active-resource focus is separate presentation state the caller tracks
    /// itself; this only reflects the prospective included/excluded state.
    #[must_use]
    pub fn resources(&self) -> ResourcesSidebarModel {
        ResourcesSidebarModel {
            rows: self
                .visible_candidates()
                .map(|candidate| ResourceRow {
                    label: source_label(candidate),
                    state: source_state(candidate, self.included.contains(&candidate.id)),
                    indent: 0,
                })
                .collect(),
        }
    }

    /// The candidates the list shows: everything not hidden this session.
    pub fn visible_candidates(&self) -> impl Iterator<Item = &SourceCandidateDto> {
        self.candidates
            .iter()
            .filter(|candidate| !self.hidden.contains(&candidate.id))
    }

    #[must_use]
    pub fn visible_len(&self) -> usize {
        self.visible_candidates().count()
    }

    /// The candidate at a Resources Sidebar row (an index into the visible list).
    #[must_use]
    pub fn candidate_at(&self, visible_index: usize) -> Option<&SourceCandidateDto> {
        self.visible_candidates().nth(visible_index)
    }

    #[must_use]
    pub fn visible_position(&self, source_id: &str) -> Option<usize> {
        self.visible_candidates()
            .position(|candidate| candidate.id == source_id)
    }

    /// How many known candidates are currently hidden.
    #[must_use]
    pub fn hidden_count(&self) -> usize {
        self.candidates
            .iter()
            .filter(|candidate| self.hidden.contains(&candidate.id))
            .count()
    }

    /// Hides a candidate for this session. An included note is excluded first
    /// through the normal toggle so the preparation is invalidated exactly as
    /// for an exclusion; the shown text and suggestion of that note close.
    /// Returns `false` for an unknown or already hidden ID.
    pub fn hide(&mut self, source_id: &str) -> bool {
        if self.hidden.contains(source_id)
            || !self
                .candidates
                .iter()
                .any(|candidate| candidate.id == source_id)
        {
            return false;
        }
        if self.included.contains(source_id) {
            let _ = self.toggle_included(source_id);
        }
        self.hidden.insert(source_id.to_owned());
        if self
            .active_detail
            .as_ref()
            .is_some_and(|detail| detail.id == source_id)
        {
            self.active_detail = None;
        }
        if self
            .active_refine_suggestion
            .as_ref()
            .is_some_and(|suggestion| suggestion.source_id == source_id)
        {
            self.active_refine_suggestion = None;
        }
        true
    }

    /// Shows every hidden note again; they come back excluded.
    pub fn show_hidden(&mut self) {
        self.hidden.clear();
    }

    /// Resolves the displayed source body back to its current visible row.
    /// Candidate pages and canonical workflow reloads may replace or reorder
    /// the vector, so the active marker must follow the source ID rather than
    /// an index captured before the asynchronous read completed.
    #[must_use]
    pub fn active_candidate_index(&self) -> Option<usize> {
        let active_id = &self.active_detail.as_ref()?.id;
        self.visible_position(active_id)
    }

    /// Closes the shown text and suggestion unless they belong to the focused
    /// row, so Refine, Accept, Reject, Edit, and Delete can never act on a note
    /// other than the one on screen.
    pub fn close_unfocused_detail(&mut self, focused_id: Option<&str>) {
        if self
            .active_detail
            .as_ref()
            .is_some_and(|detail| Some(detail.id.as_str()) != focused_id)
        {
            self.active_detail = None;
        }
        if self
            .active_refine_suggestion
            .as_ref()
            .is_some_and(|suggestion| Some(suggestion.source_id.as_str()) != focused_id)
        {
            self.active_refine_suggestion = None;
        }
    }

    /// The flat menu's inputs for the row at `focused_index`.
    #[must_use]
    pub fn menu_state(
        &self,
        focused_index: usize,
        assistant_backend_selected: bool,
    ) -> SourcesMenuState {
        let focused = self.candidate_at(focused_index);
        let focused_id = focused.map(|candidate| candidate.id.as_str());
        SourcesMenuState {
            candidate_focused: focused.is_some(),
            focused_readable: focused.is_some_and(|candidate| {
                matches!(candidate.availability.as_str(), "resident" | "cloud-only")
            }),
            focused_included: focused_id.is_some_and(|id| self.included.contains(id)),
            focused_shown: focused_id.is_some()
                && self.active_detail.as_ref().map(|detail| detail.id.as_str()) == focused_id,
            hidden_count: self.hidden_count(),
            assistant_backend_selected,
            suggestion_shown: focused_id.is_some()
                && self
                    .active_refine_suggestion
                    .as_ref()
                    .map(|suggestion| suggestion.source_id.as_str())
                    == focused_id,
            has_more: self.next_cursor.is_some(),
            included_count: self.included.len(),
        }
    }

    /// Starts Confirm Source List: returns the ordered IDs to prepare, or why
    /// it cannot start. The caller sends the prepare request with exactly
    /// these IDs and later confirms with the same ones.
    pub fn begin_confirm(&mut self) -> Result<Vec<String>, ConfirmBlock> {
        if self.included.is_empty() {
            return Err(ConfirmBlock::NothingIncluded);
        }
        if self.included.len() > diary_control_contract::MAX_SELECTED_SOURCES {
            return Err(ConfirmBlock::TooMany);
        }
        self.invalidate_preparation();
        let ids = self.ordered_included();
        self.confirm_ids = Some(ids.clone());
        Ok(ids)
    }

    /// After the preparation job succeeded: the single-use handle and the same
    /// ordered IDs, taken so neither can be reused for another confirmation.
    pub fn take_confirm_request(&mut self) -> Option<(String, Vec<String>)> {
        let ids = self.confirm_ids.clone()?;
        let handle = self.latest_preparation_handle.take()?;
        Some((handle, ids))
    }

    /// Ends the sequence after a confirmation result or a failure. The handle
    /// was consumed by the server whatever the outcome, so it is dropped.
    pub fn finish_confirm(&mut self) {
        self.confirm_ids = None;
        self.latest_preparation_handle = None;
    }

    #[must_use]
    pub const fn confirming(&self) -> bool {
        self.confirm_ids.is_some()
    }
}

/// Why Confirm Source List cannot start with the current list.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfirmBlock {
    NothingIncluded,
    TooMany,
}

impl ConfirmBlock {
    #[must_use]
    pub const fn reason(self) -> &'static str {
        match self {
            Self::NothingIncluded => "No notes are included",
            Self::TooMany => "Too many included notes; exclude some (limit 64)",
        }
    }
}

/// Maps a server job status to a outcome. Anything not terminal (queued,
/// running, cancellation-requested) is still running.
fn job_outcome(status: &str) -> JobOutcome {
    match status {
        job_status::COMPLETED => JobOutcome::Succeeded,
        status if job_status::is_terminal(status) => JobOutcome::Failed,
        _ => JobOutcome::Running,
    }
}

/// Pseudo-ISO local time (`YYYY-MM-DD HH:MM:SS`) when known; otherwise the
/// file name without its directory or `.md` extension.
#[must_use]
pub fn source_label(candidate: &SourceCandidateDto) -> String {
    candidate.local_time.clone().unwrap_or_else(|| {
        let name = candidate
            .source_note_name
            .rsplit('/')
            .next()
            .unwrap_or(&candidate.source_note_name);
        name.strip_suffix(".md").unwrap_or(name).to_owned()
    })
}

fn source_state(candidate: &SourceCandidateDto, included: bool) -> String {
    let selection = if included { "included" } else { "excluded" };
    let mut state = format!("{selection} · {}", candidate.availability);
    if candidate.directory_day.is_some()
        && candidate.header_day.is_some()
        && candidate.directory_day != candidate.header_day
    {
        state.push_str(" · date disagreement");
    }
    if candidate.warning_code.is_some() {
        state.push_str(" · warning");
    }
    state
}

#[cfg(test)]
mod tests;
