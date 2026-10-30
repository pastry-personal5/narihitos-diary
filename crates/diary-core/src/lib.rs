//! Diary workflow policy and inward-facing ports.
//!
//! This crate intentionally has no adapter, transport, terminal, or vendor
//! dependencies. P1-M4 keeps persistence representations behind the ports in
//! [`ports`].

#![forbid(unsafe_code)]

pub mod assistant;
pub mod assistant_registry;
pub mod digest;
pub mod local_markdown;
pub mod ports;
pub mod prompt_asset;
pub mod publication;
pub mod types;
pub mod workflow;

pub use assistant::*;
pub use assistant_registry::{DraftAssistantRegistry, RegistryError, RegistryInvokeError};
pub use digest::Digest;
pub use local_markdown::render_local_markdown;
pub use ports::{
    Clock, Destination, DraftAssistant, IntentTerminal, LocalOutputError, LocalOutputPort,
    LocalOutputState, OperationLog, SourceCatalog, SourceCheckpointError, SourceReader,
    SourceWriteError, SourceWriter, StartOutcome, StoreError, WorkflowStore,
    revalidate_workflow_sources, start_default_workflow,
};
pub use publication::{PublicationCoordinator, PublicationError, PublicationResult};
pub use types::*;
pub use workflow::{LEGAL_TRANSITIONS, PersistedWorkflow, Workflow, WorkflowError};
