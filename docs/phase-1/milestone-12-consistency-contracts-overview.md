# P1-M12: Consistency and contract readiness overview

Status: Done

This owner-directed milestone runs after P1-M6 and before P1-M7 despite its
number. It reconciles the active documentation and implemented lower layers
with the canonical UX vocabulary and revised product architecture, so the
terminal client is not built against obsolete manual/assistant contracts.

## Goal

Make the active documentation, core vocabulary, persistence model, and Control
API contract describe one consistent pre-M7 product and close every confirmed
lower-layer gap needed by the manual terminal workflow.

## Entry criteria

- P1-M6 is `Done`; P1-M7 remains `Planned` and no terminal-client dependency or
  macOS TIS FFI packet has been applied.
- Treat [UX terms](../ux-terms.md) and the
  [planned MVP architecture](../architecture.md) as the current
  vocabulary and contract authorities, subject to the owner decisions in
  [AGENTS.md](../../AGENTS.md).
- Review active documents before implementation. Preserve historical research
  claims as evidence, but label superseded conclusions and do not let them
  define current contracts.
- Use only synthetic source notes, workflows, runtime advertisements, and
  assistant descriptors in tests. Do not inspect private diary data, the live
  Notion root, provider credentials, or the Application Support runtime token.

## In scope

- Review every non-archived document for canonical workflow stages, chrome,
  source-note/entry language, confirmation/approval/publication verbs, and the
  assistant-backend versus composition-path distinction. Fix contradictory
  current claims, broken links, and stale milestone ownership.
- Move the assistant-neutral contract foundation from P1-M8 to this milestone:
  manual editing is an always-available workflow action, not a backend or
  composition path; preferences remember an optional stable assistant backend
  ID; and a selected backend, when one exists, owns its hosted or Apple
  on-device composition path.
- Make a deliberate breaking Control API contract revision before the first
  client exists. Capabilities expose manual availability separately from a
  collection of assistant backend descriptors, while workflow and preference
  DTOs use optional backend identity.
- Rename generic `model` state and behavior in current code to `assistant`
  terminology where it does not name Apple's Foundation Models framework or a
  concrete model-provider fact.
- Implement the three confirmed P1-M7 lower-layer gaps: a safe read-only
  runtime-advertisement loader, a server-supplied local source-note name,
  and explicit continue-without-notes directly from a newly started workflow
  when source discovery is unavailable.
- Forward-migrate SQLite state without losing workflows, entry text, source
  snapshots, receipts, or operation intents. Obsolete pre-client assistant-path
  preferences may intentionally become no selected backend.
- Requalify the affected core, SQLite, command, lifecycle, contract, and real
  loopback/process surfaces before P1-M7 becomes active.

## Out of scope

- Ratatui/Crossterm/editor dependencies, terminal rendering, TIS FFI, or any
  other P1-M7 client implementation.
- Selecting a hosted provider, credential location, or hosted HTTP dependency;
  implementing a hosted or Apple assistant adapter; or adding extraction,
  drafting, memory-review, or paragraph-review routes. Those remain P1-M8 work.
- Output connectors, preview, publication, amendments, `/mcp`, or live service
  checks.
- Rewriting historical research prose merely because it uses provider- or
  model-specific terminology. Current summaries must make supersession clear.

## Completion checklist

### Documentation consistency

- [x] Every non-archived document is reviewed. Current product and milestone
  documents use the canonical `Start`, `Sources`, `Compose`, `Review`,
  `Preview`, and `Publish` stages and the Workflow Sidebar, Resources Sidebar,
  Main Pane, Command Line, Status Line, Banner, stage-state, and resource-state
  terms where those concepts apply.
- [x] Current documents consistently use source note for input and entry for
  the finished diary; distinguish activate from include; and use confirm for a
  source/text decision, approve for an exact output plan, and publish for
  executing it. Historical evidence is either still accurate or visibly marked
  as superseded.
- [x] The product architecture, UX documents, API document, phase plan, M6-M8
  plans, README, documentation index, and changelog agree that manual editing
  is not an assistant backend or composition path, backend identity is distinct
  from composition path, and P1-M12 precedes P1-M7.
- [x] Repository-local documentation links, required title/status/summary
  structure, milestone ownership, and current contract/version references pass
  a bounded inspection.

### Core and persistence contracts

- [x] Core types expose stable assistant backend identity and optional selected
  backend state separately from hosted/Apple-on-device composition path. Generic
  active code uses assistant terminology, and an empty confirmed source
  selection disables assistant actions while leaving the Draft editor usable.
- [x] A newly started workflow can reach `SourcesConfirmed` only through the
  explicit empty-selection command when no discovery is possible; ordinary
  empty selection remains rejected and no fake `Discovered` state or candidate
  list is created.
- [x] Every source candidate carries a bounded local source-note name independent
  of its opaque command ID. The Ayumi adapter supplies its relative path; SQLite
  persists it; the Control API returns it only in source candidate/detail
  representations; and diagnostics and assistant request envelopes cannot
  receive it.
- [x] A forward SQLite migration preserves existing workflow, entry, source,
  plan, receipt, and operation data while replacing obsolete manual/path and
  generic model fields with the assistant-neutral representation. Migration
  tests cover every earlier schema version and reopen the result.

### Control and client-attachment readiness

- [x] The revised contract version exposes `manual_workflow_available`
  separately from collection-shaped assistant backend descriptors. Preferences
  and writing settings use an optional backend ID, workflow state reports the
  optional selected backend/path pair, and unknown/unavailable backend IDs fail
  closed without choosing a provider.
- [x] A read-only runtime attachment loader validates the existing directory
  and advertisement file's owner, permissions, type, link count, size, and
  descriptor/path identity without creating state or reading configuration.
  It rejects incompatible contract versions, its token-bearing result has no
  `Debug` representation, and synthetic tests cover missing, malformed,
  unsafe, oversized, and valid files.
- [x] Contract fixtures and real-loopback tests prove the source-note name,
  direct explicit no-notes path, strict unknown-field behavior, capability
  shape, optional backend selection, and mixed contract-version refusal
  assumptions needed by P1-M7.
- [x] The P1-M7 plan no longer asks its implementation to repair lower layers;
  its entry gate points to the completed P1-M12 evidence and keeps only the
  owner-approved dependency/FFI decision gate that belongs to M7.

### Evidence and gate

- [x] Focused synthetic core, SQLite, contract, command, lifecycle, and server
  tests pass, including a privacy-oriented inspection of diagnostics and
  assistant request types.
- [x] Acceptance evidence below records a dated complete gate:
  `cargo fmt --all -- --check`,
  `cargo clippy --all-targets --all-features -- -D warnings`,
  `cargo test --all-features`,
  `swift build --package-path helpers/apple-bridge`, and
  `swift test --package-path helpers/apple-bridge`.

## Acceptance evidence

Fill this in during implementation. Leave checklist items unchecked until the
result is observed; record concise synthetic evidence, not private content or
raw logs.

| Evidence | Result |
|---|---|
| Active-document inventory, terminology/ownership review, and link/status checks | 2026-10-29: reviewed all 46 active Markdown files (README, AGENTS, and 44 docs); title/status/summary and repository-local link checks passed. Bounded legacy-term and full-diff reviews found no live obsolete contract identifier outside intentional migration fixtures. |
| Core assistant-neutral and explicit empty-selection tests | Stable backend IDs, descriptor actions, optional backend/path state, Draft editor fallback, direct/discovered explicit-empty paths, and rejection of unsafe local source-note names passed in focused and full workspace tests. |
| SQLite migration and source-note-name persistence tests | Schema v4 upgraded empty and every prior schema; the populated v3 fixture reopened with workflow text, source-note name, unresolved intent, receipt, and returned ID preserved. Partial backend/path state remains fail-closed. |
| Runtime attachment safety tests | Synthetic missing, malformed, wrong-UID expectation, directory/file mode, type, symlink, hard-link, oversize, token grammar, valid-file, and descriptor/path identity cases passed; the loader creates no state and its token-bearing types omit `Debug`. |
| Contract fixtures and real-loopback/process regressions | Contract version 2, strict legacy-field rejection, descriptor collection shape, manual availability, empty backend list, optional backend selection, automatic rejection of partial/malformed backend-path pairs, source-note name, direct no-notes, and process/loopback lifecycle tests passed. Socket tests retain real-loopback coverage where the host permits binding and distinguish a network-prohibited sandbox from a product failure. |
| Documentation inspection and complete Rust/Swift gate | 2026-10-29 completion review: `cargo fmt --all -- --check`, Clippy with all targets/features and warnings denied, all-feature Rust tests/doctests, Swift build, 10 Swift tests, and `git diff --check` passed. Helper and listener tests were made deterministic under suite load, occupied ports, and restricted test sandboxes. |
