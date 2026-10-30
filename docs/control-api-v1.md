# Control API `/v1` contract

Status: Active

This document describes how the terminal client attaches to the loopback
server. The REST namespace remains `/v1`; P1-M10 advances the current native
`contract_version` to `4` for the typed Notion preview resources and rejects
incompatible clients. The checked Rust
DTO and route table in
`diary-control-contract` remain the executable contract.

## Attachment and authorization

A human starts `nd serve` in the repository root. After binding and answering
its own health probe, the process atomically publishes an owner-only runtime
advertisement in its private Application Support runtime directory. The native
client uses the shared read-only attachment loader to validate the existing
directory, file, and exact contract version before reading its numeric
`127.0.0.1` port, 256-bit ephemeral bearer token, and instance ID, then verifies that unauthenticated
`GET /v1/health` returns the same instance and contract.

Every other route requires exactly one `Authorization: Bearer <token>` header.
Every request requires exactly one numeric `Host: 127.0.0.1:<port>` header. An
absent `Origin` is valid; a present origin must be exactly
`http://127.0.0.1:<port>`. Proxy headers, `localhost`, permissive CORS, weak
ETags, and duplicate security headers are rejected. Every response, including
RFC 9457 problem JSON and SSE, uses `Cache-Control: no-store`.

## Resources and preconditions

The route inventory is frozen in `diary_control_contract::ROUTES`. The current
server exposes health/capabilities/setup, preferences, workflows, discovery,
source preparation and confirmation, explicit no-notes continuation, writing
settings, assistant backend selection, memory extraction/review, draft
requests/results, paragraph-review suggestions/resolution, opening the Draft
editor for manual composition, entry save/confirmation, jobs, and events.
P1-M9 exposes local output preview/approval/publication resources; P1-M10 adds
separate Notion-only preview/approval/publication resources after a verified
local receipt. Notion previews contain safe path labels and exact projected
blocks, never credentials or remote IDs.
Output, amendment, publication, and `/mcp` routes do not exist.
`source:read`, including availability-aware discovery and preparation, is
advertised and accepted only while the Swift helper is available; its absence
does not prevent the explicit continue-without-notes action from a newly
started workflow.

Capabilities report `manual_workflow_available` separately from
`assistant_backends[]`. Manual editing has no backend ID or composition path.
Each descriptor binds one stable backend ID to its display label, hosted or
Apple-on-device path, availability/reason, supported languages, and supported
actions. Preferences and workflows remember an optional backend ID, never an
independent path; the server derives the path. `GET /v1/assistant-backends`
returns the same safe global descriptor collection. Selecting or highlighting a
backend does not invoke it.

Assistant actions are jobs observed and cancelled through `/v1/jobs/{id}`.
Extraction accepts no content body from the client: the server rereads the
confirmed source snapshots, rechecks freshness before invocation and commit,
and constructs an ordinal-only provider envelope. `memory-review` is a
conditional human-reviewed resource. A draft request uses reviewed memory and
an explicitly requested prior draft, then populates the unconfirmed entry only
if its captured buffer version remains current. Paragraph review binds the
displayed UTF-8 range to the buffer version and exact-text digest; resolution
accepts or rejects once under the same binding. Artifacts remain durable and
inspectable when stale, but stale material cannot mutate canonical text.

Per-source resources add assistant refinement, suggestion resolution, direct
body edit, and confirmed delete. Refinement sends only readable body text and
stores a content-digest-bound proposal. Approve/edit use the digest as an
optimistic precondition. Delete requires `confirm: true` and cascades only to
linked audio paths validated by the Ayumi adapter; audio bytes are never read.

Workflow IDs use `wf_YYYYMMDD` but are opaque to clients. Workflow mutations
require strong `If-Match: "wf-<revision>"`; entry autosave instead requires
`If-Match: "buf-<buffer-version>"`. A missing precondition is `428`, a stale or
malformed one is `412`, and a typed state conflict is `409`. Preferences are a
documented last-writer-wins singleton.

Selection confirmation never accepts source bodies or freshness stamps from a
client. Preparation is a bounded job whose authenticated in-memory handle is
bound to one instance, workflow revision, and exact source-ID list. It expires
after ten minutes or on cancellation/restart. Confirmation consumes it and
revalidates every snapshot. Before confirmation, the client reviews each
prepared body and its post-read date evidence through the existing source-detail
route using one `X-ND-Preparation-Handle` header; the handle never appears in a
URL, and the route rejects duplicate, expired, or mismatched handles. Without
that header, source detail remains limited to a currently confirmed snapshot.
Each candidate and detail representation pairs its opaque command ID with a
safe local `source_note_name`; for Ayumi this is the relative source-note path. The
source-note name is not accepted back as an ID and is absent from jobs, events,
diagnostics, and assistant request envelopes. An empty selection is rejected by
the ordinary route and uses the separate explicit confirmation route. That
dedicated route may advance either `Started` or `Discovered` without inventing
a discovery result.

## Bounds and recovery

Requests are at most 8 MiB plus a 64 KiB JSON envelope (to permit one direct
source-body edit) with JSON nesting at most 16; entry, reviewed-memory,
and draft text are at most 512 KiB. Memory extraction returns at most 128
candidates of 4 KiB each and at most 64 request-local source ordinals. A
paragraph and its replacement are at most 32 KiB; explanations are at most 4
KiB; draft and paragraph results contain at most 32 validated reflection
ranges. Workflow lists are bounded to 256 and paged 50 at a time; a workflow
has at most 512 candidates, candidate-list pages contain at most 256 items, and
an entry has at most 64 selected sources. Other request strings are at most 4
KiB, and source-ID lists reject duplicates. A source is at most 8 MiB and
prepared sanitized text totals at most 16 MiB. Ordinary responses are at most
2 MiB; selected-source detail allows its 8 MiB body plus a 64 KiB envelope.

The server admits 64 concurrent requests, 32 queued jobs, four running workers,
128 live job records, and 16 SSE subscribers. Jobs time out after 120 seconds;
results expire ten minutes after reaching a terminal state. Cancellation and
timeout revoke a discovery job's right to commit before its conditional write,
so a late blocking result cannot mutate canonical state. SSE uses a 256-event
replay ring, 64-event subscriber queues, 1 KiB content-free events, 15-second
heartbeats, and a 30-minute stream lifetime. A replay larger than one subscriber
queue yields a reset instead of a partial replay. Events are notifications
only. After a reset, unknown/evicted event ID, overflow, or server restart, the
client refetches the canonical workflow and relevant jobs before mutating.

The server writes only typed, content-free request outcomes to the private
diagnostic sink. A runtime write or retention failure triggers controlled
shutdown and instance-owned runtime-file cleanup rather than leaving an
advertised server without diagnostics.

All diary days are `YYYY-MM-DD`; transport timestamps are signed Unix
milliseconds. Random instance, job, preparation, and correlation IDs contain
256 OS-random bits as 64 lowercase hexadecimal digits after their type prefix.
Unknown JSON fields, unknown assistant backend IDs, partial backend/path state,
and incompatible contract versions fail closed.
