//! Outbound adapters for Narihito's Diary.

#![forbid(unsafe_code)]

mod apple_bridge;
mod ayumi;
mod local_output;
mod notion;
mod sqlite;

pub use apple_bridge::{
    AppleBridgeClient, AppleFoundationAssistant, BridgeAvailability, BridgeRead,
};
pub use ayumi::{
    AyumiCatalog, AyumiSource, Discovery, ReadOutcome, SourceReadError, SourceWriteError,
};
pub use local_output::{LocalError, LocalOutput, LocalState};
pub use notion::{
    API_VERSION as NOTION_API_VERSION, BlockKind as NotionBlockKind, HierarchyError, HierarchyPlan,
    NotionBlock, NotionClient, NotionError, NotionPage, NotionPlan, NotionRichText,
    NotionTextAnnotations, PageChild, PageKind, RENDERER_VERSION as NOTION_RENDERER_VERSION,
    RetryAction, RetryPolicy, SameDayState, render_notion,
};
pub use sqlite::{MigrationEvent, MigrationObserver, SqliteStore};
