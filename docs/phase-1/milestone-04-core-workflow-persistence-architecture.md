# P1-M4: Core workflow and persistence architecture plan

Status: Done

This plan turns the [product architecture](../planned-mvp-architecture.md) into
an executable core and a durable SQLite adapter. Implementation starts after
P1-M2 and P1-M3 are done and the needed dependencies are approved.

## Implemented shape

- `diary-core` is standard-library only. Its `types`, `workflow`, `ports`,
  `publication`, and `digest` modules own validated values, the single
  transition/invalidation policy, SHA-256 bindings, inward ports, and
  intent-before-effect orchestration. It has no SQL, path, HTTP, Swift,
  terminal, or vendor type.
- `diary-connectors::SqliteStore` implements both `WorkflowStore` and
  `OperationLog`. Each call opens the test-selected database, enables foreign
  keys, and uses immediate transactions for migrations, compare-and-swap,
  child-record replacement, and intent-to-receipt completion.
- Schema version 1 owns workflows, independent entry-buffer versions, safe
  candidates/snapshots, reviewed memories, and preferences. Version 2 adds
  immutable plans, typed local/remote targets, per-target progress, unresolved
  intents, and versioned receipts. `PRAGMA user_version` is forward-only;
  unknown newer, corrupt, and invalid records fail without reset.
- `apps/nd/tests/p1_m4_assembly.rs` is the only composition-root assembly in
  this milestone. It uses a temporary database and starts no listener, real
  Application Support database, helper, source connector, or destination.

The production database path, process lock, worker scheduling, and transport
remain P1-M6 work. Real source, drafting, local-file, and Notion behavior
remain with P1-M5 and P1-M8 through P1-M11.

## Boundaries and deliverables

| Boundary | P1-M4 responsibility | Later owner |
|---|---|---|
| `diary-core` | Values, inward ports, use cases, typed outcomes, transition and invalidation policy, conditional revisions, plan/approval rules, and per-destination publication policy | Real source/model/output behavior in P1-M5 and P1-M8–M11 |
| `diary-connectors` | SQLite `WorkflowStore` and `OperationLog` with forward migrations and tests against temporary databases | Ayumi, Apple, local Markdown, and Notion connectors in later milestones |
| `apps/nd` | Test-only assembly proving core and SQLite work together | Production database path, server lifecycle, and listener in P1-M6 |
| Control contract and TUI | No new wire DTOs or screens | P1-M6 and P1-M7 |

Keep SQL records, paths, HTTP DTOs, terminal state, Swift types, and vendor
objects out of `diary-core`. Core tests use in-memory fakes. SQLite tests use
invented text in temporary databases and reopen the files to test durability.
P1-M4 may define core-visible job status and cancellation-request state;
worker scheduling, timeouts, and event delivery belong to P1-M6.

## Core contract

1. Define stable connector-namespaced source and destination references, diary
   day, writing language/style, source date evidence, selected source snapshots,
   user edits, confirmed entry, plan, intent, and versioned receipt values.
   Prefer typed constructors or validation that reject impossible states.
2. Keep source-reference identity separate from assistant input. The
   `DraftAssistant` request shape can carry only sanitized readable note
   bodies, reviewed memories, drafts or edited entry text, language, and style.
   It has no field for raw front matter, location, weather, IDs, attachment
   paths, audio, credentials, or the Notion root. M4 supplies fake assistant
   outcomes only; P1-M8 supplies the adapters.
3. Use `Clock` for local-day defaults, timestamps, and retention;
   `Settings` for nonsecret configuration; and `WorkflowStore` for
   preferences. Define safe typed outcomes for `Changed`,
   `Unverifiable`, `NeedsDownload`, `Unreadable`, `Conflict`,
   `AmbiguousWrite`, and `Reconciling`; adapter details stay behind the
   ports and error messages remain content-free.
4. Keep source eligibility and inclusion distinct. Date evidence proposes
   notes, including directory/header disagreements; the person can include or
   exclude any discovered note. The catalog request distinguishes bounded
   automatic discovery from an explicitly requested extra date, never an
   archive-wide query. M5 enforces Ayumi's D-1/D/D+1 window, parses real
   metadata, and measures freshness. M4 consumes synthetic evidence and stamps
   and never treats a failed read or unverifiable stamp as an empty body.

## Transition and concurrency policy

Implement the [state table](../planned-mvp-architecture.md#workflow-states-and-invalidation)
as one transition policy rather than independent handler checks. Each
state-changing command requires the expected workflow revision. A successful
mutation advances it exactly once and applies the state table's specific
downstream voiding rule; a refused command leaves persisted state unchanged.
Navigation alone has no effect. Key
workflows by diary day: choosing an unfinished day resumes it; a completed day
keeps its receipts and offers the amendment seam implemented in P1-M11.

A confirmed selection contains immutable snapshots for exactly the included
sources. Zero selected sources require the separate explicit "continue without
notes" action. That path enters `SourcesConfirmed` with an empty selection,
disables assistant extraction, drafting, and paragraph review, and still
permits the manual editor.
Source `Changed` or `Unverifiable` at a required checkpoint clears
confirmation, model results, output preview, and approval; retained user edits
become unconfirmed. Use fake freshness results at the post-read,
pre-drafting-pass, pre-entry-confirmation, pre-preview, and pre-first-write
checkpoints. M5 supplies the real metadata/digest checks.

The entry buffer has its own monotonically increasing version. Autosave compares
that version and never advances the workflow revision. If an edit follows entry
confirmation, the same buffer compare-and-swap invalidates the confirmation,
preview, and approval bindings without turning the edit into a workflow-revision
conflict. Entry confirmation checks both the workflow revision and buffer
version in the same atomic update, so a racing autosave cannot confirm an older
buffer. It refuses blank text and unresolved inferred-reflection delimiters. A
language/style change stales model material while preserving manual text. Store
the last language and composition-path choices. Concise and hosted are the
specified initial defaults; no initial writing-language default has been
decided, so the core must represent its absence until the person chooses one.

An output plan is immutable once previewed. Bind its approval to the current
workflow revision, confirmed entry digest, selected source IDs/digests, target
identities, destination state, and exact intended payload digests. A new plan
or any changed bound input revokes approval. Verified local success does not by
itself revoke unchanged approval for a pending remote destination: a pending
retry may reuse it only while a destination recheck leaves the plan digest
unchanged. A blocked remote destination requires a new destination-only preview
and approval. The core enforces local-first ordering and per-destination
outcomes: a blocked remote target does not block local approval/write; a local
conflict stops the entire plan; a remote conflict stops that destination only.
Only fake connectors exercise these rules in M4. P1-M9 and P1-M10 supply exact
payload rendering, preflight, and real writes.

## SQLite and recovery

Design the logical records and their ownership before choosing table names:

| Record | Durable content and rule |
|---|---|
| Workflow | Day, state, workflow revision, timestamps, selected source references and safe snapshots, invalidation reason, and confirmed digests |
| Entry buffer | Recoverable edited text, buffer version, and confirmation binding |
| Preferences | Last writing language and composition path; style defaults to concise for a new entry |
| Plan and per-target status | Immutable plan/approval bindings and each destination's pending, blocked, conflicting, or verified result |
| Operation intent | Plan ID, day, destination, payload digest, write attempt identity, and reconciliation status, persisted before an external effect |
| Receipt | Version, entry/source/payload digests, destination identity and returned IDs, verification status and time, without duplicate prose |

The database stores recoverable sanitized source and draft text only while
retention permits it. Never persist credentials, raw front matter, location,
weather, attachment paths, or audio in these records. Keep receipts and
digests after text purges. The SQLite adapter owns its physical schema; only
domain values cross the store ports.

Use forward-only migrations with a recorded schema version. Test new-database
creation and repeat open, plus upgrade from any earlier schema version actually
introduced during M4. Migration failure, unknown newer schema, corruption, or
an invalid record must produce a safe error without resetting or overwriting
the database. Store operations use
transactions for conditional revisions, buffer compare-and-swap, confirmation,
intent insertion, and terminal receipt updates. Reopen tests must recover
unfinished edits, preferences, plan status, intents, and receipt versions.

The production Application Support location and process lock are assembled in
P1-M6. M4 tests the SQLite adapter at a temporary path and must not create a
real user database when its tests run.

## Fake publication and crash sequence

For a fake destination, record the intent durably before invoking its write.
Record a terminal outcome only after read-back establishes the effect or the
adapter returns a definite no-effect failure; ambiguity leaves the intent
unresolved. Test a crash after the fake external effect but before outcome
storage, then reopen SQLite. An
unresolved intent gates commands for its own diary day as `Reconciling` while
other days remain usable. Reconciliation reads the fake destination before
retrying and distinguishes matching output, conflicting output, and an
unavailable or ambiguous result. A duplicate client command must not create a
second effect or bypass a pending intent. Cancellation requests do not roll
back an effect; they wait for reconciliation.

Do not encode a local-file no-clobber algorithm, Notion hierarchy rule, or
amendment replacement sequence in the SQLite adapter. Those are P1-M9,
P1-M10, and P1-M11 connector responsibilities. M4 persists the intent and
receipt information those connectors will need.

## Retention and discard

Use the injected clock to schedule purge of recoverable draft and source text
seven days after every intended output is verified. A partial or blocked
publication is not an all-verified result. Purge abandoned drafts after 30 days
without a workflow change, and support explicit discard. Discard and purge
must not erase an unresolved write intent. Preserve durable receipt IDs,
versions, digests, and timestamps. A later verified amendment
receipt version restarts the seven-day clock; M4 proves this with a synthetic
receipt, while P1-M11 implements amendment editing and guarded replacement.
Test boundary instants, reopen before purge, and repeated purge calls.

## Implementation and verification sequence

1. Confirm the entry criteria in the overview; inspect the P1-M3 workspace and
   approved dependency record. Request approval before any newly needed
   dependency, Cargo feature, or build tool.
2. Add core value types and narrow ports, then compile `diary-core` alone.
   Use type-level and fixture inspection to check the assistant data boundary.
3. Implement the transition table and invalidation tests with fake source,
   assistant, clock, and destination outcomes. Include every legal edge,
   precondition failure, stale revision, zero-source case, and freshness
   checkpoint.
4. Implement independent buffer compare-and-swap, confirmation binding,
   immutable plans, approval checks, and core-visible job status. Test racing
   updates and reopen behavior.
5. Add SQLite migrations and transactional store adapters. Test an empty
   database, reopen, upgrades for any schema predecessor introduced in M4,
   concurrent updates, corruption/unsupported schema, and absence of
   prohibited fields.
6. Add the fake write/receipt/reconcile sequence and fixed-clock retention.
   Inject crashes at each durable boundary and verify per-day gating.
7. Update affected docs with actual contracts, record acceptance evidence in
   the overview, and run the full Rust and Swift gate once after focused tests
   pass. Leave P1-M4 `Planned` until its implementation is authorized by the
   milestone order; mark it `Done` only when every checklist item has evidence.
