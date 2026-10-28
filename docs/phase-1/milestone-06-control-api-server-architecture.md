# P1-M6: Control API Server architecture plan

Status: Planned

This plan makes the control transport an inbound adapter around the completed
P1-M4 use cases and P1-M5 source connectors. It defines the manual-workflow
contract needed by P1-M7 while reserving later behavior for its own milestone.

## Preconditions and boundaries

Implementation begins only after P1-M4 and P1-M5 are `Done`, P1-M6 is `Active`,
and the dependency packet in the [overview](milestone-06-control-api-server-overview.md#entry-criteria)
has owner approval. Compare this plan with the actual core ports, SQLite
schema, source outcomes, and helper protocol at that point. The
[product architecture](../planned-mvp-architecture.md#control-api-server-module)
owns the decided server lifecycle and REST resource intent.

| Boundary | P1-M6 responsibility |
|---|---|
| `diary-control-contract` | Versioned JSON DTOs, opaque IDs, wire revisions, safe problems, event/job shapes, and contract fixtures. It remains independent of core and adapters. |
| `diary-control` | Route table, authentication and capability checks, request limits, DTO validation/mapping, bounded jobs, SSE fan-out, and content-free error translation. It calls core use cases. |
| `apps/nd` | `nd serve` composition root, configuration and state paths, lock/runtime files, adapter registration, listener selection, signals, and shutdown. |
| `diary-core` | Existing workflow and publication policy only; no HTTP, Tokio, JSON, or transport types. Narrow port changes require focused core tests. |
| `diary-connectors` | Existing SQLite and source adapters; no route or token handling. |

Use Tokio for the HTTP/event layer as already decided. Schedule blocking
SQLite calls, hashing, helper-process waits, and source reads on bounded
workers. A blocked worker must not stall the event loop. Do not add a direct
dependency from the TUI to server internals or from core to transport.

## Contract and feature boundary

Freeze `/v1` around the operations needed by the P1-M7 manual workflow.
Health returns only instance ID, contract version, and build ID without
authentication. The TUI later checks the instance ID against the owner-only
runtime file and verifies a supported contract version before sending its
bearer token; the build ID identifies the server for diagnostics. All other
routes, including SSE, require authentication. `GET` has no side effects.
Responses, including errors and SSE, use `Cache-Control: no-store`.

| Routes available in P1-M6 | Core action and wire requirement |
|---|---|
| `GET /v1/health`, `GET /v1/capabilities`, `GET /v1/setup-check` | Negotiate instance/build/contract, advertise only working operations, and report safe readiness flags without credentials or private paths. |
| `GET /v1/preferences`, `PUT /v1/preferences` | Read and update remembered choices through core preferences; validate values and expose unavailable composition paths honestly. |
| `GET /v1/workflows`, `POST /v1/workflows`, `GET /v1/workflows/{id}`, `DELETE /v1/workflows/{id}` | List unfinished first, start or resume by diary day, fetch canonical state, and discard through core policy. A completed day is not amended here. |
| `POST /v1/workflows/{id}/discoveries`, `GET /v1/workflows/{id}/sources` | Request bounded or explicitly dated discovery and return candidate references, availability, distinct date evidence, warnings, and sanitized readable text when already readable; never download an unselected cloud-only note. |
| `POST /v1/workflows/{id}/sources/prepare`, `PUT /v1/workflows/{id}/selection` | Request selected-only cloud acquisition and confirm exact fresh snapshots. Zero selection needs an explicit continue-without-notes action, not an empty array interpreted as consent. |
| `PUT /v1/workflows/{id}/entry`, `POST /v1/workflows/{id}/entry-confirmations` | Save the entry buffer conditionally and confirm only the current nonblank, marker-free entry against both workflow and buffer versions. |
| `GET /v1/jobs/{id}`, `DELETE /v1/jobs/{id}`, `GET /v1/events?workflow_id={id}` | Inspect/request cancellation of bounded work and receive revisioned notifications; reconnect by fetching canonical state. |

Do not register model extraction/drafting routes until P1-M8, output-plan and
publication routes until P1-M9/P1-M10, or amendment routes until P1-M11.
`/mcp` stays absent for Phase 1. DTOs may reserve versioned types for these
later resources, but an advertised capability must correspond to a working
adapter and route. Never expose M4 fake destinations as production output.

Define a mapping table from each wire request to the implemented use-case
command and from every typed core outcome to a stable HTTP status/problem
code. Include date and language validation, opaque source references, source
warnings, confirmed-selection digest, workflow revision, independent buffer
version, and invalidation reason in the contract where the TUI needs them.
Raw front matter, audio references, private absolute source paths, credentials,
and the Notion root do not cross the wire. Use synthetic contract fixtures or
a schema artifact checked with tests; a new generator is a build-tool approval
item.

## Conditional mutations and error behavior

Represent workflow and entry-buffer versions as separate opaque strong ETags.
All mutations of an existing workflow carry `If-Match` for the workflow
revision, except entry autosave, which uses the buffer ETag. Entry confirmation
checks both versions in one core operation. A stale precondition returns `412`
with a safe problem and no state change. A known domain conflict, including
`Reconciling`, is a typed `409`; validation failures are typed `4xx`.
Define missing-precondition, oversized-request, saturated-worker, and
internal-failure mappings explicitly in the contract. The client refetches
the canonical workflow or entry buffer after `412` and never assumes an SSE
event is the full state.

Apply authentication, `Host`/`Origin`, body size, JSON depth/field, and
concurrency checks before routing to use cases. Reject unsupported methods
and unknown fields according to the frozen contract rather than silently
altering a command. Bound response size and streaming lifetime as well as
request bodies. Errors use RFC 9457 problem details with a stable safe code,
title, and correlation reference; no private value appears in detail text or
logs. Do not place tokens or diary content in URLs, process arguments, or
event IDs.

## Foreground process and local authorization

Use the existing `ND_CONFIG_PATH` precedence and repository-local default.
Startup order is: acquire the advisory single-instance lock; verify the
configuration is present, owner-owned, and owner-only; open and migrate the
Application Support SQLite database; purge expired draft text; bind only
`127.0.0.1` using `8771`, `8772`, then the configured bounded fallback range;
write an owner-only runtime file atomically after the listener is ready; and
start background reconciliation. Failure before runtime publication leaves
no live server advertised. Exhausting the port range fails with a clear
content-free error. Isolate paths and port ranges in tests so no test touches
real Application Support state or assumes `8771` is free.

Load unresolved intents and apply their per-day `Reconciling` gates before
accepting workflow mutations; the reads that resolve them run in the
background after startup. An unavailable destination must not keep another
day from working.

The runtime directory is owner-only and the runtime file contains only
the chosen port, ephemeral full-capability control token, and instance ID.
Check ownership, mode, file type, and symlink behavior before trusting or
replacing local control files. A leftover file from a crash is not proof of
a live process: the lock and health instance must agree. Rotate the token
on each start. A clean shutdown removes its own runtime file; a crash is
recovered by the next lock holder. The first release grants the TUI one
full-capability token. Capability checks still live in the server, and future
narrower clients must get separate tokens. The token proves access by a
process in the same macOS account, not a human gesture.

Validate `Host` against the actual loopback listener and reject an unsafe
`Origin` on every route, including health. Do not enable permissive CORS.
Keep binding and header tests at a real loopback socket, not only an
in-process router. Health exposes no token or diary state. Before publishing
the runtime file, ensure the listener can answer health with the new instance
ID. On interrupt or termination, stop accepting mutations, let an in-flight
external write finish up to a bound, request cancellation of other jobs,
and leave unresolved intents for the next start. Background reconciliation
may leave an unreachable destination pending while other days remain usable.

## Jobs, events, and recovery

Long discovery, selected-source acquisition, freshness, and other blocking
work returns `202 Accepted` with a job reference. Give jobs bounded queue
capacity, concurrency, execution time, and cancellation state. An accepted
job rechecks the workflow revision at commit, so a late result cannot replace
newer selection or entry state. Cancellation is a request; it cannot undo a
completed external effect. A job whose response is lost is found through
workflow/job refetch before a command is retried. Later publication commands
need idempotency keys and destination reconciliation; M6 must not simulate
remote idempotence.

Publish SSE only after state commits. Events carry event type, workflow ID,
revision, job ID if relevant, and safe progress fields; never source text or
entry text. Bound subscriber count, event queue, and stream lifetime so a
slow client cannot retain unbounded memory or delay commits. On reconnect,
server restart, an event gap, or an unknown event ID, the client fetches the
current workflow and job representations. The workflow is authoritative;
SSE is only a notification path. Multi-client tests must show ordered
revisions for one workflow and safe refetch when notifications are missed.

## Implementation and verification sequence

1. Check the P1-M4/P1-M5 completion evidence, implemented contracts, and
   owner-approved dependency packet. Define the manual-route table, wire
   revisions, safe problem codes, and synthetic contract fixtures first.
2. Build isolated lifecycle primitives and the `nd serve` composition root.
   Prove lock, config, migration/purge, bind, runtime publication, stale-file
   recovery, token rotation, shutdown, and restart with temporary state.
3. Add loopback HTTP, health negotiation, bearer/capability checks, header
   validation, and bounds. Test these through a real socket and in-process
   route client before exposing workflow mutations.
4. Map manual workflow reads and conditional commands to core use cases.
   Test concurrent clients, independent buffer autosave, confirmation races,
   invalidation, and safe typed error responses.
5. Add bounded job workers and SSE notifications. Test cancellation, late
   results, disconnected subscribers, missing events, and canonical refetch.
6. Update the contract artifact, README, contributor startup guidance, and
   affected architecture docs. Record focused evidence in the overview and
   run the full Rust and Swift gate once after implementation. Mark P1-M6
   `Done` only when every checklist item and the gate have evidence.
