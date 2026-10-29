# P1-M7: TUI and manual workflow architecture plan

Status: Done

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
  empty-selection contracts; it remains the owner of policy and persistence.
- `diary-macos`: TIS/Core Foundation query,
  notification, ownership, and run-loop integration behind a safe fakeable
  interface. Its creation and narrow unsafe policy were approved in the
  milestone overview; M7 must not broaden that boundary.

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
| `source:read` can be unavailable before discovery, while explicit empty-selection confirmation must remain reachable. | P1-M12 lets the existing dedicated empty-selection command advance a newly started workflow only through an explicit human action and without creating candidates or pretending discovery succeeded. Verify direct and discovered-empty cases. |

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
| `Discovered` after `selection-changed`, `source-changed`, or `freshness-unverifiable` | done | stale | stale when a recovered entry buffer exists, otherwise locked | locked | locked | locked | Sources |
| `SourcesConfirmed` | done | done | available | locked | locked | locked | Compose |
| `Composing` | done | done | available | available | locked | locked | Compose |
| `Composing` returned from a previously confirmed entry (`entry-edited` or `composition-choice-changed`) | done | done | available | stale | locked | locked | Compose |
| `EntryConfirmed` | done | done | done | done | locked in M7 | locked in M7 | Review |

An invalidation code explains a regression but does not by itself set progress:
the core retains its most recent reason after a successful reconfirmation. The
mapping therefore combines state, current confirmation flags, reason, and
whether a recovered entry buffer exists. `selection-changed` (re-running
discovery on an already-confirmed workflow), `source-changed`, and
`freshness-unverifiable` (both from a freshness checkpoint) all drop the
workflow back to `Discovered` and make Sources stale. `entry-edited` (editing
confirmed text) and `composition-choice-changed` (changing language, style, or
assistant backend after the entry was confirmed) both drop the workflow back to
`Composing` with `entry_confirmed = false` and make Review stale; the mapping
must treat the two reasons identically rather than keying off `entry-edited`
alone. P1-M9 replaces the build-time Preview and Publish locks with their
normal prerequisite mapping.

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
| Find and inspect candidates or confirmed snapshots | `POST /v1/workflows/{id}/discoveries`, `GET /v1/workflows/{id}/sources`, `GET /v1/workflows/{id}/sources/{source_id}` | Render the server-supplied `source_note_name`, date evidence, availability, and warnings. Consume bounded source pagination. Discovery summaries contain no body. Before confirmation, send the completed preparation job's `result_handle` in the `X-ND-Preparation-Handle` request header; a successful detail response returns one prepared sanitized body and proves that binding was accepted. Afterward, detail without that header returns only a persisted confirmed snapshot. It performs no new source read. |
| Prepare selected sources and confirm selection | `POST /v1/workflows/{id}/sources/prepare`, `PUT /v1/workflows/{id}/selection`, `POST /v1/workflows/{id}/empty-selection-confirmations`, `GET`/`DELETE /v1/jobs/{id}` | Track accepted work as a job and review its expiring (10-minute) server-owned preparation result. The Resources Sidebar's active source is presentation state separate from the prospective included IDs. Take the handle only from the succeeded job's typed `result_handle`; use it to authorize each included source-detail read and then to confirm the exact source-ID set, never echoing a body or stamp. If the job expires or the server restarts, prepare again. The separate empty-selection command is the only zero-note continuation. |
| Choose current entry settings and open the Draft editor | `PUT /v1/workflows/{id}/writing-settings`, `POST /v1/workflows/{id}/manual-compositions` | Use the workflow ETag, send no assistant backend ID in M7, and treat the server as authoritative for language, style, selected backend/path, and invalidation. Manual editing is not a backend or composition path. Changing language, style, or backend after the entry was confirmed reverts the workflow to `Composing` with `composition-choice-changed`, exactly as if the text had been edited. |
| Save and confirm the complete manual entry | `PUT /v1/workflows/{id}/entry`, `POST /v1/workflows/{id}/entry-confirmations` | Autosave uses only the entry-buffer ETag. Confirmation supplies both current versions: the workflow ETag as `If-Match` and the entry-buffer ETag in the request body. There is no separate entry-read route; current text and the buffer version always come from the same workflow response (`ETag` plus `X-Entry-Buffer-Etag` headers, `entry_text`/`buffer_version` fields) used for every other canonical refetch. Confirmation is disabled for blank text or unresolved inferred-reflection delimiters. |
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

The blocking HTTP agent accepts only a constructed
`http://127.0.0.1:<validated-port>` base. It disables proxy discovery,
redirects, TLS, cookies, compression, and automatic retries; supplies the exact
numeric Host and absent Origin policy expected by P1-M6; and never accepts an
absolute request URL from a ViewModel. Encode JSON with the shared DTOs. For
ordinary routes, read at most the route-specific response bound plus one byte
before deserializing and reject a conflicting or oversized `Content-Length`;
for SSE, parse incrementally and enforce the server's per-event bound.
Workflow, job, and event responses use the server's 2 MiB ordinary bound
(`MAX_RESPONSE_BYTES`); a source-detail response carries a prepared body and
uses the larger ~8 MiB source bound (`MAX_SOURCE_BYTES`) instead. Applying the
smaller bound to source detail would reject valid prepared bodies. Parse
only typed problems; map transport/library failures to fixed content-free UI
codes rather than displaying arbitrary error strings.

Use separate long-lived bounded workers for ordinary request/response traffic
and the single SSE stream. The approved limits are one ordinary worker with a
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
| Initial attach cannot find or validate the runtime file, health instance, or supported version | Show the appropriate not-running, unsafe-attachment, or version-mismatch screen without sending the token or creating a workflow. Retry automatically with the approved 250 ms to 4-second bounded backoff; Retry requests the same guarded attempt immediately. |
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
focus or own actions. The Workflow Sidebar uses a fixed 24-column width both
inline and as a focused overlay: 22 columns fit its longest focused canonical
stage/progress row and two columns hold the border. Remaining width belongs to
Resources and Main according to the active width band.

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
delete-next-char and Ctrl-P/Ctrl-N move-cursor-up/down-a-line bindings cannot
consume quit or stage navigation; the editor's arrow keys remain the ordinary
way to move the cursor by line while it owns focus. Use the editor's basic
input path plus explicit M7 mappings;
ordinary Enter inserts a newline only while the editor owns focus. Bracketed
paste is one edit, enforces the 512 KiB UTF-8 byte limit before mutation, and
never partially inserts an oversized payload.

The M7 Command Line is an application parser, not a shell. Its bounded grammar
is `help`, `retry`, `day YYYY-MM-DD`, `stage <canonical-stage>`, `save`, and
`quit`; state-changing confirmation, discard, preparation, and entry-confirm
actions remain deliberate focused controls. Two 2026-10-29 real-terminal
checks (see AGENTS.md "Keybindings") together establish the actual delivery
shape, which is a **mix**, not the single case either check first assumed:
Apple Terminal's raw mode delivers a bare Hangul Compatibility Jamo
(U+3131–U+3163) one key at a time when a keystroke does not complete a
syllable, and a fully committed precomposed syllable (U+AC00–U+D7A3, e.g. 디)
when a later keystroke does complete one — and a bare jamo can itself be a
compound vowel or consonant-tail-cluster codepoint (e.g. ㅘ, U+3158), not only
one of the 24 simple jamo. Normalization therefore handles all three shapes:
a precomposed syllable is decomposed with the standard Unicode Hangul
algorithm into its lead/vowel/optional-tail jamo, each mapped to the one or
two 2-Set keys that produce it; a bare jamo (simple or compound) maps to its
key(s) the same way, using the identical lead/vowel/tail key tables, since the
Compatibility Jamo block shares the syllable algorithm's Unicode ordering.
Reject any remaining non-ASCII character. Tests cover a jamo/syllable vector
for every grammar token (for example `help`, `retry`, `day`, `stage`, `save`,
`quit`) plus the precomposed-syllable and bare-compound-jamo cases
individually, not only one representative shape. Every command has the same
guards and effect as its focused/chord path.

The Command Line occupies its own one-row zone above the Status Line. It may be
empty while closed, but input and parse feedback never replace any of Work
Status's four semantic rows. `help` opens the on-screen action reference;
`retry` performs the same guarded
attachment/refetch as Retry; `day` edits the Start day field but does not start
a workflow; `stage` only navigates to an accessible stage; `save` requests the
same conditional flush as Ctrl-S; and `quit` uses the same dirty-buffer warning
as Ctrl-D. There are no function-key aliases (see AGENTS.md "Keybindings"). If
Wave Terminal cannot distinguish a chord in the primary real-terminal check,
the gate fails until the owner explicitly revises the binding. That happened
twice during the earlier Apple Terminal checks —
Ctrl-Enter and Ctrl-Left/Ctrl-Right (replaced with Ctrl-S and Ctrl-P/Ctrl-N)
and, separately, F10 never arriving as Ctrl-D's planned alias, after which the
owner decided (2026-10-29) to drop function-key aliases from the design
entirely rather than confirm the rest one at a time.

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
macOS baseline; any registration or delivery failure selects the
500 ms query fallback without disabling editing. CF ownership, callback removal,
shutdown, duplicate notifications, missing/other language properties, and no
privacy prompt are acceptance cases.

Wave Terminal is the primary M7 real-terminal target. Run every interactive
probe there first: chord delivery, Korean Command Line normalization, Draft
editor input and bracketed paste, terminal restoration, adaptive sidebar
overlays, and IME indicator changes. Apple Terminal is secondary evidence and
retains value as a regression check for the previously observed mixed Korean
delivery and unavailable chords, but it cannot qualify Wave Terminal behavior.

## Completion checkpoint and implemented sequence

The running client integrates session authority and recovery, operable Start,
end-to-end Sources, end-to-end manual Compose/Review, the adaptive accessible
presentation shell, and Increment 6's notification-first TIS implementation.
The SSE worker does not announce a usable connection until body bytes arrive,
the Command Line has its own row, and the focus-driven width bands keep both
sidebars reachable. The full dated gate passed and the owner confirmed the Wave
Terminal primary qualification and secondary Apple Terminal regression set on
2026-10-29. Implementation followed this vertical sequence:

1. **Finish session authority and recovery.** Add typed capabilities,
   setup-check, preference read/write, and paged-list operations that are still
   missing from the worker surface. Complete HTTP header/body-bound tests. Add
   the dedicated SSE worker with heartbeat handling, `Last-Event-ID`, instance
   replacement, reset/gap/unknown-event handling, bounded-result overflow, and
   a single mutation gate. Re-enable mutation only after the active workflow
   and relevant job have been canonically reloaded.
2. **Make Start fully operable.** Introduce one focus model shared by Tab,
   arrows, Enter, Ctrl-P/Ctrl-N, and Command Line stage navigation. Wire default
   day, explicit day, start/resume, workflow pagination, selection, and guarded
   discard through the ordinary worker. Render Setup check and connection
   states without blocking the event loop. Prove this slice against a real
   `nd serve` process before moving on.
3. **Make Sources end to end.** Wire discovery and job polling, paged candidates,
   explicit-date discovery, independent active/included state, selected-only
   preparation, prepared-body inspection, preparation-handle capture, selection
   confirmation, and explicit continue-without-notes. Treat `409`, `412`, job
   expiry/cancellation, and freshness invalidation as visible resolution states,
   never automatic mutation retries.
4. **Make Compose and Review end to end.** Wire server-owned language/style and
   remembered-language preferences, manual composition, the live Draft editor,
   confirmed-source inspection with cursor restoration, roughly one-second
   debounced autosave, Ctrl-S/focus-leave flush, and unsafe-quit confirmation.
   On uncertain save or `412`, reload canonically and present local unsent and
   server text separately. Wire full-entry review, first-delimiter navigation,
   blank/reflection guards, and conditional confirmation.
5. **Complete adaptive and accessible presentation.** Give Command Line its own
   row. At 120 columns or wider show both sidebars; at 100–119 overlay Resources;
   at 80–99 overlay either sidebar. Add scrolling, focus/current/included
   markers, grapheme-aware wrapping/cursor/hit testing, date weekday formatting
   in every applicable view, `NO_COLOR`, 16-color, and terminal-control
   sanitization tests. Every command and mutation remains reachable through a
   focused control and Enter.
6. **Complete IME and terminal qualification.** Add the TIS notification
   run-loop path with deterministic fallback and shutdown tests. Run
   `chord_probe`, `command_line_probe`, `editor_probe`, and `restoration_probe`
   in Wave Terminal first, using test text, then repeat the secondary regression
   set in Apple Terminal. Exercise the integrated manual flow in Wave, including
   disconnect/restart, stale preconditions, Korean input, bracketed paste,
   overlays, autosave, dirty quit, and recovery.
7. **Close evidence and documentation.** Add real-process integration tests for
   the Start→Sources→Compose→Review confirmation slice, reconcile user-facing
   docs with the observed behavior, run the full Rust/Swift gate, and check a
   completion item only when its focused and integrated evidence is recorded.

The event/render loop never performs HTTP, filesystem, SQLite, helper, or
assistant work. On reconnect it reloads server state before re-enabling
mutations.
