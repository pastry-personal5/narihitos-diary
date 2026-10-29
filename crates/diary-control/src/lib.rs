//! Secure loopback Control API Server and application-command boundary.

#![forbid(unsafe_code)]

mod codex_assistant;
mod commands;
mod diagnostics;
mod lifecycle;
mod server;

pub use codex_assistant::{
    CodexAppServerTransport, CodexAssistant, CodexInvocation, CodexLaunch, CodexTransportError,
    ProcessCodexTransport,
};
pub use commands::{CommandError, CommandService, ControlStore};
pub use diagnostics::{DiagnosticEvent, DiagnosticLevel, DiagnosticLog, LogClock};
pub use lifecycle::{
    CodexProfileConfig, LifecycleConfig, LifecycleError, RuntimeAdvertisement, RuntimeAttachment,
    RuntimeFile, ServerLock, load_runtime_attachment, random_id,
};
pub use server::{ServerConfig, ServerHandle, bind_loopback, build_router, serve};
