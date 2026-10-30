//! Publish-stage readiness: plain-language wording for the closed reason and
//! next-step tokens the server returns, and the block that shows them.

use diary_control_contract::{DestinationReadinessDto, PublishReadinessDto, WorkflowDto};

/// What the Publish stage knows about readiness right now.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReadinessView {
    NotChecked,
    Checking,
    Checked(PublishReadinessDto),
}

impl ReadinessView {
    /// A result describes one workflow revision only; once the workflow moves
    /// on (or another day is open) the answer is stale and shows as not checked.
    #[must_use]
    pub fn resolve(
        pending: bool,
        checked: Option<&(String, PublishReadinessDto)>,
        workflow: Option<&WorkflowDto>,
    ) -> Self {
        if pending {
            return Self::Checking;
        }
        match (checked, workflow) {
            (Some((id, report)), Some(workflow))
                if *id == workflow.id && report.revision == workflow.revision =>
            {
                Self::Checked(report.clone())
            }
            _ => Self::NotChecked,
        }
    }
}

/// A partial workflow whose local file is saved can publish Notion directly
/// only while the Notion plan is `pending` (previewed and approved). Anything
/// else needs the Preview stage first; sending a publish anyway fails the
/// server's binding check and leaves the person with no explanation.
#[must_use]
pub fn needs_notion_approval(workflow: &WorkflowDto) -> bool {
    workflow.local_status.as_deref() == Some("verified")
        && workflow.state == "partial"
        && workflow.notion_status.as_deref() != Some("pending")
}

const DESTINATIONS: [&str; 2] = ["notion", "icloud"];

/// The readiness block shown under the Publish status lines.
#[must_use]
pub fn readiness_block(view: &ReadinessView) -> String {
    let mut lines = vec!["Readiness".to_owned()];
    for destination in DESTINATIONS {
        let label = destination_label(destination);
        match view {
            ReadinessView::NotChecked => lines.push(format!("  {label:<7} Not checked")),
            ReadinessView::Checking => lines.push(format!("  {label:<7} Checking...")),
            ReadinessView::Checked(report) => {
                match report
                    .destinations
                    .iter()
                    .find(|d| d.destination == destination)
                {
                    Some(item) => lines.extend(destination_lines(label, item)),
                    None => lines.push(format!("  {label:<7} Not checked")),
                }
            }
        }
    }
    lines.join("\n")
}

fn destination_lines(label: &str, item: &DestinationReadinessDto) -> Vec<String> {
    let state = match item.readiness.as_str() {
        "ready" => "READY",
        "not-ready" => "NOT READY",
        "conflict" => "CONFLICT",
        "published" => "PUBLISHED",
        _ => "UNKNOWN",
    };
    let mut lines = vec![format!(
        "  {label:<7} {state:<9}  {}",
        reason_text(&item.reason, &item.readiness)
    )];
    if let Some(next) = next_step_text(&item.next_step) {
        lines.push(format!("{:<20}Next: {next}", ""));
    }
    lines
}

fn destination_label(destination: &str) -> &str {
    match destination {
        "notion" => "Notion",
        "icloud" => "iCloud",
        other => other,
    }
}

/// One sentence per reason token. An unlisted token is shown as-is: tokens
/// are closed and content-free, so this is safe and still diagnosable.
#[must_use]
pub fn reason_text(reason: &str, readiness: &str) -> String {
    match reason {
        "none" if readiness == "ready" => "Nothing is blocking this destination.".to_owned(),
        "none" => String::new(),
        "already-published" => "Already published.".to_owned(),
        "local-file-not-verified" => {
            "The local file has not been saved and verified yet.".to_owned()
        }
        "notion-not-configured" => "Notion is not set up.".to_owned(),
        "notion-config-invalid" => "The Notion settings are not valid.".to_owned(),
        "notion-credential-invalid" => "Notion rejected the saved credential.".to_owned(),
        "notion-permission-denied" => "Notion denied access to the configured page.".to_owned(),
        "notion-root-unverifiable" => "The configured Notion page could not be found.".to_owned(),
        "notion-hierarchy-ambiguous" => {
            "More than one Notion year or month page matches this day.".to_owned()
        }
        "notion-remote-unavailable" => "Notion could not be reached.".to_owned(),
        "notion-response-invalid" => "Notion sent a response this app could not use.".to_owned(),
        "notion-retry-deferred" => "Notion asked to try again later.".to_owned(),
        "notion-render-invalid" => "The entry could not be converted to Notion blocks.".to_owned(),
        "notion-same-day-conflict" => {
            "Notion already has different content for this day.".to_owned()
        }
        "notion-write-ambiguous" => {
            "An earlier Notion write may or may not have landed.".to_owned()
        }
        "notion-unavailable" => "Notion could not be checked.".to_owned(),
        "icloud-not-configured" => "The iCloud mirror is not set up.".to_owned(),
        "icloud-folder-unavailable" => "The iCloud folder could not be read.".to_owned(),
        "icloud-different-file-exists" => {
            "iCloud already has a different file for this day.".to_owned()
        }
        other => format!("Reason: {other}"),
    }
}

/// `None` when there is nothing for the person to do.
#[must_use]
pub fn next_step_text(step: &str) -> Option<String> {
    Some(match step {
        "none" => return None,
        "publish-local-first" => "Publish the local file first.".to_owned(),
        "configure-notion" => "Set up Notion, then check again.".to_owned(),
        "fix-notion-access" => {
            "Share the Notion page with the integration or fix the credential, then check again."
                .to_owned()
        }
        "check-notion-page" => {
            "Look for duplicate or existing pages in Notion, then check again.".to_owned()
        }
        "retry-later" => "Try again in a moment.".to_owned(),
        "configure-icloud" => "Set an iCloud folder if you want a mirror.".to_owned(),
        "check-icloud-folder" => {
            "Make sure iCloud Drive is available, then check again.".to_owned()
        }
        "resolve-icloud-file" => {
            "Move or compare the existing iCloud file, then check again.".to_owned()
        }
        "preview-and-approve" => {
            "Preview and approve it in the Preview stage to publish.".to_owned()
        }
        "publish-approved" => "Approved. Choose Publish approved destination.".to_owned(),
        other => format!("Next step: {other}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(
        destination: &str,
        readiness: &str,
        reason: &str,
        next: &str,
    ) -> DestinationReadinessDto {
        DestinationReadinessDto {
            destination: destination.to_owned(),
            readiness: readiness.to_owned(),
            reason: reason.to_owned(),
            next_step: next.to_owned(),
        }
    }

    fn report(revision: u64) -> PublishReadinessDto {
        PublishReadinessDto {
            revision,
            destinations: vec![
                item(
                    "notion",
                    "not-ready",
                    "notion-permission-denied",
                    "fix-notion-access",
                ),
                item("icloud", "ready", "none", "preview-and-approve"),
            ],
        }
    }

    #[test]
    fn unchecked_and_checking_name_both_destinations() {
        let text = readiness_block(&ReadinessView::NotChecked);
        assert!(text.contains("Notion  Not checked"));
        assert!(text.contains("iCloud  Not checked"));
        let text = readiness_block(&ReadinessView::Checking);
        assert!(text.contains("Notion  Checking..."));
        assert!(text.contains("iCloud  Checking..."));
    }

    #[test]
    fn a_result_shows_state_reason_and_next_step_per_destination() {
        let text = readiness_block(&ReadinessView::Checked(report(7)));
        assert!(text.contains("NOT READY"));
        assert!(text.contains("Notion denied access to the configured page."));
        assert!(text.contains("Next: Share the Notion page"));
        assert!(text.contains("READY"));
        assert!(text.contains("Nothing is blocking this destination."));
        assert!(text.contains("Next: Preview and approve it in the Preview stage"));
    }

    #[test]
    fn an_unlisted_token_is_shown_not_hidden() {
        assert_eq!(reason_text("new-reason", "not-ready"), "Reason: new-reason");
        assert_eq!(
            next_step_text("new-step").as_deref(),
            Some("Next step: new-step")
        );
        assert_eq!(next_step_text("none"), None);
    }

    fn workflow(id: &str, revision: u64) -> WorkflowDto {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "diary_day": "2026-10-30",
            "state": "partial",
            "revision": revision,
            "buffer_version": 0,
            "language": null,
            "style": "concise",
            "assistant_backend_id": null,
            "composition_path": null,
            "selection_digest": null,
            "explicit_empty_selection": false,
            "invalidation_code": null,
            "entry_text": "",
            "entry_confirmed": true,
            "local_status": "verified",
            "notion_status": "blocked",
            "icloud_status": null,
            "amendment": null,
            "candidates": []
        }))
        .expect("synthetic workflow")
    }

    #[test]
    fn publish_waits_for_a_notion_plan_that_is_previewed_and_approved() {
        let with = |state: &str, local: Option<&str>, notion: Option<&str>| {
            let mut dto = workflow("2026-10-30", 1);
            dto.state = state.to_owned();
            dto.local_status = local.map(str::to_owned);
            dto.notion_status = notion.map(str::to_owned);
            dto
        };
        // Local saved, Notion still blocked or conflicted: preview first.
        for notion in [Some("blocked"), Some("conflict"), None] {
            assert!(needs_notion_approval(&with(
                "partial",
                Some("verified"),
                notion
            )));
        }
        // An approved, unchanged Notion plan publishes directly.
        assert!(!needs_notion_approval(&with(
            "partial",
            Some("verified"),
            Some("pending")
        )));
        // Other states are not this guard's concern.
        assert!(!needs_notion_approval(&with(
            "approved",
            Some("verified"),
            Some("pending")
        )));
        assert!(!needs_notion_approval(&with(
            "partial",
            Some("pending"),
            Some("blocked")
        )));
    }

    #[test]
    fn a_result_is_stale_after_the_revision_or_day_changes() {
        let checked = ("2026-10-30".to_owned(), report(7));
        let current = workflow("2026-10-30", 7);
        assert!(matches!(
            ReadinessView::resolve(false, Some(&checked), Some(&current)),
            ReadinessView::Checked(_)
        ));
        let moved = workflow("2026-10-30", 8);
        assert_eq!(
            ReadinessView::resolve(false, Some(&checked), Some(&moved)),
            ReadinessView::NotChecked
        );
        let other_day = workflow("2026-10-31", 7);
        assert_eq!(
            ReadinessView::resolve(false, Some(&checked), Some(&other_day)),
            ReadinessView::NotChecked
        );
        assert_eq!(
            ReadinessView::resolve(true, Some(&checked), Some(&current)),
            ReadinessView::Checking
        );
        assert_eq!(
            ReadinessView::resolve(false, None, Some(&current)),
            ReadinessView::NotChecked
        );
    }
}
