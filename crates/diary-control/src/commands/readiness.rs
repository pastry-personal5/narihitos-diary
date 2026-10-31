//! Read-only readiness checks for each remote.

use super::*;
use std::time::Instant;

use crate::diagnostics::NOTION_PREVIEW_REASONS;

/// How ready one destination is. Mirrors the wire values.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Readiness {
    Ready,
    NotReady,
    Published,
}

impl Readiness {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::NotReady => "not-ready",
            Self::Published => "published",
        }
    }
}

/// One destination's answer: closed tokens only, never a path or response.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DestinationReadiness {
    pub destination: &'static str,
    pub readiness: Readiness,
    pub reason: &'static str,
    pub next_step: &'static str,
    /// The check that settled the answer (a closed diagnostics token).
    pub decided_by: &'static str,
    pub duration_ms: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublishReadiness {
    pub revision: u64,
    pub destinations: Vec<DestinationReadiness>,
    pub local_verified: bool,
    pub notion_configured: bool,
    pub icloud_configured: bool,
    pub duration_ms: u64,
}

impl CommandService {
    /// Reports why Notion and the iCloud mirror are or are not ready. Writes
    /// nothing: no plan change, no intent, no workflow revision.
    pub fn check_publish_readiness(
        &self,
        day: &DiaryDay,
        expected_revision: u64,
    ) -> Result<PublishReadiness, CommandError> {
        let started = Instant::now();
        self.ensure_clear(day)?;
        let workflow = self.load_workflow(day)?;
        ensure_revision(&workflow, expected_revision)?;
        let local_verified = progress_of(&workflow, "local") == Some(DestinationStatus::Verified);
        let destinations = if local_verified {
            vec![
                timed(|| self.notion_readiness(day, &workflow)),
                timed(|| self.mirror_readiness(day, &workflow)),
            ]
        } else {
            vec![
                local_unverified("notion"),
                local_unverified(MIRROR_DESTINATION),
            ]
        };
        Ok(PublishReadiness {
            revision: workflow.revision(),
            destinations,
            local_verified,
            notion_configured: self.notion_output.is_some(),
            icloud_configured: self.mirror_output.is_some(),
            duration_ms: elapsed_ms(started),
        })
    }

    fn notion_readiness(&self, day: &DiaryDay, workflow: &Workflow) -> DestinationReadiness {
        let status = progress_of(workflow, "notion");
        let Some(notion) = self.notion_output.as_ref() else {
            return not_ready(
                "notion",
                "notion-not-configured",
                "configure-notion",
                "not-configured",
            );
        };
        match notion.preflight(day, &workflow.entry().text) {
            Err(reason) => notion_not_ready(&reason, "preflight-error"),
            Ok(preflight) => match preflight.status {
                NotionPreflightStatus::Ready => ready("notion", status, "preflight-status"),
                NotionPreflightStatus::Blocked => notion_not_ready(
                    preflight.reason.as_deref().unwrap_or("notion-unavailable"),
                    "preflight-status",
                ),
                NotionPreflightStatus::Existing => match notion.observe_existing(day) {
                    Ok(Some(_))
                        if self.notion_section_is_ours(day)
                            && self.notion_receipt_matches_entry(day, workflow) =>
                    {
                        published("notion")
                    }
                    Ok(Some(_)) => ready("notion", None, "preflight-status"),
                    _ => notion_not_ready(
                        preflight
                            .reason
                            .as_deref()
                            .unwrap_or("notion-same-day-conflict"),
                        "preflight-status",
                    ),
                },
            },
        }
    }

    fn mirror_readiness(&self, day: &DiaryDay, workflow: &Workflow) -> DestinationReadiness {
        let status = progress_of(workflow, MIRROR_DESTINATION);
        let Some(mirror) = self.mirror_output.as_ref() else {
            return not_ready(
                MIRROR_DESTINATION,
                "icloud-not-configured",
                "configure-icloud",
                "not-configured",
            );
        };
        let payload = match self.local_final_text(workflow) {
            Ok(payload) => payload,
            Err(_) => return local_unverified(MIRROR_DESTINATION),
        };
        match mirror.inspect(day, &payload) {
            Err(_) => not_ready(
                MIRROR_DESTINATION,
                "icloud-folder-unavailable",
                "check-icloud-folder",
                "mirror-inspect",
            ),
            // Existing iCloud content is not an obstacle: the Remote menu
            // offers a backed-up Overwrite for it.
            Ok(LocalOutputState::Identical) => published(MIRROR_DESTINATION),
            Ok(LocalOutputState::Absent | LocalOutputState::Different(_)) => {
                ready(MIRROR_DESTINATION, status, "mirror-inspect")
            }
        }
    }
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// Stamps how long one destination's check took.
fn timed(check: impl FnOnce() -> DestinationReadiness) -> DestinationReadiness {
    let started = Instant::now();
    let mut item = check();
    item.duration_ms = elapsed_ms(started);
    item
}

pub(super) fn progress_of(workflow: &Workflow, destination: &str) -> Option<DestinationStatus> {
    workflow
        .destination_progress()
        .iter()
        .find(|item| item.destination == destination)
        .map(|item| item.status)
}

fn ready(
    destination: &'static str,
    status: Option<DestinationStatus>,
    decided_by: &'static str,
) -> DestinationReadiness {
    DestinationReadiness {
        destination,
        readiness: Readiness::Ready,
        reason: "none",
        // An approved, unchanged plan publishes directly; otherwise the plan
        // still needs its own preview and approval.
        next_step: if status == Some(DestinationStatus::Pending) {
            "publish-approved"
        } else {
            "preview-and-approve"
        },
        decided_by,
        duration_ms: 0,
    }
}

fn published(destination: &'static str) -> DestinationReadiness {
    DestinationReadiness {
        destination,
        readiness: Readiness::Published,
        reason: "already-published",
        next_step: "none",
        decided_by: "published",
        duration_ms: 0,
    }
}

fn not_ready(
    destination: &'static str,
    reason: &'static str,
    next_step: &'static str,
    decided_by: &'static str,
) -> DestinationReadiness {
    DestinationReadiness {
        destination,
        readiness: Readiness::NotReady,
        reason,
        next_step,
        decided_by,
        duration_ms: 0,
    }
}

fn local_unverified(destination: &'static str) -> DestinationReadiness {
    not_ready(
        destination,
        "local-file-not-verified",
        "publish-local-first",
        "local-gate",
    )
}

/// Collapses a connector reason onto the closed `notion-*` set; anything
/// unlisted becomes `notion-unavailable` so free text never reaches a client.
fn known_notion_reason(reason: &str) -> &'static str {
    NOTION_PREVIEW_REASONS
        .iter()
        .copied()
        .find(|known| *known == reason && known.starts_with("notion-"))
        .unwrap_or("notion-unavailable")
}

fn notion_not_ready(reason: &str, decided_by: &'static str) -> DestinationReadiness {
    let reason = known_notion_reason(reason);
    let next_step = match reason {
        "notion-not-configured" => "configure-notion",
        "notion-credential-invalid" | "notion-permission-denied" | "notion-root-unverifiable" => {
            "fix-notion-access"
        }
        "notion-config-invalid" | "notion-hierarchy-ambiguous" | "notion-same-day-conflict" => {
            "check-notion-page"
        }
        _ => "retry-later",
    };
    not_ready("notion", reason, next_step, decided_by)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::{READINESS_DECIDERS, READINESS_NEXT_STEPS, READINESS_REASONS};

    fn assert_closed(item: &DestinationReadiness) {
        assert!(READINESS_REASONS.contains(&item.reason), "{}", item.reason);
        assert!(
            READINESS_NEXT_STEPS.contains(&item.next_step),
            "{}",
            item.next_step
        );
        assert!(
            READINESS_DECIDERS.contains(&item.decided_by),
            "{}",
            item.decided_by
        );
    }

    #[test]
    fn every_notion_reason_maps_into_the_closed_vocabularies() {
        for reason in NOTION_PREVIEW_REASONS.iter().copied().chain(["free text"]) {
            assert_closed(&notion_not_ready(reason, "preflight-error"));
        }
    }

    #[test]
    fn fixed_answers_use_closed_tokens() {
        for item in [
            ready("notion", None, "preflight-status"),
            ready(
                MIRROR_DESTINATION,
                Some(DestinationStatus::Pending),
                "mirror-inspect",
            ),
            published("notion"),
            local_unverified(MIRROR_DESTINATION),
            not_ready(
                "notion",
                "notion-not-configured",
                "configure-notion",
                "not-configured",
            ),
        ] {
            assert_closed(&item);
        }
    }
}
