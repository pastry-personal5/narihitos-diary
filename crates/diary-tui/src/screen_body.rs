//! Pure screen-body builders for the connected terminal session.

use diary_control_contract::{
    CapabilitiesDto, LocalPreviewDto, NotionPreviewDto, PreferencesDto, SetupCheckDto,
    SourceDetailDto, WorkflowDto,
};

use crate::amendment_view;
use crate::assistant_view::{AssistantResource, AssistantViewModel};
use crate::compose_view::ComposeViewModel;
use crate::date_display::format_diary_day;
use crate::focus::{FocusModel, FocusZone};
use crate::readiness_view::{ReadinessView, readiness_block};
use crate::review_view::first_unresolved_delimiter;
use crate::sources_menu::{SourcesMenuItem, SourcesMenuState, label as menu_label};
use crate::sources_view::{SourcesViewModel, source_label};

#[allow(clippy::too_many_arguments)]
pub fn compose_screen_body(
    focus: &FocusModel,
    workflow: Option<&WorkflowDto>,
    compose: &ComposeViewModel,
    assistant: &AssistantViewModel,
    active_resource: Option<&AssistantResource>,
    source: Option<&SourceDetailDto>,
    settings_pending: bool,
    manual_pending: bool,
    save_pending: bool,
) -> String {
    if let Some(source) = source {
        return format!(
            "Read-only confirmed source: {}\nFile: {}\n\n{}\n\nSelect Draft in Resources to return; its cursor and buffer are preserved.",
            source.local_time.as_deref().unwrap_or("unknown time"),
            source.source_note_name,
            source.body
        );
    }
    let marker = |index| {
        if focus.zone == FocusZone::Main && focus.main_control_index == index {
            ">"
        } else {
            " "
        }
    };
    let language = workflow
        .and_then(|value| value.language.as_deref())
        .unwrap_or("not selected");
    let style = workflow.map_or("concise", |value| value.style.as_str());
    let state = workflow.map_or("unavailable", |value| value.state.as_str());
    let backend = workflow
        .and_then(|value| value.assistant_backend_id.as_deref())
        .unwrap_or("manual");
    let path = workflow
        .and_then(|value| value.composition_path.as_deref())
        .unwrap_or("manual");
    let save = if save_pending {
        "saving"
    } else if compose.is_dirty() {
        "unsaved"
    } else {
        "saved"
    };
    let mut body = format!(
        "Workflow: {state}\nWriting language: {language}\nStyle: {style}\nAssistant: {backend} · {path}\nDraft: {save}\n\n{} Language: US English\n{} Language: Korean\n{} Style: concise\n{} Style: standard\n{} Style: reflective\n{} Open manual Draft editor{}\n{} Save/flush Draft\n{} Keep local Draft after conflict\n{} Use server Draft after conflict\n{} Review complete entry\n{} Extract memories\n{} Confirm reviewed memories\n{} Draft with selected assistant\n{} Review focused paragraph\n{} Cancel assistant action\n{} Refresh assistant resources\n{} Accept current suggestion\n{} Reject current suggestion",
        marker(0),
        marker(1),
        marker(2),
        marker(3),
        marker(4),
        marker(5),
        if manual_pending { " (working)" } else { "" },
        marker(6),
        marker(7),
        marker(8),
        marker(9),
        marker(10),
        marker(11),
        marker(12),
        marker(13),
        marker(14),
        marker(15),
        marker(16),
        marker(17),
    );
    if settings_pending {
        body.push_str("\n\nWriting settings are being saved.");
    }
    if let Some(server_text) = &compose.conflicting_server_text {
        body.push_str("\n\n--- Local Draft (preserved) ---\n");
        body.push_str(&compose.text);
        body.push_str("\n\n--- Server Draft (canonical) ---\n");
        body.push_str(server_text);
        body.push_str("\n\nChoose Keep local or Use server; no merge or replay is automatic.");
    } else {
        body.push_str("\n\nEnter opens the selected control. Once the editor opens, Esc leaves it and flushes pending edits.");
    }
    if matches!(
        active_resource,
        Some(AssistantResource::MemoryReview | AssistantResource::MemoryCandidate(_))
    ) {
        body.push_str("\n\n--- Reviewed memories (editable selection) ---\n");
        body.push_str(&assistant.reviewed_memory_draft);
        body.push_str("\n\nActivate a candidate in Resources to include/exclude it, then Confirm reviewed memories.");
    }
    if let Some(draft) = &assistant.draft_result {
        body.push_str("\n\n--- Current assistant draft artifact");
        if draft.stale {
            body.push_str(" (stale; inspect only)");
        }
        body.push_str(" ---\n");
        body.push_str(&draft.text);
    }
    if let Some(suggestion) = &assistant.active_suggestion {
        body.push_str("\n\n--- Paragraph suggestion ---\n");
        body.push_str(&suggestion.replacement);
        if suggestion.stale {
            body.push_str("\n(stale; cannot apply)");
        }
    }
    body
}

pub fn review_screen_body(
    focus: &FocusModel,
    workflow: Option<&WorkflowDto>,
    compose: &ComposeViewModel,
    confirmation_pending: bool,
) -> String {
    let marker = |index| {
        if focus.zone == FocusZone::Main && focus.main_control_index == index {
            ">"
        } else {
            " "
        }
    };
    let language = workflow
        .and_then(|value| value.language.as_deref())
        .unwrap_or("not selected");
    let guard = if compose.text.trim().is_empty() {
        "blank entries cannot be confirmed"
    } else if first_unresolved_delimiter(&compose.text).is_some() {
        "resolve inferred-reflection delimiters before confirming"
    } else if compose.is_dirty() {
        "save the Draft before confirming"
    } else {
        "ready"
    };
    format!(
        "Complete entry · {language}\nConfirmation: {guard}\n\n{} Confirm complete entry{}\n{} Back to Draft editor\n{} Find first unresolved reflection marker\n{} Save/flush Draft\n\n--- Full edited entry ---\n{}",
        marker(0),
        if confirmation_pending {
            " (working)"
        } else {
            ""
        },
        marker(1),
        marker(2),
        marker(3),
        compose.text,
    )
}

pub fn preview_screen_body(
    focus: &FocusModel,
    preview: Option<&LocalPreviewDto>,
    notion: Option<&NotionPreviewDto>,
) -> String {
    let marker = |index| {
        if focus.zone == FocusZone::Main && focus.main_control_index == index {
            ">"
        } else {
            " "
        }
    };
    let mut text = format!(
        "{} Generate/refresh preview\n{} Approve exact plan\n{} Save conflict under another day\n",
        marker(0),
        marker(1),
        marker(2)
    );
    let local_verified = notion
        .map(|notion| &notion.workflow)
        .or(preview.map(|preview| &preview.workflow))
        .is_some_and(|workflow| workflow.local_status.as_deref() == Some("verified"));
    if crate::existing_entry_view::offered(local_verified, preview, notion) {
        text.push_str(&format!(
            "{} Append to the existing entry\n{} Overwrite the existing entry\n",
            marker(crate::existing_entry_view::APPEND_CONTROL),
            marker(crate::existing_entry_view::OVERWRITE_CONTROL)
        ));
    }
    text.push('\n');
    if let Some(notion) = notion {
        text.push_str(&format!(
            "Path: {}\nPosition: {}\nRenderer: {} · API: {}\nSHA-256: {}\nNotion: {}\nYear: {} · Month: {}\n",
            notion.path.join(" → "),
            if notion.placement.is_empty() {
                "end"
            } else {
                &notion.placement
            },
            notion.renderer_version,
            notion.api_version,
            notion.payload_digest,
            notion.status,
            if notion.year_exists {
                "existing"
            } else {
                "create"
            },
            if notion.month_exists {
                "existing"
            } else {
                "create"
            },
        ));
        if let Some(reason) = &notion.reason {
            text.push_str(&format!("Reason: {reason}\n"));
        }
        if let Some(mode) = &notion.existing_mode {
            text.push_str(&format!(
                "{}\n",
                crate::existing_entry_view::mode_line(mode)
            ));
        }
        text.push('\n');
        if !notion.current_blocks.is_empty() {
            let label = match notion.existing_mode.as_deref() {
                Some("append") => "will be appended to",
                _ => "will be replaced",
            };
            text.push_str(&format!("Existing section ({label}):\n"));
            for block in &notion.current_blocks {
                text.push_str(&format!(
                    "{}{}: {}\n",
                    "  ".repeat(usize::from(block.depth)),
                    block.kind,
                    block.text
                ));
            }
            text.push_str("\nPlanned:\n");
        }
        for block in &notion.blocks {
            text.push_str(&format!(
                "{}{}: {}\n",
                "  ".repeat(usize::from(block.depth)),
                block.kind,
                block.text
            ));
        }
    } else if let Some(preview) = preview {
        text.push_str(&format!(
            "Target: {}\nSHA-256: {}\nLocal: {}\nNotion: {}\n",
            preview.target, preview.payload_digest, preview.local_status, preview.notion_status,
        ));
        if let Some(target) = &preview.icloud_target {
            text.push_str(&format!(
                "iCloud mirror: {target} ({})\n",
                preview.icloud_status.as_deref().unwrap_or("pending")
            ));
        }
        if let Some(mode) = &preview.existing_mode {
            text.push_str(&format!(
                "{}\n",
                crate::existing_entry_view::mode_line(mode)
            ));
        }
        text.push_str(&format!("\n{}\n", preview.payload));
        for change in &preview.intended_changes {
            text.push_str(&format!("Planned: {change}\n"));
        }
        if let Some(existing) = &preview.existing_text {
            let label = match preview.existing_mode.as_deref() {
                Some("append") => "will be appended to",
                Some("overwrite") => "will be replaced",
                _ => "conflict",
            };
            text.push_str(&format!("\nExisting target ({label}):\n{existing}\n"));
        }
    } else {
        text.push_str("Generate a read-only output preview to inspect the exact destination.\n");
    }
    text
}

pub fn amendment_preview_screen_body(
    focus: &FocusModel,
    workflow: Option<&WorkflowDto>,
    preview: Option<&diary_control_contract::AmendmentPreviewDto>,
) -> String {
    let marker = |index| {
        if focus.zone == FocusZone::Main && focus.main_control_index == index {
            ">"
        } else {
            " "
        }
    };
    let mut text = String::new();
    if let Some(workflow) = workflow {
        if let Some(banner) = amendment_view::banner(workflow) {
            text.push_str(banner);
            text.push_str("\n\n");
        }
        if let Some(status) = amendment_view::status_text(workflow) {
            text.push_str(status);
            text.push_str("\n\n");
        }
    }
    text.push_str(&format!(
        "{} Preview the replacement\n{} Approve exact plan\n",
        marker(0),
        marker(1)
    ));
    let choices = workflow
        .and_then(amendment_view::conflict_choices)
        .unwrap_or(&[]);
    for (index, choice) in [
        amendment_view::ConflictChoice::Reread,
        amendment_view::ConflictChoice::AbandonAmendment,
    ]
    .into_iter()
    .enumerate()
    {
        if choices.contains(&choice) {
            text.push_str(&format!("{} {}\n", marker(index + 2), choice.label()));
        }
    }
    text.push('\n');
    if let Some(preview) = preview {
        for line in amendment_view::preview_lines(preview) {
            text.push_str(&line);
            text.push('\n');
        }
    }
    text
}

pub fn publish_screen_body(
    focus: &FocusModel,
    workflow: Option<&WorkflowDto>,
    readiness: &ReadinessView,
) -> String {
    let marker = |index| {
        if focus.zone == FocusZone::Main && focus.main_control_index == index {
            ">"
        } else {
            " "
        }
    };
    let icloud = workflow.and_then(|w| w.icloud_status.as_deref());
    let local = workflow
        .and_then(|w| w.local_status.as_deref())
        .unwrap_or("pending");
    // A verified mirror stays quiet: a note on the local row, no row of its own.
    let local_note = if icloud == Some("verified") {
        " (also saved to iCloud)"
    } else {
        ""
    };
    let icloud_row = match icloud {
        Some(status) if status != "verified" => format!("iCloud mirror: {status}\n"),
        _ => String::new(),
    };
    let completion = workflow
        .and_then(amendment_view::completion_text)
        .map(|text| format!("{text}\n\n"))
        .unwrap_or_default();
    let primary = if workflow.is_some_and(amendment_view::offers_amendment) {
        "Amend this entry"
    } else {
        "Publish approved destination"
    };
    format!(
        "{completion}Local publication: {local}{local_note}\n{icloud_row}Notion: {}\n\n{}\n\n{} {primary}\n{} Cancel pending publication\n{} Check Notion and iCloud readiness",
        workflow
            .and_then(|w| w.notion_status.as_deref())
            .unwrap_or("blocked"),
        readiness_block(readiness),
        marker(0),
        marker(1),
        marker(2)
    )
}

#[allow(clippy::too_many_arguments)]
pub fn start_screen_body(
    focus: &FocusModel,
    selected_day: Option<&str>,
    day_editing: bool,
    day_draft: &str,
    selected_workflow: Option<&diary_control_contract::WorkflowSummaryDto>,
    has_more: bool,
    capabilities: Option<&CapabilitiesDto>,
    setup: Option<&SetupCheckDto>,
    preferences: Option<&PreferencesDto>,
    assistant: &AssistantViewModel,
    mutations_enabled: bool,
    start_pending: bool,
    more_pending: bool,
    discard_pending: bool,
    help_open: bool,
) -> String {
    let marker = |index| {
        if focus.zone == FocusZone::Main && focus.main_control_index == index {
            ">"
        } else {
            " "
        }
    };
    let day = if day_editing {
        format!("{day_draft}_")
    } else {
        selected_day.map_or_else(
            || "Mac-local today".to_owned(),
            |day| format_diary_day(day, 16),
        )
    };
    let selected = selected_workflow.map_or_else(
        || "none".to_owned(),
        |workflow| format_diary_day(&workflow.diary_day, 16),
    );
    let manual = capabilities.map_or("checking", |value| {
        if value.manual_workflow_available {
            "available"
        } else {
            "unavailable"
        }
    });
    let setup_label = setup.map_or_else(
        || "checking".to_owned(),
        |value| {
            format!(
                "config={} database={} Apple helper={}",
                ready_label(value.config_safe),
                ready_label(value.database_ready),
                ready_label(value.apple_helper_available)
            )
        },
    );
    let action_state = if mutations_enabled {
        "ready"
    } else {
        "paused during canonical reload"
    };
    let selected_assistant = preferences
        .and_then(|value| value.last_assistant_backend_id.as_deref())
        .and_then(|id| assistant.backends.iter().find(|backend| backend.id == id))
        .map_or_else(
            || "manual".to_owned(),
            |backend| {
                format!(
                    "{} · {} ({})",
                    backend.display_label,
                    backend.composition_path,
                    if backend.available {
                        "available"
                    } else {
                        "unavailable"
                    }
                )
            },
        );
    let mut body = format!(
        "Setup: {setup_label}\nManual workflow: {manual}\nAssistant: {selected_assistant}\nActions: {action_state}\nSelected workflow: {selected}\n\n{} Day: {day}\n{} Start/resume chosen day{}\n{} Resume selected workflow\n{} Discard selected workflow{}\n{} Load more workflows{}\n{} Retry/refresh\n{} Help\n{} Save/flush (Compose only)\n{} Quit",
        marker(0),
        marker(1),
        if start_pending { " (working)" } else { "" },
        marker(2),
        marker(3),
        if discard_pending { " (working)" } else { "" },
        marker(4),
        if more_pending {
            " (working)"
        } else if has_more {
            ""
        } else {
            " (no more)"
        },
        marker(5),
        marker(6),
        marker(7),
        marker(8),
    );
    if help_open {
        body.push_str(
            "\n\nTab/Shift-Tab or Left/Right changes focus. Up/Down chooses an item. Enter activates it. Ctrl-G opens Command Line; Ctrl-P/Ctrl-N changes stage; Ctrl-C twice quits.",
        );
    }
    body
}

#[allow(clippy::too_many_arguments)]
pub fn sources_screen_body(
    focus: &FocusModel,
    sources: &SourcesViewModel,
    menu_state: &SourcesMenuState,
    menu_items: &[SourcesMenuItem],
    explicit_day: Option<&str>,
    date_editing: bool,
    date_draft: &str,
    page_pending: bool,
    confirmation_pending: bool,
    job_pending: bool,
) -> String {
    let active = sources.candidate_at(focus.resource_index);
    let active_label = active.map_or_else(|| "none".to_owned(), source_label);
    let active_file = active.map_or("none", |candidate| candidate.source_note_name.as_str());
    let directory_day = active
        .and_then(|candidate| candidate.directory_day.as_deref())
        .map_or_else(|| "unknown".to_owned(), |day| format_diary_day(day, 16));
    let header_day = active
        .and_then(|candidate| candidate.header_day.as_deref())
        .map_or_else(|| "unknown".to_owned(), |day| format_diary_day(day, 16));
    let availability = active.map_or("unknown", |candidate| candidate.availability.as_str());
    let warning = active
        .and_then(|candidate| candidate.warning_code.as_deref())
        .unwrap_or("none");
    let progress = if confirmation_pending {
        "confirming"
    } else if sources.confirming() {
        "preparing included notes"
    } else if page_pending {
        "loading candidates"
    } else if job_pending {
        "source job running"
    } else {
        "idle"
    };
    let progress = sources.job_progress_percent.map_or_else(
        || progress.to_owned(),
        |percent| format!("{progress} · {percent}%"),
    );
    let hidden = if menu_state.hidden_count > 0 {
        format!(" · {} hidden (u: show)", menu_state.hidden_count)
    } else {
        String::new()
    };
    let extra_day = if date_editing {
        format!("\nExtra date: {date_draft}_ (Enter searches that date; Esc cancels)")
    } else {
        explicit_day.map_or_else(String::new, |day| format!("\nExtra date: {day}"))
    };
    let mut body = format!(
        "Active candidate: {active_label}\nFile: {active_file}\nDirectory date: {directory_day}\nHeader date: {header_day}\nAvailability: {availability}\nWarning: {warning}\nIncluded: {} of {}{hidden}{extra_day}\nWork: {progress}\n",
        sources.included.len(),
        sources.visible_len(),
    );
    for (index, item) in menu_items.iter().enumerate() {
        let marker = if focus.zone == FocusZone::Main && focus.main_control_index == index {
            ">"
        } else {
            " "
        };
        body.push_str(&format!("\n{marker} {}", menu_label(*item, menu_state)));
        if *item == SourcesMenuItem::LoadMore && sources.next_cursor.is_none() {
            body.push_str(" (no more)");
        }
    }
    if let Some(detail) = &sources.active_detail {
        body.push_str("\n\n--- Source text ---\n");
        body.push_str(&detail.body);
    }
    if let Some(suggestion) = &sources.active_refine_suggestion {
        body.push_str("\n\n--- Suggested refinement ---\n");
        body.push_str(&suggestion.suggested_body);
    }
    body
}

const fn ready_label(ready: bool) -> &'static str {
    if ready { "ready" } else { "unavailable" }
}
