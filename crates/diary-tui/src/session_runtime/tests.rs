use super::*;

#[test]
fn start_screen_exposes_every_command_equivalent_as_a_focused_control() {
    let focus = FocusModel {
        zone: FocusZone::Main,
        main_control_index: 5,
        ..FocusModel::default()
    };
    let capabilities = CapabilitiesDto {
        capabilities: vec!["read".to_owned(), "workflow:write".to_owned()],
        manual_workflow_available: true,
        assistant_backends: Vec::new(),
        disabled_routes: Vec::new(),
    };
    let setup = SetupCheckDto {
        config_safe: true,
        database_ready: true,
        apple_helper_available: false,
        notion_configured: false,
        notion_reason_code: Some("notion-not-configured".to_owned()),
        manual_workflow_available: true,
    };
    let body = start_screen_body(
        &focus,
        None,
        false,
        "",
        None,
        false,
        Some(&capabilities),
        Some(&setup),
        None,
        &AssistantViewModel::default(),
        true,
        false,
        false,
        false,
        true,
    );
    for label in [
        "Day:",
        "Start/resume chosen day",
        "Retry/refresh",
        "Help",
        "Save/flush",
        "Quit",
    ] {
        assert!(body.contains(label), "missing focused action: {label}");
    }
    assert!(body.contains("> Retry/refresh"));
    assert!(body.contains("Manual workflow: available"));
}

#[test]
fn focused_main_control_is_scrolled_into_the_visible_window() {
    let body = "header\nline\nline\nline\n> selected\nlast";
    assert_eq!(ensure_selected_line_visible(body, 0, 3), 2);
    assert_eq!(ensure_selected_line_visible(body, 4, 3), 4);
    assert_eq!(ensure_selected_line_visible(body, 5, 3), 4);
}

fn sources_view_with_detail(id: &str) -> SourcesViewModel {
    let mut view = SourcesViewModel::default();
    view.apply_source_detail(ClientOutcome::Ok(crate::client::SourceDetailResponse {
        detail: diary_control_contract::SourceDetailDto {
            id: id.to_owned(),
            source_note_name: "note".to_owned(),
            local_time: None,
            body: "synthetic body".to_owned(),
            content_digest: "digest".to_owned(),
            directory_day: None,
            header_day: None,
            warning_code: None,
        },
        preparation_handle: None,
    }));
    view
}

#[test]
fn opening_source_text_stops_the_focused_control_from_pinning_the_scroll_back() {
    // A line starting with '>' near the top, as the Sources header
    // always renders for whichever Main-zone control is focused.
    let body = "> Inspect active prepared source\nmore header\n\n--- Source text ---\nfirst body line\nsecond body line";
    let sources = sources_view_with_detail("src_1");
    let mut shown = Some("src_1".to_owned());
    // The person paged down to read the body; before the fix this would
    // snap back up toward the '>' line every frame.
    let scroll = resolve_main_scroll(Stage::Sources, &sources, body, 4, 3, &mut shown);
    assert_eq!(scroll, 4);
}

#[test]
fn opening_a_different_source_resets_the_scroll_to_the_top() {
    let sources = sources_view_with_detail("src_2");
    let mut shown = Some("src_1".to_owned());
    let scroll = resolve_main_scroll(Stage::Sources, &sources, "body", 7, 3, &mut shown);
    assert_eq!(scroll, 0);
    assert_eq!(shown, Some("src_2".to_owned()));
}

#[test]
fn canonical_source_reordering_keeps_focus_on_the_same_source() {
    let mut sources = SourcesViewModel {
        candidates: vec![
            diary_control_contract::SourceCandidateDto {
                id: "src_1".to_owned(),
                source_note_name: "first".to_owned(),
                local_time: None,
                directory_day: None,
                header_day: None,
                warning_code: None,
                availability: "local".to_owned(),
                selected: false,
            },
            diary_control_contract::SourceCandidateDto {
                id: "src_2".to_owned(),
                source_note_name: "second".to_owned(),
                local_time: None,
                directory_day: None,
                header_day: None,
                warning_code: None,
                availability: "local".to_owned(),
                selected: false,
            },
        ],
        ..SourcesViewModel::default()
    };
    let mut focus = FocusModel {
        zone: FocusZone::Resources,
        resource_index: 0,
        ..FocusModel::default()
    };
    let focused_id = focused_source_id(Stage::Sources, &focus, &sources);

    sources.candidates.swap(0, 1);
    reanchor_source_focus(&mut focus, &sources, focused_id.as_deref());

    assert_eq!(focus.resource_index, 1);
    assert_eq!(sources.candidates[focus.resource_index].id, "src_1");
}

#[test]
fn entering_a_stage_focuses_its_own_resource_not_the_previous_stages_row() {
    let shown = sources_view_with_detail("src_2");
    let mut sources = SourcesViewModel {
        candidates: ["src_1", "src_2"]
            .into_iter()
            .map(|id| diary_control_contract::SourceCandidateDto {
                id: id.to_owned(),
                source_note_name: id.to_owned(),
                local_time: None,
                directory_day: None,
                header_day: None,
                warning_code: None,
                availability: "local".to_owned(),
                selected: false,
            })
            .collect(),
        ..SourcesViewModel::default()
    };
    assert_eq!(stage_entry_resource_index(Stage::Sources, &sources, 4), 0);
    sources.active_detail = shown.active_detail;
    assert_eq!(stage_entry_resource_index(Stage::Sources, &sources, 4), 1);
    assert_eq!(stage_entry_resource_index(Stage::Compose, &sources, 4), 4);
    assert_eq!(stage_entry_resource_index(Stage::Start, &sources, 4), 0);
}

#[test]
fn job_polls_put_recovery_first_and_alternate_source_and_assistant() {
    assert_eq!(
        next_job_poll(Some("recovery"), Some("source"), Some("assistant"), true),
        Some("recovery")
    );
    assert_eq!(
        next_job_poll(None, Some("source"), Some("assistant"), false),
        Some("source")
    );
    assert_eq!(
        next_job_poll(None, Some("source"), Some("assistant"), true),
        Some("assistant")
    );
    assert_eq!(
        next_job_poll(None, Some("source"), None, true),
        Some("source")
    );
    assert_eq!(next_job_poll(None, None, None, false), None);
}

#[test]
fn a_source_reload_outside_sources_leaves_that_stages_focus_alone() {
    let sources = SourcesViewModel::default();
    let mut focus = FocusModel {
        zone: FocusZone::Main,
        resource_index: 3,
        main_control_index: 17,
        ..FocusModel::default()
    };
    let focused_id = focused_source_id(Stage::Compose, &focus, &sources);

    reanchor_source_focus(&mut focus, &sources, focused_id.as_deref());

    assert_eq!(focus.resource_index, 3);
    assert_eq!(focus.main_control_index, 17);
}

#[test]
fn sources_screen_keeps_inclusion_separate_from_the_active_read_only_body() {
    let mut sources = SourcesViewModel {
        candidates: vec![diary_control_contract::SourceCandidateDto {
            id: "src_1".to_owned(),
            source_note_name: "2026/10/29/note.md".to_owned(),
            local_time: None,
            directory_day: Some("2026-10-29".to_owned()),
            header_day: Some("2026-10-28".to_owned()),
            warning_code: Some("source-warning".to_owned()),
            availability: "resident".to_owned(),
            selected: false,
        }],
        ..SourcesViewModel::default()
    };
    let _ = sources.toggle_included("src_1");
    sources.apply_source_detail(ClientOutcome::Ok(crate::client::SourceDetailResponse {
        detail: diary_control_contract::SourceDetailDto {
            id: "src_1".to_owned(),
            source_note_name: "2026/10/29/note.md".to_owned(),
            local_time: None,
            body: "synthetic source text".to_owned(),
            content_digest: "digest".to_owned(),
            directory_day: Some("2026-10-29".to_owned()),
            header_day: Some("2026-10-28".to_owned()),
            warning_code: Some("source-warning".to_owned()),
        },
        preparation_handle: Some("prep_1".to_owned()),
    }));
    let state = sources.menu_state(0, true);
    let items = visible_items(&state);
    let body = sources_screen_body(
        &FocusModel::default(),
        &sources,
        &state,
        &items,
        None,
        false,
        "",
        false,
        false,
        false,
    );
    assert!(body.contains("Directory date: 2026-10-29"));
    assert!(body.contains("Header date: 2026-10-28"));
    assert!(body.contains("Included: 1 of 1"));
    assert!(body.contains("synthetic source text"));
    assert!(!body.contains("reviewed"));
    for action in [
        "Exclude this note",
        "Refine with assistant",
        "Edit note",
        "Delete note and linked audio",
        "Confirm Source List",
        "Refresh notes",
        "Shortcuts help",
    ] {
        assert!(body.contains(action), "missing Sources action: {action}");
    }
    assert!(!body.contains("Accept refinement"));
    assert!(sources.included.contains("src_1"));
}

#[test]
fn sources_screen_marks_the_focused_menu_item_and_counts_hidden_notes() {
    let mut sources = SourcesViewModel {
        candidates: vec![diary_control_contract::SourceCandidateDto {
            id: "src_1".to_owned(),
            source_note_name: "note.md".to_owned(),
            local_time: None,
            directory_day: None,
            header_day: None,
            warning_code: None,
            availability: "resident".to_owned(),
            selected: false,
        }],
        ..SourcesViewModel::default()
    };
    sources.hide("src_1");
    let state = sources.menu_state(0, false);
    let items = visible_items(&state);
    let focus = FocusModel {
        zone: FocusZone::Main,
        main_control_index: 0,
        ..FocusModel::default()
    };
    let body = sources_screen_body(
        &focus,
        &sources,
        &state,
        &items,
        Some("2026-10-27"),
        false,
        "",
        false,
        false,
        false,
    );
    assert!(body.contains("1 hidden (u: show)"));
    assert!(body.contains("Extra date: 2026-10-27"));
    assert!(body.contains("> Show hidden notes (1)"));
    assert!(body.contains("Continue without notes"));
}

fn composing_workflow() -> WorkflowDto {
    WorkflowDto {
        id: "wf_20261029".to_owned(),
        diary_day: "2026-10-29".to_owned(),
        state: "composing".to_owned(),
        revision: 4,
        buffer_version: 2,
        language: Some("en-US".to_owned()),
        style: "concise".to_owned(),
        assistant_backend_id: None,
        composition_path: None,
        selection_digest: Some("selection".to_owned()),
        explicit_empty_selection: false,
        invalidation_code: None,
        entry_text: "server".to_owned(),
        entry_confirmed: false,
        local_status: None,
        notion_status: None,
        amendment: None,
        candidates: vec![
            diary_control_contract::SourceCandidateDto {
                id: "selected".to_owned(),
                source_note_name: "selected.md".to_owned(),
                local_time: None,
                directory_day: Some("2026-10-29".to_owned()),
                header_day: Some("2026-10-29".to_owned()),
                warning_code: None,
                availability: "local".to_owned(),
                selected: true,
            },
            diary_control_contract::SourceCandidateDto {
                id: "excluded".to_owned(),
                source_note_name: "excluded.md".to_owned(),
                local_time: None,
                directory_day: Some("2026-10-29".to_owned()),
                header_day: Some("2026-10-29".to_owned()),
                warning_code: None,
                availability: "local".to_owned(),
                selected: false,
            },
        ],
    }
}

#[test]
fn automatic_discovery_runs_once_only_for_a_writable_started_workflow() {
    let mut workflow = composing_workflow();
    workflow.state = "started".to_owned();
    let mut attempted = BTreeSet::new();
    assert!(should_auto_discover(
        &workflow, true, true, false, &attempted
    ));
    attempted.insert(workflow.id.clone());
    assert!(!should_auto_discover(
        &workflow, true, true, false, &attempted
    ));
    attempted.clear();
    for (mutate, read, busy) in [
        (false, true, false),
        (true, false, false),
        (true, true, true),
    ] {
        assert!(!should_auto_discover(
            &workflow, mutate, read, busy, &attempted
        ));
    }
    for state in ["discovered", "sources-confirmed", "completed"] {
        workflow.state = state.to_owned();
        assert!(!should_auto_discover(
            &workflow, true, true, false, &attempted
        ));
    }
}

#[test]
fn compose_resources_list_the_draft_and_only_confirmed_sources() {
    let workflow = composing_workflow();
    let (resources, kinds) = AssistantViewModel::default().resources(Some(&workflow), true);
    assert_eq!(resources.rows.len(), 3);
    assert_eq!(resources.rows[0].label, "Draft");
    assert_eq!(resources.rows[0].state, "unsaved");
    assert_eq!(resources.rows[2].label, "selected.md");
    assert!(matches!(kinds[2], AssistantResource::ConfirmedSource(0)));
}

#[test]
fn compose_and_review_show_both_conflict_versions_and_the_full_entry() {
    let workflow = composing_workflow();
    let mut compose = ComposeViewModel::default();
    compose.load_from_canonical(&workflow);
    compose.edit("local complete entry".to_owned());
    let mut newer = workflow.clone();
    newer.buffer_version = 3;
    newer.entry_text = "server complete entry".to_owned();
    compose.load_from_canonical(&newer);

    let compose_body = compose_screen_body(
        &FocusModel::default(),
        Some(&newer),
        &compose,
        &AssistantViewModel::default(),
        Some(&AssistantResource::Draft),
        None,
        false,
        false,
        false,
    );
    assert!(compose_body.contains("local complete entry"));
    assert!(compose_body.contains("server complete entry"));

    let review_body = review_screen_body(&FocusModel::default(), Some(&newer), &compose, false);
    assert!(review_body.contains("Complete entry · en-US"));
    assert!(review_body.contains("local complete entry"));
    assert!(review_body.contains("save the Draft before confirming"));
}

/// A preview of a file that already holds `existing content`.
fn local_preview_with(local: &str, mode: Option<&str>) -> LocalPreviewDto {
    LocalPreviewDto {
        workflow: composing_workflow(),
        target: "/tmp/entry.md".to_owned(),
        payload: "planned".to_owned(),
        payload_digest: "digest".to_owned(),
        plan_digest: "plan".to_owned(),
        existing_text: Some("existing content".to_owned()),
        intended_changes: Vec::new(),
        local_status: local.to_owned(),
        existing_mode: mode.map(str::to_owned),
    }
}

#[test]
fn preview_shows_only_the_local_file_and_never_mentions_a_remote() {
    let mut workflow = composing_workflow();
    workflow.local_status = Some("changed".to_owned());
    let mut preview = local_preview_with("changed", None);
    preview.workflow = workflow.clone();
    preview.payload = "---\ndate: \"2026-10-29\"\n---\n\nplanned".to_owned();
    let focus = FocusModel {
        zone: FocusZone::Main,
        main_control_index: 1,
        ..FocusModel::default()
    };
    let body = preview_screen_body(&focus, Some(&preview));
    assert!(body.contains("planned"));
    assert!(body.contains("existing content"));
    assert!(body.contains("> Approve exact plan"));
    let lowered = body.to_lowercase();
    assert!(!lowered.contains("conflict"));
    assert!(!lowered.contains("icloud"));
    assert!(!lowered.contains("notion"));
}

#[test]
fn publish_local_shows_the_local_save_and_points_to_the_remote_stage() {
    let mut workflow = workflow_dto_for_publish_tests();
    workflow.state = "completed".to_owned();
    workflow.local_status = Some("verified".to_owned());
    workflow.entry_text = "A visible confirmed diary entry.".to_owned();
    let preview = local_preview_with("pending", Some("append"));
    let body = publish_screen_body(&FocusModel::default(), Some(&workflow), Some(&preview));
    assert!(body.contains("Local publication: verified"));
    assert!(body.contains("Go to Publish Remote"));
    assert!(!body.contains("Notion:"));
    assert!(!body.contains("readiness"));
    assert!(body.contains("Existing local entry:\n  existing content"));
    assert!(body.contains("Diary entry to publish:\n  A visible confirmed diary entry."));
    let focus = FocusModel {
        zone: FocusZone::Main,
        main_control_index: 1,
        ..FocusModel::default()
    };
    let pending = workflow_dto_for_publish_tests();
    let body = publish_screen_body(&focus, Some(&pending), None);
    assert!(body.contains("  Publish approved local file"));
    assert!(body.contains("> Cancel pending publication"));
}

fn workflow_dto_for_publish_tests() -> WorkflowDto {
    let mut workflow = composing_workflow();
    workflow.local_status = None;
    workflow.notion_status = None;
    workflow
}

fn fresh_local_preview() -> LocalPreviewDto {
    let mut preview = local_preview_with("pending", None);
    preview.existing_text = None;
    preview
}

#[test]
fn append_and_overwrite_are_offered_only_for_an_existing_local_entry() {
    use crate::existing_entry_view::{mode_for, offered};
    assert!(!offered(None));
    assert!(!offered(Some(&fresh_local_preview())));
    // The preview already defaults to Append; the choice is to switch.
    assert!(offered(Some(&local_preview_with(
        "pending",
        Some("append")
    ))));
    assert!(offered(Some(&local_preview_with("pending", None))));
    assert_eq!(mode_for(2), Some("append"));
    assert_eq!(mode_for(3), Some("overwrite"));
    assert_eq!(mode_for(1), None);
}

#[test]
fn preview_lists_the_commands_and_the_chosen_mode_only_when_an_entry_exists() {
    let focus = FocusModel {
        zone: FocusZone::Main,
        main_control_index: 2,
        ..FocusModel::default()
    };
    let existing = local_preview_with("pending", Some("append"));
    let body = preview_screen_body(&focus, Some(&existing));
    assert!(body.contains("> Append to the existing entry"));
    assert!(body.contains("  Overwrite the existing entry"));
    assert!(body.contains("kept; the entry is added below it"));
    let plain = preview_screen_body(&focus, Some(&fresh_local_preview()));
    assert!(!plain.contains("Append to the existing entry"));
    let chosen = preview_screen_body(
        &focus,
        Some(&local_preview_with("pending", Some("overwrite"))),
    );
    assert!(chosen.contains("Mode: Overwrite"));
    assert!(chosen.contains("Existing file (will be replaced)"));
}
