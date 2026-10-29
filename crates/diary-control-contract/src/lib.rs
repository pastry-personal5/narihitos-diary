//! Versioned wire contract shared by the local Control API Server and clients.

#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};

pub const API_VERSION: &str = "v1";
pub const CONTRACT_VERSION: u32 = 1;
pub const MAX_REQUEST_BYTES: usize = 1024 * 1024;
pub const MAX_ENTRY_BYTES: usize = 512 * 1024;
pub const MAX_STRING_BYTES: usize = 4 * 1024;
pub const MAX_COLLECTION_ITEMS: usize = 256;
pub const MAX_CANDIDATES: usize = 512;
pub const MAX_SELECTED_SOURCES: usize = 64;
pub const MAX_PREPARED_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_SOURCE_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
pub const WORKFLOW_PAGE_SIZE: usize = 50;
pub const SOURCE_PAGE_SIZE: usize = MAX_COLLECTION_ITEMS;
pub const MAX_WORKFLOWS: usize = 256;
pub const JOB_QUEUE_CAPACITY: usize = 32;
pub const JOB_CONCURRENCY: usize = 4;
pub const MAX_JOBS: usize = 128;
pub const MAX_SUBSCRIBERS: usize = 16;
pub const EVENT_REPLAY_CAPACITY: usize = 256;
pub const SUBSCRIBER_QUEUE_CAPACITY: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RouteSpec {
    pub method: &'static str,
    pub path: &'static str,
    pub capability: Option<&'static str>,
    pub authenticated: bool,
}

pub const ROUTES: &[RouteSpec] = &[
    RouteSpec {
        method: "GET",
        path: "/v1/health",
        capability: None,
        authenticated: false,
    },
    RouteSpec {
        method: "GET",
        path: "/v1/capabilities",
        capability: Some("read"),
        authenticated: true,
    },
    RouteSpec {
        method: "GET",
        path: "/v1/setup-check",
        capability: Some("read"),
        authenticated: true,
    },
    RouteSpec {
        method: "GET",
        path: "/v1/preferences",
        capability: Some("read"),
        authenticated: true,
    },
    RouteSpec {
        method: "PUT",
        path: "/v1/preferences",
        capability: Some("workflow:write"),
        authenticated: true,
    },
    RouteSpec {
        method: "GET",
        path: "/v1/workflows",
        capability: Some("read"),
        authenticated: true,
    },
    RouteSpec {
        method: "POST",
        path: "/v1/workflows",
        capability: Some("workflow:write"),
        authenticated: true,
    },
    RouteSpec {
        method: "GET",
        path: "/v1/workflows/{id}",
        capability: Some("read"),
        authenticated: true,
    },
    RouteSpec {
        method: "DELETE",
        path: "/v1/workflows/{id}",
        capability: Some("workflow:write"),
        authenticated: true,
    },
    RouteSpec {
        method: "POST",
        path: "/v1/workflows/{id}/discoveries",
        capability: Some("source:read"),
        authenticated: true,
    },
    RouteSpec {
        method: "GET",
        path: "/v1/workflows/{id}/sources",
        capability: Some("read"),
        authenticated: true,
    },
    RouteSpec {
        method: "GET",
        path: "/v1/workflows/{id}/sources/{source_id}",
        capability: Some("read"),
        authenticated: true,
    },
    RouteSpec {
        method: "POST",
        path: "/v1/workflows/{id}/sources/prepare",
        capability: Some("source:read"),
        authenticated: true,
    },
    RouteSpec {
        method: "PUT",
        path: "/v1/workflows/{id}/selection",
        capability: Some("workflow:write"),
        authenticated: true,
    },
    RouteSpec {
        method: "POST",
        path: "/v1/workflows/{id}/empty-selection-confirmations",
        capability: Some("workflow:write"),
        authenticated: true,
    },
    RouteSpec {
        method: "PUT",
        path: "/v1/workflows/{id}/writing-settings",
        capability: Some("workflow:write"),
        authenticated: true,
    },
    RouteSpec {
        method: "POST",
        path: "/v1/workflows/{id}/manual-compositions",
        capability: Some("workflow:write"),
        authenticated: true,
    },
    RouteSpec {
        method: "PUT",
        path: "/v1/workflows/{id}/entry",
        capability: Some("workflow:write"),
        authenticated: true,
    },
    RouteSpec {
        method: "POST",
        path: "/v1/workflows/{id}/entry-confirmations",
        capability: Some("workflow:write"),
        authenticated: true,
    },
    RouteSpec {
        method: "GET",
        path: "/v1/jobs/{id}",
        capability: Some("read"),
        authenticated: true,
    },
    RouteSpec {
        method: "DELETE",
        path: "/v1/jobs/{id}",
        capability: Some("workflow:write"),
        authenticated: true,
    },
    RouteSpec {
        method: "GET",
        path: "/v1/events",
        capability: Some("read"),
        authenticated: true,
    },
];

macro_rules! dto {
    ($(#[$meta:meta])* $visibility:vis struct $name:ident { $($body:tt)* }) => {
        $(#[$meta])*
        #[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
        #[serde(deny_unknown_fields)]
        $visibility struct $name { $($body)* }
    };
}

dto!(
    pub struct HealthDto {
        pub api_version: String,
        pub contract_version: u32,
        pub instance_id: String,
        pub build_version: String,
        pub ready: bool,
    }
);

dto!(
    pub struct CapabilitiesDto {
        pub capabilities: Vec<String>,
        pub composition_paths: Vec<CompositionPathDto>,
        pub disabled_routes: Vec<String>,
    }
);

dto!(
    pub struct CompositionPathDto {
        pub id: String,
        pub available: bool,
        pub reason_code: Option<String>,
    }
);

dto!(
    pub struct SetupCheckDto {
        pub config_safe: bool,
        pub database_ready: bool,
        pub apple_helper_available: bool,
        pub notion_configured: bool,
        pub manual_workflow_available: bool,
    }
);

dto!(
    pub struct PreferencesDto {
        pub last_language: Option<String>,
        pub last_composition_path: String,
    }
);

dto!(
    pub struct WorkflowListDto {
        pub workflows: Vec<WorkflowSummaryDto>,
        pub next_cursor: Option<String>,
    }
);

dto!(
    pub struct WorkflowSummaryDto {
        pub id: String,
        pub diary_day: String,
        pub state: String,
        pub revision: u64,
        pub buffer_version: u64,
        pub updated_at_unix_ms: i64,
        pub discarded: bool,
    }
);

dto!(
    pub struct WorkflowDto {
        pub id: String,
        pub diary_day: String,
        pub state: String,
        pub revision: u64,
        pub buffer_version: u64,
        pub language: Option<String>,
        pub style: String,
        pub composition_path: String,
        pub selection_digest: Option<String>,
        pub explicit_empty_selection: bool,
        pub invalidation_code: Option<String>,
        pub entry_text: String,
        pub entry_confirmed: bool,
        pub candidates: Vec<SourceCandidateDto>,
    }
);

dto!(
    pub struct SourceCandidateDto {
        pub id: String,
        pub directory_day: Option<String>,
        pub header_day: Option<String>,
        pub warning_code: Option<String>,
        pub availability: String,
        pub selected: bool,
    }
);

dto!(
    pub struct SourceListDto {
        pub sources: Vec<SourceCandidateDto>,
        pub next_cursor: Option<String>,
    }
);

dto!(
    pub struct SourceDetailDto {
        pub id: String,
        pub body: String,
        pub content_digest: String,
        pub directory_day: Option<String>,
        pub header_day: Option<String>,
        pub warning_code: Option<String>,
    }
);

dto!(
    pub struct StartWorkflowRequest {
        pub diary_day: Option<String>,
    }
);
dto!(
    pub struct DiscoveryRequest {
        pub explicit_date: Option<String>,
    }
);
dto!(
    pub struct PrepareSourcesRequest {
        pub source_ids: Vec<String>,
    }
);
dto!(
    pub struct ConfirmSelectionRequest {
        pub preparation_handle: String,
        pub source_ids: Vec<String>,
    }
);
dto!(
    pub struct WritingSettingsRequest {
        pub language: String,
        pub style: String,
        pub composition_path: String,
    }
);
dto!(
    pub struct SaveEntryRequest {
        pub text: String,
    }
);
dto!(
    pub struct ConfirmEntryRequest {
        pub buffer_etag: String,
    }
);
dto!(
    pub struct JobAcceptedDto {
        pub job_id: String,
    }
);

dto!(
    pub struct JobDto {
        pub id: String,
        pub kind: String,
        pub status: String,
        pub workflow_id: String,
        pub progress_percent: u8,
        pub result_handle: Option<String>,
        pub error_code: Option<String>,
        pub expires_at_unix_ms: i64,
    }
);

dto!(
    pub struct EventDto {
        pub id: u64,
        pub instance_id: String,
        pub event_type: String,
        pub workflow_id: Option<String>,
        pub revision: Option<u64>,
        pub job_id: Option<String>,
        pub progress_percent: Option<u8>,
    }
);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProblemDto {
    #[serde(rename = "type")]
    pub type_uri: String,
    pub title: String,
    pub status: u16,
    pub code: String,
    pub instance: Option<String>,
}

#[must_use]
pub fn workflow_id(day: &str) -> Option<String> {
    if day.len() == 10
        && day.as_bytes()[4] == b'-'
        && day.as_bytes()[7] == b'-'
        && day
            .bytes()
            .enumerate()
            .all(|(index, byte)| index == 4 || index == 7 || byte.is_ascii_digit())
    {
        Some(format!("wf_{}{}{}", &day[0..4], &day[5..7], &day[8..10]))
    } else {
        None
    }
}

#[must_use]
pub fn workflow_day(id: &str) -> Option<String> {
    let raw = id.strip_prefix("wf_")?;
    if raw.len() != 8 || !raw.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    Some(format!("{}-{}-{}", &raw[0..4], &raw[4..6], &raw[6..8]))
}

#[must_use]
pub fn workflow_etag(revision: u64) -> String {
    format!("\"wf-{revision}\"")
}

#[must_use]
pub fn buffer_etag(version: u64) -> String {
    format!("\"buf-{version}\"")
}

#[must_use]
pub fn parse_workflow_etag(value: &str) -> Option<u64> {
    parse_etag(value, "wf-")
}

#[must_use]
pub fn parse_buffer_etag(value: &str) -> Option<u64> {
    parse_etag(value, "buf-")
}

fn parse_etag(value: &str, prefix: &str) -> Option<u64> {
    let inner = value.strip_prefix('"')?.strip_suffix('"')?;
    inner.strip_prefix(prefix)?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_and_strong_etags_have_frozen_grammar() {
        assert_eq!(workflow_id("2026-10-29").as_deref(), Some("wf_20261029"));
        assert_eq!(workflow_day("wf_20261029").as_deref(), Some("2026-10-29"));
        assert_eq!(workflow_etag(7), "\"wf-7\"");
        assert_eq!(buffer_etag(8), "\"buf-8\"");
        assert_eq!(parse_workflow_etag("\"wf-7\""), Some(7));
        assert_eq!(parse_buffer_etag("W/\"buf-8\""), None);
    }

    #[test]
    fn request_dtos_reject_unknown_fields() {
        let error =
            serde_json::from_str::<SaveEntryRequest>(r#"{"text":"synthetic","unexpected":true}"#)
                .unwrap_err();
        assert!(error.to_string().contains("unknown field"));
    }

    #[test]
    fn route_table_excludes_later_milestones() {
        assert!(ROUTES.iter().all(|route| {
            !route.path.contains("mcp")
                && !route.path.contains("publication")
                && !route.path.contains("amend")
                && !route.path.contains("draft")
        }));
        assert_eq!(
            ROUTES.iter().filter(|route| !route.authenticated).count(),
            1
        );
    }
}
