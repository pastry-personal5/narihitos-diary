//! Diary workflow policy and inward-facing ports.
//!
//! This crate intentionally has no adapter, transport, terminal, or vendor
//! dependencies. P1-M4 keeps persistence representations behind the ports in
//! [`ports`].

#![forbid(unsafe_code)]

pub mod digest;
pub mod ports;
pub mod publication;
pub mod types;
pub mod workflow;

pub use digest::Digest;
pub use ports::{
    Clock, Destination, IntentTerminal, OperationLog, SourceCatalog, SourceCheckpointError,
    SourceReader, StartOutcome, StoreError, WorkflowStore, revalidate_workflow_sources,
    start_default_workflow,
};
pub use publication::{PublicationCoordinator, PublicationError, PublicationResult};
pub use types::*;
pub use workflow::{LEGAL_TRANSITIONS, PersistedWorkflow, Workflow, WorkflowError};
