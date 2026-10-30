//! Session focus, source, polling, and scrolling helpers.

use super::*;

pub fn should_auto_discover(
    workflow: &WorkflowDto,
    can_mutate: bool,
    can_read_sources: bool,
    source_job_busy: bool,
    attempted: &BTreeSet<String>,
) -> bool {
    can_mutate
        && can_read_sources
        && !source_job_busy
        && workflow.state == "started"
        && !attempted.contains(&workflow.id)
}

/// Requests the body of a candidate for display, regardless of whether it's
/// included: activating a source and including it are independent actions
/// (product-behavior.md, ux-tui.md). Every view is an ad hoc, single-source
/// preview that never touches the preparation Confirm Source List performs.
pub fn request_inspect_source(
    worker: &Worker,
    sources_view: &mut SourcesViewModel,
    current_workflow: &Option<WorkflowDto>,
    resource_index: usize,
    pending_source_job_kind: &mut Option<SourceJobKind>,
) {
    let Some(workflow) = current_workflow.as_ref() else {
        return;
    };
    let Some(candidate) = sources_view.candidate_at(resource_index) else {
        return;
    };
    let source_id = candidate.id.clone();
    sources_view.preview_source_id = Some(source_id.clone());
    *pending_source_job_kind = Some(SourceJobKind::Preview);
    if worker
        .try_send(Intent::PrepareSources {
            workflow_id: workflow.id.clone(),
            revision: workflow.revision,
            source_ids: vec![source_id],
        })
        .is_err()
    {
        sources_view.preview_source_id = None;
        *pending_source_job_kind = None;
    }
}

pub const START_CONTROL_COUNT: usize = 9;
pub const COMPOSE_CONTROL_COUNT: usize = 18;
pub const REVIEW_CONTROL_COUNT: usize = 4;

pub fn focus_item_count(
    focus: &FocusModel,
    resource_count: usize,
    control_count: usize,
    utility_count: usize,
) -> usize {
    match focus.zone {
        FocusZone::Workflow => ALL_STAGES.len(),
        FocusZone::Resources => resource_count,
        FocusZone::Main => control_count,
        FocusZone::Utility => utility_count,
    }
}

pub fn focused_source_id(
    stage: Stage,
    focus: &FocusModel,
    sources: &SourcesViewModel,
) -> Option<String> {
    (stage == Stage::Sources)
        .then(|| sources.candidate_at(focus.resource_index))
        .flatten()
        .map(|candidate| candidate.id.clone())
}

/// The Resources row focused on entering `stage`: the source being shown in
/// Sources, the active draft resource in Compose, and the top row otherwise.
pub fn stage_entry_resource_index(
    stage: Stage,
    sources: &SourcesViewModel,
    compose_active_resource: usize,
) -> usize {
    match stage {
        Stage::Sources => sources.active_candidate_index().unwrap_or(0),
        Stage::Compose => compose_active_resource,
        _ => 0,
    }
}

/// Picks the one job to poll next. Recovery reads come first because
/// mutations stay disabled until they land; source and assistant jobs then
/// alternate so one long-running job cannot starve the other's polling.
pub fn next_job_poll<'a>(
    recovery: Option<&'a str>,
    source: Option<&'a str>,
    assistant: Option<&'a str>,
    prefer_assistant: bool,
) -> Option<&'a str> {
    recovery.or(if prefer_assistant {
        assistant.or(source)
    } else {
        source.or(assistant)
    })
}

/// Keeps the Resources Sidebar cursor on the same source after a canonical
/// reload or page response replaces the candidate vector. If that source was
/// removed, or another stage is shown (no focused source), focus is left
/// alone: the per-frame clamp bounds it with the current stage's counts,
/// whereas clamping here with Sources' counts would move Compose's focus.
pub fn reanchor_source_focus(
    focus: &mut FocusModel,
    sources: &SourcesViewModel,
    previous_source_id: Option<&str>,
) {
    if let Some(index) =
        previous_source_id.and_then(|source_id| sources.visible_position(source_id))
    {
        focus.resource_index = index;
    }
}

/// Picks the Main Pane's scroll offset for this frame. Opening a source's
/// text (or a refinement suggestion) resets the view to the top so freshly
/// revealed content starts predictably rather than at whatever offset a
/// previously shown source left behind. While that text is open, its own
/// scrolling governs the viewport: the focused Main-zone control's header
/// line (e.g. "Inspect active prepared source") must not keep pinning the
/// view back up and away from the body text rendered below it, which would
/// make the viewport snap back every frame the moment a person tries to
/// page down into the text.
pub fn resolve_main_scroll(
    stage: Stage,
    sources_view: &SourcesViewModel,
    body: &str,
    current: u16,
    viewport_height: u16,
    shown_source_detail_id: &mut Option<String>,
) -> u16 {
    let current_detail_id = sources_view.active_detail.as_ref().map(|detail| &detail.id);
    let mut scroll = current;
    if current_detail_id != shown_source_detail_id.as_ref() {
        *shown_source_detail_id = current_detail_id.cloned();
        scroll = 0;
    }
    let viewing_source_text = stage == Stage::Sources
        && (sources_view.active_detail.is_some()
            || sources_view.active_refine_suggestion.is_some());
    if viewing_source_text {
        scroll
    } else {
        ensure_selected_line_visible(body, scroll, viewport_height)
    }
}

pub fn ensure_selected_line_visible(body: &str, current: u16, viewport_height: u16) -> u16 {
    if viewport_height == 0 {
        return 0;
    }
    let selected = body
        .lines()
        .position(|line| line.starts_with('>'))
        .and_then(|index| u16::try_from(index).ok());
    match selected {
        Some(line) if line < current => line,
        Some(line) if line >= current.saturating_add(viewport_height) => {
            line.saturating_sub(viewport_height - 1)
        }
        _ => current,
    }
}
