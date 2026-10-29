# Control API `/v1` contract

Status: Active

This document describes how the P1-M7 terminal client attaches to the loopback
server after P1-M12's pre-client contract revision. The REST namespace remains
`/v1`; the current native `contract_version` is `2` and rejects incompatible
clients. The checked Rust DTO and route table in
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

The route inventory is frozen in `diary_control_contract::ROUTES`. The pre-M7 server
exposes health/capabilities/setup, preferences, workflows, discovery, source
preparation and confirmation, explicit no-notes continuation, writing
settings, opening the Draft editor for manual composition, entry save/confirmation, jobs, and events.
Draft-assistant, output, amendment, publication, and `/mcp` routes do not exist.
`source:read`, including availability-aware discovery and preparation, is
advertised and accepted only while the Swift helper is available; its absence
does not prevent the explicit continue-without-notes action from a newly
started workflow.

Capabilities report `manual_workflow_available` separately from
`assistant_backends[]`. Manual editing has no backend ID or composition path.
Before P1-M8 the backend array is empty; writing settings therefore accept only
a null assistant backend ID. A future descriptor binds one stable backend ID to
its display label, hosted or Apple-on-device path, availability/reason,
supported languages, and supported actions. Preferences remember an optional
backend ID, never an independent path.

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

Requests are at most 1 MiB with JSON nesting at most 16; entry text is at most
512 KiB. Workflow lists are bounded to 256 and paged 50 at a time; a workflow
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
