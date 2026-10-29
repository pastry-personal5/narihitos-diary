# P1-M8: Draft assistants architecture plan

Status: Done

This plan extends the implemented manual workflow with action-specific,
version-bound assistant proposals. It keeps provider execution at the outer
edge, makes the server the only workflow authority, and prevents a delayed or
cancelled model result from overwriting current work.

## Affected modules and interfaces

| Area | Implemented baseline | P1-M8 change |
|---|---|---|
| `diary-core` | Backend ID/descriptor/selection, one flat assistant envelope, string responses, reviewed-memory text, assistant draft, and freshness invalidation | Registry policy, action-specific request/result types, source-linked memory candidates, artifact bindings, guarded draft population, paragraph suggestions, validation, and commit preconditions |
| `diary-connectors` | SQLite schema v4, Ayumi/Apple source adapters, and no assistant adapter | Forward migration, artifact persistence, chosen hosted adapter, Apple assistant client, and optional approved Codex process adapter |
| `diary-control-contract` | Native contract version 2, descriptor DTOs, manual routes, generic jobs/SSE | Deliberate M8 contract revision, assistant resources and problems, bounded result DTOs, and strict fixtures |
| `diary-control` | Empty backend list, rejection of non-null selections, in-memory bounded jobs, strict scalar configuration | Registry assembly, selected-backend validation, assistant commands/jobs, commit fences, dynamic readiness, and collection-shaped configuration |
| `helpers/apple-bridge` | Protocol v1 iCloud commands with a 64 KiB frame | Versioned Foundation Models availability and action commands behind a fakeable service, plus an approved bounded content transport |
| `diary-tui` and `apps/nd` | Manual Compose/Review, bounded HTTP/SSE workers, autosave/recovery, adaptive layout | Backend/resource ViewModels, memory review, editable assistant drafts, paragraph review, progress/cancel, and assistant recovery inside Compose |

The TUI remains an HTTP client. It never loads provider configuration, starts a
helper or provider process, reads SQLite, or constructs assistant prompts.

## Registry and selection authority

Server startup assembles a `DraftAssistantRegistry` keyed by
`AssistantBackendId`. Core policy and transport consume a collection and resolve
an ID; no layer switches on a hosted/Apple/Codex provider enum. Duplicate IDs
fail startup before runtime advertisement. Descriptor order is deterministic,
but identity never depends on list position.

Each descriptor contains a stable ID, bounded display label, composition path,
current availability with a safe reason code, supported output languages, and
supported actions. Availability is a snapshot for display, not permission to
skip an invocation-time check. Apple locale/model readiness, hosted credentials
or reachability, and an approved child process may change after startup, so the
server rechecks the selected backend and requested action immediately before
each call.

Preferences remember the last selected backend ID only. When a workflow has no
selection, the server resolves the remembered ID if it is registered and usable;
otherwise it selects the owner-chosen hosted default when that backend is
usable. Selection never invokes it automatically. Manual composition has
neither a backend ID nor a composition path. Changing selection is a conditional
workflow mutation and invalidates affected assistant artifacts while preserving
manual entry text.

## Action-specific privacy boundary

The existing `DraftAssistantRequest` and `DraftAssistantResponse` are migration
placeholders, not the final M8 API. Replace them with a closed action-specific
conversation so an adapter cannot observe every nominally allowed data class on
every call.

| Action | Data the server may supply | Valid structured result | Data deliberately absent |
|---|---|---|---|
| Extract memories | Confirmed readable bodies and request-local source ordinals | Bounded memory candidates with text and one-or-more request-local source ordinals | Writing language/style (sources are en-US only per the decided language scope, and extraction does not produce prose; drafting applies language/style), persistent source IDs/digests, note names, front matter, paths, metadata, entry text, Notion data |
| Draft entry | Human-reviewed memory text, language, style, and prior draft text only when the person explicitly requests revision | Entry text plus validated inferred-reflection ranges | Source bodies and identities, focused paragraph, publication state |
| Review paragraph | The exact displayed focused paragraph, language, and style | Replacement plus validated inferred-reflection ranges plus optional bounded explanation | Other paragraphs, complete entry, source bodies/identities, memories unless a later owner decision expands this row |

Request-local source ordinals exist only to correlate extraction output. The
server validates them against the ordered confirmed snapshots, then persists
local source-snapshot bindings with each memory candidate. Ordinals are not
stable identifiers and never appear in later drafting requests. This satisfies
source-linked review without sending a source path, name, or persistent ID.

Constructors enforce per-field and aggregate bounds before adapter code runs.
Provider serializers accept only these types, and privacy tests inspect their
actual serialized requests. Prompts and schemas treat source text as quoted,
untrusted content. No adapter receives a callback or capability for filesystem,
shell, connectors, workflow transitions, confirmation, approval, or
publication.

For an agent-style backend, omission from the Rust port is necessary but not
sufficient: its process/API configuration must also deny unrelated tools,
filesystem access, shell execution, and external actions. If that cannot be
proven for an integration surface, that surface is not eligible for M8.

## Artifact identity, review, and invalidation

Every assistant artifact has an opaque local ID, the backend ID and
composition path that produced it (provenance, shown in the UI, never itself
a reason to discard the artifact), the action and a safe creation time, and a
digest of its content, never the content itself, in diagnostics or SSE. Beyond
that common envelope, each artifact type records only the binding data its row
in the invalidation matrix below depends on, so a binding field that is not a
staleness trigger for a given artifact type is not stored as one:

- **Memory candidates**: the confirmed source-selection digest only. Backend,
  language, and style are recorded as provenance but are not staleness
  bindings — extracted facts do not change meaning when the person later picks
  a different backend or style for drafting.
- **Reviewed memories**: the memory-candidate set they were confirmed from
  (so a later candidate change can be detected), with the same
  non-triggering backend/language/style provenance as candidates. The person's
  own edits to reviewed-memory text are not provider output and are never
  staled by a provider or settings change.
- **Draft proposals**: confirmed source-selection digest, writing language and
  style, backend ID, and the reviewed-memory revision/digest they were
  drafted from. Any of these changing stales the draft. (Workflow revision is
  deliberately not a binding here: it bumps on unrelated mutations too, and
  using it would widen staleness beyond this matrix.)
- **Paragraph suggestions**: writing language and style, backend ID, the
  entry buffer version, UTF-8 byte range, and selected-text digest of their
  target, and the confirmed source-selection digest. The source digest is a
  binding even though the paragraph-review request never carries source text
  (see the action-specific privacy boundary above) — binding data is
  server-side bookkeeping, never part of what is sent to a provider, so
  excluding something from egress does not exclude it from the binding. This
  is required because accept/reject are plain conditional writes, not jobs:
  the uniform freshness recheck in "Jobs, freshness, cancellation, and
  recovery" below only runs while a job is in flight, so a *completed*
  suggestion sitting unreviewed needs its own binding to go stale once the
  person rereads and reconfirms sources — otherwise its buffer/range/digest
  could still match after reconfirmation and a stale suggestion could be
  accepted.

Extraction creates a memory-candidate resource. The person can include, exclude,
or edit candidates and conditionally replace the reviewed-memory set. Drafting
is disabled until this explicit memory review completes. A memory change stales
draft proposals but does not alter the entry buffer.

Drafting captures the current entry buffer version. On successful completion,
the server may populate the buffer and advance its version only if that captured
version and every other artifact binding still match. The completed assistant
draft then becomes the active editable text in the Main Pane, as required by the
decided TUI behavior. If the person typed or another client saved in the
meantime, the result remains an inspectable stale assistant artifact and cannot
replace the newer buffer. Populating the editor creates ordinary unconfirmed
text; it is not acceptance or confirmation of the complete entry.

The provider returns structured inferred-reflection ranges rather than trusted
reserved delimiters. Core validates UTF-8 boundaries, ordering, overlap, count,
and aggregate size, rejects raw reserved delimiter characters in assistant
output, and inserts the delimiters itself. Existing Review behavior then points
to the first unresolved marker and refuses complete-entry confirmation.

Prior draft text sent for a revision drafting request, and the exact focused
paragraph sent for paragraph review, may already contain core-inserted
reflection delimiters marking an unresolved inference. Rather than stripping
those markers and trusting a provider to reproduce them accurately — which
would let a revision silently drop an unresolved marker or invent an unmarked
one, with nothing in the request or response to catch it — the server refuses
to start a revision-drafting or paragraph-review action while the target text
still contains an unresolved reflection marker, with a safe typed reason
telling the person to resolve it in the editor first. This is the same
constraint already in place for complete-entry confirmation, extended one step
earlier. Both actions remain available once no unresolved marker remains in
the target, at which point the request contains plain text with no delimiters
to round-trip. A fresh extraction-then-draft pass is unaffected: reviewed
memories are plain text the person wrote or edited, never markered text.

Paragraph review targets the blank-line-delimited Markdown paragraph containing
the editor cursor. The TUI flushes the entry and shows the exact canonical
target before submission. The server derives and stores its byte range and
digest. Accept conditionally replaces the target and advances the buffer
version; reject records the resolution without changing a byte. Because the
suggestion binds the complete buffer version, any intervening edit anywhere in
the entry makes it stale; because it also binds the confirmed
source-selection digest, a reread-and-reconfirm after a detected source
change makes it stale too, even though the buffer itself did not move.
Acceptance and rejection are idempotent terminal resolutions and neither
confirms the complete entry.

Invalidation follows this matrix; a changed row stales the listed artifacts
and nothing else. An artifact not listed for a given change survives it:

| Change | Memory candidates | Reviewed memories | Draft proposals | Paragraph suggestions | Entry buffer |
|---|---|---|---|---|---|
| Source selection or snapshot digest | Staled | Staled (derived from staled candidates) | Staled | Staled | Unaffected |
| Writing language or style | Unaffected | Unaffected | Staled | Staled | Unaffected |
| Backend selection | Unaffected | Unaffected | Staled | Staled | Unaffected |
| Reviewed-memory edit/replacement | Unaffected (superseded by the edit) | — | Staled | Unaffected | Unaffected |
| Entry buffer edit (manual or applied suggestion) | Unaffected | Unaffected | Unaffected (inspectable; cannot reapply to a changed buffer) | Staled | — |

A backend or style change does not stale already human-reviewed memories: the
person may extract with one backend and draft with another, or change style
after review, without repeating extraction. Only drafting and paragraph
review are backend/style-sensitive, because their output is prose in that
language and style; memory candidates are backend-neutral facts. Stale
resources remain bounded and inspectable long enough to explain recovery but
cannot be adopted or applied. Existing discard and retention policies purge
their content with the workflow's other draft/source text.

## Jobs, freshness, cancellation, and recovery

Assistant calls use the existing bounded server job system but add an explicit
commit fence and assistant-specific limits from the approval packet. One
workflow may have at most one active assistant mutation job in total, counted
across extraction, drafting, and paragraph review together, not one slot per
action kind; other workflows remain governed by the global queue and worker
caps. A request for a second action while one is active is rejected with a
safe "assistant job already active" problem rather than queued, even if the
two actions target different artifacts (for example, a paragraph-review
request while a draft job is still running). This keeps the commit-fence and
staleness reasoning in the jobs sequence below to one job at a time per
workflow; running paragraph review concurrently with drafting is not approved
scope and would need its own cross-artifact race analysis first.

The server performs this sequence:

1. Authenticate and validate the conditional request, workflow state, selected
   backend, action capability, language, and bounds.
2. Flush no client data implicitly; build the request from canonical persisted
   state only. Revalidate every confirmed source before acquiring commit
   authority.
3. Capture that action's artifact binding (per the per-artifact-type list
   above, not a uniform tuple), enqueue one bounded job, and invoke exactly
   one registry member.
4. On cancellation or timeout, signal the provider/helper/child, revoke commit
   authority first, and discard all late output.
5. Validate structured output, unconditionally recheck source freshness, then
   recheck that artifact type's own captured bindings.
6. Persist the complete artifact with a compare-and-swap. Only after that
   durable commit succeeds may the job become `completed` and emit a
   content-free event.

The job DTO and SSE event expose only job/artifact IDs, kind, progress, terminal
status, and safe reason code. They never carry a prompt, source ordinal/body,
memory, draft, paragraph, provider payload, or child output. The client fetches
the canonical artifact resource after completion.

Jobs remain instance-local. A server restart turns an interrupted call into a
safe failed/absent job with no partial artifact; durable artifacts committed
before the restart remain recoverable. Cancellation is successful only when the
commit fence prevents later mutation, even if an underlying blocking API cannot
be stopped immediately. Automatic retries are disallowed unless the approved
provider packet proves the request was never delivered; the person may start a
new action from current state.

## Control API revision

Advance the native contract version while retaining the `/v1` namespace. Add
strict DTOs and route-table entries only after an endpoint review fixes exact
names, methods, capabilities, ETag type, bounds, and problems. The resource set
must cover:

- reading backend capabilities and readiness;
- starting extraction, drafting, and paragraph-review jobs;
- reading and conditionally replacing the current memory review;
- listing/fetching durable current assistant artifacts after job completion or
  restart;
- conditionally accepting or rejecting a paragraph suggestion; and
- observing and cancelling jobs through the existing job resources.

Commands name or resolve exactly one backend and must match the workflow's
current selection. The server derives the descriptor's composition path; a
client cannot submit a path. Assistant mutations use the workflow ETag, entry
buffer ETag, or both according to the artifact bindings. Unknown backend or
artifact IDs, unavailable/unsupported actions, stale bindings, already-resolved
suggestions, and disabled empty-selection actions have distinct safe problem
codes.

Large text does not ride in SSE or job DTOs. Collection endpoints are paged or
strictly capped, `Cache-Control: no-store` remains universal, and response
bounds account for the approved maximum draft or paragraph size. A contract
fixture proves unknown fields and old/new mixed clients fail closed.

## Global configuration and Codex profiles

The approved configuration shape is a collection of structured records, not a
singleton and not a shell-command string:

```yaml
assistants:
  codex_profiles:
    - alias: "pp7"
      codex_home: "/Users/user1/.codex-pp7"
    - alias: "work"
      codex_home: "/Users/user1/.codex-work"
      codex_executable: "/absolute/path/to/codex"  # optional; else resolved and pinned at config-load time
```

This structured shape supersedes an earlier approved `alias`/`run_command`
shell-lookalike-string shape (`run_command: "CODEX_HOME=... codex"`), which no
implementation ever used; `codex_home` and `codex_executable` directly replace
it, so there is no command grammar to parse.

The owner-approved hand-rolled parser accepts exactly this nested shape.
Parsing and readiness are two different gates, and only syntax and shape can
fail server startup; an unmounted volume or not-yet-installed `codex` instead
makes that profile's descriptor unavailable:

- **Parse-time (startup error):** unknown or duplicate keys, duplicate
  aliases, invalid backend-ID aliases, a missing or non-absolute
  `codex_home`, a non-absolute `codex_executable` when present, control
  characters, path-traversal segments, and excessive counts/lengths. Alias
  order is display order; renaming an alias creates a new backend identity
  such as `codex.pp7` and requires reselection.
- **Readiness (typed unavailable backend, checked only once a Codex adapter
  packet is approved, and rechecked before each launch, not at startup):**
  `codex_home` exists and is a directory with safe ownership, the resolved
  executable exists and is runnable, and `PATH` resolution succeeds when
  `codex_executable` is absent. A profile failing readiness is unavailable,
  never a startup failure, and never causes the server to probe a different
  home.

`codex_home` and `codex_executable` are data, never shell source: there is no
command string to tokenize, so there is no quoting, operator, substitution, or
redirection grammar to reject, only two absolute-path fields. When
`codex_executable` is absent, the server resolves `codex` on
its own launch `PATH` once at config-load time and pins that resolved absolute
path for the life of the process, so a later `PATH` change cannot silently
change which binary a profile launches. Pinning the resolved path only
removes the `PATH` dependency if `codex` is a native binary; if it is a
script (for example starting `#!/usr/bin/env …`), its interpreter is still
looked up through the *child* process's `PATH` when the script runs, so the
Codex adapter packet must check what `codex` actually resolves to (a `file`
check on the resolved path is enough to tell a native binary from a script)
and, if it is a script, set the child's `PATH` explicitly rather than relying
on pinning alone. The separately approved adapter supplies the one
`CODEX_HOME` child-process environment assignment (derived from `codex_home`,
never passed through a shell) and appends its own fixed argv; user
configuration cannot add flags, environment variables, or prompts.

Rust does not read, copy, enumerate, or log credential/configuration files
inside a Codex home directory — only the directory's own existence and safe
ownership (owned by the server's user or root, not group- or other-writable;
the same check applies to the resolved executable, where a root-owned system
install is the normal, safe case, not a riskier one) is checked at readiness
time, never the directory's contents. It does not inherit a profile from the
server's own `CODEX_HOME` or mutate the parent environment. Raw path values
stay inside config validation and process setup; API state, SQLite, TUI text,
diagnostics, prompts, and safe errors use only the alias-derived backend
ID/display label.

The approved Codex adapter registers each profile as a separate hosted registry
member and invokes only `codex app-server` with structured output, explicit
tool/filesystem/network denial, bounded in-memory capture, and termination of
the complete child process group. It never forwards raw stdout/stderr to logs
or clients. A profile that fails a content-free readiness check is a typed
unavailable backend; the server never probes a different home or account.

**Rejected earlier `codex exec` candidate (historical rationale).** Before the
owner selected `codex app-server`, the owner confirmed `pp7` and `work` are separate
Codex/ChatGPT accounts, so the per-alias `codex_home` separation above stays
in the same spirit as originally approved, now expressed through the
structured-fields shape. Current upstream
`codex exec` documentation (OpenAI's [non-interactive-mode](https://developers.openai.com/codex/noninteractive.md)
and [configuration reference](https://developers.openai.com/codex/config-reference)
pages) supports a one-shot, non-interactive invocation shaped like:
`codex exec --json --sandbox read-only --skip-git-repo-check --cd <private
empty dir> ...`. `--sandbox read-only` denies filesystem writes and
`--skip-git-repo-check` is needed because that private directory is not a Git
repository, but **`--cd` only sets the working directory — it does not confine
reads**. A sandboxed Codex process can still open any path its account can
read, including this repo's `config/global_config.yaml`, which holds the
plaintext Notion token (AGENTS.md "Private state and credential"). Hostile
text inside a diary source note could try to steer the agent into reading and
echoing that file. `--json` gives JSON-Lines output the adapter can parse
without scraping interactive text, but it is not a confinement mechanism
either. The adapter packet must therefore separately prove filesystem-read
and network denial — for example a container, a macOS Seatbelt/App Sandbox
profile, or another OS-level jail that actually restricts readable paths and
outbound connections, not flags alone — or this integration surface is
ineligible under this plan's own rule ("if that cannot be proven … not
eligible for M8"). The packet must also verify what each account's own
`$CODEX_HOME/config.toml` enables (MCP servers, web search, alternate model
providers) and prove the fixed argv neutralizes all of it; this needs its own
verification against the current Codex config reference rather than the
unverified secondary sources this review could reach. A secondary (unverified
against the primary references) source
also describes an `--output-schema <file>` flag for constraining the final
message to a JSON Schema, which would let the adapter request the same
structured shape as the hosted/Apple adapters; and an approval-policy flag
(`-a`/`--ask-for-approval`, values seen as `untrusted`/`on-failure`/
`on-request`/`never`) that must be pinned to a non-interactive value so the
child never blocks waiting for a human who is not there. The adapter packet
must re-verify these exact flag names and values against the then-current
Codex CLI version before relying on them, and still owns the
authentication/account-eligibility, version-compatibility,
timeout/cancellation, and retention-policy items already required above.

## Apple helper boundary

Add a fakeable `FoundationModelService` beside the existing `SourceService` and
version the helper protocol deliberately. Availability reports only safe reason
codes and supported app languages. Before every action, the live service checks
the current model availability and target locale; it does not assume that a
startup result remains true.

The approval packet chooses either a safely increased bounded frame or an
owner-only handoff mechanism for assistant content, for both the request and
the response direction — a drafted entry can approach the existing 512 KiB
entry bound, which will not fit a 64 KiB frame, so either the bound for
assistant output is capped well below the frame or the response is also
routed through a handoff rather than only the request.

If a handoff is chosen, reuse the already-implemented P1-M5 isolated
mode-0700 per-request handoff directory and cleanup design (see "Swift
iCloud command group and byte handoff" in the
[P1-M5 architecture plan](milestone-05-source-ingestion-apple-bridge-architecture.md))
for this new direction rather than specifying a second, parallel mechanism.
M5's handoff flows helper-to-Rust, created and cleaned by the helper; M8's
request handoff flows the other way, so the server — not the helper — creates
the per-request directory and payload file and is responsible for cleaning it
after success, failure, timeout, cancellation, or crash, while the helper only
validates and reads the exact path it was given (type, ownership, mode, link
count, size). The response direction, if also handed off, reverses this again
with the helper as creator/cleaner. No configured source path, Codex path, or
arbitrary file path is accepted, and no handoff file is reused across
requests.

Use Apple's runtime
[`contextSize` and token-count APIs](https://developer.apple.com/documentation/foundationmodels/systemlanguagemodel)
to reject an oversized request before generation
and guided structured output for each action. The helper returns only the
versioned result or stable diagnostic code. Raw prompts, generated content, and
framework error strings never reach standard error. Swift tests inject a fake
service for every success and failure; a live model is not an automated gate.

## TUI integration

M8 adds resources inside Compose and does not add or rename a workflow stage.
The server-supplied backend collection drives selection; a list larger than two
must scroll and render without a provider-specific branch. Backend and path are
visible in the Start Setup check, Compose Main Pane, Work Status, and Status
Line. Selection is informational and never triggers egress; only focused
Extract, Draft, or Review actions do.

The Resources Sidebar adds memory review, assistant drafts, and paragraph
suggestions alongside the editable draft and confirmed source notes. Activating
a resource only changes the Main Pane. Memory review exposes source linkage
using local source-note labels resolved by the server/TUI from confirmed
snapshots, never values returned by the provider. Draft replacement conflicts
and paragraph accept/reject show their impact and stale state without relying
on color.

All assistant HTTP work stays on the existing bounded worker path. The editor
remains interactive while a job runs; autosave has priority before any action
that depends on entry text. SSE is a notification only, so completion triggers
a canonical artifact/workflow fetch. Disconnect, event gap, restart, or queue
overflow disables mutations until canonical reload but leaves unsent text and
already fetched proposals readable. Manual editing is always available after
source confirmation, including every provider failure.

The existing bounded Command Line grammar need not grow: focus plus Enter can
reach every assistant action. Any future token requires a separate bounded
grammar and Korean 2-Set mapping tests; no provider prompt, alias command, or
arbitrary text is ever interpreted as a shell command.

## Implementation sequence

1. Obtain and record the owner-approved contract and migration packet first;
   it unblocks steps 2–4 and 8. Freeze the exact contract/schema versions and
   migration fixtures. The configuration, hosted, Apple, and optional Codex
   packets are obtained separately and only unblock their own later steps (5,
   6, 7, and the Codex runtime portion of 5), per the overview's split gate.
2. Replace the placeholder flat assistant types with action-specific requests,
   structured outputs, artifact bindings, validators, and registry policy.
   Prove privacy and routing with three fake backends before network/process
   code exists.
3. Forward-migrate SQLite for memory candidates/revisions, draft proposals,
   paragraph suggestions/resolutions, and bindings. Prove every prior schema
   reopens with existing content and receipts intact.
4. Revise the native Control API contract and server commands. Add conditional
   artifact resources, bounded jobs, freshness/commit fencing, cancellation,
   safe problems, and content-free events; prove mixed-version refusal.
5. Implement the approved strict collection parser. Keep Codex profiles inert
   unless their runtime adapter is separately approved; otherwise qualify the
   fixed process boundary entirely with fake launchers first.
6. Implement and qualify the chosen hosted adapter against a fake transport,
   including exact serialization, output validation, timeout/cancellation, no
   automatic ambiguous retry, and redaction.
7. Version the Apple helper, implement the approved payload transport and
   fakeable Foundation Models service, and qualify Rust/Swift failure and
   cleanup paths without a live model.
8. Extend the existing TUI client/worker/ViewModels and Compose resources for
   selection, memory review, guarded draft population, paragraph review,
   progress, cancellation, staleness, and recovery.
9. Run fake-backed vertical flows for both production adapter contracts and a
   three-backend registry, then reconcile user, API, UX, configuration, privacy,
   and contributor documentation.
10. Run the full Rust/Swift gate and record dated synthetic acceptance evidence
    in the milestone overview.

The hosted default is a preference, not a consent screen. The backend/path
labels disclose the active composition boundary, while the explicit assistant
action is the only trigger for content egress. A later Codex MCP client remains
a separate inbound interface and is not implemented here.
