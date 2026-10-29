# P1-M7: TUI and manual workflow architecture plan

Status: Planned

This plan connects the approved terminal information architecture to the
Control API without creating a second workflow authority in the client.

## Affected modules and interfaces

- `diary-tui`: terminal guard, typed API client, SSE worker, ViewModels, views,
  editor buffer, Command Line mapping, and IME adapter.
- `diary-control-contract`: the P1-M12 representations, problems,
  ETags, and event payloads needed by the manual workflow.
- `apps/nd`: default TUI startup, runtime-file attachment, and terminal error
  handling.
- `diary-control`: the P1-M12 runtime-attachment, source-note naming, and
  no-notes contracts; it remains the owner of policy and persistence.
- Proposed `diary-macos`: TIS/Core Foundation query,
  notification, ownership, and run-loop integration behind a safe fakeable
  interface. Creating it and its narrow unsafe policy requires the owner
  approval recorded in the milestone overview.

The TUI has no filesystem, SQLite, source-reader, helper, or assistant adapter.
A missing P1-M7 operation in the completed server contract is a
planning/contract gap: stop and amend the owning P1-M12 plan rather than
inventing a client-side workaround or a parallel endpoint.

## Implemented-contract readiness audit

Before the TUI manifest changes, verify the P1-M12 evidence for these findings:

| Finding | Required correction and evidence |
|---|---|
| Runtime advertisements previously had no safe client read operation. | P1-M12 added a descriptor-validated, size-bounded, read-only loader. It creates no state, acquires no server lock, and reads no configuration. Verify its missing, malformed, owner/mode/type/link, path-identity, oversized, and valid-file tests. |
| `SourceCandidateDto` previously exposed only an opaque hash, so distinct notes could not be identified in Sources. | P1-M12 added the trusted server-supplied `source_note_name` to candidate and detail DTOs while retaining the opaque command ID. Verify it is absent from diagnostics and assistant request envelopes. |
| `source:read` can be unavailable before discovery, while explicit no-notes confirmation must remain reachable. | P1-M12 lets the existing dedicated empty-selection command advance a newly started workflow only through an explicit human action and without creating candidates or pretending discovery succeeded. Verify direct and discovered-empty cases. |

P1-M12 implements these corrections. They are not permission for M7 to add
routes or duplicate lower-layer behavior.

## Canonical stages in M7

The left Workflow Sidebar is stable across milestones. M7 maps the current
server representation onto the six canonical user-facing stages from
[UX terms](../ux-terms.md#canonical-workflow-stages); it does not expose the
server's state enum as navigation labels.

| Stage | M7 behavior |
|---|---|
| Start | Attach, setup status, choose a day, and start, resume, or discard. |
| Sources | Discover, include or exclude, prepare, inspect readable text, and confirm the source set or explicit empty set. |
| Compose | Choose language/style and use the Draft editor manually; confirmed source notes remain inspectable. Assistant controls arrive in P1-M8 without adding a stage. |
| Review | Inspect and confirm the complete edited entry. |
| Preview | Visible but locked until P1-M9. |
| Publish | Visible but locked until P1-M9 and P1-M10. |

The mapping from canonical workflow state to stage progress is one pure
ViewModel function with table-driven tests. The `current` marker is separate
presentation state. Navigation can change the current stage or active resource
but never synthesize or mutate server workflow state.

| Server condition | Start | Sources | Compose | Review | Preview | Publish | Default current |
|---|---|---|---|---|---|---|---|
| No attached workflow | available | locked | locked | locked | locked | locked | Start |
| `Started`, or initial `Discovered` | done | available | locked | locked | locked | locked | Sources |
| `Discovered` after source/freshness invalidation | done | stale | stale when a recovered entry buffer exists, otherwise locked | locked | locked | locked | Sources |
| `SourcesConfirmed` | done | done | available | locked | locked | locked | Compose |
| `Composing` | done | done | available | available | locked | locked | Compose |
| `Composing` after `entry-edited` | done | done | available | stale | locked | locked | Compose |
| `EntryConfirmed` | done | done | done | done | locked in M7 | locked in M7 | Review |

An invalidation code explains a regression but does not by itself set progress:
the core retains its most recent reason after a successful reconfirmation. The
mapping therefore combines state, current confirmation flags, reason, and
whether a recovered entry buffer exists. Source invalidation makes Sources
stale and keeps recovered Compose text inspectable but unconfirmed; EntryEdited
makes Review stale. P1-M9 replaces the build-time Preview and Publish locks
with their normal prerequisite mapping.

The pure mapping also handles every later state already representable by the
core (`OutputPlanned`, `Approved`, `Publishing`, `Partial`, and `Completed`). M7
shows their canonical progress read-only and keeps Preview/Publish actions
locked with an “available in a later milestone/client” explanation; it never
maps a completed entry back into the Draft editor or accidentally begins an
amendment. An unknown state is a fail-closed contract mismatch, not Start.

## Client-to-server boundary

The typed client is the only route from a ViewModel to persisted workflow
state. It exposes purpose-named operations, not general request construction;
route paths, bearer handling, ETags, and RFC 9457 problem decoding stay inside
the API-client module. The following map is the P1-M7 surface provided by
P1-M12 over the P1-M6 server. It is a verification aid, not permission to add a
missing route.

| Terminal-client intent | Control API resource | Client and ViewModel rule |
|---|---|---|
| Attach and determine whether actions are available | `GET /v1/health`, then authenticated `GET /v1/capabilities` and `GET /v1/setup-check` | Compare runtime-file and health instance IDs and contract version before bearer use. Keep setup results informational; a failed setup check never turns into a client-side configuration probe. |
| Choose defaults and remembered entry settings | `GET`/`PUT /v1/preferences` | Render only server-accepted values. A rejected preference edit remains local presentation input until the person changes or discards it. |
| Start, resume, list, or discard a workflow | `GET`/`POST /v1/workflows`, `GET`/`DELETE /v1/workflows/{id}` | Consume bounded pagination in server order so unfinished drafts remain first. Send no diary day for the server-owned Mac-local “today” default. Use the current workflow ETag for discard and refetch after a stale precondition or domain conflict. Completed workflows are read-only in M7. |
| Find and inspect candidates or confirmed snapshots | `POST /v1/workflows/{id}/discoveries`, `GET /v1/workflows/{id}/sources`, `GET /v1/workflows/{id}/sources/{source_id}` | Render the server-supplied `source_note_name`, date evidence, availability, and warnings. Consume bounded source pagination. Discovery summaries contain no body. Before confirmation, source detail carries the active preparation handle in its header and returns one prepared sanitized body; afterward it returns only a persisted confirmed snapshot. It performs no new source read. |
| Prepare selected sources and confirm selection | `POST /v1/workflows/{id}/sources/prepare`, `PUT /v1/workflows/{id}/selection`, `POST /v1/workflows/{id}/empty-selection-confirmations`, `GET`/`DELETE /v1/jobs/{id}` | Track accepted work as a job and review its expiring server-owned preparation result. The Resources Sidebar's active source is presentation state separate from the prospective included IDs. Confirm by handle, never by echoing a body or stamp. If the job expires or the server restarts, prepare again. The separate empty-selection command is the only zero-note continuation. |
| Choose current entry settings and open the Draft editor | `PUT /v1/workflows/{id}/writing-settings`, `POST /v1/workflows/{id}/manual-compositions` | Use the workflow ETag, send no assistant backend ID in M7, and treat the server as authoritative for language, style, selected backend/path, and invalidation. Manual editing is not a backend or composition path. |
| Save and confirm the complete manual entry | `PUT /v1/workflows/{id}/entry`, `POST /v1/workflows/{id}/entry-confirmations` | Autosave uses only the entry-buffer ETag. Confirmation supplies both current versions and is disabled for blank text or unresolved inferred-reflection delimiters. |
| Notice committed changes | `GET /v1/events?workflow_id={id}` plus canonical workflow/job reads | Treat every event as a prompt to reload, never as the full state or a confirmation receipt. |

No P1-M7 ViewModel calls assistant, publication, amendment, or `/mcp` resources.
The client must not synthesize source availability, workflow transitions,
selection digests, entry confirmation, backend availability, or composition
path from local data. P1-M12 separates `manual_workflow_available` from the
collection-shaped assistant backend descriptors before M7. That collection is
empty in the manual-only slice; P1-M8 registers production backends without
another manual/path migration.

### Attachment and HTTP safety

`apps/nd` resolves the existing Application Support runtime location without
loading product configuration or creating state, then calls the shared safe
reader. Reading necessarily places the token in private process memory; the
security invariant is that it lives only in a non-`Debug` attachment/client
value and is not placed in a request until unauthenticated health matches the
advertised numeric host/port, instance ID, API version, and contract version.
Do not inspect the lock file. A vanished file is “not running”; an unsafe or
malformed file is a distinct safe attachment error.

The proposed blocking HTTP agent accepts only a constructed
`http://127.0.0.1:<validated-port>` base. It disables proxy discovery,
redirects, TLS, cookies, compression, and automatic retries; supplies the exact
numeric Host and absent Origin policy expected by P1-M6; and never accepts an
absolute request URL from a ViewModel. Encode JSON with the shared DTOs. For
ordinary routes, read at most the route-specific response bound plus one byte
before deserializing and reject a conflicting or oversized `Content-Length`;
for SSE, parse incrementally and enforce the server's per-event bound. Parse
only typed problems; map transport/library failures to fixed content-free UI
codes rather than displaying arbitrary error strings.

Use separate long-lived bounded workers for ordinary request/response traffic
and the single SSE stream. The proposed limits are one ordinary worker with a
16-command queue, one event worker with one active stream, and a 32-result
channel; duplicate UI intents are disabled while pending. The owner approves
these limits with the dependency packet. Connect, send, and header phases use
2-, 5-, and 5-second limits, and ordinary bodies use 30 seconds. An SSE body
uses a 45-second total window—long enough to observe two 15-second server
heartbeats—then reconnects deliberately rather than leaving a blocking read
unbounded. Reconnect backoff is 250 ms, 500 ms, 1 second, 2 seconds, then a
4-second ceiling. Neither worker mutates ViewModel state.

For the same instance, reconnect SSE with the last completely decoded event ID
in `Last-Event-ID`. For a changed instance, discard the token and event cursor
and repeat the full health/attachment sequence. A reset, unknown or evicted
event, decode error, queue overflow, or restart disables mutations and triggers
canonical workflow and relevant-job reads. Events only prompt reads; job
completion and workflow confirmation are accepted only from their canonical
REST responses.

## Local presentation state and recovery

The terminal owner is the sole renderer and input dispatcher. Bounded workers
perform API calls and event-stream reads, returning typed results over bounded
channels; workers do not mutate a ViewModel. A ViewModel contains the last
canonical server representation, current canonical stage, active resource,
prospective included source IDs, focus and layout state, a four-row Work Status
presentation model, an in-memory editor buffer and cursor, a monotonically
increasing local edit generation, and a pending-intent label. Active resource
and inclusion are independent. The only
durable text is the server entry buffer; a local unsent buffer disappears if
the process ends before a flush.

| Condition | Required client behavior |
|---|---|
| Initial attach cannot find or validate the runtime file, health instance, or supported version | Show the appropriate not-running, unsafe-attachment, or version-mismatch screen without sending the token or creating a workflow. Retry only when the person requests it. |
| Transport loss, server restart, SSE gap, unknown event, or worker overflow | Mark mutations disabled, retain readable canonical content and unsent editor text, reconnect with bounded backoff, then refetch the workflow and relevant job before enabling a new mutation. |
| `412 Precondition Failed` for workflow or entry buffer | Preserve the attempted selection or editor text in presentation state, fetch canonical state, identify the changed version, and require deliberate resolution. Do not replay the request. |
| Typed `409` job/domain conflict, job cancellation, or freshness invalidation | Show the safe server problem and impact notice, retain local text as unconfirmed, and direct the person to the current Sources or Compose stage. Do not convert it to a retry. |
| Editor flush is unavailable | Continue editing the in-memory buffer, show that it is unsaved, and ask before quitting. A later successful connection may offer a conditional save against the freshly fetched buffer version. |

An impact notice precedes an action that will invalidate a later confirmation.
Navigation alone never invalidates state. Activating a source replaces the Main
Pane with a read-only body view; returning to the draft restores the editor
buffer and cursor. On a narrow terminal, the same state and actions remain
available through focused controls after a sidebar collapses; the client never
hides a required confirmation merely to fit width.

Autosave has at most one request in flight. The request captures buffer ETag,
text, and local generation. A successful response advances the canonical
buffer version but replaces local text only when no newer generation exists;
otherwise the newer text stays dirty and schedules another conditional save.
Timeouts and unknown outcomes always refetch before another save. On `412`, show
the canonical server text and retained local text as separate choices; never
silently merge diary prose or replay it against a new buffer version.

## Work Status and compact status

Work Status is a full-width, bordered pane directly above the three-column
Workflow Sidebar, Resources Sidebar, and Main Pane region. Its visible title is
`Work Status`, and its body is always exactly four text rows; the title and top
and bottom borders do not count toward those four rows. The pane does not take
focus or own actions.

The four rows prioritize: current or most recent work and its outcome; pending
or blocked work; connection and draft-save safety; and the newest useful safe
notice or next action. Each priority owns one visual row and truncates by
display width; empty priorities leave a blank row rather than shrinking the
pane. Messages come from typed ViewModel state and fixed client messages. They
may use a sanitized presentation name already allowed on the current screen,
but never diary text, credentials, raw request data, or arbitrary
transport/server strings. A Banner in the Main Pane still holds the full
explanation and controls for a condition that needs a decision.

The bottom Status Line remains the compact ambient summary for diary day,
workflow state, connection, IME, progress, short error state, and action hints.
Work Status may intentionally repeat important connection, progress, save, or
error state from the Status Line so that useful information remains readable
without relying on the single-line summary. Tests cover ordering, replacement,
display-width truncation, safe rendering, and this intentional duplication.

## Terminal, editor, and command input

Global chords are intercepted before `ratatui-textarea`, so its default Ctrl-D
delete and Ctrl-Left/Ctrl-Right word-motion bindings cannot consume quit or
stage navigation. Use the editor's basic input path plus explicit M7 mappings;
ordinary Enter inserts a newline only while the editor owns focus. Bracketed
paste is one edit, enforces the 512 KiB UTF-8 byte limit before mutation, and
never partially inserts an oversized payload.

The M7 Command Line is an application parser, not a shell. Its bounded grammar
is `help`, `retry`, `day YYYY-MM-DD`, `stage <canonical-stage>`, `save`, and
`quit`; state-changing confirmation, discard, preparation, and entry-confirm
actions remain deliberate focused controls. Normalize only characters produced
by the 2-Set Korean layout into lowercase ASCII command input, then reject any
remaining non-ASCII or extra argument. Every command has the same guards and
effect as its focused/chord path.

`help` opens the on-screen action reference; `retry` performs the same guarded
attachment/refetch as Retry; `day` edits the Start day field but does not start
a workflow; `stage` only navigates to an accessible stage; `save` requests the
same conditional flush as Ctrl-Enter/F3; and `quit` uses the same dirty-buffer
warning as Ctrl-D/F10. If Apple Terminal cannot distinguish a primary chord in
the real-terminal check, its function-key alias remains available but the gate
fails until the owner explicitly revises the primary binding; ordinary Enter
must never be silently treated as Ctrl-Enter.

Source and entry text are untrusted terminal content. Preserve their exact
server/editor value, but render C0/C1 controls other than intended line breaks
and tabs as visible placeholders so ESC/OSC sequences can never reach the
terminal backend. Width, truncation, wrapping, cursor position, hit testing,
and narrow-layout decisions use grapheme/display width rather than bytes,
scalar count, or the sanitized display string's source indices.
Color may reinforce these states but never carries them; rendering honors
`NO_COLOR` and remains legible with the 16-color palette.

The TIS boundary performs all Carbon/Core Foundation calls on the verified
run-loop/thread arrangement and sends only `EN`, `KO`, or unknown state to the
ViewModel. First run a production-shape notification probe on the supported
macOS/Apple Terminal baseline; any registration or delivery failure selects the
500 ms query fallback without disabling editing. CF ownership, callback removal,
shutdown, duplicate notifications, missing/other language properties, and no
privacy prompt are acceptance cases.

## Approach and sequence

1. Verify P1-M12 is `Done`, inspect its runtime-attachment and contract
   fixtures, and record owner approval for the exact
   dependency, worker-limit, workspace, and unsafe-FFI packet. Stop if the
   implemented surface still cannot express the manual flow.
2. Build a terminal guard that restores terminal state on normal return,
   handled error, and panic unwind. Read the owner-only runtime advertisement,
   match its instance ID against unauthenticated health, and verify API/contract
   versions before attaching the control token to a request. Keep only the safe
   build/version fields for diagnostics, and do not print any runtime-file field.
3. Implement the typed bounded HTTP/SSE client and fake boundary, including
   route-specific body caps, timeouts, paging, conditional headers, instance
   replacement, event cursors, canonical refetch, and fixed safe errors.
4. Implement the Standard Layout shell with a full-width four-content-row Work
   Status pane above a left Workflow Sidebar, middle Resources Sidebar, and
   right Main Pane, then add connection states, the compact Status Line, focus
   model, the 80-by-30 terminal-size fallback, and bounded API/event workers.
   The Workflow Sidebar renders the six canonical stages, server-derived
   progress, and the presentation-only current marker. The Resources Sidebar
   renders the active stage's source/input/output resources and their explicit
   state, using indented trees where a server resource has a hierarchy. The
   terminal owner polls worker completions and renders; reconnect and an SSE gap
   trigger a canonical workflow refetch before a mutation can be enabled.
5. Add Start and Sources ViewModels over authoritative workflow DTOs. Keep
   candidate discovery, the active-resource/included-set distinction,
   selected-only cloud preparation, prepared-body inspection in the Main Pane,
   snapshot review, empty-selection confirmation, job progress, and conditional
   conflicts as explicit screen states. A `412`, `409`, cancelled job, or
   freshness invalidation never becomes an automatic repeat command.
6. Add Compose and Review ViewModels with a multiline editor, an in-memory
   unsent buffer, roughly one-second
   debounced conditional saves, focus-leave and quit flushes, server-draft
   recovery, complete-entry review, and confirmation guards. Distinguish the
   entry-buffer version from the workflow ETag; on either conflict, refetch and
   retain local unsent text for a person to resolve. Keep the draft and
   confirmed sources in the Resources Sidebar; opening a source is read-only and
   returning to the draft restores cursor and selection. Recovered text is
   editable but is never itself evidence of confirmation.
7. Represent language/style as server-owned workflow state, remembering only
   the last language and optional assistant backend ID while retaining the concise new-workflow
   style default. Render any reserved inferred-reflection delimiter defensively
   and block entry
   confirmation until it is resolved, while leaving extraction and drafting for
   P1-M8. Implement accessible focus actions, safe Ctrl chords, Hangul-to-ASCII
   Command Line mapping, grapheme-aware display width, and the TIS run-loop /
   500 ms polling adapter. Use Ctrl-G (F2) for Command Line, Ctrl-Enter (F3)
   to save/flush, Ctrl-D (F10) to quit, and Ctrl-Left/Ctrl-Right (F6/F7) for
   previous/next workflow stage. Leave assistant selection, draft generation,
   and paragraph review to P1-M8; their later controls extend Compose rather
   than changing the stage model or layout.
8. Test ViewModels and rendering with fakes, then record the required real
   Terminal and Korean IME acceptance evidence.

The event/render loop never performs HTTP, filesystem, SQLite, helper, or
assistant work. On reconnect it reloads server state before re-enabling
mutations.
