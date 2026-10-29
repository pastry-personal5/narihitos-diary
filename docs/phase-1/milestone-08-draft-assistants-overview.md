# P1-M8: Draft assistants overview

Status: Active

This milestone adds optional assistant backends to the completed manual
workflow without weakening the Draft editor, source-freshness, or human-review
contracts. The plan is grounded in the implemented P1-M12/P1-M7 baseline.
Provider-neutral core, schema, Control API, and TUI work needs only the
contract and migration packet below; the strict `assistants:` configuration
parser (today the Codex profile collection, and any later hosted/Apple keys
added to the same file) needs the configuration packet; and each production
adapter (hosted and Apple) needs only its own packet; the hosted-default
packet is Codex, reached only through `codex app-server`. See "Decision and
approval gate" for the exact split.

## Goal

Offer a chosen hosted default and Apple on-device memory extraction, drafting,
and paragraph review through an extensible backend registry. Every action uses
exactly one selected backend, sends only its action-specific allowed data, and
produces a version-bound proposal that cannot confirm or publish an entry.

## Implemented baseline

P1-M8 extends these observed contracts rather than rebuilding them:

- P1-M12 implemented stable backend IDs and descriptors, optional backend
  selection, hosted versus Apple-on-device composition paths, assistant-neutral
  preferences, a placeholder `DraftAssistant` port, and contract version 2.
- SQLite already preserves reviewed-memory text and an assistant draft, while
  workflow invalidation clears or stales assistant material when sources or
  writing choices change.
- P1-M7 implemented the manual Start through Review vertical slice, bounded HTTP
  and SSE workers, job polling and cancellation, autosave with an independent
  buffer version, recovery, the six canonical stages, and the shared
  three-pane layout M8 must extend.
- The server still advertises no assistant backends and rejects every non-null
  backend selection. No extraction, memory, draft, suggestion, or paragraph
  review route exists.
- The current core request/response placeholders are not yet the M8 contract:
  they are one flat envelope of strings, cannot represent source-linked memory
  candidates, and do not prevent an action from receiving irrelevant allowed
  fields.
- The strict global-config reader currently accepts only the existing scalar
  sections; it cannot parse the approved collection of Codex profiles. The
  Apple helper protocol is iCloud-only and caps one frame at 64 KiB, so M8 must
  approve an assistant payload budget and transport before sending confirmed
  sources to it.

## Decision and approval gate

M8 is active for planning. The gate below is split by what it blocks, so
provider-neutral work is not held hostage to a provider choice that is still
undecided:

- The **contract and migration packet** (its own required section below) gates
  the provider-neutral work: action-specific core types, the SQLite forward
  migration, the Control API contract revision, the registry policy, and the
  TUI vertical slice. All of this is proven with fake backends only and does
  not require the provider, configuration, or Apple packets. Implementation
  sequence steps 2–4 and 8 in the architecture plan depend only on this
  packet.
- The **configuration packet** (its own required section below) gates step 5:
  implementing the strict parser for the `assistants:` section of
  `config/global_config.yaml` — today just the Codex profile collection, but
  also any new key a later hosted or Apple packet adds to that file (for
  example, if the hosted packet chooses to store its credential there). Any
  new YAML dependency needed to parse that section is approved by this packet,
  not inferred from the contract/migration packet. It does not gate steps 2–4
  or 8, which do not read `config/global_config.yaml` at all; it may gate part
  of step 6 or 7 if the hosted or Apple packet, once approved, adds
  configuration there.
- The **hosted-default packet** gates building and qualifying the production
  adapter (sequence step 6). The owner approved OpenAI's Codex, reached only
  through the `codex app-server` JSON-RPC surface, as this adapter — see the
  Phase 1 changelog entry recorded alongside this revision. This merges what
  was previously a separate "conditional Codex packet" into this required
  packet: every configured `assistants.codex_profiles` entry becomes a
  registry member once this packet lands (no further Codex-specific
  approval gates registration or launch), and the Codex runtime checklist
  items below are required, not "not applicable." Per-action request/response
  byte bounds default to the existing 512 KiB complete-entry limit until this
  packet's own bounds work says otherwise.
- The **Apple on-device packet** gates only the Foundation Models protocol
  version, service, and production adapter (sequence step 7).

Before manifests, DTOs, stored schemas, adapters, configuration syntax, or
live assistant calls change, the owner must approve and record the applicable
packet(s) above in `AGENTS.md` and the Phase 1 changelog.

### Required contract and migration packet (resolved)

- **Versions:** native-client contract version **3** (from 2) and SQLite
  schema version **5** (from 4). No version is skipped; nothing else is
  queued ahead of M8 that would need the room. Mixed-version clients/servers
  fail closed per the existing contract-version check.
- **Routes**, consistent with the existing `/v1/workflows/{id}/<resource>`
  and generic `/v1/jobs/{id}` conventions in `crates/diary-control/src/server.rs`:
  - `GET /v1/assistant-backends` — global registry descriptors (stable ID,
    label, composition path, availability/reason code, supported languages
    and actions). Not per-workflow; folded into `/v1/capabilities` as a
    field is also acceptable at implementation time, but the dedicated route
    is the one new resources must target.
  - `GET/PUT /v1/workflows/{id}/assistant-selection` — read/change the
    workflow's selected backend ID.
  - `POST /v1/workflows/{id}/memory-extractions` — start an extraction job;
    returns a job reference resolved at `/v1/jobs/{id}`.
  - `GET/PUT /v1/workflows/{id}/memory-review` — current memory-candidate
    set and reviewed-memory text; `PUT` conditionally replaces the reviewed
    set.
  - `POST /v1/workflows/{id}/draft-requests` — start a drafting job.
  - `GET /v1/workflows/{id}/draft-result` — the durable current draft
    artifact (survives restart).
  - `POST /v1/workflows/{id}/paragraph-review-requests` — start a paragraph-
    review job targeting the canonical focused paragraph the server derives.
  - `GET /v1/workflows/{id}/paragraph-suggestions` and
    `GET /v1/workflows/{id}/paragraph-suggestions/{suggestion_id}` — list and
    fetch durable suggestions.
  - `PUT /v1/workflows/{id}/paragraph-suggestions/{suggestion_id}/resolution`
    — conditionally accept or reject; body names the resolution and the
    bindings (buffer version/ETag, byte range, digest) it must still match.
  - Job observation/cancellation reuses the existing `GET/DELETE
    /v1/jobs/{id}`; no new job route is added.
- **Bounds** (new `diary-control-contract` constants, sized from the
  existing `MAX_ENTRY_BYTES = 512 * 1024`, `MAX_STRING_BYTES = 4 * 1024`,
  `MAX_COLLECTION_ITEMS = 256`, `MAX_CANDIDATES = 512`, and
  `MAX_SELECTED_SOURCES = 64` in `crates/diary-control-contract/src/lib.rs`):
  - `MAX_MEMORY_CANDIDATES = 128` per extraction result (sub-cap of the
    existing `MAX_CANDIDATES`, since one memory candidate is far larger than
    one discovery candidate).
  - `MAX_MEMORY_CANDIDATE_BYTES = MAX_STRING_BYTES` (4 KiB) per candidate's
    text.
  - `MAX_REVIEWED_MEMORY_BYTES = MAX_ENTRY_BYTES` (512 KiB) for the combined
    reviewed-memory text — bounded the same as a complete entry since it is
    person-edited free text with no smaller natural cap.
  - `MAX_SOURCE_ORDINALS = MAX_SELECTED_SOURCES` (64), matching the existing
    confirmed-selection cap.
  - `MAX_DRAFT_RESULT_BYTES = MAX_ENTRY_BYTES` (512 KiB) — a draft becomes
    entry text, so it cannot exceed what an entry itself may hold.
  - `MAX_PARAGRAPH_BYTES = MAX_STRING_BYTES * 8` (32 KiB) for a focused
    paragraph's input and its replacement — large enough for any realistic
    Markdown paragraph, far under the entry cap.
  - `MAX_SUGGESTION_EXPLANATION_BYTES = MAX_STRING_BYTES` (4 KiB) for the
    optional bounded explanation.
  - `MAX_REFLECTION_RANGES = 32` per result (draft or paragraph), each range
    a validated `(start, end)` UTF-8 byte pair.
  These are the hosted/Apple adapters' request/response bounds by default,
  per the hosted-default and Apple packets above, until either supplies a
  tighter number from its own provider limits.
- **Problem codes** follow the existing safe-error naming already in the
  Control API (short, snake_case, resource-scoped, no internal detail):
  `assistant_backend_unavailable`, `assistant_backend_unsupported_action`,
  `assistant_backend_unsupported_language`, `assistant_job_already_active`,
  `assistant_binding_stale`, `assistant_suggestion_already_resolved`,
  `assistant_action_disabled_empty_selection`, and
  `assistant_unresolved_reflection_marker` (the refusal described in the
  architecture plan's "Artifact identity, review, and invalidation" section).
- How the implemented v4 row maps onto v5: the existing `reviewed_memories`
  text column, `assistant_draft` text column, and single coarse
  `assistant_material_stale` flag (set by `invalidate_composition` on a
  backend, language, or style change, and by `invalidate_sources` clearing
  both columns outright on a source change, per
  `crates/diary-core/src/workflow.rs`) are replaced by per-artifact-type
  records with their own bindings. The safe default is to migrate existing v4
  rows as stale-but-inspectable v5 memory/draft records rather than
  discarding them, since existing recoverable drafts must survive the
  migration. This introduces two deliberate behavior changes the packet must
  record, not one: (1) a source change today discards reviewed memories and
  the draft outright via `clear_after_selection`; M8 instead keeps them as a
  stale, inspectable record — a real widening of what remains inspectable
  after a source change. (2) reviewed memories, which the coarse v4 flag
  staled on any backend/language/style change, now survive a backend or
  style change under the M8 invalidation matrix.

### Required hosted-default packet (resolved)

- **Provider and surface:** OpenAI's Codex, reached exclusively through the
  `codex app-server` JSON-RPC protocol over stdio. Not the Codex SDK (TypeScript,
  unreachable from Rust without a Node dependency this repo does not have) and
  not bare `codex exec` CLI flags (`--sandbox read-only`/`--cd` set a working
  directory but do not restrict which paths a command may read — confirmed
  against current Codex documentation). Supported output languages/actions
  match the contract/migration packet's action table (extraction, drafting,
  paragraph review); en-US and ko-KR per the decided language scope.
- **Process model:** one `codex app-server` child process per assistant job,
  not a persistent server per profile. Sequence: launch with
  `CODEX_HOME=<profile's codex_home>` and no inherited parent environment
  beyond what the process needs; JSON-RPC `initialize`; `thread/start` (or
  `thread/fork` with `ephemeral: true`) with no prior history; one
  `turn/start` carrying the action-specific prompt and `outputSchema`; collect
  the final structured message; terminate the complete process group. This
  keeps `CODEX_HOME` isolation, cancellation, and "no partial mutation
  survives a restart" process-scoped and easy to reason about; a persistent
  server per profile was considered and rejected for exactly that reason.
- **Per-turn confinement**, set on every `turn/start`, never left to the
  profile's own `config.toml`:
  - `sandboxPolicy: { type: "readOnly", access: { type: "restricted",
    readableRoots: [] } }` and `networkAccess: false` — no readable path at
    all; diary content rides in the prompt, never the filesystem, so the
    account can read nothing, including `config/global_config.yaml`'s
    Notion token.
  - MCP servers disabled (`-c mcp_servers={}` or the equivalent app-server
    launch override) and web search disabled (`-c web_search="disabled"`),
    so no account-side tool exists for a hostile prompt to invoke regardless
    of what the account's own `config.toml` otherwise enables.
  - Approval policy set so the child never blocks waiting for a human who
    is not present (the non-interactive/never-ask equivalent of `-a never`).
  - Local history/rollout persistence disabled (`history.persistence =
    "none"`, or an `ephemeral` thread) so no diary text is written inside
    `codex_home`.
  - Readiness check pins a minimum Codex version (0.138.0, the floor for
    restricted read-access policies) against the resolved binary at each
    profile's `codex_executable`; the installed development binary is
    0.165.0. A profile whose resolved binary is older is a typed unavailable
    backend, not a startup failure.
- **Credential and account location:** inside each profile's own
  `codex_home`, managed by Codex's own login state. Rust never reads, copies,
  enumerates, or logs it — only the directory's own existence and safe
  ownership is checked (unchanged from the already-approved configuration
  shape). Provider-side data use/retention is OpenAI's standard Codex/API
  account policy for the account the owner signs that profile into; this
  packet does not change or extend it.
- **Dependency:** a JSON-RPC-over-stdio client against the `codex app-server`
  child's stdin/stdout (length-prefixed or newline-delimited per the
  `app-server` protocol's actual framing — confirm against the resolved
  binary's protocol version at `initialize` time before freezing a Rust
  struct). No new third-party crate is required beyond what `diary-connectors`
  already uses for process spawning and JSON; if one is added, it follows the
  repository's existing dependency approval rule (Apache-2.0-compatible
  license, Rust 1.89 compatibility).
- **Limits and retries:** per-action request/response bounds default to the
  existing 512 KiB complete-entry limit (see the contract/migration packet's
  bounds table). Timeout and cancellation follow the jobs sequence in the
  architecture plan (kill the process group; revoke commit authority first).
  Assistant calls are never automatically retried after an ambiguous or
  partially delivered request — the person starts a new action from current
  state.
- **Structured output:** `turn/start`'s per-turn `outputSchema` (a JSON
  Schema) requests the action-specific result shape directly; core still
  validates the returned JSON (UTF-8 boundaries, reflection-range ordering,
  bounds) rather than trusting the schema alone, matching the Apple adapter's
  validation posture.
- **Test fakes:** a fake JSON-RPC-over-stdio peer (no real `codex` binary) for
  routing, confinement-parameter, timeout, cancellation, and malformed-output
  tests; qualification uses synthetic prose only, never a live account.

### Required configuration packet (resolved)

- **No new dependency.** `crates/diary-control/src/lifecycle.rs`'s
  `parse_config` is a hand-rolled, flat, line-based parser (one section
  level, `section:` then `  key: value`, rejecting unknown sections/keys).
  Pulling in a YAML crate for one nested `list-of-maps` shape would widen
  trust surface for no benefit, so the packet extends this same hand-rolled
  parser with exactly one more indentation level, scoped only to
  `assistants:` / `codex_profiles:` — not general YAML (no anchors, no
  arbitrary nesting, no flow style). A top-level `assistants:` section is
  added to the existing `matches!(section, "sources" | "paths" | "notion" |
  "server")` allow-list; under it, only a `codex_profiles:` key holding a
  `- alias: … / codex_home: … / codex_executable: …` block-sequence of
  2-indent-deeper maps is accepted.
- **Parse-time rules:** duplicate top-level `assistants:` section, duplicate
  or unknown keys inside a profile item, a duplicate `alias` across items,
  an `alias` that is not a safe bounded identifier (ASCII
  `[a-z0-9_-]{1,32}`, so it can form a backend ID like `codex.pp7` without
  escaping), a missing or non-absolute `codex_home`, a non-absolute
  `codex_executable` when present, control characters or path-traversal
  segments (`..`) in either path, and more than `MAX_COLLECTION_ITEMS`
  (256) profile items are each a startup error (`LifecycleError::InvalidConfig`,
  unchanged error type).
- Define duplicate-key, unknown-field, collection-size, alias, and
  absolute-path-length/validity behavior for `codex_home` and the optional
  `codex_executable`. There is no command string to parse under the
  structured-fields shape, so no quoting or shell-grammar rule is needed.
- Preserve owner-only loading and fail closed before server advertisement.
  Malformed configuration is a startup error; a syntactically valid optional
  adapter that lacks runtime credentials or readiness is a typed unavailable
  descriptor so manual editing and other backends remain usable. Missing
  credentials must never silently select another account.

### Required Apple on-device packet (resolved)

- **Deployment target:** the latest stable macOS/Foundation Models SDK only;
  no older-OS deployment floor is pinned (unlike the Codex 0.138.0 version
  floor). A readiness check against an unsupported OS/SDK is a typed
  unavailable backend, not a startup failure.
- **Production-adapter scope:** all three assistant actions — memory
  extraction, two-pass drafting, and paragraph review — for both `en-US` and
  `ko-KR`, matching the Codex hosted default's action/language coverage.
  Guided-output schemas, model/session lifecycle, and dynamic
  availability/locale checks are implemented for all three actions, not a
  drafting-only slice.
- **Payload transport:** per-action request/response bodies, including a
  drafted entry approaching the existing 512 KiB entry bound, travel over the
  Swift bridge's existing in-band JSON frame with its cap raised to cover that
  bound, in both directions. This does not reuse or extend the P1-M5 private
  handoff-file transport (owner-only creation, exact-path validation, cleanup,
  timeout, crash recovery); the bridge's framing is reimplemented as needed to
  carry the larger cap safely (length-prefixing, read/write buffering, and the
  existing 128-byte request-ID cap and protocol-only standard output are kept).
  Per-action token and output limits are still derived from the runtime model
  capability within that raised frame cap.
- Helper process timeout/cancellation behavior and safe mapping for unavailable,
  unsupported locale, guardrail refusal, context overflow, invalid output,
  crash, and cancellation. Tests use a fake model service only.

### Codex configuration and hosted-adapter integration (resolved)

The repo-local, owner-only `config/global_config.yaml` may contain zero or more
named Codex profiles as structured records: a user-chosen `alias`, an absolute
`codex_home`, and an optional absolute `codex_executable` (otherwise resolved
and pinned once at config-load time). For example, alias `pp7` may set
`codex_home: "/Users/user1/.codex-pp7"`. This approves profile multiplicity
and this structured representation; it supersedes an earlier approved
`alias`/`run_command` shell-lookalike-string shape, which no implementation
ever used.

Codex is now the approved hosted default (see the resolved hosted-default
packet above and the Phase 1 changelog), reached only through the
`codex app-server` JSON-RPC surface — not `codex exec`, not the TypeScript
SDK. Every configured profile becomes a separate hosted registry member once
the hosted-default packet's implementation lands; there is no further,
separate "register and launch" gate. Authentication/account eligibility is
whatever account the owner has signed into that profile's `codex_home`
(checked only for safe ownership, never for contents); version compatibility
is the 0.138.0 floor above; working directory, tool/filesystem/network
denial, stdout/stderr handling, timeout, cancellation, and retention policy
are specified in the resolved hosted-default packet above.

## Entry criteria

- [x] P1-M7 is `Done`, the phase plan marks P1-M8 `Active`, and the implemented
  M12/M7 core, SQLite, Control API, helper, and TUI seams have been inspected.
- [x] The active plan distinguishes already-implemented assistant-neutral
  foundations from M8 work. Codex is the required hosted default (not a
  separate conditional adapter); see the resolved hosted-default packet.
- [x] The owner approves the contract and migration packet (contract v3,
  schema v5, routes, bounds, and problem codes recorded above) before DTOs,
  the SQLite migration, the Control API revision, or the TUI vertical slice
  are implemented.
- [x] The owner approves the configuration packet (hand-rolled parser
  extension, no new dependency, recorded above) before the strict parser
  for the `assistants:` section of `config/global_config.yaml` is
  implemented.
- [x] The owner approves the hosted-default packet (OpenAI's Codex via
  `codex app-server`, recorded in `AGENTS.md` "Decided" and the Phase 1
  changelog) before that adapter is implemented.
- [x] The owner approves the Apple packet (deployment-target floor, raised
  bridge-frame payload transport, and all-three-action production-adapter
  scope, recorded above and in the Phase 1 changelog) before the Apple
  production adapter is implemented. Any dependency or Swift-package change
  follows the repository dependency approval rule.
- [x] Codex is the approved production M8 hosted backend; the Codex runtime
  checklist items below are required, not "not applicable." The
  already-approved profile configuration shape (recorded in `AGENTS.md`
  "Decided" and the Phase 1 changelog) covers both parsing and, once the
  hosted-default packet's implementation lands, registration and launch
  through `codex app-server`.

Automated tests use fake providers, a fake Foundation Models service, fake
process launchers, and synthetic prose. They never call a live model, inspect a
real Codex home, read a private diary, or send content to an external service.

## In scope

- A collection-shaped registry with stable IDs, labels, composition paths,
  dynamic availability, safe reason codes, supported languages, and per-action
  capabilities. Tests use at least three simultaneous fake backends.
- Action-specific extraction, draft, and paragraph-review contracts; durable
  memory/draft/suggestion resources; conditional result commits and paragraph
  accept/reject; and
  source/settings/buffer bindings that make late results stale instead of
  overwriting current work.
- The Codex (`codex app-server`) hosted default and Apple on-device production
  adapters behind the same `DraftAssistant` policy boundary.
- Strict parsing for zero, one, or many approved Codex profile records, plus
  registering each as a hosted registry member and launching confined
  `codex app-server` processes against them.
- Control API capabilities and assistant resources, bounded background jobs,
  cancellation/commit fencing, content-free SSE and diagnostics, and restart
  recovery.
- Compose-stage backend selection, memory review, editable assistant drafts,
  paragraph suggestion accept/reject, progress/cancellation, visible
  backend/path labels, and manual fallback.
- Type-level and runtime enforcement of the hosted-data egress envelope.

## Out of scope

- Automatic source confirmation, memory acceptance, entry confirmation, output
  approval, publication, amendments, or an agent-client `/mcp` surface.
- Launching Codex through anything other than `codex app-server` (no
  `codex exec` shelling-out, no Node/TypeScript SDK dependency).
- Arbitrary shell syntax, provider-selected commands, changing the parent `nd`
  environment, enumerating unconfigured homes, or reading/copying a Codex
  home's credentials from Rust.
- Giving any adapter a shell, filesystem, connector, workflow-transition,
  confirmation, approval, or publication capability. An agent-style adapter
  must also deny those tools at its own process/API boundary.
- Sending front matter, metadata, persistent source IDs, paths, audio,
  credentials, Notion details, logs, or arbitrary errors to an assistant.
- Any assistant action for an explicitly confirmed empty source selection.

## Completion checklist

### Decisions, dependencies, and migration

- [x] The owner-approved contract/migration, configuration, and hosted
  (Codex/`codex app-server`) packets are each recorded before their own
  respective implementation begins; the Apple packet remains pending and
  gates only the Apple production adapter (sequence step 7). Manifests and
  resolved dependency metadata must match the approved packet.
- [ ] The contract version is deliberately advanced for the M8 native-client
  resources, mixed versions fail closed, and a forward SQLite migration
  preserves every existing workflow, source snapshot, entry buffer, preference,
  output record, receipt, and operation intent.
- [ ] `config/global_config.example.yaml` documents zero-or-more Codex profiles
  in the structured `alias`/`codex_home`/`codex_executable` shape. Strict
  synthetic parse-time tests (these alone gate server startup) cover zero,
  one, and multiple items; duplicate or invalid aliases; duplicate/unknown
  keys; a missing, relative, or otherwise unsafe `codex_home`; a relative or
  unsafe `codex_executable` when present; and excessive values. Separate
  readiness tests cover a nonexistent/unsafe-owned home directory, a
  nonexistent or non-runnable executable, a resolved binary below the
  0.138.0 restricted-read floor, and `PATH` resolution failure when
  `codex_executable` is absent — each surfacing as a typed unavailable
  backend, never a startup failure, without reading a real profile.
- [ ] Every configured profile maps to one distinct descriptor and
  remembered backend ID, and fake-launch tests (a fake JSON-RPC-over-stdio
  peer, no real `codex` binary) prove that only the chosen profile's
  `codex_home`-derived `CODEX_HOME` assignment, the restricted `turn/start`
  confinement parameters, and fixed launch arguments are used, without a
  shell or parent-environment mutation.

### Core policy, privacy, and persistence

- [ ] The flat placeholder request/response types are replaced by
  action-specific types. Extraction can send only confirmed readable bodies
  plus request-local source ordinals — not writing language or style, since
  sources are en-US only and extraction produces no prose; drafting can send
  only reviewed memories, explicitly selected prior draft text, and writing
  settings; paragraph review can send only the displayed focused paragraph
  and writing settings.
- [ ] Extraction returns bounded source-linked memory candidates. Persistent
  links use local source-snapshot digests or identities resolved by the server;
  persistent source IDs, note names, metadata, and paths never enter an
  assistant request.
- [ ] Memory candidates, confirmed memories, draft proposals, and paragraph
  suggestions each persist only the binding fields their own artifact type
  needs (per the architecture plan's per-type binding list), not a uniform
  backend/settings/workflow/buffer tuple. A source, language, style, backend,
  memory, or entry change invalidates exactly the affected material per the
  invalidation matrix, without discarding manual entry text.
- [ ] Draft output is validated and conditionally populates the unconfirmed
  entry buffer only when the buffer version captured by the job is still
  current. A completed draft opens as editable text as decided; a late result
  cannot overwrite locally newer text, and ordinary complete-entry confirmation
  remains separate.
- [ ] Paragraph review binds the displayed target to the entry-buffer version,
  UTF-8 byte range, and exact-text digest. Accept applies a still-current
  replacement once; reject leaves the entry byte-for-byte unchanged; any buffer
  edit makes the old suggestion stale.
- [ ] The core, not provider-authored raw markup, owns inferred-reflection
  delimiters. Malformed, unbalanced, oversized, or out-of-range structured
  output is rejected; unresolved inferred reflections continue to block entry
  confirmation.
- [ ] Adapters receive no application tools or workflow authority. A synthetic
  source containing hostile instructions can produce only a bounded proposal
  and cannot cause a file, process, connector, confirmation, approval, or
  publication side effect.

### Registry, jobs, and production adapters

- [ ] Registry tests run at least three fake backends concurrently and prove
  stable-ID selection, default/fallback behavior, language/action capability
  checks, dynamic availability, remembered-selection resolution, and routing to
  exactly one backend without provider branches in core, transport, or TUI.
- [ ] Every assistant job unconditionally rechecks source freshness before
  invocation and again before committing a successful result. Completion also
  conditionally verifies that action's own captured bindings per the
  architecture plan's per-artifact-type list, which binds the confirmed
  source-selection digest for every artifact type, including paragraph
  suggestions, so a later source change still stales a completed suggestion
  even though accept/reject are plain conditional writes, not jobs; a stale,
  cancelled, timed-out, or superseded result cannot mutate canonical state or
  emit content through the job/SSE representation.
- [ ] Cancellation reaches the hosted request, helper, or approved child
  process and revokes commit authority. Server restart leaves no partial
  assistant mutation; already committed artifacts remain recoverable.
- [ ] Hosted egress-envelope tests inspect serialized requests and prove that
  only the action-specific allowed fields can leave. Prompts, responses,
  provider bodies, child stdout/stderr, paths, commands, and credentials never
  enter diagnostics or arbitrary client-visible errors.
- [ ] The hosted and Apple production adapters implement the same action policy
  and map availability, unsupported language/action, refusal, context/size
  overflow, invalid output, timeout, and cancellation to safe typed outcomes.
- [ ] Apple helper fake tests cover model readiness, both output locales,
  unsupported locale, schema validation, input/output bounds, private handoff
  cleanup if used, crash, timeout, and cancellation without a live model.

### Control API and TUI vertical slice

- [ ] Capabilities and Setup check expose only safe descriptor/readiness facts.
  The new memory, draft, suggestion, accept/reject, and cancellation
  resources use conditional writes, bounded DTOs, safe problems, canonical
  reload after gaps, and content-free job/SSE events.
- [ ] Inside Compose, the TUI lists every registered backend, distinguishes
  unavailable from unsupported, labels the selected backend and path in the
  Start Setup check, Main Pane, Work Status, and Status Line, and never treats
  the label as consent or triggers a call on selection/highlight alone.
- [ ] The Resources Sidebar keeps confirmed sources, memory review, current
  draft, and assistant suggestions reachable. The person explicitly confirms
  or edits memories and accepts or rejects a paragraph suggestion; a completed
  current draft opens as editable unconfirmed text, while stale resources remain
  inspectable but cannot be applied.
- [ ] Assistant progress and cancellation do not block editor input or HTTP/SSE
  recovery. Before an action, unsent editor text is flushed or the action waits;
  after any failure, cancellation, disconnect, or restart the manual Draft
  editor and recoverable canonical text remain available.
- [ ] Focus/Enter exposes every assistant action without adding a workflow
  stage, bare-letter shortcut, function-key alias, or unbounded Command Line
  grammar. In-memory terminal and fake-client tests cover all adaptive width
  bands, Korean display/input, safe text rendering, stale outcomes, and backend
  lists larger than two.

### Documentation and gate

- [ ] Synthetic end-to-end tests cover the hosted and Apple adapter contracts,
  three-backend selection, two-pass memory review and drafting, guarded draft
  population, paragraph accept/reject, mid-job invalidation, cancellation,
  restart recovery, and manual fallback.
- [ ] README, product, architecture, API, UX, configuration, contributor, and
  privacy documentation name the chosen provider behavior, exact egress
  boundary, on-device availability behavior, and any approved Codex surface.
- [ ] The standard Rust and Swift milestone gate passes and dated acceptance
  evidence is recorded below.

## Acceptance evidence

Fill this in during implementation. Leave implementation items unchecked until
the result is observed; record synthetic evidence only.

| Evidence | Result |
|---|---|
| M8 baseline and plan review | 2026-10-29: reconciled the plan with completed P1-M12/P1-M7 code and docs; separated required and conditional decision gates; identified action-envelope, stale-result, config-parser, and helper-frame gaps. |
| M8 plan review continued | 2026-10-29: split the approval gate into five independently-gating packets (contract/migration, configuration, hosted, Apple, conditional Codex) so provider-neutral contract/schema/registry/TUI work is not blocked on an undecided provider, and widened the configuration packet to the whole `assistants:` section; replaced "as applicable" invalidation with an explicit per-artifact-type binding list and matrix, so a backend or style change no longer stales already-reviewed memories, while a source change still stales every artifact including a completed paragraph suggestion, now via its own source-selection-digest binding rather than the job-only freshness recheck; replaced an unenforceable reflection-delimiter "round-trip" with a refusal rule and added reflection ranges to the paragraph-review result type; made the one-active-job rule explicit (one per workflow across all three actions, not per action); named the exact v4-to-v5 field mapping and its two deliberate behavior changes; recorded the previously undocumented Codex profile-configuration approval in `AGENTS.md` and the changelog; and added a researched candidate `codex exec` fixed-argv surface for the still-ungated, conditional Codex adapter packet, including the correction that `--sandbox read-only`/`--cd` deny writes but do not confine reads, so that packet must separately prove filesystem-read and network denial with an actual OS sandbox. |
| M8 hosted-provider decision | 2026-10-29: owner approved OpenAI's Codex, reached only through `codex app-server`, as the hosted default, merging the former conditional-Codex gate into the required hosted-default packet; recorded in `AGENTS.md` "Decided" and the Phase 1 changelog. Verified against current Codex documentation: restricted `readOnlyAccess`/`networkAccess` sandboxing, MCP/web-search disable, `history.persistence = "none"`, and `turn/start` `outputSchema` are real, version-gated (0.138.0+) mechanisms; installed `codex-cli` is 0.165.0. Apple packet (protocol version, payload transport) remains pending. |
| Owner-approved contract/migration, configuration, and hosted (Codex) packets | Approved 2026-10-29; recorded above and in the Phase 1 changelog. Apple packet pending. |
| Contract/schema migration and mixed-version tests | Pending |
| Strict configuration and Codex-profile registration/launch behavior | Pending |
| Registry, action-specific egress, and three-fake routing tests | Pending |
| Hosted and Apple production-adapter contract tests | Pending |
| Jobs, cancellation, stale commit, persistence, and recovery tests | Pending |
| Compose TUI vertical flows and adaptive rendering | Pending |
| Documentation inspection and complete Rust/Swift gate | Pending |
