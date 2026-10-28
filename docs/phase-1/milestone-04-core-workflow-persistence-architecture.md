# P1-M4: Core workflow and persistence architecture plan

Status: Planned

This plan implements the invariant center of the architecture before any
connector or transport can accidentally own diary policy.

## Affected modules and interfaces

- `diary-core`: value objects, use cases, ports, state transitions, revision and
  invalidation policy, jobs, output plans, and typed outcomes.
- `diary-connectors`: SQLite implementations of `WorkflowStore` and
  `OperationLog` only.
- `apps/nd`: database location and adapter assembly for tests or later startup.

## Approach and sequence

1. Define core values and port conversations without filesystem paths, SQL
   records, HTTP DTOs, terminal state, or vendor responses.
2. Implement the workflow states in order, centralizing preconditions and
   downstream invalidation so use cases cannot skip review or approval.
3. Separate the rapidly saved entry buffer from workflow revisions and bind
   confirmations and output approvals to immutable revisions and digests.
4. Design a forward-only SQLite schema and migrations for workflow revisions,
   snapshots, preferences, operation intents, and versioned receipts.
5. Implement startup recovery, per-day reconciliation gates, retention, and
   discard through injected `Clock` and fake destination ports.
6. Prove the whole state table and crash boundaries with deterministic,
   synthetic tests before connecting real adapters.

SQLite remains an outer adapter: core tests use in-memory fakes, and SQL types
do not cross the persistence ports.
