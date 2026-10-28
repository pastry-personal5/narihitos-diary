# P1-M4: Core workflow and persistence overview

Status: Planned

This milestone implements the server-owned diary state machine and durable
local state behind inward-facing ports. It establishes policy before any real
source, model, HTTP, terminal, or output adapter is connected.

## Goal

Make every workflow transition, invalidation, revision check, recovery rule,
and retention rule executable and testable with fake adapters and SQLite.

## In scope

- Core value objects, connector-neutral ports, typed outcomes, and the complete
  workflow state machine.
- Workflow revisions, entry-buffer versions, confirmation and approval binding,
  job state, and per-day reconciliation gating.
- SQLite schema, forward migrations, `WorkflowStore`, `OperationLog`, receipts,
  preferences, draft recovery, discard, and retention purges.
- Synthetic tests for every state transition and voiding rule.

## Out of scope

- Real source, model, HTTP, TUI, local-output, or Notion adapters.
- Choosing the hosted provider or inspecting the configured Notion root.
- External writes or tests containing private diary prose.

## Completion checklist

- [ ] Core types and ports enforce the architecture's dependency direction and
  cannot express credentials, attachment paths, audio, or vendor objects.
- [ ] Tests cover every workflow state, mutation precondition, downstream
  invalidation, empty-source confirmation, blank-entry refusal, and stale
  workflow revision.
- [ ] Entry autosave uses a buffer version independently from workflow revisions,
  and tests cover concurrent and recovered edits.
- [ ] SQLite migrations persist and recover workflows, preferences, receipts,
  operation intents, and safe source snapshots without storing credentials.
- [ ] Retention tests cover verified-output purging, amendment restart, abandoned
  draft purging, explicit discard, and durable receipt/digest preservation.
- [ ] Operation-intent and per-day reconciliation behavior is proven with fake
  destinations, including a simulated crash before receipt storage.
- [ ] Affected architecture and contributor documentation reflects implemented
  contracts without claiming an outer adapter exists.
- [ ] The standard Rust and Swift milestone gate passes and acceptance evidence
  is recorded.
