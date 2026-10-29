//! Outbound adapters for Narihito's Diary.

#![forbid(unsafe_code)]

mod apple_bridge;
mod ayumi;
mod sqlite;

pub use apple_bridge::{
    AppleBridgeClient, AppleFoundationAssistant, BridgeAvailability, BridgeRead,
};
pub use ayumi::{
    AyumiCatalog, AyumiSource, Discovery, ReadOutcome, SourceReadError, SourceWriteError,
};
pub use sqlite::SqliteStore;
