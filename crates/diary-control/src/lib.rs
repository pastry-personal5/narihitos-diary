//! Secure loopback Control API Server and application-command boundary.

#![forbid(unsafe_code)]

mod codex_assistant;
mod commands;
mod diagnostic_tags;
mod diagnostics;
mod lifecycle;
mod server;

pub use codex_assistant::{
    CodexAppServerTransport, CodexAssistant, CodexInvocation, CodexLaunch, CodexTransportError,
    ProcessCodexTransport,
};
pub use commands::{
    AmendmentNotionPreview, AmendmentPreview, AmendmentSeeded, CommandError, CommandService,
    ControlStore, NotionPreview, local_body,
};
pub use diagnostic_tags::{DiagnosticTag, DiagnosticTagger};
pub use diagnostics::{
    AssistantSnapshot, DiagnosticEvent, DiagnosticLevel, DiagnosticLog, LogClock, ValidationReport,
};
pub use lifecycle::{
    CodexProfileConfig, LifecycleConfig, LifecycleError, NotionRuntimeConfig, RuntimeAdvertisement,
    RuntimeAttachment, RuntimeFile, ServerLock, load_or_create_tag_key, load_runtime_attachment,
    random_id,
};
pub use server::{ServerConfig, ServerHandle, bind_loopback, build_router, serve};
