//! Outbound adapters for Narihito's Diary.

#![forbid(unsafe_code)]

mod apple_bridge;
mod ayumi;
mod local_output;
mod sqlite;

pub use apple_bridge::{
    AppleBridgeClient, AppleFoundationAssistant, BridgeAvailability, BridgeRead,
};
pub use ayumi::{
    AyumiCatalog, AyumiSource, Discovery, ReadOutcome, SourceReadError, SourceWriteError,
};
pub use local_output::{LocalError, LocalOutput, LocalState};
pub use sqlite::{MigrationEvent, MigrationObserver, SqliteStore};
