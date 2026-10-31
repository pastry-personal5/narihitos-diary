//! The Publish Remote stage: the menu for one remote and what it shows.
//!
//! Remotes (iCloud, Notion) are listed in the Resources Sidebar. Selecting one
//! shows what it already holds without asking, and a menu to see what would be
//! pushed, check readiness, and push. Content already at the remote is never a
//! dead end: Notion offers Append and Overwrite; iCloud offers Overwrite.

use diary_control_contract::{RemoteDto, RemotePreviewDto};

use crate::focus::{FocusModel, FocusZone};
use crate::main_pane::indent_diary_block;
use crate::readiness_view::{ReadinessView, readiness_block, reason_text};

/// One entry of a remote's menu, in display order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RemoteControl {
    /// Refresh the read-only plan.
    ShowPlan,
    CheckReadiness,
    /// Keep what the remote holds and add the entry under a dated separator.
    Append,
    /// Back up (where possible) and replace what the remote holds.
    Overwrite,
    /// Push the plan shown; the push is the approval.
    Push,
}

impl RemoteControl {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::ShowPlan => "Show what will be pushed",
            Self::CheckReadiness => "Check readiness",
            Self::Append => "Append to the existing entry",
            Self::Overwrite => "Overwrite the existing entry",
            Self::Push => "Push",
        }
    }
}

/// The controls the current preview offers. Append and Overwrite appear while
/// the remote holds content; Push appears once there is a plan to push.
#[must_use]
pub fn controls(preview: Option<&RemotePreviewDto>) -> Vec<RemoteControl> {
    let mut controls = vec![RemoteControl::ShowPlan, RemoteControl::CheckReadiness];
    if let Some(preview) = preview {
        if preview.status == "exists" {
            if preview.id == "notion" {
                controls.push(RemoteControl::Append);
            }
            controls.push(RemoteControl::Overwrite);
        }
        if preview.plan_digest.is_some() {
            controls.push(RemoteControl::Push);
        }
    }
    controls
}

/// The status word shown beside a remote.
#[must_use]
pub fn status_label(status: &str) -> &'static str {
    match status {
        "not-pushed" => "Not pushed",
        "exists" => "Exists",
        "pushed" => "Pushed",
        "blocked" => "Blocked",
        _ => "Unknown",
    }
}

/// Plain-language line for a chosen mode at a remote.
#[must_use]
pub fn mode_line(mode: &str) -> &'static str {
    match mode {
        "append" => {
            "Mode: Append — the existing text is kept and the entry is added under a dated separator"
        }
        "overwrite" => {
            "Mode: Overwrite — the existing content is replaced (a file is backed up first)"
        }
        _ => "Mode: unknown",
    }
}

/// The Main Pane body for the selected remote.
#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn remote_screen_body(
    focus: &FocusModel,
    completed: bool,
    remote: Option<&RemoteDto>,
    preview: Option<&RemotePreviewDto>,
    diary_entry: Option<&str>,
    readiness: &ReadinessView,
    loading: bool,
    overwrite_pending: bool,
) -> String {
    if !completed {
        return "Publish to Local first. Remotes unlock once the local file is saved.\n".to_owned();
    }
    let Some(remote) = remote else {
        let mut text = "No remotes yet.\n".to_owned();
        push_diary_entry(&mut text, diary_entry);
        return text;
    };
    let marker = |index: usize| {
        if focus.zone == FocusZone::Main && focus.main_control_index == index {
            ">"
        } else {
            " "
        }
    };
    let status = preview.map_or(remote.status.as_str(), |preview| preview.status.as_str());
    let mut text = format!(
        "{}: {}{}\n\n",
        remote.label,
        status_label(status),
        if loading { " (checking...)" } else { "" }
    );
    for (index, control) in controls(preview).into_iter().enumerate() {
        text.push_str(&format!("{} {}\n", marker(index), control.label()));
    }
    if overwrite_pending {
        text.push_str(
            "\nOverwrite replaces what is there. Press Enter again to confirm, Esc to cancel.\n",
        );
    }
    text.push('\n');
    let Some(preview) = preview else {
        text.push_str(if loading {
            "Looking at what is there...\n"
        } else {
            "Select Show what will be pushed to look at this remote.\n"
        });
        text.push('\n');
        text.push_str(&readiness_block(readiness));
        push_diary_entry(&mut text, diary_entry);
        return text;
    };
    if preview.status == "blocked" {
        text.push_str(&format!(
            "Warning: {}\n",
            blocked_text(preview.reason.as_deref())
        ));
        if let Some(url) = &preview.page_url {
            text.push_str(&format!(
                "Fix it by hand in the Notion app or on Notion Web, then check readiness:\n  {url}\n"
            ));
        }
    }
    if !preview.target.is_empty() {
        text.push_str(&format!("Target: {}\n", preview.target.join(" → ")));
    }
    if !preview.placement.is_empty() {
        text.push_str(&format!("Position: {}\n", preview.placement));
    }
    if let Some(mode) = &preview.existing_mode {
        text.push_str(&format!("{}\n", mode_line(mode)));
    }
    for change in &preview.intended_changes {
        text.push_str(&format!("Planned: {change}\n"));
    }
    if let Some(existing) = existing_entry_text(preview) {
        text.push_str(&format!(
            "\nAlready there ({}):\n{}\n",
            label_for(preview),
            indent_diary_block(&existing)
        ));
    }
    if let Some(payload) = &preview.payload
        && preview.status != "pushed"
    {
        text.push_str(&format!("\nWill be written:\n{payload}\n"));
    }
    if !preview.blocks.is_empty() {
        text.push_str("\nWill be pushed:\n");
        push_blocks(&mut text, &preview.blocks);
    }
    text.push('\n');
    text.push_str(&readiness_block(readiness));
    push_diary_entry(&mut text, diary_entry);
    text
}

/// The exact existing-content rendering used by the Main Pane and its
/// semantic foreground accent.
#[must_use]
pub fn existing_entry_text(preview: &RemotePreviewDto) -> Option<String> {
    if let Some(existing) = &preview.existing_text {
        return Some(existing.clone());
    }
    if preview.current_blocks.is_empty() {
        return None;
    }
    let mut text = String::new();
    push_blocks(&mut text, &preview.current_blocks);
    text.pop();
    Some(text)
}

fn push_diary_entry(text: &mut String, diary_entry: Option<&str>) {
    if let Some(entry) = diary_entry {
        text.push_str("\n\nDiary entry to publish:\n");
        text.push_str(&indent_diary_block(entry));
    }
}

fn label_for(preview: &RemotePreviewDto) -> &'static str {
    if preview.status == "not-pushed" {
        return "this app's earlier copy; it will be updated";
    }
    match preview.existing_mode.as_deref() {
        Some("append") => "will be appended to",
        Some("overwrite") => "will be replaced",
        _ if preview.id == "icloud" => "choose Overwrite to back up and replace this file",
        _ => "choose Append or Overwrite",
    }
}

fn push_blocks(text: &mut String, blocks: &[diary_control_contract::NotionBlockDto]) {
    for block in blocks {
        text.push_str(&format!(
            "{}{}: {}\n",
            "  ".repeat(usize::from(block.depth)),
            block.kind,
            block.text
        ));
    }
}

fn blocked_text(reason: Option<&str>) -> String {
    match reason {
        Some("notion-same-day-conflict") => {
            "this Notion month page has a section for this day that cannot be changed safely (for example, two sections for the same day)."
                .to_owned()
        }
        Some("notion-hierarchy-ambiguous") => {
            "more than one Notion year or month page matches this day.".to_owned()
        }
        Some(reason) => reason_text(reason, "not-ready"),
        None => "this remote cannot be pushed to right now.".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use diary_control_contract::WorkflowDto;

    fn workflow() -> WorkflowDto {
        serde_json::from_value(serde_json::json!({
            "id": "wf_20261029", "diary_day": "2026-10-29", "state": "completed",
            "revision": 7, "buffer_version": 0, "language": null, "style": "concise",
            "assistant_backend_id": null, "composition_path": null, "selection_digest": null,
            "explicit_empty_selection": false, "invalidation_code": null, "entry_text": "",
            "entry_confirmed": true, "candidates": []
        }))
        .unwrap()
    }

    fn preview(status: &str, plan: Option<&str>) -> RemotePreviewDto {
        RemotePreviewDto {
            id: "icloud".to_owned(),
            label: "iCloud".to_owned(),
            status: status.to_owned(),
            reason: None,
            workflow: workflow(),
            target: vec!["2026-10-29.md".to_owned()],
            payload: Some("# new".to_owned()),
            existing_text: None,
            intended_changes: Vec::new(),
            placement: String::new(),
            blocks: Vec::new(),
            current_blocks: Vec::new(),
            existing_mode: None,
            plan_digest: plan.map(str::to_owned),
            page_url: None,
        }
    }

    fn remote(status: &str) -> RemoteDto {
        RemoteDto {
            id: "icloud".to_owned(),
            label: "iCloud".to_owned(),
            status: status.to_owned(),
            reason: None,
        }
    }

    #[test]
    fn a_fresh_remote_offers_push_and_no_append_or_overwrite() {
        let preview = preview("not-pushed", Some("sha256:a"));
        assert_eq!(
            controls(Some(&preview)),
            vec![
                RemoteControl::ShowPlan,
                RemoteControl::CheckReadiness,
                RemoteControl::Push
            ]
        );
    }

    #[test]
    fn existing_content_offers_append_and_overwrite_and_never_a_conflict_dead_end() {
        let mut preview = preview("exists", None);
        preview.id = "notion".to_owned();
        preview.existing_text = Some("old entry".to_owned());
        assert_eq!(
            controls(Some(&preview)),
            vec![
                RemoteControl::ShowPlan,
                RemoteControl::CheckReadiness,
                RemoteControl::Append,
                RemoteControl::Overwrite
            ]
        );
        let body = remote_screen_body(
            &FocusModel::default(),
            true,
            Some(&remote("exists")),
            Some(&preview),
            Some("Today was clear.\nIt stayed quiet."),
            &ReadinessView::NotChecked,
            false,
            false,
        );
        assert!(body.contains("iCloud: Exists"));
        assert!(body.contains("old entry"));
        assert!(body.contains("Diary entry to publish:\n  Today was clear.\n  It stayed quiet."));
        assert!(body.contains("choose Append or Overwrite"));
        assert!(!body.to_lowercase().contains("conflict"));
        // Choosing a mode adds a plan, and Push appears.
        preview.existing_mode = Some("append".to_owned());
        preview.plan_digest = Some("sha256:b".to_owned());
        assert_eq!(controls(Some(&preview)).last(), Some(&RemoteControl::Push));
    }

    #[test]
    fn icloud_existing_content_only_offers_backup_and_overwrite() {
        let mut preview = preview("exists", None);
        preview.existing_text = Some("another entry".to_owned());
        assert_eq!(
            controls(Some(&preview)),
            vec![
                RemoteControl::ShowPlan,
                RemoteControl::CheckReadiness,
                RemoteControl::Overwrite,
            ]
        );
        let body = remote_screen_body(
            &FocusModel::default(),
            true,
            Some(&remote("exists")),
            Some(&preview),
            None,
            &ReadinessView::NotChecked,
            false,
            false,
        );
        assert!(body.contains("choose Overwrite"));
        assert!(!body.contains("Append"));
    }

    #[test]
    fn a_blocked_notion_page_shows_a_warning_and_the_url() {
        let mut preview = preview("blocked", None);
        preview.reason = Some("notion-same-day-conflict".to_owned());
        preview.page_url = Some("https://www.notion.so/abc123".to_owned());
        let body = remote_screen_body(
            &FocusModel::default(),
            true,
            Some(&remote("blocked")),
            Some(&preview),
            None,
            &ReadinessView::NotChecked,
            false,
            false,
        );
        assert!(body.contains("Warning:"));
        assert!(body.contains("two sections for the same day"));
        assert!(body.contains("https://www.notion.so/abc123"));
        assert_eq!(controls(Some(&preview)).len(), 2);
    }

    #[test]
    fn the_stage_is_closed_until_the_local_file_is_saved() {
        let body = remote_screen_body(
            &FocusModel::default(),
            false,
            None,
            None,
            None,
            &ReadinessView::NotChecked,
            false,
            false,
        );
        assert!(body.contains("Publish to Local first"));
    }

    #[test]
    fn overwrite_asks_for_a_second_enter() {
        let body = remote_screen_body(
            &FocusModel::default(),
            true,
            Some(&remote("exists")),
            Some(&preview("exists", Some("sha256:c"))),
            None,
            &ReadinessView::NotChecked,
            false,
            true,
        );
        assert!(body.contains("Press Enter again to confirm, Esc to cancel"));
    }
}
