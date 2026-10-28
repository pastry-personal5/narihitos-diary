# P1-M2: Software architecture plan

Status: Active

This plan identifies the affected boundaries and planning sequence. The
technology-neutral product design is the [planned MVP architecture](../planned-mvp-architecture.md).

## Affected modules and interfaces

- Shared Rust domain and application use cases, including connector ports.
- The local Control API Server and its REST, event, and future MCP transports.
- Source, composition, persistence, and destination adapters.
- A TUI client with a local HTTP client and MVVM presentation state.

## Approach and sequence

1. Derive invariant workflow and data contracts from the completed research and
   owner decisions.
2. Define ports and dependency direction before selecting server or database
   libraries.
3. Design the server-owned workflow state, REST resources, streaming events,
   local authorization, and conflict handling.
4. Place the TUI and future MCP surface outside the use cases, then map the
   pipeline and connector-specific failure behavior.
5. Review the design against synthetic source, output, and concurrent-client
   scenarios and keep the unresolved Notion page check at runtime.
