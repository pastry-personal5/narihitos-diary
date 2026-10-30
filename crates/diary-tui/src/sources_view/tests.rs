use std::cell::RefCell;

use diary_control_contract::{JobAcceptedDto, JobDto};

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

fn candidate(id: &str, name: &str) -> SourceCandidateDto {
    SourceCandidateDto {
        id: id.to_owned(),
        source_note_name: name.to_owned(),
        local_time: None,
        directory_day: Some("2026-10-29".to_owned()),
        header_day: Some("2026-10-29".to_owned()),
        warning_code: None,
        availability: "local".to_owned(),
        selected: false,
    }
}

fn workflow_with_candidates(candidates: Vec<SourceCandidateDto>) -> WorkflowDto {
    serde_json::from_value(serde_json::json!({
        "id": "wf_20261029",
        "diary_day": "2026-10-29",
        "state": "discovered",
        "revision": 2,
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
        "candidates": candidates
    }))
    .unwrap()
}

#[test]
fn toggle_included_ignores_an_unknown_source_id() {
    let mut view = SourcesViewModel::default();
    assert!(!view.toggle_included("unknown"));
    assert!(view.included.is_empty());
}

#[test]
fn toggle_included_flips_membership_for_a_known_candidate() {
    let mut view = SourcesViewModel {
        candidates: vec![candidate("src_1", "2026-10-28 note")],
        ..SourcesViewModel::default()
    };
    assert!(view.toggle_included("src_1"));
    assert!(view.included.contains("src_1"));
    assert!(view.toggle_included("src_1"));
    assert!(!view.included.contains("src_1"));
}

#[test]
fn loading_candidates_drops_an_included_id_no_longer_present() {
    let mut view = SourcesViewModel {
        candidates: vec![candidate("src_1", "old note")],
        included: BTreeSet::from(["src_1".to_owned(), "src_stale".to_owned()]),
        ..SourcesViewModel::default()
    };
    view.load_candidates_from(&workflow_with_candidates(vec![candidate(
        "src_1", "old note",
    )]));
    assert!(view.included.contains("src_1"));
    assert!(!view.included.contains("src_stale"));
}

#[test]
fn resources_reflects_prospective_inclusion_independent_of_server_selected_flag() {
    let mut view = SourcesViewModel {
        candidates: vec![candidate("src_1", "note a"), candidate("src_2", "note b")],
        ..SourcesViewModel::default()
    };
    view.toggle_included("src_1");
    let rows = view.resources();
    assert_eq!(rows.rows[0].state, "included · local");
    assert_eq!(rows.rows[1].state, "excluded · local");
}

#[test]
fn changing_inclusion_invalidates_the_preparation_but_keeps_the_shown_text() {
    let mut view = SourcesViewModel {
        candidates: vec![candidate("src_1", "note a")],
        latest_preparation_handle: Some("prep_1".to_owned()),
        active_detail: Some(detail("src_1")),
        ..SourcesViewModel::default()
    };
    view.toggle_included("src_1");
    assert_eq!(view.latest_preparation_handle, None);
    assert!(view.active_detail.is_some());
}

#[test]
fn succeeded_preparation_uses_only_the_jobs_typed_result_handle() {
    let mut view = SourcesViewModel {
        pending_preparation_job: Some("job_1".to_owned()),
        included: BTreeSet::from(["src_1".to_owned()]),
        ..SourcesViewModel::default()
    };
    let outcome = view.apply_job_status(ClientOutcome::Ok(JobDto {
        id: "job_1".to_owned(),
        kind: "source-preparation".to_owned(),
        status: "completed".to_owned(),
        workflow_id: "wf_20261029".to_owned(),
        progress_percent: 100,
        result_handle: Some("prep_1".to_owned()),
        error_code: None,
        expires_at_unix_ms: 0,
    }));
    assert_eq!(outcome, Some(JobOutcome::Succeeded));
    assert_eq!(view.latest_preparation_handle.as_deref(), Some("prep_1"));
    assert_eq!(view.pending_preparation_job, None);
}

#[test]
fn preview_job_acceptance_never_touches_bulk_preparation_bookkeeping() {
    let mut view = SourcesViewModel {
        pending_preparation_job: Some("bulk_job".to_owned()),
        latest_preparation_handle: Some("bulk_handle".to_owned()),
        ..SourcesViewModel::default()
    };
    view.preview_source_id = Some("src_other".to_owned());
    view.apply_job_accepted(
        SourceJobKind::Preview,
        ClientOutcome::Ok(JobAcceptedDto {
            job_id: "preview_job".to_owned(),
        }),
    );
    assert_eq!(view.pending_preview_job.as_deref(), Some("preview_job"));
    assert_eq!(view.pending_preparation_job.as_deref(), Some("bulk_job"));
    assert_eq!(
        view.latest_preparation_handle.as_deref(),
        Some("bulk_handle")
    );
}

#[test]
fn succeeded_preview_job_sets_only_the_preview_handle() {
    let mut view = SourcesViewModel {
        pending_preview_job: Some("preview_job".to_owned()),
        preview_source_id: Some("src_1".to_owned()),
        latest_preparation_handle: Some("bulk_handle".to_owned()),
        ..SourcesViewModel::default()
    };
    let outcome = view.apply_job_status(ClientOutcome::Ok(JobDto {
        id: "preview_job".to_owned(),
        kind: "source-preparation".to_owned(),
        status: "completed".to_owned(),
        workflow_id: "wf_20261029".to_owned(),
        progress_percent: 100,
        result_handle: Some("preview_handle_1".to_owned()),
        error_code: None,
        expires_at_unix_ms: 0,
    }));
    assert_eq!(outcome, Some(JobOutcome::Succeeded));
    assert_eq!(view.pending_preview_job, None);
    assert_eq!(view.preview_handle.as_deref(), Some("preview_handle_1"));
    assert_eq!(view.preview_source_id.as_deref(), Some("src_1"));
    // The bulk confirm-selection handle is untouched.
    assert_eq!(
        view.latest_preparation_handle.as_deref(),
        Some("bulk_handle")
    );
}

#[test]
fn previewing_a_candidate_shows_text_without_touching_the_bulk_handle() {
    let mut view = SourcesViewModel {
        preview_source_id: Some("src_1".to_owned()),
        ..SourcesViewModel::default()
    };
    view.apply_preview_detail(ClientOutcome::Ok(SourceDetailResponse {
        detail: diary_control_contract::SourceDetailDto {
            id: "src_1".to_owned(),
            source_note_name: "2026-10-28 note".to_owned(),
            local_time: None,
            body: "synthetic body".to_owned(),
            content_digest: "digest".to_owned(),
            directory_day: Some("2026-10-28".to_owned()),
            header_day: Some("2026-10-28".to_owned()),
            warning_code: None,
        },
        preparation_handle: None,
    }));
    assert_eq!(
        view.active_detail
            .as_ref()
            .map(|detail| detail.body.as_str()),
        Some("synthetic body")
    );
    assert_eq!(view.latest_preparation_handle, None);
    assert_eq!(view.preview_source_id, None);
}

#[test]
fn active_candidate_index_follows_the_displayed_source_across_reordering() {
    let mut view = SourcesViewModel {
        candidates: vec![candidate("src_1", "first"), candidate("src_2", "second")],
        active_detail: Some(diary_control_contract::SourceDetailDto {
            id: "src_1".to_owned(),
            source_note_name: "src_1".to_owned(),
            local_time: None,
            body: "body".to_owned(),
            content_digest: "digest".to_owned(),
            directory_day: None,
            header_day: None,
            warning_code: None,
        }),
        ..SourcesViewModel::default()
    };
    assert_eq!(view.active_candidate_index(), Some(0));

    view.candidates.swap(0, 1);

    assert_eq!(view.active_candidate_index(), Some(1));
}

fn detail(id: &str) -> diary_control_contract::SourceDetailDto {
    diary_control_contract::SourceDetailDto {
        id: id.to_owned(),
        source_note_name: id.to_owned(),
        local_time: None,
        body: "body".to_owned(),
        content_digest: "digest".to_owned(),
        directory_day: None,
        header_day: None,
        warning_code: None,
    }
}

fn resident(id: &str) -> SourceCandidateDto {
    SourceCandidateDto {
        availability: "resident".to_owned(),
        ..candidate(id, id)
    }
}

fn view_with(ids: &[&str]) -> SourcesViewModel {
    SourcesViewModel {
        candidates: ids.iter().map(|id| resident(id)).collect(),
        ..SourcesViewModel::default()
    }
}

#[test]
fn hide_excludes_removes_from_the_list_and_invalidates_preparation() {
    let mut view = view_with(&["a", "b", "c"]);
    view.toggle_included("b");
    view.latest_preparation_handle = Some("prep".to_owned());
    view.active_detail = Some(detail("b"));
    assert!(view.hide("b"));
    assert!(!view.included.contains("b"));
    assert_eq!(view.latest_preparation_handle, None);
    assert!(view.active_detail.is_none());
    assert_eq!(view.visible_len(), 2);
    assert_eq!(view.hidden_count(), 1);
    assert_eq!(view.candidate_at(1).map(|c| c.id.as_str()), Some("c"));
    assert_eq!(view.resources().rows.len(), 2);
    assert!(!view.hide("b"));
    assert!(!view.hide("unknown"));
}

#[test]
fn hide_closes_suggestion_and_excludes_note_from_confirmation() {
    let mut view = view_with(&["a", "b"]);
    view.toggle_included("a");
    view.toggle_included("b");
    view.active_refine_suggestion = Some(SourceRefineSuggestionDto {
        source_id: "a".to_owned(),
        original_body: "before".to_owned(),
        suggested_body: "after".to_owned(),
        binding_digest: "digest".to_owned(),
    });
    assert!(view.hide("a"));
    assert!(view.active_refine_suggestion.is_none());
    assert_eq!(view.begin_confirm(), Ok(vec!["b".to_owned()]));
}

#[test]
fn show_hidden_restores_notes_excluded() {
    let mut view = view_with(&["a", "b"]);
    view.toggle_included("a");
    view.hide("a");
    view.show_hidden();
    assert_eq!(view.visible_len(), 2);
    assert!(!view.included.contains("a"));
}

#[test]
fn hiding_survives_a_reload_but_a_server_selected_hidden_note_stays_excluded() {
    let mut view = view_with(&["a", "b"]);
    view.load_candidates_from(&workflow_with_candidates(vec![
        resident("a"),
        resident("b"),
    ]));
    view.hide("a");
    let mut selected = resident("a");
    selected.selected = true;
    view.load_candidates_from(&workflow_with_candidates(vec![selected, resident("b")]));
    assert_eq!(view.hidden_count(), 1);
    assert!(!view.included.contains("a"));
}

#[test]
fn a_different_workflow_clears_hidden_notes() {
    let mut view = view_with(&["a"]);
    view.load_candidates_from(&workflow_with_candidates(vec![resident("a")]));
    view.hide("a");
    let mut other = workflow_with_candidates(vec![resident("a")]);
    other.id = "wf_20261030".to_owned();
    view.load_candidates_from(&other);
    assert_eq!(view.hidden_count(), 0);
}

#[test]
fn a_preview_finishing_after_the_note_was_hidden_is_not_shown() {
    let mut view = view_with(&["a"]);
    view.hide("a");
    view.preview_source_id = Some("a".to_owned());
    view.apply_preview_detail(ClientOutcome::Ok(SourceDetailResponse {
        detail: detail("a"),
        preparation_handle: None,
    }));
    assert!(view.active_detail.is_none());
}

#[test]
fn moving_focus_closes_text_and_suggestion_of_another_note() {
    let mut view = view_with(&["a", "b"]);
    view.active_detail = Some(detail("a"));
    view.close_unfocused_detail(Some("a"));
    assert!(view.active_detail.is_some());
    view.close_unfocused_detail(Some("b"));
    assert!(view.active_detail.is_none());
}

#[test]
fn menu_state_tracks_the_focused_visible_row() {
    let mut view = view_with(&["a", "b"]);
    view.toggle_included("b");
    view.active_detail = Some(detail("b"));
    let state = view.menu_state(1, true);
    assert!(state.candidate_focused && state.focused_readable);
    assert!(state.focused_included && state.focused_shown);
    assert!(!view.menu_state(0, true).focused_shown);
    assert!(!view.menu_state(5, true).candidate_focused);
    assert_eq!(state.included_count, 1);
    let cloud_denied = SourcesViewModel {
        candidates: vec![SourceCandidateDto {
            availability: "permission-denied".to_owned(),
            ..candidate("p", "p")
        }],
        ..SourcesViewModel::default()
    };
    assert!(!cloud_denied.menu_state(0, false).focused_readable);
}

#[test]
fn confirm_uses_one_ordered_list_and_a_handle_that_cannot_be_reused() {
    let mut view = view_with(&["b", "a"]);
    view.toggle_included("b");
    view.toggle_included("a");
    assert_eq!(
        view.begin_confirm(),
        Ok(vec!["a".to_owned(), "b".to_owned()])
    );
    assert!(view.confirming());
    view.latest_preparation_handle = Some("prep_1".to_owned());
    assert_eq!(
        view.take_confirm_request(),
        Some(("prep_1".to_owned(), vec!["a".to_owned(), "b".to_owned()]))
    );
    assert_eq!(view.take_confirm_request(), None);
    view.finish_confirm();
    assert!(!view.confirming());
}

#[test]
fn confirm_refuses_nothing_included_and_more_than_the_limit() {
    let mut view = view_with(&["a"]);
    assert_eq!(view.begin_confirm(), Err(ConfirmBlock::NothingIncluded));
    let ids: Vec<String> = (0..=diary_control_contract::MAX_SELECTED_SOURCES)
        .map(|index| format!("src_{index:03}"))
        .collect();
    let mut many = SourcesViewModel {
        candidates: ids.iter().map(|id| resident(id)).collect(),
        ..SourcesViewModel::default()
    };
    for id in &ids {
        many.toggle_included(id);
    }
    assert_eq!(many.begin_confirm(), Err(ConfirmBlock::TooMany));
    assert!(!many.confirming());
}

#[test]
fn a_failed_or_lost_preparation_ends_the_confirm_sequence() {
    let mut view = view_with(&["a"]);
    view.toggle_included("a");
    view.begin_confirm().unwrap();
    view.apply_job_accepted(
        SourceJobKind::Preparation,
        ClientOutcome::Transport(TransportError::ConnectFailed),
    );
    assert!(!view.confirming());
    view.begin_confirm().unwrap();
    view.pending_preparation_job = Some("job".to_owned());
    view.forget_job("job");
    assert!(!view.confirming());
}

#[test]
fn start_discovery_records_the_accepted_job_id() {
    let transport = FakeTransport::new();
    transport.push(
        "POST",
        "/v1/workflows/wf_20261029/discoveries",
        RawResponse {
            status: 202,
            body: serde_json::to_vec(&JobAcceptedDto {
                job_id: "job_1".to_owned(),
            })
            .unwrap(),
            ..RawResponse::default()
        },
    );
    let client = ApiClient::new(transport);
    let mut view = SourcesViewModel::default();
    view.start_discovery(&client, &attachment(), "wf_20261029", 1, None);
    assert_eq!(view.pending_discovery_job.as_deref(), Some("job_1"));
    assert_eq!(view.last_error, None);
}

#[test]
fn poll_job_maps_server_statuses_to_outcomes() {
    for (status, expected) in [
        ("completed", JobOutcome::Succeeded),
        ("failed", JobOutcome::Failed),
        ("cancelled", JobOutcome::Failed),
        ("queued", JobOutcome::Running),
        ("running", JobOutcome::Running),
        ("cancellation-requested", JobOutcome::Running),
    ] {
        let transport = FakeTransport::new();
        transport.push(
            "GET",
            "/v1/jobs/job_1",
            RawResponse {
                status: 200,
                body: serde_json::to_vec(&JobDto {
                    id: "job_1".to_owned(),
                    kind: "discovery".to_owned(),
                    status: status.to_owned(),
                    workflow_id: "wf_20261029".to_owned(),
                    progress_percent: 0,
                    result_handle: None,
                    error_code: None,
                    expires_at_unix_ms: 0,
                })
                .unwrap(),
                ..RawResponse::default()
            },
        );
        let client = ApiClient::new(transport);
        let mut view = SourcesViewModel::default();
        assert_eq!(
            view.poll_job(&client, &attachment(), "job_1"),
            Some(expected)
        );
    }
}

#[test]
fn a_job_the_server_forgot_stops_being_pending_without_touching_others() {
    let mut view = SourcesViewModel {
        pending_discovery_job: Some("discovery_job".to_owned()),
        pending_preview_job: Some("preview_job".to_owned()),
        preview_source_id: Some("src_1".to_owned()),
        latest_preparation_handle: Some("bulk_handle".to_owned()),
        job_progress_percent: Some(40),
        ..SourcesViewModel::default()
    };
    assert!(view.is_pending_job("preview_job"));
    assert!(!view.is_pending_job("unknown_job"));

    view.forget_job("preview_job");

    assert!(!view.is_pending_job("preview_job"));
    assert_eq!(view.preview_source_id, None);
    assert_eq!(view.job_progress_percent, None);
    assert!(view.is_pending_job("discovery_job"));
    assert_eq!(
        view.latest_preparation_handle.as_deref(),
        Some("bulk_handle")
    );
}

#[test]
fn prepared_detail_retains_the_server_accepted_job_handle() {
    let transport = FakeTransport::new();
    transport.push(
        "GET",
        "/v1/workflows/wf_20261029/sources/src_1",
        RawResponse {
            status: 200,
            body: serde_json::json!({
                "id": "src_1",
                "source_note_name": "2026-10-28 note",
                "body": "synthetic body",
                "content_digest": "digest",
                "directory_day": "2026-10-28",
                "header_day": "2026-10-28",
                "warning_code": null
            })
            .to_string()
            .into_bytes(),
            ..RawResponse::default()
        },
    );
    let client = ApiClient::new(transport);
    let mut view = SourcesViewModel::default();
    let handle =
        view.fetch_preparation_handle(&client, &attachment(), "wf_20261029", "src_1", "handle_abc");
    assert_eq!(handle.as_deref(), Some("handle_abc"));
    assert_eq!(
        view.latest_preparation_handle.as_deref(),
        Some("handle_abc")
    );
}

#[test]
fn confirm_empty_selection_is_the_only_zero_note_continuation() {
    let transport = FakeTransport::new();
    transport.push(
        "POST",
        "/v1/workflows/wf_20261029/empty-selection-confirmations",
        RawResponse {
            status: 200,
            body: workflow_body_bytes("sources-confirmed"),
            ..RawResponse::default()
        },
    );
    let client = ApiClient::new(transport);
    let mut view = SourcesViewModel::default();
    let workflow = view.confirm_empty_selection(&client, &attachment(), "wf_20261029", 1);
    assert_eq!(workflow.unwrap().state, "sources-confirmed");
}

#[test]
fn confirm_source_list_confirms_with_no_note_text_ever_opened() {
    // Only the selection PUT is registered: any source-detail GET (a note
    // being opened) would hit the fake transport's "unexpected request".
    let transport = FakeTransport::new();
    transport.push(
        "PUT",
        "/v1/workflows/wf_20261029/selection",
        RawResponse {
            status: 200,
            body: workflow_body_bytes("sources-confirmed"),
            ..RawResponse::default()
        },
    );
    let client = ApiClient::new(transport);
    let mut view = view_with(&["src_a", "src_b"]);
    view.toggle_included("src_a");
    view.toggle_included("src_b");
    view.begin_confirm().unwrap();
    view.pending_preparation_job = Some("job_1".to_owned());
    let outcome = view.apply_job_status(ClientOutcome::Ok(JobDto {
        id: "job_1".to_owned(),
        kind: "source-preparation".to_owned(),
        status: "completed".to_owned(),
        workflow_id: "wf_20261029".to_owned(),
        progress_percent: 100,
        result_handle: Some("prep_1".to_owned()),
        error_code: None,
        expires_at_unix_ms: 0,
    }));
    assert_eq!(outcome, Some(JobOutcome::Succeeded));
    assert!(view.active_detail.is_none());
    let (handle, ids) = view.take_confirm_request().unwrap();
    let workflow = view.confirm_selection(&client, &attachment(), "wf_20261029", 2, &handle);
    assert_eq!(workflow.unwrap().state, "sources-confirmed");
    assert_eq!(ids, view.ordered_included());
}

fn workflow_body_bytes(state: &str) -> Vec<u8> {
    serde_json::json!({
        "id": "wf_20261029",
        "diary_day": "2026-10-29",
        "state": state,
        "revision": 2,
        "buffer_version": 0,
        "language": null,
        "style": "concise",
        "assistant_backend_id": null,
        "composition_path": null,
        "selection_digest": null,
        "explicit_empty_selection": true,
        "invalidation_code": null,
        "entry_text": "",
        "entry_confirmed": false,
        "candidates": []
    })
    .to_string()
    .into_bytes()
}

#[test]
fn source_label_prefers_local_time_and_omits_the_extension() {
    let mut row = candidate("src_1", "2026/10/29/2026-10-29_15-30-00-123.md");
    assert_eq!(source_label(&row), "2026-10-29_15-30-00-123");
    row.local_time = Some("2026-10-30 00:30:00".to_owned());
    assert_eq!(source_label(&row), "2026-10-30 00:30:00");
}
