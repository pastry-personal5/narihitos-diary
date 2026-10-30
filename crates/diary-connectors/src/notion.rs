//! Deterministic Notion projection, hierarchy classification, and retry policy.
//!
//! This module deliberately keeps credentials and remote identifiers out of
//! its public preview types. The HTTP adapter consumes the same [`NotionPlan`]
//! that Preview displays and read-back verifies.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use diary_core::{
    DiaryDay, Digest, InspectionOutcome, NotionBlockPreview, NotionOutputPort, NotionPreflight,
    NotionPreflightStatus, NotionReplacePreflight, NotionReplaceRequest, NotionRequest,
    WriteOutcome,
};
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use serde::{Deserialize, Serialize};

pub const API_VERSION: &str = "2026-03-11";
pub const RENDERER_VERSION: u32 = 1;
pub const MAX_RICH_TEXT_CHARS: usize = 2_000;
pub const MAX_APPEND_BLOCKS: usize = 100;
pub const MAX_CHILDREN: usize = 100;

mod projection;
pub use projection::{
    BlockKind, NotionBlock, NotionError, NotionPlan, NotionRichText, NotionTextAnnotations,
    render_notion,
};
use projection::{chunk_blocks, plan_digest};

mod hierarchy;
pub use hierarchy::{
    HierarchyError, HierarchyPlan, NotionPage, PageChild, PageKind, SameDayState,
    classify_same_day, resolve_child,
};

mod retry;
pub use retry::{RetryAction, RetryPolicy, SetupReason, content_free_reason};

#[derive(Clone, Debug)]
struct RemoteBlock {
    id: String,
    block: NotionBlock,
}

trait NotionRemote: Send + Sync {
    fn list_children(
        &self,
        parent_id: &str,
        cancelled: &AtomicBool,
    ) -> Result<Vec<PageChild>, RemoteError>;
    fn list_blocks(
        &self,
        parent_id: &str,
        cancelled: &AtomicBool,
    ) -> Result<Vec<RemoteBlock>, RemoteError>;
    fn create_page(
        &self,
        parent_id: &str,
        title: &str,
        cancelled: &AtomicBool,
    ) -> Result<String, RemoteError>;
    fn append_blocks(
        &self,
        parent_id: &str,
        blocks: &[NotionBlock],
        after: Option<&str>,
        cancelled: &AtomicBool,
    ) -> Result<Vec<RemoteBlock>, RemoteError>;
    /// Moves one block (and its children) to the trash.
    fn trash_block(&self, block_id: &str, cancelled: &AtomicBool) -> Result<(), RemoteError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RemoteError {
    Blocked(SetupReason),
    Ambiguous,
    Pending,
    Invalid,
}

mod output;
pub use output::NotionClient;

mod transport;
use transport::*;

#[cfg(test)]
mod tests;
