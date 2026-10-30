# P1-M18: Clean architecture refactor plan

Status: Done

This plan moves responsibilities across existing Phase 1 modules in reviewable
slices. The dependency direction in [architecture.md](../architecture.md)
remains the rule: TUI → Control API → application/core, with adapters at the
outer edge.

## Current pressure points

The inventory on 2026-10-30 found these production files carrying several
concerns together: `apps/nd/src/main.rs` (about 4,900 lines, including a
roughly 2,800-line connected-session loop), `diary-connectors/src/sqlite.rs`
(about 4,100), `diary-core/src/workflow.rs` (about 3,800),
`diary-control/src/server.rs` (about 3,700),
`diary-connectors/src/notion.rs` (about 3,400), and
`diary-control/src/commands.rs` (about 2,600). These counts locate work; they
are not targets or acceptance thresholds. Tests inside the files contribute
to the totals and should follow their subject when modules move.

| Boundary | Current mixed responsibilities | Intended owner |
|---|---|---|
| `apps/nd` / `diary-tui` | Process startup, mutable session state, worker results, key dispatch, and screen strings in `main.rs` | `apps/nd` assembles and launches; `diary-tui` owns session model, reducer, effects, and views |
| `diary-control/server.rs` / `commands.rs` | Route handlers, job state, SSE, DTO conversion, and application decisions in broad modules | Transport modules validate/map; command modules execute use cases; core owns invariants |
| `diary-control-contract/lib.rs` | Shared wire DTOs, route metadata, validation, and fixtures in one module | Resource-grouped contract modules with one exported version and strict parsing rules |
| `diary-core/workflow.rs` | Every workflow state, artifact, source, publication, and amendment transition in one module | Cohesive workflow submodules share one aggregate and explicit invariant helpers |
| `diary-connectors/sqlite.rs` | Migrations, SQL, row encoding/decoding, workflow store, operation log, and tests | Schema/migrations, codecs, workflow repository, operation repository, each using the same store connection policy |
| `diary-connectors/notion.rs` | Markdown rendering, hierarchy, HTTP/retry, publication, and reconciliation | Projection, hierarchy, transport, and output adapter modules with one canonical plan format |

## Implementation inventory

| Slice | Public seam and owning rule | Baseline evidence | Interface change expected |
|---|---|---|---|
| TUI | `Worker` sends typed `Intent`; `run_connected_session` currently owns reducer, pending state, focus, and rendering | `diary-tui` unit tests, `apps/nd/tests/control_api_assembly.rs`, `tui_worker.rs` | Internal module and controller API only |
| Control API | `build_router` owns transport; `CommandService` owns use cases; `Workflow` owns revisions and invalidation | `diary-control` tests and loopback tests under `apps/nd/tests` | Internal route/DTO module layout; no wire break planned |
| Core | `Workflow` and its persisted record own state transitions, artifact bindings, and publication state | `diary-core` workflow tests, core assembly tests | Internal module layout; no state or persisted-record break planned |
| SQLite | `SqliteStore` implements `WorkflowStore` and `OperationLog`; migrations are append-only | `diary-connectors` SQLite reopen and migration tests | Internal module layout; no schema change planned |
| Notion | `NotionClient` implements `NotionOutputPort`; `NotionPlan` defines the shared projection | `diary-connectors` renderer, hierarchy, retry, and interruption tests | Internal module layout; no Notion payload change planned |

The baseline is the full gate recorded in P1-M17; M18 will run focused tests
while moving each seam and its own full gate at completion. No wire or schema
break is currently required; if one becomes useful, record its exact before
and after contract here before implementation.

### Implemented ownership (2026-10-30)

| Slice | Owner after refactor | Public seam and evidence |
|---|---|---|
| Binary and TUI | `apps/nd/src/main.rs` assembles the process; `apps/nd/src/tui` attaches the client. `diary-tui/session_runtime` owns connected-session state and worker/input dispatch; `session` reduces SSE outcomes; `screen_body` and existing view modules render. | `run_connected_session` is exported by `diary-tui`; the binary injects the paragraph digest function. Existing TUI session, input, and assembly tests pass. |
| Control API | `server` retains router, authentication, and shared state; `server/{workflow_routes,source_routes,assistant_routes,publication_routes,event_routes,jobs,dto}` own transport concerns. `commands` and its resource modules own use cases, including saved assistant selection at workflow start. | `build_router` and wire routes retain their signatures. Control service and loopback tests pass. |
| Contract | `routes` owns route inventory; `dto` owns versioned wire types and validation; `tests` owns contract fixtures. | Public reexports and contract version remain unchanged; contract tests pass. DTOs stay together because their nested workflow and publication representations are shared across resource routes. |
| Core | `workflow/{sources,assistants,composition,publication_flow,amendments,invariants}` group transitions around the same `Workflow` aggregate. | Persisted workflow shape is unchanged; core workflow tests pass. |
| SQLite | `sqlite/{schema,workflow_store,operation_log,write_records,read_records,retention,intent_codec,value_codec}` separate SQL and encoding roles. | `SqliteStore` ports and schema/migrations are unchanged; reopen and migration tests pass. |
| Notion | `notion/{projection,hierarchy,retry,transport,output}` separate rendering, path checks, HTTP policy, and writes. | `NotionClient` port and payload projection are unchanged; fake HTTP and ambiguous-write tests pass. |

The connected-session dispatcher remains one large function (about 2,800
lines). It owns one terminal session's ordered worker results, key dispatch,
and local pending state; its order is part of editor recovery and job safety.
Extracting an all-state parameter object solely to shorten it would obscure
those relationships. Event reduction, rendering, client attachment, and tests
have separate owners, so the remaining length is a localized review cost
rather than mixed crate ownership. `server.rs` also retains a substantial
router/auth/shared-state body; its resource handlers, jobs, and DTO mapping
have moved out. No API wire shape, contract version, SQLite schema, or Notion
payload changed. The proposed typed TUI command effects and resource-grouped
DTO split were unnecessary for these boundaries and are not introduced.

## Refactor rules

1. Preserve observable Phase 1 behavior and data safety. Before moving a rule,
   name its owner and the test that proves it. Add a characterization test only
   for a concrete gap; do not copy implementation into tests.
2. Move code by responsibility rather than by a line quota. A module may stay
   large when its contents form one cohesive operation; a smaller module still
   needs a clear reason to exist. Avoid forwarding-only layers and giant
   parameter bundles that merely hide the old function.
3. Keep one source of truth for workflow revision, entry buffer version,
   approval bindings, cancellation state, and publication intents. Rendering
   and HTTP handlers must not replicate those rules.
4. Use typed commands/results across the TUI worker and server use-case seams.
   Network, SQLite, Notion, Apple, and Codex calls remain in their existing
   outer layers. No private diary content is added to diagnostics or paths.
5. Breaking an internal or wire API is allowed. If a wire shape changes,
   advance the contract version deliberately and update all callers, fixtures,
   health negotiation, and docs in the same slice. If stored data changes,
   append a migration and prove old-database reopen; never edit an applied
   migration or discard user data.
6. Treat the configured Notion root as undecided and untouched. Use synthetic
   services and temporary stores only. Ask separately before changing
   dependencies, build tools, or another indexed decided behavior.

## Sequence and review packets

1. **Baseline and map.** Record the module dependency graph, public exports,
   large-function responsibilities, existing test locations, and any gaps.
   Run affected baseline tests. Keep the planned moves below flexible only
   where the inventory gives a concrete reason and record departures here.
2. **TUI.** Move the connected-session state and event/result reducer out of
   `apps/nd/src/main.rs`; make key handling produce typed effects and let one
   controller execute them on the existing bounded worker. Move screen-body
   builders and focus helpers to `diary-tui` presentation modules. Keep editor
   unsent text, cursor, pending jobs, and canonical reload ownership explicit.
   Verify scripted input, resize, reconnection, and source/assistant flows.
3. **Control API.** Group handlers and wire DTOs by
   workflow/source/assistant/publication, then isolate job scheduling, SSE
   replay, auth/validation, and DTO mapping.
   Move any remaining rule decisions into command use cases. Preserve
   preconditions, typed problems, bounded jobs, and content-free diagnostics.
   Verify route fixtures and loopback tests after each group.
4. **Core and persistence.** Group workflow methods by source selection,
   composition/artifacts, publication, and amendment with common invariant
   helpers. Then split SQLite schema/migrations, row codecs, workflow store,
   and operation log. Keep the aggregate and transaction boundaries explicit;
   use reopen, stale-revision, crash-boundary, and migration tests.
5. **Notion connector.** Extract Markdown projection and canonical digest,
   hierarchy resolution, HTTP queue/retry, and output write/reconcile logic.
   Ensure Preview, write, and read-back still share one representation. Use the
   existing fake HTTP and interruption tests; no live hierarchy access.
6. **Cross-layer cleanup.** Remove unused exports and duplicated mappings,
   update public API and architecture docs, and review remaining large files
   for mixed ownership. Run the full Rust and Swift gate once at the end, then
   update the overview checklist with evidence.

Each packet keeps code, focused tests, and affected docs together. If one
packet exposes a behavior defect, add the smallest regression test and fix it
within the packet; a new product decision remains a separate owner question.

## Verification boundaries

- Core tests exercise state/invalidation without HTTP, SQLite, or terminal.
- Repository tests use temporary SQLite databases and old-schema fixtures;
  they verify round-trip, compare-and-swap, receipts, and reopen.
- Connector tests use synthetic Notion responses, fake Codex/Apple boundaries,
  and temporary files; they verify projection, retry, and ambiguous writes.
- Server tests exercise strict DTOs, auth, conditional writes, jobs, SSE, and
  diagnostic records through loopback without live providers.
- TUI tests drive typed events and render with an in-memory terminal, plus the
  existing pty checks where process behavior matters. They verify that
  rendering stays pure and worker effects remain bounded.
