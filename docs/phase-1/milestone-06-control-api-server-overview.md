# P1-M6: Control API Server overview

Status: Planned

This milestone exposes the completed core and source adapters through a
loopback Control API Server and makes `nd serve` a recoverable foreground
process. It supplies the transport needed by P1-M7 without moving diary policy
into HTTP handlers.

## Goal

Provide the versioned REST, SSE, job, lifecycle, and local authorization
contract through which the Phase 1 TUI uses diary workflows.

## Entry criteria

- P1-M4 and P1-M5 are `Done`, and the phase plan marks P1-M6 `Active` before
  implementation. Inspect their implemented ports, source outcomes, SQLite
  schema, and Swift-helper behavior rather than assuming planned signatures.
- Before changing a manifest, Cargo feature, or build tool, prepare an
  owner-reviewed dependency packet: each direct dependency and feature set,
  owning package, purpose, Rust 1.89 and license compatibility, transitive
  concerns, and the standard-library or already-approved alternative. Include
  any schema generator or test tool. Tokio is the decided server runtime, but
  its crate version and features and all HTTP/JSON/locking dependencies still
  need approval. Record the approval in the Phase 1 changelog.
- Use synthetic workflows, temporary configuration and state, fake source
  adapters, and fake destinations in tests. Do not inspect the private Ayumi
  tree or configured Notion root, or select a hosted provider in this milestone.

## In scope

- Foreground startup and bounded shutdown, single-instance lock, owner-only
  configuration and runtime files, production database path, migrations, purge,
  and background intent reconciliation.
- Loopback-only binding with bounded port fallback, ephemeral control token,
  capability checks, `Host`/`Origin` validation, bounded requests and work,
  safe problem details, and content-free diagnostics.
- Versioned DTOs and routes for the completed manual workflow: readiness,
  preferences, start/resume/list/discard, discovery, source preparation and
  confirmation, entry autosave and confirmation, jobs, and SSE notifications.
- Typed contract fixtures and in-process plus real-loopback transport tests for
  the later TUI client.

## Out of scope

- `/mcp`, remote access, browser clients, agent tokens, and a claim that a
  same-user process holding the control token is a human.
- Terminal screens and model extraction/drafting (P1-M7/P1-M8); real local or
  Notion output routes and connectors (P1-M9/P1-M10); amendments (P1-M11).
  Routes for those features remain unavailable until their owning milestones.
- Broadening the listener beyond `127.0.0.1`, selecting the hosted provider,
  or inspecting private source or Notion data during tests.

## Completion checklist

### Process and security

- [ ] An isolated `nd serve` process test proves lock, configuration checks,
  migration and purge ordering, listener readiness before runtime-file
  publication, duplicate-process refusal, stale-file recovery, background
  reconciliation, and bounded signal shutdown. An unresolved intent gates only
  its diary day and survives restart when its destination is unavailable.
- [ ] Listener tests prove `127.0.0.1` binding, the configured bounded sequence
  beginning at `8771`, fallback and exhausted-range behavior. Runtime directory
  and file ownership/modes, atomic replacement, token rotation, and cleanup
  after clean shutdown are checked; a crashed server's leftover file is never
  trusted without the lock and matching health instance.
- [ ] Tests reject absent or unsafe configuration before opening the database
  or listener. They prove the Notion credential never enters runtime data,
  health, DTOs, errors, process arguments, or logs.
- [ ] Real HTTP tests cover version/build discovery, bearer authorization and
  capability denial, loopback `Host` and `Origin` checks, request and concurrency
  bounds, no-store responses, safe RFC 9457 errors, and content-free logs.

### Workflow contract

- [ ] A reviewed `/v1` route and DTO table covers every P1-M7 manual-flow
  action and maps each to an implemented use case. Unsupported model, output,
  amendment, and MCP actions are not advertised or accidentally callable;
  route handlers contain no source, confirmation, approval, or publication
  policy.
- [ ] Multi-client tests prove atomic workflow `If-Match` checks, separate
  entry-buffer `If-Match` checks, confirmation against both current versions,
  stable status/problem mappings, and no mutation after a stale request.
- [ ] Job and SSE tests prove accepted work, bounded execution, progress,
  timeout, cancellation requests, event ordering, reconnect and missed-event
  recovery by canonical state refetch. SSE contains no diary text or credential.
- [ ] Contract fixtures/schema and startup/client-attachment documentation
  match the implementation, including runtime-file and health negotiation,
  disabled later routes, and `/mcp` being outside Phase 1.

### Evidence and gate

- [ ] Acceptance evidence below records focused tests and a dated full gate:
  `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features -- -D warnings`,
  `cargo test --all-features`, `swift build --package-path helpers/apple-bridge`,
  and `swift test --package-path helpers/apple-bridge`.

## Acceptance evidence

Fill this in during implementation. Leave checklist items unchecked until
their results are observed; record concise synthetic evidence, not raw logs.

| Evidence | Result |
|---|---|
| P1-M4/P1-M5 entry gate and owner-approved dependency packet | Pending |
| Process lifecycle, filesystem permissions, and isolated restart tests | Pending |
| Loopback, authentication, origin, bounds, and redaction tests | Pending |
| DTO, multi-client revision, job, and SSE tests | Pending |
| Documentation and contract inspection | Pending |
| Full Rust and Swift gate, with date and tool versions | Pending |
