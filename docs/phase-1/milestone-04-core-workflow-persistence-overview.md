# P1-M4: Core workflow and persistence overview

Status: Done

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
- Use only dependencies and build tools approved through P1-M3 plus the
  P1-M4 SQLite packet below. Any further dependency, Cargo feature, or build
  tool needs owner approval first.
- The hosted provider remains undecided and the configured Notion root remains
  uninspected; neither is needed for this milestone. No live page inspection or
  provider choice is implied by this plan.

## Approved dependency packet

The owner approved this packet on 2026-10-28 before manifest changes:

| Owning package | Direct dependency | Requirement and features | Purpose | Compatibility and alternative |
|---|---|---|---|---|
| `diary-connectors` | `rusqlite` | `0.40.2`; default system-SQLite linkage; no optional features | Implement transactional SQLite migrations, compare-and-swap storage, intents, and receipts. | Rust 1.88 MSRV fits the project's Rust 1.89 minimum. `rusqlite`, `libsqlite3-sys`, and the small default Rust dependency set use Apache-2.0-compatible permissive licenses. Direct SQLite FFI was rejected because it would require unsafe adapter code and duplicate maintained bindings; bundled SQLite was rejected to avoid a C compilation step and `cc` build dependency. |

No additional formatter, generator, task runner, hosted provider, credential
scheme, HTTP dependency, or Swift package was approved.

## In scope

- Connector-neutral value objects, ports, typed outcomes, and transition policy
  for the [workflow state table](../architecture.md#workflow-states-and-invalidation).
- Source-selection and freshness outcomes supplied by fakes, the explicit
  empty-selection path, manual entry confirmation, and placeholders for later
  reviewed drafting without implementing an assistant adapter.
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

### Completion-review hardening

- [x] Retry commands implement the architecture's real state edges: a pending
  destination can re-enter `Publishing` only with an unchanged rechecked plan,
  while a blocked remote destination requires a new destination preview and
  destination-scoped approval.
- [x] Source and destination identities are unambiguous and unique in digest
  bindings, and restored workflows reject inconsistent state, plan, approval,
  progress, or confirmation records.
- [x] Operation-log tests prove that only exact duplicate attempts are
  idempotent, receipts must match their unresolved intents, and an `Absent`
  reconciliation resolves the old intent before a safe retry.
- [x] Discard and retention advance both concurrency versions when they clear
  text, leave recoverable workflow records internally valid, preserve receipts
  and unresolved intents, and permit a later workflow change to restart the
  retention clock.

### Core contract and transition policy

- [x] Core values and ports compile independently of SQLite, HTTP, terminal,
  Swift, and vendor packages. The assistant-facing request types contain only
  allowed composition text and choices; they have no front-matter, credential,
  attachment-path, audio, or Notion-root fields.
- [x] A table-driven synthetic test enters every state in the product
  architecture, checks each legal transition and refused precondition, and
  proves each upstream mutation's specified downstream voiding rule while
  backward navigation alone changes nothing.
- [x] Tests cover selection include/exclude regardless of date evidence,
  changed and unverifiable freshness outcomes, retention of unconfirmed user
  edits after invalidation, explicit continuation with zero selected notes,
  disabled assistant actions for that case, and refusal to confirm a blank entry or
  unresolved inferred-reflection marker.
- [x] A diary day has at most one current workflow: choosing an unfinished day
  resumes it, and choosing a completed day preserves its receipt for the later
  amendment path. A fixed-clock test checks the Mac-local default day.
- [x] At M4 completion, the last language and composition-path choices
  persisted while a new entry defaulted to concise style and hosted
  composition. P1-M12 supersedes that pre-client representation with an
  optional assistant backend ID and backend-owned path. With no remembered
  language, the core requires an explicit choice rather than inventing a
  default. Changing language or style stales assistant material without
  discarding manually edited text.

### Revisions, storage, and recovery

- [x] Conditional workflow mutations reject stale revisions atomically. Entry
  autosave checks a separate buffer version; tests cover concurrent autosaves,
  a confirmation racing with autosave, edits after confirmation, and recovered
  versions after reopening SQLite.
- [x] Plans and approvals bind the confirmed entry, selection/source digests,
  target identity, exact intended destination state and payload digest, and
  inspected destination state. Tests prove a changed input or new plan
  invalidates approval, a blocked remote retry requires new destination-only
  preview and approval, and a pending remote retry reuses approval only while
  the rechecked plan digest is unchanged.
- [x] Forward-only SQLite migrations work from an empty database and on reopen,
  with upgrade tests for any prior schema version introduced in this milestone.
  Reopen recovers unfinished workflow state, buffer, safe source snapshots,
  preferences, intents, and versioned receipts with no secret or prohibited
  source metadata persisted.
- [x] Concurrent store operations preserve compare-and-swap and intent/receipt
  transaction boundaries. A corrupt or unsupported schema fails explicitly
  without silently resetting user state.

### Recovery, retention, and evidence

- [x] Fake destinations prove intent-before-write ordering, crash recovery
  after an external effect but before receipt storage, duplicate-command
  handling, core-visible job cancellation state, and per-day `Reconciling`
  gates that leave other days usable.
- [x] A fixed-clock test covers the seven-day purge after every output verifies,
  the 30-day abandoned-draft purge, explicit discard, a new verified amendment
  version restarting retention, and preservation of receipts, digests, and
  unresolved intents.
- [x] Affected architecture and contributor docs describe the implemented
  contracts and leave later adapter behavior with its owning milestone.
- [x] Acceptance evidence below records focused test results and a dated full
  gate: `cargo fmt --all -- --check`, `cargo clippy
  --all-targets --all-features -- -D warnings`, `cargo test
  --all-features`, `swift build --package-path helpers/apple-bridge`, and
  `swift test --package-path helpers/apple-bridge`.

## Acceptance evidence

The fixtures below contain invented prose and temporary paths only. No test
opens the production Application Support database, reads Ayumi notes, or calls
an external destination.

| Evidence | Result |
|---|---|
| Core dependency and allowed-data inspection | On 2026-10-28, `cargo metadata --no-deps --format-version 1` reported zero `diary-core` dependencies, Rust 1.89, and Apache-2.0. Source inspection found the assistant request limited to sanitized note bodies, reviewed memories, draft/edited text, language, and style; the SQL schema contains no credential, front-matter, location, weather, attachment, audio, or Notion-root field. |
| State, freshness, revision, and approval tests | Twelve `diary-core` tests passed. They cover all ten states and legal forward edges, refused preconditions, navigation, all freshness checkpoints, empty selection, explicit language, editor races, invalidation, exact plan bindings, local-first outcomes, changed-plan/entry revocation, pending reuse, blocked retry approval, and cancellation state. |
| Migration, reopen, concurrency, and privacy tests | Seven `diary-connectors` tests plus the `nd` assembly test passed against temporary databases. Evidence covers empty creation, v1-to-v2 upgrade, repeat open, unsupported/corrupt failure without reset, preferences/workflow/plan recovery, real concurrent compare-and-swap, independent buffer conflicts, and safe schema fields. |
| Intent, reconciliation, and retention tests | Fake-destination tests passed for durable intent-before-effect ordering, crash-window reopen, per-day reconciliation, cancellation, matching read-back, duplicate commands, transactional receipts, seven- and 30-day boundary purges, explicit discard, repeated purge, amendment-version retention restart, and preservation of receipt digests and unresolved intents. |
| Full Rust and Swift gate, with date and tool versions | Passed on 2026-10-28: `cargo fmt --all -- --check`, warnings-denied workspace Clippy, `cargo test --all-features` (7 connector, 12 core, 4 command-shell, and 1 assembly tests), Swift build, Swift test (8 cases), and P1-M4-scoped `git diff --check`. Versions: rustc/Cargo 1.98.0, rustfmt 1.9.0, Clippy 0.1.98, Swift 6.4, Xcode 27.1 (27A9275). |
| Completion-review hardening | On 2026-10-29, the full gate passed with 15 core, 13 connector, 4 command-shell, 1 assembly, and 8 Swift tests, plus all doctests, formatting, warnings-denied workspace Clippy, both Swift build/test commands, and `git diff --check`. Added destination-scoped pending retries, verified-destination non-regression, restored approval-scope validation, exact intent/receipt/reconciliation checks, dual-version discard checks, and a discard-then-edit retention-clock restart. |
