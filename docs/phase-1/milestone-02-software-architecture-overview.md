# P1-M2: Software architecture overview

Status: Active

This milestone defines a reviewable software architecture for the macOS diary
application. Its product architecture is recorded separately so that it can be
read without the milestone plan.

## Goal

Specify the boundaries, workflows, and interfaces needed for a local control
server, a TUI client, and extensible input and output connectors.

## In scope

- Research the relevant architecture principles and current HTTP/MCP transport
  contracts.
- Define dependency direction, module responsibilities, connector ports, and
  workflow state and failure semantics.
- Specify the local control API, TUI MVVM role, event streaming, and the future
  MCP integration boundary.
- Preserve the owner-approved source, composition, output, and privacy rules.

## Out of scope

- Production Rust code or dependencies; those belong to subsequent implementation.
- Inspecting or changing the configured Notion root page.
- Publishing an entry or modifying private source data.

## Completion checklist

- [x] The architecture document states the dependency rule and assigns each
  concern to a module with a testable boundary.
- [x] Input and output connector contracts support additional implementations
  without changing diary policy or the TUI.
- [x] The control server contract covers REST resources, streaming, concurrency,
  authorization, process ownership, and ports beginning at 8771.
- [x] The TUI's MVVM responsibilities, the workflow pipeline, and a future MCP
  Streamable HTTP endpoint are specified without bypassing review or consent.
- [x] Source freshness, draft recovery, local-first publication, Notion
  reconciliation, and the unresolved root-page preflight are preserved.
- [x] The architecture has current primary-source references, is indexed, and
  contains no phase or milestone references.
- [ ] The required gate passes: `cargo fmt --all -- --check`,
  `cargo clippy --all-targets --all-features -- -D warnings`, and
  `cargo test --all-features`.

## Acceptance evidence

The checked design items are evidenced by the
[architecture proposal](../planned-mvp-architecture.md), its
[index entry](../README.md), and the owner direction in [AGENTS.md](../../AGENTS.md).
Local link validation found no missing links, the requested terminology check
found no phase or milestone references in the architecture proposal, and
`git diff --check` passed. The gate item remains unchecked because the
repository still lacks a Cargo manifest.
