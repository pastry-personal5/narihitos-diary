//! Remote readiness: plain-language wording for the closed reason and
//! next-step tokens the server returns, and the block that shows them.

use diary_control_contract::{DestinationReadinessDto, RemoteReadinessDto, WorkflowDto};

/// What a remote's menu knows about its readiness right now.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReadinessView {
    NotChecked,
    Checking,
    Checked(RemoteReadinessDto),
}

/// A readiness answer: the workflow it was asked for and the remote.
pub type CheckedReadiness = (String, String, RemoteReadinessDto);

impl ReadinessView {
    /// A result describes one remote of one workflow revision only; once the
    /// workflow moves on (or another remote or day is open) the answer is stale
    /// and shows as not checked.
    #[must_use]
    pub fn resolve(
        pending: bool,
        checked: Option<&CheckedReadiness>,
        workflow: Option<&WorkflowDto>,
        remote: &str,
    ) -> Self {
        if pending {
            return Self::Checking;
        }
        match (checked, workflow) {
            (Some((id, checked_remote, report)), Some(workflow))
                if *id == workflow.id
                    && checked_remote == remote
                    && report.revision == workflow.revision =>
            {
                Self::Checked(report.clone())
            }
            _ => Self::NotChecked,
        }
    }
}

/// The readiness lines shown in a remote's menu.
#[must_use]
pub fn readiness_block(view: &ReadinessView) -> String {
    match view {
        ReadinessView::NotChecked => "Readiness: Not checked".to_owned(),
        ReadinessView::Checking => "Readiness: Checking...".to_owned(),
        ReadinessView::Checked(report) => {
            let mut lines = vec!["Readiness".to_owned()];
            lines.extend(destination_lines(&report.readiness));
            lines.join("\n")
        }
    }
}

fn destination_lines(item: &DestinationReadinessDto) -> Vec<String> {
    let state = match item.readiness.as_str() {
        "ready" => "READY",
        "not-ready" => "NOT READY",
        "published" => "PUBLISHED",
        _ => "UNKNOWN",
    };
    let mut lines = vec![format!(
        "  {state:<9}  {}",
        reason_text(&item.reason, &item.readiness)
    )];
    if let Some(next) = next_step_text(&item.next_step) {
        lines.push(format!("  Next: {next}"));
    }
    lines
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
            "Notion has duplicate or malformed sections for this day.".to_owned()
        }
        "notion-write-ambiguous" => {
            "An earlier Notion write may or may not have landed.".to_owned()
        }
        "notion-unavailable" => "Notion could not be checked.".to_owned(),
        "icloud-not-configured" => "iCloud is not set up.".to_owned(),
        "local-not-published" => "The local file has not been saved yet.".to_owned(),
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
        "configure-icloud" => "Set an iCloud folder if you want to use iCloud.".to_owned(),
        "check-icloud-folder" => {
            "Make sure iCloud Drive is available, then check again.".to_owned()
        }
        "resolve-icloud-file" => {
            "Move or compare the existing iCloud file, then check again.".to_owned()
        }
        "preview-and-approve" => "Review the plan, then Push.".to_owned(),
        "publish-approved" => "Choose Push.".to_owned(),
        other => format!("Next step: {other}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(revision: u64, readiness: &str, reason: &str, next: &str) -> RemoteReadinessDto {
        RemoteReadinessDto {
            revision,
            readiness: DestinationReadinessDto {
                destination: "notion".to_owned(),
                readiness: readiness.to_owned(),
                reason: reason.to_owned(),
                next_step: next.to_owned(),
            },
        }
    }

    #[test]
    fn unchecked_and_checking_are_plain() {
        assert_eq!(
            readiness_block(&ReadinessView::NotChecked),
            "Readiness: Not checked"
        );
        assert_eq!(
            readiness_block(&ReadinessView::Checking),
            "Readiness: Checking..."
        );
    }

    #[test]
    fn a_result_shows_state_reason_and_next_step() {
        let text = readiness_block(&ReadinessView::Checked(report(
            7,
            "not-ready",
            "notion-permission-denied",
            "fix-notion-access",
        )));
        assert!(text.contains("NOT READY"));
        assert!(text.contains("Notion denied access to the configured page."));
        assert!(text.contains("Next: Share the Notion page"));
        let text = readiness_block(&ReadinessView::Checked(report(
            7,
            "ready",
            "none",
            "preview-and-approve",
        )));
        assert!(text.contains("READY"));
        assert!(text.contains("Nothing is blocking this destination."));
        assert!(text.contains("Next: Review the plan, then Push."));
    }

    #[test]
    fn a_result_applies_to_one_remote_and_one_revision() {
        let workflow: WorkflowDto = serde_json::from_value(serde_json::json!({
            "id": "wf_20261029", "diary_day": "2026-10-29", "state": "completed",
            "revision": 7, "buffer_version": 0, "language": null, "style": "concise",
            "assistant_backend_id": null, "composition_path": null, "selection_digest": null,
            "explicit_empty_selection": false, "invalidation_code": null, "entry_text": "",
            "entry_confirmed": true, "candidates": []
        }))
        .unwrap();
        let checked = (
            "wf_20261029".to_owned(),
            "notion".to_owned(),
            report(7, "ready", "none", "none"),
        );
        assert!(matches!(
            ReadinessView::resolve(false, Some(&checked), Some(&workflow), "notion"),
            ReadinessView::Checked(_)
        ));
        assert_eq!(
            ReadinessView::resolve(false, Some(&checked), Some(&workflow), "icloud"),
            ReadinessView::NotChecked
        );
        assert_eq!(
            ReadinessView::resolve(true, None, Some(&workflow), "notion"),
            ReadinessView::Checking
        );
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
}
