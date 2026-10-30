//! Versioned wire contract shared by the local Control API Server and clients.

#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};

pub const API_VERSION: &str = "v1";
pub const CONTRACT_VERSION: u32 = 8;
/// Covers one maximum-size source body plus a bounded JSON envelope. Individual
/// routes still enforce their narrower action-specific limits.
pub const MAX_REQUEST_BYTES: usize = 8 * 1024 * 1024 + 64 * 1024;
pub const MAX_ENTRY_BYTES: usize = 512 * 1024;
pub const MAX_STRING_BYTES: usize = 4 * 1024;
pub const MAX_COLLECTION_ITEMS: usize = 256;
pub const MAX_CANDIDATES: usize = 512;
pub const MAX_SELECTED_SOURCES: usize = 64;
pub const MAX_MEMORY_CANDIDATES: usize = 128;
pub const MAX_MEMORY_CANDIDATE_BYTES: usize = MAX_STRING_BYTES;
pub const MAX_REVIEWED_MEMORY_BYTES: usize = MAX_ENTRY_BYTES;
pub const MAX_SOURCE_ORDINALS: usize = MAX_SELECTED_SOURCES;
pub const MAX_DRAFT_RESULT_BYTES: usize = MAX_ENTRY_BYTES;
pub const MAX_PARAGRAPH_BYTES: usize = MAX_STRING_BYTES * 8;
pub const MAX_SUGGESTION_EXPLANATION_BYTES: usize = MAX_STRING_BYTES;
pub const MAX_REFLECTION_RANGES: usize = 32;
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

/// The `JobDto::status` values the server emits. Clients branch on these
/// constants rather than literals so a renamed status fails to compile
/// instead of silently never matching. Timeouts are `failed` with an
/// `error_code`, never a separate status.
pub mod job_status {
    pub const QUEUED: &str = "queued";
    pub const RUNNING: &str = "running";
    pub const CANCELLATION_REQUESTED: &str = "cancellation-requested";
    pub const COMPLETED: &str = "completed";
    pub const FAILED: &str = "failed";
    pub const CANCELLED: &str = "cancelled";

    /// Whether a job in `status` will never change again.
    #[must_use]
    pub fn is_terminal(status: &str) -> bool {
        matches!(status, COMPLETED | FAILED | CANCELLED)
    }

    /// Whether a job in `status` still occupies queue or worker capacity.
    #[must_use]
    pub fn is_active(status: &str) -> bool {
        matches!(status, QUEUED | RUNNING | CANCELLATION_REQUESTED)
    }
}

/// The `ProblemDto::code` for a job ID the server does not (or no longer)
/// knows, e.g. after its terminal result expired.
pub const JOB_NOT_FOUND: &str = "job-not-found";

mod dto;
mod routes;
pub use dto::*;
pub use routes::*;

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
mod tests;
