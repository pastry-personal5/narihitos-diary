# P1-M4: Core workflow and persistence overview

Status: Planned

This milestone implements the server-owned diary state machine and private
SQLite state behind inward-facing ports. Synthetic adapters exercise the
workflow contract before later milestones connect real sources, drafting,
transports, or destinations.

## Goal

Make workflow transitions, revision checks, invalidation, recovery, and
retention executable and testable without an external service or private data.

## Entry criteria

- P1-M2 and P1-M3 are `Done`, and the phase plan marks P1-M4 `Active`
  before implementation starts.
- Use only dependencies and build tools approved through P1-M3. Any new
  dependency, Cargo feature, or build tool needs owner approval first.
- The hosted provider remains undecided and the configured Notion root remains
  uninspected; neither is needed for this milestone. No live page inspection or
  provider choice is implied by this plan.

## In scope

- Connector-neutral value objects, ports, typed outcomes, and transition policy
  for the [workflow state table](../planned-mvp-architecture.md#workflow-states-and-invalidation).
- Source-selection and freshness outcomes supplied by fakes, the explicit
  empty-selection path, manual entry confirmation, and placeholders for later
  reviewed drafting without implementing a model.
- Workflow revisions and independent entry-buffer versions, with atomic stale
  update rejection and confirmation/approval bound to exact state and digests.
- Core publication orchestration exercised with fake destinations: per-target
  status, write-ahead intents, receipts, uncertain-write reconciliation, and a
  gate for only the affected diary day.
- A forward-migrated private SQLite `WorkflowStore` and `OperationLog`,
  recoverable text, preferences, versioned receipts, discard, and retention.

## Out of scope

- Ayumi parsing, iCloud reads, freshness measurement, and the Swift command
  group (P1-M5); HTTP and server jobs (P1-M6); the TUI (P1-M7).
- Hosted or Apple drafting (P1-M8), real local output and publication UI
  (P1-M9), Notion publication (P1-M10), and amendment editing or replacement
  (P1-M11). M4 only provides the storage and policy seams they require.
- A production database startup path, external writes, private diary prose in
  fixtures, or a live Notion compatibility check.

## Completion checklist

### Core contract and transition policy

- [ ] Core values and ports compile independently of SQLite, HTTP, terminal,
  Swift, and vendor packages. The assistant-facing request types contain only
  allowed composition text and choices; they have no front-matter, credential,
  attachment-path, audio, or Notion-root fields.
- [ ] A table-driven synthetic test enters every state in the product
  architecture, checks each legal transition and refused precondition, and
  proves each upstream mutation's specified downstream voiding rule while
  backward navigation alone changes nothing.
- [ ] Tests cover selection include/exclude regardless of date evidence,
  changed and unverifiable freshness outcomes, retention of unconfirmed user
  edits after invalidation, explicit continuation with zero selected notes,
  disabled model steps for that case, and refusal to confirm a blank entry or
  unresolved inferred-reflection marker.
- [ ] A diary day has at most one current workflow: choosing an unfinished day
  resumes it, and choosing a completed day preserves its receipt for the later
  amendment path. A fixed-clock test checks the Mac-local default day.
- [ ] The last language and composition-path choices persist, while a new
  entry defaults to concise style and hosted composition. With no remembered
  language, the core requires an explicit choice rather than inventing a
  default. Changing language or style stales model material without discarding
  manual text.

### Revisions, storage, and recovery

- [ ] Conditional workflow mutations reject stale revisions atomically. Entry
  autosave checks a separate buffer version; tests cover concurrent autosaves,
  a confirmation racing with autosave, edits after confirmation, and recovered
  versions after reopening SQLite.
- [ ] Plans and approvals bind the confirmed entry, selection/source digests,
  target identity, exact intended destination state and payload digest, and
  inspected destination state. Tests prove a changed input or new plan
  invalidates approval, a blocked remote retry requires new destination-only
  preview and approval, and a pending remote retry reuses approval only while
  the rechecked plan digest is unchanged.
- [ ] Forward-only SQLite migrations work from an empty database and on reopen,
  with upgrade tests for any prior schema version introduced in this milestone.
  Reopen recovers unfinished workflow state, buffer, safe source snapshots,
  preferences, intents, and versioned receipts with no secret or prohibited
  source metadata persisted.
- [ ] Concurrent store operations preserve compare-and-swap and intent/receipt
  transaction boundaries. A corrupt or unsupported schema fails explicitly
  without silently resetting user state.

### Recovery, retention, and evidence

- [ ] Fake destinations prove intent-before-write ordering, crash recovery
  after an external effect but before receipt storage, duplicate-command
  handling, core-visible job cancellation state, and per-day `Reconciling`
  gates that leave other days usable.
- [ ] A fixed-clock test covers the seven-day purge after every output verifies,
  the 30-day abandoned-draft purge, explicit discard, a new verified amendment
  version restarting retention, and preservation of receipts, digests, and
  unresolved intents.
- [ ] Affected architecture and contributor docs describe the implemented
  contracts and leave later adapter behavior with its owning milestone.
- [ ] Acceptance evidence below records focused test results and a dated full
  gate: `cargo fmt --all -- --check`, `cargo clippy
  --all-targets --all-features -- -D warnings`, `cargo test
  --all-features`, `swift build --package-path helpers/apple-bridge`, and
  `swift test --package-path helpers/apple-bridge`.

## Acceptance evidence

Fill this in during implementation. Keep every checklist item unchecked until
its result is observed; use synthetic fixtures and concise results rather than
raw logs.

| Evidence | Result |
|---|---|
| Core dependency and allowed-data inspection | Pending |
| State, freshness, revision, and approval tests | Pending |
| Migration, reopen, concurrency, and privacy tests | Pending |
| Intent, reconciliation, and retention tests | Pending |
| Full Rust and Swift gate, with date and tool versions | Pending |
