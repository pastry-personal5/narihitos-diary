# P1-M8: Draft assistants overview

Status: Planned

This milestone adds a registry of optional assistant backends behind the
existing `DraftAssistant` port while preserving the always-available Draft
editor. It delivers reviewed two-pass drafting and paragraph review without
hard-coding the TUI or workflow to exactly two implementations. It cannot start
until the hosted-provider decisions in `AGENTS.md` are resolved.

## Goal

Offer hosted-default and Apple on-device memory extraction, drafting, and
paragraph review through an extensible backend registry without allowing
assistant output to bypass human review or the approved data-egress limit.

## Decision gate

Before implementation, the owner must choose the hosted provider, credential
storage, and HTTP dependency. Record those decisions in `AGENTS.md` and the
Phase 1 changelog; do not infer them from this plan.

## Entry criteria

- P1-M7 is `Done`, the phase plan marks P1-M8 `Active`, and the implemented TUI
  stage/resource model, Control API contract, preferences, and SQLite schema
  have been inspected.
- The owner-approved hosted-provider decision includes the credential location,
  HTTP dependency and features, license/Rust compatibility, request limits,
  timeout/cancellation behavior, and a safe test strategy. Any manifest or
  Swift-package change follows the normal dependency approval rule.
- P1-M12's assistant-neutral contract and persistence migration are complete.
  Any further contract version or schema change is planned explicitly before
  changing DTOs or stored preferences. Automated tests use fake providers and
  synthetic prose; no live model or private diary is used.

## In scope

- A collection-shaped backend registry with stable IDs, labels, composition
  paths, availability, language support, and per-action capabilities. The first
  production members are the chosen hosted default and Apple on-device adapter;
  additional approved adapters do not require new workflow stages.
- Source-linked memory extraction, memory review, drafting, inferred-reflection
  markers, paragraph review with accept/reject, validation, cancellation,
  timeouts, and manual fallback.
- Swift helper model commands and the selected hosted adapter.
- Assistant-backend and language preferences plus visible backend/path labels;
  the backend descriptor owns composition path.
- Type-level and runtime enforcement of the hosted-data egress envelope.

## Out of scope

- Automatic confirmation, output approval, publication, and agent-client MCP.
- A Codex-backed production adapter unless its process/API, credential,
  dependency, privacy, and lifecycle choices receive separate owner approval.
- Sending front matter, metadata, paths, audio, credentials, or Notion details.
- Model use for a confirmed empty source selection.

## Completion checklist

- [ ] The hosted provider, credential location, and HTTP dependency have explicit
  owner approval and are documented before manifests or adapters change.
- [ ] The registry exposes a collection rather than fixed hosted/Apple fields;
  tests register at least three fake backends and prove selection and routing by
  stable ID with no UI or workflow branch per backend.
- [ ] The P1-M12 manual-availability, backend-descriptor, optional-selection,
  and migrated preference contracts remain intact while assistant routes and
  result representations are added. Mixed-version clients fail closed.
- [ ] The chosen hosted and Apple production adapters implement the same
  `DraftAssistant` contract and report availability, language support,
  per-action capabilities, timeouts, cancellation, and invalid output as typed
  outcomes that preserve manual editing in the Draft editor.
- [ ] Tests prove that hosted requests can contain only readable bodies,
  memories, drafts, edited text or the focused paragraph, language, and
  style—not excluded data classes.
- [ ] Assistant adapters have no shell, filesystem, connector, confirmation,
  approval, or publication tools. A synthetic source containing hostile
  instructions can produce only a validated suggestion and no side effect.
- [ ] Extraction returns source-linked memories for review, and drafting consumes
  only confirmed memories and never confirms its own result. A request reaches
  exactly one selected backend and contains only the currently confirmed source
  snapshots, never excluded, merely active, or stale sources.
- [ ] Paragraph review binds a suggestion to the entry-buffer version, byte
  range, and selected-text digest. Accept applies a still-current replacement;
  reject leaves the entry byte-for-byte unchanged; a stale suggestion cannot be
  applied.
- [ ] Inferred reflections remain visibly delimited and block entry confirmation
  until reviewed; malformed or oversized output is rejected safely.
- [ ] Apple helper tests cover availability, unsupported locale, crash, timeout,
  cancellation, and schema-invalid output without calling a live model.
- [ ] Inside the existing Compose stage, the TUI lists all registered backends
  and labels the selected backend and path in Setup, Main Pane, Work Status, and
  Status Line. The middle Resources Sidebar keeps the draft, confirmed sources,
  and assistant suggestions reachable. A completed assistant draft becomes
  active as editable text in the Main Pane; manual editing remains available
  after any failure.
- [ ] Synthetic end-to-end composition tests cover both production adapter
  contracts, multi-backend selection, paragraph accept/reject, and source
  freshness checks before each assistant call.
- [ ] User-facing privacy/composition documentation names the chosen provider
  behavior and the exact egress boundary.
- [ ] The standard Rust and Swift milestone gate passes and acceptance evidence
  is recorded.

## Acceptance evidence

Fill this in during implementation. Leave checklist items unchecked until the
result is observed; record synthetic evidence only.

| Evidence | Result |
|---|---|
| Owner-approved provider, credential, HTTP, and migration packet | Pending |
| Registry and three-fake routing tests | Pending |
| P1-M12 contract compatibility and any M8 migration tests | Pending |
| Hosted egress-envelope and production-adapter contract tests | Pending |
| Apple helper fake availability/failure tests | Pending |
| Drafting, paragraph accept/reject, freshness, and TUI vertical flows | Pending |
| Documentation inspection and full Rust/Swift gate | Pending |
