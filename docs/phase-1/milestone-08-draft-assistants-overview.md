# P1-M8: Draft assistants overview

Status: Active

This milestone adds optional assistant backends to the completed manual
workflow without weakening the Draft editor, source-freshness, or human-review
contracts. The plan is grounded in the implemented P1-M12/P1-M7 baseline.
Provider-neutral core, schema, Control API, and TUI work needs only the
contract and migration packet below; the strict `assistants:` configuration
parser (today the Codex profile collection, and any later hosted/Apple keys
added to the same file) needs the configuration packet; and each production
adapter (hosted, Apple, and any later Codex adapter) needs only its own
packet. See "Decision and approval gate" for the exact split.

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
- The **hosted-default packet** gates only building and qualifying the chosen
  hosted production adapter (sequence step 6). Per-action request/response
  byte bounds for that adapter default to the existing 512 KiB complete-entry
  limit until the provider packet supplies tighter numbers; they are not
  invented ahead of the provider choice.
- The **Apple on-device packet** gates only the Foundation Models protocol
  version, service, and production adapter (sequence step 7).
- The **conditional Codex packet**, if pursued, gates only registering and
  launching a Codex backend. The configuration packet above gates parsing the
  profile collection; it does not make profiles runnable.

Before manifests, DTOs, stored schemas, adapters, configuration syntax, or
live assistant calls change, the owner must approve and record the applicable
packet(s) above in `AGENTS.md` and the Phase 1 changelog.

### Required contract and migration packet

- The M8 native-client contract version (advancing from 2) and SQLite schema
  version (advancing from 4), the resources/DTOs/problems they add, and the
  forward-migration fixtures that must reopen every prior schema version with
  existing workflow, source, draft, preference, output, and receipt state
  intact.
- Action-specific request/response bounds (field counts, string lengths,
  candidate counts) sized from the existing 512 KiB entry limit and the
  Control API's existing numeric bounds, not from an undecided provider.
- The exact new route names for memory-review and draft-result resources
  (the architecture plan's open route-naming items).
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

### Required hosted-default packet

- Provider and model/API surface, supported output languages and actions, and
  whether it is a direct model API or a separately approved agent process.
- Credential and account location, provider-side data use/retention posture,
  network endpoint policy, and content-free availability check.
- Exact direct dependency and features, or exact executable/integration
  surface; license and Rust 1.89 compatibility; executable/version policy when
  applicable.
- Per-action request, response, token/byte, concurrency, timeout, cancellation,
  and retry limits. Assistant calls must not be automatically retried after an
  ambiguous or partially delivered request.
- Structured-output strategy, stable safe error mapping, test fakes, and any
  provider-specific qualification that uses synthetic prose only.

### Required configuration packet

- Choose either an approved YAML dependency or a documented bounded extension
  of the existing strict parser. Define duplicate-key, unknown-field,
  collection-size, alias, and absolute-path-length/validity behavior for
  `codex_home` and the optional `codex_executable`. There is no command string
  to parse under the structured-fields shape, so no quoting or shell-grammar
  rule is needed.
- Preserve owner-only loading and fail closed before server advertisement.
  Malformed configuration is a startup error; a syntactically valid optional
  adapter that lacks runtime credentials or readiness is a typed unavailable
  descriptor so manual editing and other backends remain usable. Missing
  credentials must never silently select another account.

### Required Apple on-device packet

- Swift/Foundation Models API and deployment-target assumptions, protocol
  version, guided-output schemas, model/session lifecycle, and dynamic
  availability and locale checks for `en-US` and `ko-KR`.
- Per-action token and output limits derived from the runtime model capability,
  plus a private bounded payload transport for both directions, compatible
  with the existing 64 KiB bridge frame or routed through a handoff instead. A
  drafted entry can approach the existing 512 KiB entry bound, so the response
  direction needs the same transport decision as the request direction, not
  just the request. If private handoff files are chosen, reuse the existing
  P1-M5 handoff design (owner-only creation, exact-path validation, cleanup,
  timeout, and crash recovery already specified there) rather than a new one.
- Helper process timeout/cancellation behavior and safe mapping for unavailable,
  unsupported locale, guardrail refusal, context overflow, invalid output,
  crash, and cancellation. Tests use a fake model service only.

### Approved Codex configuration requirement, conditional adapter gate

The repo-local, owner-only `config/global_config.yaml` may contain zero or more
named Codex profiles as structured records: a user-chosen `alias`, an absolute
`codex_home`, and an optional absolute `codex_executable` (otherwise resolved
and pinned once at config-load time). For example, alias `pp7` may set
`codex_home: "/Users/user1/.codex-pp7"`. This approves profile multiplicity
and this structured representation only; it supersedes an earlier approved
`alias`/`run_command` shell-lookalike-string shape, which no implementation
ever used.

It does not choose Codex as the hosted default or approve starting a Codex
process with diary text. A production Codex adapter additionally requires an
owner-approved supported integration (`codex exec`, the Codex SDK, app-server,
or another [documented Codex surface](https://developers.openai.com/blog/codex-as-a-platform)),
fixed arguments and structured output,
authentication/account eligibility, version compatibility, working directory,
tool/filesystem/network denial, stdout/stderr handling, timeout, cancellation,
and provider retention policy. If that packet is not approved, M8 may parse and
validate the profile collection but must not register profiles as backends or
launch them.

## Entry criteria

- [x] P1-M7 is `Done`, the phase plan marks P1-M8 `Active`, and the implemented
  M12/M7 core, SQLite, Control API, helper, and TUI seams have been inspected.
- [x] The active plan distinguishes already-implemented assistant-neutral
  foundations from M8 work and distinguishes the required hosted default from a
  conditional Codex adapter.
- [ ] The owner approves the contract and migration packet before DTOs, the
  SQLite migration, the Control API revision, or the TUI vertical slice are
  implemented.
- [ ] The owner approves the configuration packet before the strict parser
  for the `assistants:` section of `config/global_config.yaml` (the Codex
  profile collection and any later hosted/Apple keys added there), or any new
  YAML dependency it needs, is implemented.
- [ ] The owner approves the hosted-default packet before the chosen hosted
  adapter is implemented, and the Apple packet before the Apple production
  adapter is implemented. Any dependency or Swift-package change follows the
  repository dependency approval rule.
- [ ] If Codex will be a production M8 backend, the owner separately approves
  its integration and operational packet above; otherwise all Codex runtime
  checklist items are explicitly recorded as not applicable. The already
  owner-approved profile configuration shape (recorded in `AGENTS.md`
  "Composition" and the Phase 1 changelog) covers parsing only, not
  registration or launch.

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
- The chosen hosted default and Apple on-device production adapters behind the
  same `DraftAssistant` policy boundary.
- Strict parsing for zero, one, or many approved Codex profile records. Runtime
  descriptors and process launches are included only if the separate Codex
  adapter gate is approved.
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
- A production Codex adapter without its separate approval packet, even though
  profile configuration syntax is planned.
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

- [ ] The owner-approved contract/migration, configuration, hosted, Apple, and
  conditional Codex packets are each recorded before their own respective
  implementation begins; manifests and resolved dependency metadata match the
  approved packet.
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
  readiness tests, run only once a Codex adapter packet is approved, cover a
  nonexistent/unsafe-owned home directory, a nonexistent or non-runnable
  executable, and `PATH` resolution failure when `codex_executable` is
  absent — each surfacing as a typed unavailable backend, never a startup
  failure, without reading a real profile.
- [ ] If a Codex adapter is approved, every configured profile maps to one
  distinct descriptor and remembered backend ID, and fake-launch tests prove
  that only the chosen profile's `codex_home`-derived `CODEX_HOME` assignment
  and fixed argv are used without a shell or parent-environment mutation. If
  it is not approved, tests prove that profiles are never registered or
  launched.

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
| Owner-approved contract/migration, configuration, provider/Apple, and optional Codex packets | Pending |
| Contract/schema migration and mixed-version tests | Pending |
| Strict configuration and conditional Codex-profile behavior | Pending |
| Registry, action-specific egress, and three-fake routing tests | Pending |
| Hosted and Apple production-adapter contract tests | Pending |
| Jobs, cancellation, stale commit, persistence, and recovery tests | Pending |
| Compose TUI vertical flows and adaptive rendering | Pending |
| Documentation inspection and complete Rust/Swift gate | Pending |
