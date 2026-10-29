# P1-M6: Control API Server architecture plan

Status: Done

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
| `diary-control` | Route table, authentication and capability checks, request limits, DTO validation/mapping, bounded jobs, SSE fan-out, content-free error translation, and the typed rotating diagnostic sink. It calls core use cases. |
| `apps/nd` | `nd serve` composition root, configuration, state and log paths, lock/runtime files, diagnostic initialization, adapter registration, listener selection, signals, and shutdown. |
| `diary-core` | Existing workflow and publication policy only; no HTTP, Tokio, JSON, or transport types. Narrow port changes require focused core tests. |
| `diary-connectors` | Existing SQLite and source adapters; no route or token handling. |

Use Tokio for the HTTP/event layer as already decided. Schedule blocking
SQLite calls, hashing, helper-process waits, and source reads on bounded
workers. A blocked worker must not stall the event loop. Do not add a direct
dependency from the TUI to server internals or from core to transport.

### Implemented-contract audit

P1-M4 and P1-M5 are complete, so M6 must close the following observed gaps
before freezing DTOs. These are narrow application/store changes, not new
product policy.

| Implemented shape | M6 resolution |
|---|---|
| `WorkflowStore` loads by `DiaryDay` and has no list operation. | Add a bounded summary query to the inward store port and SQLite adapter. Keep `DiaryDay` as the durable identity; the stable opaque wire workflow ID resolves to that key and is never parsed by a client. Do not add a second durable UUID. |
| `WorkflowStore::discard` is currently unconditional even though discard is a workflow mutation. | Make discard compare the supplied workflow revision in the same transaction that clears recoverable text. A stale `DELETE` must not discard newer editor or selection state. |
| Core exposes workflow methods and store/source ports, but no load-effect-CAS command service. | Add purpose-named commands in `diary-control` that receive injected ports. Handlers only authenticate, validate/map DTOs, invoke one command, and map its typed result. |
| The source reader returns `SelectedSource` with a trusted snapshot and possibly newer date evidence, while `confirm_sources` currently accepts snapshots alone. | Keep prepared bodies/stamps server-owned. Add the narrow core command shape needed to confirm reviewed `SelectedSource` values, update matching candidate date evidence, and advance the workflow once in the same CAS. Never deserialize a client-authored snapshot or stamp into core. |
| Entry save mutates by buffer version, while the general SQLite CAS also checks the loaded workflow revision. | The autosave command retries an internal stale-revision CAS only after reloading and proving the client's buffer ETag still matches and the canonical state is editable. A changed buffer returns `412`; a state conflict returns its typed problem. The retry is bounded and never replays an external effect. |
| Core has language/style/path and `begin_manual_composition` commands, but the planned route table omitted them. | Expose conditional per-workflow writing-settings and manual-composition commands. Keep remembered preferences a distinct resource. Only paths advertised as available may execute. |
| Workflow choice commands and `save_preferences` are separate operations. | Make the writing-settings command persist the current workflow and the remembered language/composition choice in one SQLite transaction; style remains per-entry and defaults to concise. Keep direct preference editing a separate resource. |
| Empty selection requires an explicit confirmation in core, but an empty selection payload is ambiguous on the wire. | Give “continue without notes” its own conditional command route. The ordinary selection-confirmation route rejects an empty selected set. |

Focused core and SQLite tests must accompany any port or command changes. Do
not move HTTP, JSON, Tokio, paths, job records, or runtime tokens into
`diary-core`.

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
| `GET /v1/health`, `GET /v1/capabilities`, `GET /v1/setup-check` | Negotiate instance/build/contract, advertise only working operations, and report safe readiness flags without credentials or private paths. Only health is unauthenticated. Setup checks validate local shape/availability and do not inspect the Notion root. |
| `GET /v1/preferences`, `PUT /v1/preferences` | Read and update remembered choices through core preferences; validate values and expose unavailable composition paths honestly. The contract explicitly documents last-writer-wins semantics for this non-workflow singleton. |
| `GET /v1/workflows`, `POST /v1/workflows`, `GET /v1/workflows/{id}`, `DELETE /v1/workflows/{id}` | List unfinished first, start or resume by diary day, fetch canonical state, and discard through core policy. A completed day is not amended here. |
| `POST /v1/workflows/{id}/discoveries`, `GET /v1/workflows/{id}/sources`, `GET /v1/workflows/{id}/sources/{source_id}` | Request bounded or explicitly dated discovery and return candidate summaries, availability, distinct date evidence, and warnings. The detail read returns either a currently confirmed persisted snapshot or one source from the exact expiring preparation named by the `X-ND-Preparation-Handle` header. It never reads the filesystem itself, exposes a body outside those two reviewed bindings, puts the handle in a URL, or downloads a cloud-only note. |
| `POST /v1/workflows/{id}/sources/prepare`, `PUT /v1/workflows/{id}/selection`, `POST /v1/workflows/{id}/empty-selection-confirmations` | Request selected-only cloud acquisition, review the authenticated job result, and confirm exact fresh server-owned snapshots by preparation handle. The separate empty-selection command is the only no-notes path. |
| `PUT /v1/workflows/{id}/writing-settings`, `POST /v1/workflows/{id}/manual-compositions` | Set the current language/style/path and enter the always-available manual composition state through core commands. Unavailable model paths may be shown as unavailable but cannot be executed in M6. |
| `PUT /v1/workflows/{id}/entry`, `POST /v1/workflows/{id}/entry-confirmations` | Save the entry buffer conditionally. Before confirmation, revalidate every confirmed source at `BeforeEntryConfirmation`, persist any resulting invalidation, and confirm only the current nonblank, marker-free entry against both workflow and buffer versions. |
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
Map adapter diagnostics to a bounded enumeration or reviewed safe text; never
serialize an arbitrary `Display`, `Debug`, or error-chain value into a DTO.
Raw front matter, audio references, private absolute source paths, credentials,
and the Notion root do not cross the wire. Use synthetic contract fixtures or
a schema artifact checked with tests; a new generator is a build-tool approval
item.

Freeze explicit numeric limits with the contract before implementation: JSON
body/depth/string/collection limits, candidate and selected-source counts,
total prepared text, response size, jobs and subscribers, queue depth, worker
concurrency, execution/stream lifetime, and port-attempt count. The existing
8 MiB per-source limit is an upper bound, not permission for an unbounded
multi-source response. Paginate bounded lists or expose bounded detail reads;
do not silently truncate a candidate set or return source bodies in the main
workflow representation. Contract fixtures record diary days as `YYYY-MM-DD`,
the timestamp representation, ID grammar, unknown-field rejection, media
types, and every strong ETag grammar.

Source preparation is an authenticated, expiring in-memory job result keyed by
an unguessable server-issued handle and bound to the workflow ID/revision and
the exact selected source references. It contains the sanitized body, trusted
stamp, and post-read date evidence for review. Selection confirmation supplies
only that handle and source choices; the server revalidates every prepared
snapshot immediately before a single conditional commit. A changed,
unverifiable, expired, cancelled, or pre-restart preparation cannot confirm and
requires a new read. Prepared results are never durable and SSE carries only a
job reference and safe progress, not a body, filename, source reference, or
warning text.

M6 owns the `PostRead` check used by preparation and the
`BeforeEntryConfirmation` check used by the manual flow. Later milestones own
the extraction, drafting, preview, and first-write checkpoints. If a source
check changes or invalidates a workflow, the command service conditionally
persists that invalidated canonical state before returning the typed failure;
an error response must not leave stale confirmation or preview state durable.

## Conditional mutations and error behavior

Represent workflow and entry-buffer versions as separate opaque strong ETags.
All mutations of an existing workflow carry `If-Match` for the workflow
revision, except entry autosave, which uses the buffer ETag. Entry confirmation
checks both versions in one core operation. Missing `If-Match` for a workflow
or entry mutation is `428`; a stale precondition returns `412` with a safe
problem and no state change. A known domain conflict, including
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
Capture the launch working directory once; Phase 1 resolves the default config
and `.local/logs/control-api/` beneath that root and never searches parent
directories or follows a later working-directory change. Startup order is:
create and validate the owner-only Application Support/runtime directories;
acquire the advisory single-instance lock; verify the configuration is present,
owner-owned, owner-only, regular, and not linked; initialize and prune the
diagnostic directory; securely open and migrate the Application Support SQLite
database; purge expired draft text; load unresolved-intent gates; bind only
`127.0.0.1` using `8771`, `8772`, then the configured bounded fallback range;
make health answer for the new instance; atomically publish the owner-only
runtime file; and start background reconciliation. The dependency/contract
approval packet names the exact bounded port-attempt count and config key.
Failure before runtime publication leaves no live server advertised. Exhausting
the port range fails with a clear content-free error. Isolate paths and port
ranges in tests so no test touches real Application Support state or assumes
`8771` is free.

An absent or unsafe config file is fatal, but optional feature availability is
not. In particular, a missing/unconfigured Notion destination or unavailable
Apple helper is reported through authenticated setup/capability DTOs and does
not prevent a manual no-notes workflow. M6 does not contact the Notion root,
create an entries directory, or select a hosted provider during startup.
Create the database, journals/WAL files, lock, runtime file, and their parent
directories without following symlinks; verify opened-file ownership, type,
mode, link count, and identity at the descriptor boundary. Database and runtime
files are mode `0600`; their private directories are mode `0700`. Unsupported
schema, corruption, unsafe sidecars, migration failure, or purge failure stops
startup without replacing existing state.

Load unresolved intents and apply their per-day `Reconciling` gates before
accepting workflow mutations; the reads that resolve them run in the
background after startup. An unavailable destination must not keep another
day from working.

The runtime directory is owner-only and the runtime file contains only
the chosen port, ephemeral full-capability control token, and instance ID.
Check ownership, mode, file type, and symlink behavior before trusting or
replacing local control files. A leftover file from a crash is not proof of
a live process: the lock and health instance must agree. Rotate the token
on each start; generate at least 256 bits from the operating-system CSPRNG and
compare its fixed representation without data-dependent early exit. Publish
the runtime file with a same-directory temporary file, sync, atomic rename,
and directory sync. A clean shutdown removes only its matching instance's
runtime file before releasing the lock; a crash is recovered by the next lock
holder. The first release grants the TUI one
full-capability token. Capability checks still live in the server, and future
narrower clients must get separate tokens. The token proves access by a
process in the same macOS account, not a human gesture.

Accept only the exact numeric loopback `Host` and chosen port for the listener;
do not accept `localhost` or forwarded-host headers. An absent `Origin` is
valid for the native client, while a present origin must exactly match the
listener. Apply these checks on every route, including health, before bearer
processing, and do not enable CORS or trust proxy headers.
Keep binding and header tests at a real loopback socket, not only an
in-process router. Health exposes no token or diary state. Before publishing
the runtime file, ensure the listener can answer health with the new instance
ID. On interrupt or termination, stop accepting mutations, let an in-flight
external write finish up to a bound, request cancellation of other jobs,
and leave unresolved intents for the next start. Background reconciliation
may leave an unreachable destination pending while other days remain usable.

## Internal diagnostics and coding-agent access

The server owns one internal file sink in addition to concise foreground
standard error. Its production default is
`.local/logs/control-api/` relative to the server working directory. This path
is already beneath the repository's Git-ignored `.local/` tree and is separate
from the Application Support runtime directory that contains the control
token. Tests inject an isolated log path; production does not accept a request,
header, or API parameter that redirects logging. Create the directory as mode
`0700` and segments as mode `0600`; reject symlinks, non-regular files, wrong
owners, hard-linked segments, and group/world-accessible pre-existing paths.
Logging initialization and retention cleanup happen after the single-instance
lock and validated configuration but before the database or listener opens, so
an unsafe or unprunable log directory fails startup without publishing runtime
state. Validate opened descriptors, not only path metadata, so a path swap
cannot bypass the checks.

Use a versioned UTF-8, one-record-per-line format designed for `tail`, `rg`,
Codex, and Claude Code. The v1 grammar is
`ndlog-v1 ts_unix_ms=<u64> level=<LEVEL> component=<token> event=<token>`
followed by event-specific `key=<token-or-u64>` fields in sorted-key order.
Levels are `TRACE`, `DEBUG`, `INFO`, `WARN`, or `ERROR`; tokens are nonempty,
at most 64 ASCII bytes, and contain only letters, digits, `.`, `_`, `:`, or
`-`. A complete encoded record, including its one terminating LF, is at most
1 KiB. The event schema may add an optional random correlation reference and
numeric duration, status, and capacity values, but the logging API does not
accept free-form messages, unknown keys, or `Display`/`Debug` objects. In
particular, never log diary/source text, front matter, dates or source
references, private paths, filenames, request targets or query strings,
headers, bodies, tokens, credentials, runtime-file contents, model/provider
payloads, platform error descriptions, or arbitrary error chains. Route
diagnostics use a predefined route-template code and stable outcome code rather
than the raw URL or error. Reject every value that cannot satisfy the grammar;
never escape private input into a line. The same sanitization applies to the
limited warning/error records mirrored to standard error.

Keep an active `control-api.log`; atomically close and rename it at a UTC-day
change in either direction or before it would exceed 8 MiB. On startup,
validate complete records and truncate only an incomplete trailing record back
to the last LF before appending; a malformed complete record or unsafe closed
segment fails initialization rather than being silently reinterpreted. Rotated
names contain only the rotation Unix timestamp plus a collision sequence and
are created with `create_new`, so restart or clock rollback cannot overwrite a
segment. Run
maintenance at startup, after every rotation, and at least hourly while the
server is running. The hourly task closes an idle active segment when its UTC
day has ended, so an old final record cannot remain indefinitely merely because
no newer event arrives. Delete closed segments whose rotation/last-record time
is at or before `now - 72 hours`; then, if the directory still exceeds 24 MiB,
delete the oldest closed segments until it is within the cap. Never delete the
current active segment or an unrecognized entry. Include every regular entry
in the cap calculation; if an unrecognized entry prevents compliance, fail
initialization or request controlled shutdown rather than deleting it. A fake
clock and injected filesystem boundary make the exact cutoff, idle rotation,
restart, rollback, collision, torn-write, malformed-segment, and failure cases
deterministic. If initialization or pruning cannot uphold permissions or the
retention contract, refuse startup. A later
write, rotation, or prune failure emits one content-free standard-error code,
stops file logging, and requests a controlled server shutdown rather than
silently abandoning the retention promise.

The log directory is readable directly by same-account coding agents and is
not exposed by REST, SSE, or MCP. P1-M6 completion adds short read-only
instructions to `AGENTS.md`—automatically shared with Claude Code through the
repository's `CLAUDE.md` import—and the contribution guide. Those instructions
show bounded commands such as `tail -n 200 .local/logs/control-api/control-api.log`
and `rg 'level=(WARN|ERROR)' .local/logs/control-api/`, prohibit permission
changes, and reiterate that agents must not read the separate Application
Support runtime directory or its token. Agent-read acceptance uses synthetic
records from the repository root and does not grant either agent an API
capability.

## Jobs, events, and recovery

Long discovery, selected-source acquisition, freshness, and other blocking
work returns `202 Accepted` with a job reference. Give jobs bounded queue
capacity, concurrency, execution time, result size, result lifetime, and
cancellation state. Job records and prepared results are process-local in M6;
after restart their IDs are unknown and canonical workflow state determines
what must be repeated. An accepted job captures the initiating workflow
revision and rechecks it before any commit, so a late result cannot replace
newer selection or entry state. Discovery commits candidates conditionally;
source preparation does not commit a selection and instead yields the bounded
review handle described above. Cancellation is a request; it cannot undo a
completed download or other external effect. A job whose response is lost is
queried before a command is retried while that server instance is still live.
Later publication commands need idempotency keys and destination
reconciliation; M6 must not simulate remote idempotence.

Publish state-change SSE only after state commits. Events carry event type,
workflow ID, revision, job ID if relevant, and safe progress fields; never
source text or entry text. Event IDs are instance-scoped, monotonically
increasing sequence values with a bounded replay window. Bound subscriber
count, replay/event queues, heartbeat cadence, and stream lifetime so a slow
client cannot retain unbounded memory or delay commits. A reconnect with a
still-buffered `Last-Event-ID` may replay notifications; a server restart, evicted/unknown ID,
subscriber overflow, or any detected gap yields a reset notification and the
client fetches the current workflow and relevant job representations before
mutating. The workflow is authoritative; SSE is only a notification path.
Multi-client tests must show ordered revisions for one workflow, no event
before its commit, and safe refetch when notifications are missed.

## Implementation and verification sequence

1. Check the P1-M4/P1-M5 completion evidence and implemented contracts. Close
   the implemented-contract audit with focused core/SQLite tests, then obtain
   approval for the dependency and exact-limit packet.
2. Define the complete P1-M7 manual-route table, command mapping, wire
   revisions, ETags, safe problem codes, numeric bounds, and synthetic contract
   fixtures before starting the listener.
3. Build isolated lifecycle primitives, the typed rotating diagnostic sink,
   and the `nd serve` composition root. Prove lock, config, log permissions and
   retention, migration/purge, bind, runtime publication, stale-file recovery,
   token rotation, shutdown, and restart with temporary state.
4. Add loopback HTTP, health negotiation, bearer/capability checks, header
   validation, and bounds. Test these through a real socket and in-process
   route client before exposing workflow mutations.
5. Map manual workflow reads and conditional commands through the command
   service. Test list/identity behavior, trusted source preparation, explicit
   no-notes continuation, writing settings/manual start, concurrent clients,
   independent buffer autosave, pre-confirmation freshness, confirmation races,
   durable invalidation on access failure, and safe typed error responses.
6. Add bounded job workers and SSE notifications. Test cancellation, expiry,
   restart, late results, disconnected/slow subscribers, replay gaps, and
   canonical refetch.
7. Update the contract artifact, README, contributor startup guidance, and
   affected architecture docs. Record focused evidence in the overview and
   run the full Rust and Swift gate once after implementation. Mark P1-M6
   `Done` only when every checklist item and the gate have evidence.
