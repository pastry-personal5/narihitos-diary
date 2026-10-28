# P1-M7: TUI and manual workflow architecture plan

Status: Planned

This plan connects the approved terminal information architecture to the
Control API without creating a second workflow authority in the client.

## Affected modules and interfaces

- `diary-tui`: terminal guard, typed API client, SSE worker, ViewModels, views,
  editor buffer, Command Line mapping, and IME adapter.
- `diary-control-contract`: the already-implemented representations, problems,
  ETags, and event payloads needed by the manual workflow.
- `apps/nd`: default TUI startup, runtime-file attachment, and terminal error
  handling.
- `diary-control`: only a defect correction required to expose an implemented
  P1-M6 manual-flow use case; it remains the owner of policy and persistence.

The TUI has no filesystem, SQLite, source-reader, helper, or model adapter. A
missing P1-M7 operation in the completed server contract is a planning/contract
gap: stop and amend the owning P1-M6 plan rather than inventing a client-side
workaround or a parallel endpoint.

## Client-to-server boundary

The typed client is the only route from a ViewModel to persisted workflow
state. It exposes purpose-named operations, not general request construction;
route paths, bearer handling, ETags, and RFC 9457 problem decoding stay inside
the API-client module. The following map is the P1-M7 surface expected from
P1-M6. It is a verification aid, not permission to add a missing route.

| TUI intent | P1-M6 resource | Client and ViewModel rule |
|---|---|---|
| Attach and determine whether actions are available | `GET /v1/health`, then authenticated `GET /v1/capabilities` and `GET /v1/setup-check` | Compare runtime-file and health instance IDs and contract version before bearer use. Keep setup results informational; a failed setup check never turns into a client-side configuration probe. |
| Choose defaults and remembered entry settings | `GET`/`PUT /v1/preferences` | Render only server-accepted values. A rejected preference edit remains local presentation input until the person changes or discards it. |
| Start, resume, list, or discard a workflow | `GET`/`POST /v1/workflows`, `GET`/`DELETE /v1/workflows/{id}` | List unfinished drafts first. Use the current workflow ETag for discard and refetch after a stale precondition or domain conflict. |
| Find and inspect candidates | `POST /v1/workflows/{id}/discoveries`, `GET /v1/workflows/{id}/sources` | Render date evidence, availability, and warnings as supplied. The detail view may display only sanitized readable text returned by the server. |
| Prepare selected cloud-only sources and confirm selection | `POST /v1/workflows/{id}/sources/prepare`, `PUT /v1/workflows/{id}/selection`, `GET`/`DELETE /v1/jobs/{id}` | Track accepted work as a job; review the returned snapshot before confirmation. A zero-note continuation is an explicit server command, never an implicit empty selection. |
| Save and confirm the complete manual entry | `PUT /v1/workflows/{id}/entry`, `POST /v1/workflows/{id}/entry-confirmations` | Autosave uses only the entry-buffer ETag. Confirmation supplies both current versions and is disabled for blank text or unresolved inferred-reflection delimiters. |
| Notice committed changes | `GET /v1/events?workflow_id={id}` plus canonical workflow/job reads | Treat every event as a prompt to reload, never as the full state or a confirmation receipt. |

No P1-M7 ViewModel calls model, publication, amendment, or `/mcp` resources.
The client must not synthesize source availability, workflow transitions,
selection digests, entry confirmation, or composition-path availability from
local data.

## Local presentation state and recovery

The terminal owner is the sole renderer and input dispatcher. Bounded workers
perform API calls and event-stream reads, returning typed results over bounded
channels; workers do not mutate a ViewModel. A ViewModel contains the last
canonical server representation, focus and layout state, an in-memory editor
buffer, and a pending-intent label. The only durable text is the server entry
buffer; a local unsent buffer disappears if the process ends before a flush.

| Condition | Required client behavior |
|---|---|
| Initial attach cannot find or validate the runtime file, lock, health instance, or supported version | Show the appropriate not-running or version-mismatch screen without sending the token or creating a workflow. Retry only when the person requests it. |
| Transport loss, server restart, SSE gap, unknown event, or worker overflow | Mark mutations disabled, retain readable canonical content and unsent editor text, reconnect with bounded backoff, then refetch the workflow and relevant job before enabling a new mutation. |
| `412 Precondition Failed` for workflow or entry buffer | Preserve the attempted selection or editor text in presentation state, fetch canonical state, identify the changed version, and require deliberate resolution. Do not replay the request. |
| Typed `409` job/domain conflict, job cancellation, or freshness invalidation | Show the safe server problem and impact notice, retain local text as unconfirmed, and direct the person to the current source or editor step. Do not convert it to a retry. |
| Editor flush is unavailable | Continue editing the in-memory buffer, show that it is unsaved, and ask before quitting. A later successful connection may offer a conditional save against the freshly fetched buffer version. |

An impact notice precedes an action that will invalidate a later confirmation.
Navigation alone never invalidates state. On a narrow terminal, the same state
and actions remain available through the focused controls after the Sidebar
collapses; the client never hides a required confirmation merely to fit width.

## Approach and sequence

1. Inspect P1-M6's actual runtime-file and contract fixtures. Build a terminal
   guard that restores terminal state on normal return, handled error, and panic
   unwind. Read the owner-only runtime file, match its instance ID against
   unauthenticated health, and verify the contract version before loading or
   sending the control token. Keep the build ID for diagnostics, and do not
   print any runtime-file field.
2. Implement the Standard Layout shell as a left Workflow Sidebar, Main Pane,
   and right Resources Sidebar, then add connection states, Status Line, focus
   model, terminal-size fallback, and bounded API/event workers. The Workflow
   Sidebar renders server-derived stage state only; the Resources Sidebar
   renders the active screen's source/input/output resources and their explicit
   state, using indented trees where a server resource has a hierarchy. The
   terminal owner polls worker completions and renders; reconnect and an SSE
   gap trigger a canonical workflow refetch before a mutation can be enabled.
3. Add Start and source-selection ViewModels over authoritative workflow DTOs.
   Keep candidate discovery, selection edits, selected-only cloud preparation,
   snapshot review, empty-selection confirmation, job progress, and conditional
   conflicts as explicit screen states. A `412`, `409`, cancelled job, or
   freshness invalidation never becomes an automatic repeat command.
4. Add a multiline editor with an in-memory unsent buffer, roughly one-second
   debounced conditional saves, focus-leave and quit flushes, server-draft
   recovery, complete-entry review, and confirmation guards. Distinguish the
   entry-buffer version from the workflow ETag; on either conflict, refetch and
   retain local unsent text for a person to resolve. Recovered text is editable
   but is never itself evidence of confirmation.
5. Represent language/style as server-owned workflow and preference state.
   Render any reserved inferred-reflection delimiter defensively and block entry
   confirmation until it is resolved, while leaving extraction and drafting for
   P1-M8. Implement accessible focus actions, safe Ctrl chords, Hangul-to-ASCII
   Command Line mapping, grapheme-aware display width, and the TIS run-loop /
   500 ms polling adapter. Use Ctrl-G (F2) for Command Line, Ctrl-Enter (F3)
   to save/flush, Ctrl-D (F10) to quit, and Ctrl-Left/Ctrl-Right (F6/F7) for
   previous/next workflow step.
6. Test ViewModels and rendering with fakes, then record the required real
   Terminal and Korean IME acceptance evidence.

The event/render loop never performs HTTP, filesystem, SQLite, helper, or model
work. On reconnect it reloads server state before re-enabling mutations.
