# Phase 1 changelog

Status: Active

This changelog records owner decisions and material plan changes for Phase 1.

## Entries

- 2026-10-30 — P1-M9 completed — Added canonical local
  Markdown rendering, configured entry-root plumbing, read-only preview,
  digest-bound approval, atomic no-clobber local creation, receipt/read-back,
  restart reconciliation, publication jobs with precommit cancellation, and
  Preview/Publish TUI controls with conflict day carry-over. Synthetic
  adapter, SQLite, and loopback API/worker tests cover exact output, conflict
  refusal, changed destination bindings, cancellation and crash boundaries,
  restart recovery, and a lost response after the local write. The full Rust
  and Swift gate passed.

- 2026-10-30 — Owner decision and P1-M9 plan review — Local output front
  matter uses `date` and `language`. The P1-M9 plan now specifies canonical
  preview bytes, approval invalidation, race-free no-clobber finalization,
  uncertain-write reconciliation, per-day gating, and API/TUI acceptance.

- 2026-10-29 — M8 post-implementation correction — The Codex app-server
  transport now follows the complete documented connection lifecycle:
  `initialize`, the required `initialized` notification, `thread/start`, and
  `turn/start`. It includes complete client metadata, accepts output only when
  `turn/completed` reports `turn.status: completed`, maps failed and interrupted
  turns to safe typed outcomes, and unconditionally kills and reaps the per-job
  child after success or any protocol error.

- 2026-10-29 — Owner decision — Sources-stage Ayumi maintenance — P1-M8 is
  reopened for per-source assistant refinement with approve/reject, direct
  body editing, and explicitly confirmed hard deletion. Approve/edit writes
  the real note; delete removes it and only safely resolved linked audio. The
  canonical six workflow stages remain unchanged. This amends the earlier
  three-action Apple adapter scope: Codex and Apple now both support the fourth
  source-refinement action; refinement input remains en-US Ayumi text.

- 2026-10-29 — Owner decision — M8 Apple on-device packet — The owner approved
  the Apple on-device production-adapter packet, the last of M8's five gating
  packets. Deployment target: the latest stable macOS/Foundation Models SDK
  only, with no older-OS floor pinned. Production-adapter scope: all three
  assistant actions (memory extraction, two-pass drafting, paragraph review)
  for both `en-US` and `ko-KR`, matching the Codex hosted default's action and
  language coverage — not a drafting-only slice. Payload transport: request
  and response bodies, including a drafted entry approaching the existing
  512 KiB entry bound, travel over the Swift bridge's existing in-band JSON
  frame with its cap raised to cover that bound, in both directions; this
  does not reuse or extend the P1-M5 private handoff-file transport, and the
  bridge's framing is reimplemented as needed to carry the raised cap safely.
  Separately, the AGENTS.md "Undecided" item about Ayumi language metadata is
  closed without adding a front-matter field: Ayumi source notes are always
  en-US (already recorded in `docs/product-behavior.md`), so no metadata-based
  language rejection is needed; the `ko-KR` output option is a translation
  choice made at Compose time, not a property read from source notes.
- 2026-10-29 — Owner decision — M8 hosted-default provider, Codex adapter
  merge, and packet scope — The owner approved OpenAI's Codex as the Phase 1
  hosted-default assistant provider, reached exclusively through the
  `codex app-server` JSON-RPC surface over stdio — not the Codex SDK (which
  is TypeScript and unreachable from Rust) and not raw `codex exec` CLI
  flags (`--sandbox read-only`/`--cd` set a working directory but do not
  restrict which paths a command may read). This merges the previously
  separate "hosted-default" and "conditional Codex" packets in the M8
  overview into one required packet: the Codex runtime checklist items
  become required, not "not applicable," and every configured
  `assistants.codex_profiles` entry becomes a registry member once this
  packet lands, with the first-listed profile as the default when more than
  one exists. Confinement comes from Codex's own permission-profile /
  app-server `sandboxPolicy` enforcement (OS-level: Seatbelt on macOS),
  verified against current Codex documentation: a `turn/start` call sets
  `sandboxPolicy` to `readOnly` with `access: { type: "restricted",
  readableRoots: [] }` (no readable paths at all; diary content rides in the
  prompt, not the filesystem) and `networkAccess: false`; disables all MCP
  servers (`[mcp_servers]` empty) and web search (`web_search = "disabled"`);
  sets the approval policy so the child never blocks waiting for a human;
  and disables local transcript persistence (`history.persistence = "none"`,
  or an `ephemeral` thread) so no diary text lands inside `codex_home`. A
  fresh `codex app-server` process is spawned per assistant job and its
  process group is killed on completion, cancellation, timeout, or server
  restart — not a persistent server per profile — so `CODEX_HOME` isolation
  and "no partial mutation survives a restart" stay process-scoped. The
  installed `codex-cli` is version 0.165.0, above the 0.138.0 floor this
  restricted-read mechanism requires; the adapter's readiness check pins
  that floor against the resolved binary. Structured results use
  `turn/start`'s per-turn `outputSchema` (a JSON Schema), matching the
  action-specific result shapes already required by the contract/migration
  packet. The Notion PAT's existing storage location and format
  (`config/global_config.yaml`, plaintext, owner-only, git-ignored) is
  unchanged by this decision; it is a separate credential from the Codex
  account and is never read by or sent to an assistant adapter. This
  resolves the "hosted provider" item removed from `AGENTS.md` "Undecided."

- 2026-10-29 — Owner decision — M8 contract/migration and configuration
  packets resolved — The owner approved the contract/migration packet
  (native-client contract version 3, SQLite schema version 5; the
  `/v1/assistant-backends`, `/v1/workflows/{id}/assistant-selection`,
  `/memory-extractions`, `/memory-review`, `/draft-requests`,
  `/draft-result`, `/paragraph-review-requests`, and `/paragraph-suggestions`
  routes; new `diary-control-contract` bounds sized from the existing
  `MAX_ENTRY_BYTES`/`MAX_STRING_BYTES`/`MAX_CANDIDATES`/
  `MAX_SELECTED_SOURCES` constants; and `assistant_*`-prefixed snake_case
  problem codes) and the configuration packet (extend the existing
  hand-rolled line-based parser in `crates/diary-control/src/lifecycle.rs`
  with one more indentation level scoped only to `assistants:` /
  `codex_profiles:`, rather than adding a YAML dependency) exactly as
  recorded in the M8 overview's "Required contract and migration packet"
  and "Required configuration packet" sections. This unblocks
  implementation sequence steps 2-5 and 8 in the M8 architecture plan. The
  Apple on-device packet remains pending.

- 2026-10-29 — Owner decision — Codex profile configuration shape revised to
  structured fields — The owner approved replacing the shell-lookalike
  `run_command` string (`alias`/`run_command`, e.g. `CODEX_HOME=/Users/user1/.codex-pp7 codex`)
  with structured fields: `alias`, an absolute `codex_home`, and an optional
  absolute `codex_executable` (otherwise resolved and pinned once at
  config-load time, so `nd serve`'s launch `PATH` cannot change which binary
  runs). Example:
  ```yaml
  assistants:
    codex_profiles:
      - alias: "pp7"
        codex_home: "/Users/user1/.codex-pp7"
      - alias: "work"
        codex_home: "/Users/user1/.codex-work"
        codex_executable: "/absolute/path/to/codex"
  ```
  This removes the shell-grammar quoting/command-length parsing rules the
  `run_command` form needed — the parser now only validates two path fields
  per profile — and otherwise approves the same scope as before: profile
  multiplicity and this representation only. It does not select Codex as a
  provider or approve starting a Codex process with diary text; those remain
  gated on the separate Codex adapter packet in the M8 overview. This
  supersedes the `run_command` representation recorded in the entry below and
  in `AGENTS.md` "Composition," which has been updated to match.

- 2026-10-29 — Owner decision recorded — Codex profile configuration shape
  (superseded above by the structured-fields revision) —
  The owner confirmed, retroactively recorded here because the prior M8
  planning session had not logged it in `AGENTS.md` or this changelog despite
  the M8 overview already describing it as approved: the repo-local,
  owner-only `config/global_config.yaml` may hold zero or more named Codex
  profiles as user-chosen `alias`/`run_command` pairs, for example alias `pp7`
  mapping to `CODEX_HOME=/Users/user1/.codex-pp7 codex`. This approves profile
  multiplicity and the strict base-launcher representation only; it does not
  select Codex as a provider, approve starting a Codex process with diary
  text, or approve any other Codex integration detail — those remain gated on
  the separate Codex adapter packet in the M8 overview. See `AGENTS.md`
  "Composition."

- 2026-10-29 — P1-M8 plan review continued — Picked up an M8 plan review that
  a prior Codex session left mid-edit (uncommitted changes to the M8 overview,
  architecture plan, and cross-referencing docs) and completed it:
  - Split the single owner-approval gate into five independently-gating
    packets — contract/migration, configuration, hosted, Apple, and
    conditional Codex — and fixed the spots (architecture sequence step 1,
    the overview's intro, and the phase-1.md P1-M8 notes) that still read as
    if all packets must be approved before any implementation starts. The
    configuration packet now explicitly gates the strict parser for the
    whole `assistants:` section of `config/global_config.yaml` (today the
    Codex profile collection, and any later hosted/Apple keys added there,
    such as a hosted credential), not just the Codex shape, and is back in
    the entry criteria.
  - Replaced the "as applicable" invalidation language with an explicit
    invalidation matrix, then rescoped each artifact type's binding-data list
    to match it instead of giving every artifact the same binding tuple:
    memory candidates and reviewed memories are no longer staled by a backend
    or writing-language/style change, matching the fact that extracted
    memories are backend-neutral. A source change still stales every
    artifact, including a completed, not-yet-resolved paragraph suggestion —
    its binding list gained the confirmed source-selection digest precisely
    because accept/reject are plain conditional writes, not jobs, so the
    job-level freshness recheck alone cannot catch a source change that
    happened after the paragraph-review job completed but before a later
    accept. Dropped "workflow
    revision" from the draft-proposal bindings, since it bumps on unrelated
    mutations and would silently widen staleness. Dropped writing
    language/style from the extraction request type, since sources are en-US
    only and extraction produces no prose.
  - Replaced an unenforceable reflection-delimiter "round-trip" design
    (strip markers, trust the provider to reproduce them, re-insert
    structured ranges "independent of what was stripped") with a refusal
    rule: revision drafting and paragraph review refuse to start while their
    target still contains an unresolved reflection marker, the same
    constraint complete-entry confirmation already enforces, one step
    earlier. Added inferred-reflection ranges to the paragraph-review result
    type so a provider-written replacement cannot carry unmarked inference.
  - Made the one-active-assistant-job rule explicit: one active job per
    workflow counted across all three actions together, not one slot per
    action kind, so a paragraph-review request cannot run concurrently with
    an in-flight draft job.
  - Named the exact v4-to-v5 field mapping the contract/migration packet must
    cover (`reviewed_memories`, `assistant_draft`, and the single coarse
    `assistant_material_stale` flag becoming per-artifact-type records) and
    its two deliberate behavior changes: a source change today discards
    reviewed memories/draft outright, M8 keeps them stale but inspectable;
    and reviewed memories now survive a backend or style change.
  - Added a researched, non-binding candidate `codex exec` fixed-argv surface
    for the still-ungated Codex adapter packet (`--json`, `--sandbox
    read-only`, `--skip-git-repo-check`, `--cd <private dir>`), then corrected
    an overclaim in that same addition: `--cd`/`--sandbox read-only` deny
    writes but do not confine reads, so the packet must separately prove
    filesystem-read and network denial with an actual OS sandbox (such as
    macOS Seatbelt/App Sandbox) and audit each account's own
    `$CODEX_HOME/config.toml` for MCP/web-search/model-provider settings, or
    the surface is not eligible, per the plan's existing "cannot be proven"
    rule.
  - Pointed the Apple assistant-payload transport at the existing P1-M5
    byte-handoff design as the reviewed starting point if a handoff (rather
    than a larger frame) is the approved choice, named which side creates and
    cleans the handoff root in each direction (server for the new
    Rust-to-Swift request direction, helper for the reversed response
    direction, mirroring M5), and flagged that a draft near the existing
    512 KiB entry bound will not fit the 64 KiB helper frame, so the Apple
    packet must cap output below the frame or route the response through the
    handoff too.
  No provider, dependency, or live model call was selected by this review.

- 2026-10-29 — P1-M8 activated and plan reviewed — Marked P1-M8 active after
  completed P1-M7 and reconciled its plan with the implemented M12/M7 baseline.
  The revised plan separates required hosted/configuration/Apple decision
  packets from the conditional Codex-adapter gate; replaces the flat assistant
  envelope with action-specific egress and source-linked memory requirements;
  binds durable proposals against stale workflow state; adds freshness and
  cancellation commit fencing; and calls out the strict-config collection and
  Apple helper 64 KiB transport gaps. Production implementation remains blocked
  on the unchecked owner approvals; no provider, dependency, or live model call
  was selected by this review.

- 2026-10-29 — P1-M7 completed — The owner confirmed that the required Wave
  Terminal primary qualification and Apple Terminal secondary regression set
  passed, including the four interactive probes and integrated manual-flow
  cases in the milestone checklist. Together with the already-passed automated
  acceptance, documentation, and full Rust/Swift gate, this completes P1-M7.

- 2026-10-29 — Root Make aliases — Added a dependency-free root `Makefile`
  with phony targets for help, Rust/Swift build and tests, client/server runs,
  Rust formatting and Clippy, and the complete ordered milestone gate. The
  underlying Cargo and Swift commands remain explicit in agent and contributor
  documentation.

- 2026-10-29 — P1-M7 increment 7 integration evidence — Extended the
  real-process acceptance path through explicit empty-source confirmation,
  manual composition, complete-entry confirmation, server restart with a new
  runtime attachment, and canonical recovery of the confirmed synthetic text.
  Reconciled README, contributor, product, and UX docs with the live manual
  slice, exact bounded Command Line, focused-only confirmations, locked later
  stages, and still-pending terminal qualification. The complete Rust and Swift
  gate passed (`diary-tui` 165 tests, `diary-control` 23, `nd` unit 5 plus all
  integration targets, and 10 Swift XCTest cases). Wave Terminal qualification
  and the secondary Apple Terminal regression run remain the only P1-M7 work.

- 2026-10-29 — P1-M7 increment 6 IME implementation — Added the
  notification-first TIS language path: the narrow macOS boundary now reads the
  supported language property, registers and removes its observer on the main
  run loop, coalesces bounded updates, and feeds `EN`/`KO`/unknown state to the
  live TUI. Registration or delivery failure permanently selects the existing
  500 ms query fallback without disabling editing. Focused macOS/TUI/control
  tests, the read-only main-thread smoke probe, the real REST+SSE worker test,
  and targeted warning-denied Clippy pass. Validation also fixed a missing
  `workflow.changed` event for newly created workflows without changing the
  event schema or routes. Wave and secondary Apple Terminal qualification
  remain pending, so Increment 6 is not yet fully accepted.

- 2026-10-29 — P1-M7 increment 5 implementation — Completed the adaptive,
  accessible presentation shell: a dedicated Command Line row, the approved
  120+/100–119/80–99 sidebar bands and focus overlays, bounded resource/Main
  scrolling, text state markers, display-width wrapping and clipping,
  space-aware weekday dates, a `NO_COLOR`-aware basic palette, and a 512 KiB
  complete-entry input limit. Updated user-facing TUI status. The real-process
  check also tightened SSE readiness so Connected requires body bytes and small
  event frames are fed incrementally from Ureq's buffered reader; the complete
  `nd` package suite then passed three consecutive runs. Focused
  `diary-tui` (161) and `diary-control` (23) tests and targeted
  warning-denied Clippy pass. No dependency or API contract changed.

- 2026-10-29 — P1-M7 increment 4 implementation — Completed live manual
  Compose and Review: server-owned language/style settings with language-only
  preference retention, manual editor entry, confirmed-source inspection,
  generation-bound debounced and explicit saves, canonical reconciliation of
  uncertain writes, deliberate two-version conflict resolution, guarded quit,
  full-entry review, reflection/blank guards, and conditional confirmation.
  The empty-source path now opens manual composition automatically. Focused
  tests and Clippy with warnings denied pass; no dependency or server contract
  changed.

- 2026-10-29 — P1-M7 increment 3 implementation — Completed the live Sources
  slice: bounded discovery/preparation jobs, progress polling, explicit-date
  discovery, source pagination, independent focus/active/included state,
  warning and dual-date evidence, selected-only preparation, prepared-body
  inspection, review gating, exact conditional confirmation, and two-step
  continue-without-notes. Corrected a prior M7 client/evidence error: the
  preparation handle comes from the succeeded job's `result_handle` and must be
  sent on source-detail requests; it is not a source-detail response header.
  Focused tests and Clippy with warnings denied pass, with no dependency or
  server-contract change.

- 2026-10-29 — P1-M7 increment 2 implementation — Made Start operable in the
  live TUI with a shared focus model, complete non-editor key classification,
  visible pane/item focus, Setup and capability status, Mac-local-today or
  explicit day choice, workflow pagination, start/resume, guarded two-step
  discard, retry/help/save-state/quit controls, and accessible stage
  navigation. All stateful actions use the bounded worker and session mutation
  gate. Focused tests and Clippy with warnings denied pass; no dependency or
  lower-layer contract changed.

- 2026-10-29 — P1-M7 increment 1 implementation — Completed the session
  authority and recovery foundation: typed capabilities/setup/preferences and
  workflow pagination, stricter HTTP bounds, a dedicated bounded SSE worker,
  heartbeat and event validation, `Last-Event-ID` reconnect, instance
  reattachment, and a fail-closed mutation gate that requires canonical reload
  after reset, unknown/decode/overflow, or transport loss. Integrated it into
  the live `nd` loop without adding dependencies. The increment also fixed an
  existing bounded-worker test race that could hang during failure unwinding.
  Focused Clippy and tests plus `cargo test --all-features` pass; no private
  diary source or live external service was accessed.

- 2026-10-29 — P1-M7 ongoing-plan review — Distinguished isolated component
  tests from the still-missing human-usable vertical slice and replaced the
  stale build-from-zero sequence with seven acceptance-sized integration
  increments. The plan now calls out missing session/SSE recovery, focused
  Start actions, end-to-end Sources/Compose/Review, a dedicated Command Line
  row, the decided 100/120-column sidebar overlays, TIS notifications, and
  real-process acceptance. Wave Terminal is the primary qualification target;
  Apple Terminal evidence is secondary. The already-evidenced M12 and
  dependency/FFI entry gate is checked complete. No production code,
  dependency, build tool, private source tree, or live service changed.

- 2026-10-29 — Owner decision — P1-M7 keybindings drop function-key aliases
  entirely. A real-terminal check had already found F10 never arrives as a
  distinct chord at all; rather than confirm the remaining aliases (F2, F3,
  F6, F7) one at a time, the owner decided every P1-M7 action is reachable
  only by its Ctrl chord or by focus and Enter, with no function-key
  fallback. Supersedes the F-key aliases recorded in the 2026-10-28 entry
  below (itself already revised once: Ctrl-Enter and Ctrl-Left/Ctrl-Right
  were replaced with Ctrl-S and Ctrl-P/Ctrl-N after a real-terminal check).
- 2026-10-29 — P1-M12 completion review — Tightened source-note presentation
  names against absolute, traversal-like, empty-component, oversized, and
  control-character forms; made shared workflow DTO decoding reject malformed
  or partial assistant selections; and corrected the last current UX use of
  generic model terminology. Stabilized helper-timeout and loopback tests so
  unrelated process load, occupied fixed ports, or a network-prohibited test
  sandbox do not masquerade as product regressions. The complete Rust and Swift
  gate passed again.

- 2026-10-29 — Owner decision: P1-M7 Work Status — Added a full-width Work
  Status pane above the Workflow Sidebar, Resources Sidebar, and Main Pane. Its
  bordered body has four content rows for current/recent work, pending or
  blocked work, connection and draft-save safety, and a useful safe notice or
  next action. The compact bottom Status Line remains and may intentionally
  duplicate important state. The planned minimum terminal size increases from
  80 by 24 to 80 by 30; no production code or dependency changed.

- 2026-10-29 — P1-M12 completion — Reviewed all active documentation and the
  implemented codebase, made UX and assistant terminology canonical, and
  completed the breaking pre-client contract version 2 migration. Manual
  editing is now separate from assistant backend/path selection; source-note
  names, direct explicit no-notes continuation, and a read-only hardened
  runtime attachment loader prepare the lower layers for P1-M7. SQLite v4
  forward-migrates existing workflow, source, receipt, and operation state.
  The full Rust and Swift gate passed; P1-M7 remains `Planned` pending its
  dependency and macOS FFI approval packet.

- 2026-10-29 — Owner-directed P1-M12 start and milestone reorder — Added
  P1-M12 Consistency and contract readiness after completed P1-M6 and before
  P1-M7 implementation. M12 reviews every active document first, adopts the
  canonical UX vocabulary, moves the assistant-neutral backend/path contract
  foundation ahead of the first client, and then fixes the source-note name,
  explicit no-notes, and safe runtime-attachment lower-layer gaps already found
  by the M7 review. The unshipped Control API may break and increment its
  contract version; durable workflow, source, output, receipt, and operation
  state must be forward-migrated. P1-M7 remains `Planned` and its dependency/FFI
  packet remains unapproved.

- 2026-10-29 — P1-M7 contract-readiness and implementation-plan review —
  Audited the planned TUI against the completed P1-M6 code and found three
  pre-implementation corrections: a safe runtime-advertisement reader, a
  server-supplied local source-note name, and a reachable explicit
  no-notes transition when source reading is unavailable. Strengthened M7 with
  bounded HTTP/SSE and autosave recovery, future-state fail-closed behavior,
  terminal-control rendering, a minimal Command Line grammar, and exact
  dependency/unsafe-FFI and worker-limit packets pending owner approval. No
  manifest, production code, dependency, or build tool changed.
- 2026-10-29 — P1-M7 planning and composition UX review — Canonicalized the
  user-facing stages as Start, Sources, Compose, Review, Preview, and Publish.
  Defined the left Workflow Sidebar, middle Resources Sidebar, and right Main Pane
  interaction, including active-source versus included-source state and
  prepared source-text inspection. Refined the composition architecture to use
  one `DraftAssistant` port with a collection-shaped backend registry, route
  each action to one selected backend, and support version-bound paragraph
  review suggestions with explicit accept/reject. The chosen hosted default and
  Apple on-device adapter remain P1-M8 work; a Codex-backed adapter is an
  extension point, not a provider decision. The hosted provider, credential,
  and HTTP dependency remain Undecided, and no dependency or production code
  changed.
- 2026-10-29 — P1-M6 completion — Marked P1-M6 `Done`. Implemented the
  owner-approved Tokio/Axum loopback server, strict v1 DTO and ETag contract,
  purpose-named command/CAS layer, per-day reconciliation gates, server-owned
  expiring source preparation, bounded jobs and SSE, private lock/runtime/token
  lifecycle, and typed rotating diagnostics. Synthetic core/SQLite, real-socket,
  isolated-process restart, permission, retention, capacity, and privacy tests
  pass. The full Rust and Swift gate passed with Rust/Cargo 1.98.0 and Apple
  Swift 6.4; no private diary tree, live Notion root, or model/provider was
  accessed.
- 2026-10-29 — P1-M6 owner approval and start — Marked P1-M6 `Active` and
  approved the dependency and exact-limit packet in the milestone overview.
  Approved exact direct dependencies are Tokio 1.53.1, Axum 0.8.9, Serde
  1.0.229, Serde JSON 1.0.151, futures-core 0.3.34, getrandom 0.3.4, subtle
  2.6.1, and fs2 0.4.3 with only the recorded features. The packet adds no
  build tool, generator, logging/config framework, HTTP client, hosted-provider
  choice, or Swift package. It also freezes the v1 transport, source, job, SSE,
  identifier, port-attempt, diagnostic, and shutdown bounds and the optional
  `server.port_fallback_count` configuration key.
- 2026-10-29 — P1-M6 planning review — Reconciled the server plan with the
  implemented M4/M5 contracts. Added the missing workflow-list, writing-setting,
  manual-composition, and explicit empty-selection command boundaries; made
  prepared source bodies/stamps server-owned and expiring with a freshness
  recheck; defined buffer-only autosave CAS recovery; and tightened startup,
  degraded optional capabilities, runtime/database permissions, log crash
  recovery, numeric transport limits, job expiry, and SSE gap behavior. Fixed
  the diagnostic `rg` example to match uppercase log levels. P1-M6 remains
  `Planned`; no dependency, listener, production state, or private data was
  added or accessed.
- 2026-10-29 — P1-M6 logging requirement — Planned an internal owner-only
  Control API Server log under `.local/logs/control-api/`, with UTC-day and
  8 MiB rotation, a strict rolling 72-hour retention limit, a 24 MiB directory
  cap, and content-free typed records. Codex and Claude Code may read that
  Git-ignored directory directly for diagnosis without a control token; logs
  remain absent from REST, SSE, and MCP, and agents remain barred from the
  separate runtime-token directory. P1-M6 remains `Planned`; no implementation
  or dependency change was made.
- 2026-10-29 — P1-M5 post-completion review — Bounded discovery and selected
  reads at the opened-file boundary; rejected lone-CR, nested/duplicate date,
  and additional ambiguous audio-link forms; isolated helper handoffs per
  concurrent request; capped download waits to the Swift protocol limit;
  tightened response JSON, source-date, containment, and file-identity checks;
  and made helper access failure invalidate stale workflow state before its
  typed error returns. The complete gate passed with 46 Rust tests, all
  doctests, and 10 Swift tests. Added synthetic regressions only; no private
  source tree or live service was accessed.
- 2026-10-29 — Completion — Marked P1-M5 `Done`. Added connector-neutral
  source ports and SQLite v3 availability persistence; bounded Ayumi discovery,
  Markdown/audio privacy filtering, selected snapshots, and freshness checks;
  a dependency-free Rust helper client with deterministic lookup, process
  bounds, cancellation, typed failures, and private handoff cleanup; and Swift
  iCloud availability, download, and coordinated-read commands behind fakes.
  The full gate passed with 42 Rust tests, all doctests, and 10 Swift tests. No
  private iCloud tree, live model, Notion service, HTTP listener, or TUI was
  opened.
- 2026-10-29 — M4/M5 implementation review — Hardened M4 publication retries
  so reused approval is durably destination-scoped, verified destinations
  cannot regress, blocked destinations cannot change outside an approved retry,
  and restored approval scopes must name a remote target. Extended retention
  evidence through discard, both concurrency-version increments, restart, and
  repurge. Hardened the implemented M5 Ayumi reader against date-directory
  symlink escapes, malformed timestamp acceptance, unsafe attachment paths,
  fabricated file identities, metadata races, disappearance, and forged nested
  source references. The full Rust and Swift gate passed. M5 remains `Active`:
  the Swift iCloud commands, byte handoff, Rust helper client, and checkpoint
  integration are still pending.
- 2026-10-28 — Owner decision — P1-M7 uses Ctrl-G (F2) for Command Line,
  Ctrl-Enter (F3) to save/flush, Ctrl-D (F10) to quit, and
  Ctrl-Left/Ctrl-Right (F6/F7) for previous/next workflow step. The existing
  real-terminal Korean-IME acceptance check remains required.
- 2026-10-28 — Owner decision — TUI dates use `YYYY-MM-DD`, adding an
  abbreviated weekday as `YYYY-MM-DD (Mon)` where space and context permit.
  The ISO date remains visible in every form.
- 2026-10-28 — Planning review — Strengthened the P1-M7 manual-TUI plan with
  P1-M6 and dependency entry gates, validated runtime-file/health attachment,
  explicit server-authority and conditional-save recovery rules, selected-only
  source snapshot review, real-terminal IME evidence, and an acceptance-evidence
  table. No implementation, dependency, or product decision was added; P1-M7
  remains planned behind P1-M6.
- 2026-10-28 — Planning review — Expanded P1-M6 with P1-M4/P1-M5 entry gates,
  an explicit dependency and build-tool approval step, the P1-M7 manual-flow
  route boundary, separate workflow and entry-buffer preconditions, isolated
  lifecycle and loopback security tests, bounded jobs and SSE recovery, and an
  acceptance-evidence table. P1-M6 remains planned; no server implementation,
  dependency, provider, or live-data access was added.
- 2026-10-28 — Completion — Marked P1-M4 `Done`. Added the dependency-free
  `diary-core` workflow, revision, approval, publication, and retention
  contracts; a `rusqlite` system-SQLite adapter with forward v1/v2 migrations,
  transactional compare-and-swap, durable intents, and versioned receipts;
  and test-only `nd` assembly. Twenty-four Rust tests and eight Swift tests
  passed with formatting, warnings-denied Clippy, both builds, privacy/schema
  inspection, and a P1-M4-scoped `git diff --check`. No production database, live source,
  model, file destination, Notion call, HTTP listener, or TUI was added.
- 2026-10-28 — Owner approval and start — Marked P1-M4 `Active` after P1-M3
  completion. The owner approved `rusqlite` 0.40.2 for `diary-connectors`
  with system-SQLite linkage and no optional Cargo features. The packet adds
  no build tool, bundled C compilation, hosted provider, credential decision,
  HTTP dependency, or Swift package. Direct unsafe SQLite FFI was rejected in
  favor of maintained MIT-licensed bindings compatible with Apache-2.0.
- 2026-10-28 — Completion review — Rechecked P1-M3 code, tests, configuration,
  and its P1-M4/P1-M5 follow-on plans. Corrected the Ayumi config example to
  use one iCloud directory name, removed a contradictory blanket dependency
  approval claim and an undecided initial-language default, and aligned the M4
  autosave and pending/blocked retry rules with the product architecture. The
  command-shell tests now cover unsupported arguments and reliably clean
  scratch state. The helper bounds request IDs, distinguishes JSON booleans
  from integer protocol versions, and tests exact and oversized frames so
  responses remain capped. All four Rust command-shell tests, eight Swift
  protocol tests, the complete Rust and Swift gate, and `git diff --check`
  passed.
- 2026-10-28 — Completion — Marked P1-M3 `Done`. Added the six-package Rust
  workspace with one `nd` binary, explicit non-functional default and `serve`
  shells, a Swift 6.2/macOS 26 helper package with a bounded versioned fake
  protocol, safe configuration example and ignore rules, and reproducible
  contributor commands. Workspace, target, command, helper-stream, and ignore
  inspections passed. The complete Rust and Swift gate passed with Rust/Cargo
  1.98.0 and Apple Swift 6.4 from Xcode 27.1; no third-party dependency or
  later-milestone product behavior was added.
- 2026-10-28 — Planning review — Expanded the P1-M5 plan with milestone entry
  gates, bounded source identity and date evidence, malformed-header and audio
  privacy behavior, a private byte handoff that fits the approved 64 KiB helper
  envelope, explicit failure mapping, checkpoint freshness tests, and an
  acceptance-evidence table. P1-M5 remains planned behind P1-M3 and P1-M4;
  no source implementation or dependency was added.
- 2026-10-28 — Owner approval — Approved the P1-M3 dependency and toolchain
  packet before manifest changes. The approval covers this scaffold only;
  later dependency, feature, and build-tool changes still require approval
  under `AGENTS.md`. P1-M3 remains third-party-dependency free: six separate
  Rust packages use only standard-library or local path
  dependencies, and the Swift package uses SwiftPM, XCTest, and Foundation.
  Selected Rust 1.89 with edition 2024 and Swift tools 6.2 with macOS 26.0.
  The helper runs once per request and exchanges one newline-delimited UTF-8
  JSON frame in each direction, capped at 64 KiB; standard output is protocol
  only. Lookup prefers `ND_APPLE_BRIDGE_PATH`, then the installed `libexec`
  path, then the development build. Configuration prefers `ND_CONFIG_PATH`
  over repo-local `config/global_config.yaml` and later server loading must
  reject an absent or non-owner-only file. Keeping all outer crates preserves
  the architecture visibly without premature combination.
- 2026-10-28 — Planning review — Expanded the P1-M4 plan with milestone entry
  gates, an explicit boundary between core policy and later adapters,
  conditional workflow and buffer revisions, SQLite migration and privacy
  checks, fake crash reconciliation, retention boundaries, and an acceptance
  evidence table. No implementation, dependency, or product decision was added;
  P1-M4 remains planned behind P1-M3.
- 2026-10-28 — Completion — Marked P1-M2 `Done` after its architecture,
  UX reconciliation, owner-decision records, and completion checklist were
  complete and the milestone's documented alternative gate passed. P1-M3
  remains `Planned` until its dependency, Cargo-feature, build-tool, and Swift
  toolchain approval packet is owner-reviewed; no manifest, dependency, or
  production code was added by this transition.
- 2026-10-28 — Review — Strengthened the P1-M3 scaffold plan with an explicit
  P1-M2 entry gate, a dependency and toolchain approval record, a concrete
  workspace and dependency map, observable non-functional `nd` command shells,
  Swift helper protocol failure cases, safe configuration requirements, and a
  structured acceptance-evidence baseline. The review selected no dependency,
  build tool, hosted provider, credential location, or HTTP client; those
  approval and decision points remain open before implementation.
- 2026-10-28 — Planning — Expanded Phase 1 from the P1-M3 scaffold through
  eight additional implementation milestones, P1-M4 through P1-M11. The order
  follows the architecture's dependency direction: core/persistence, sources
  and Apple bridge, control server, TUI/manual workflow, draft assistants,
  local publication, Notion publication, then amendments and release
  qualification. P1-M8 retains the hosted-provider decision gate, P1-M10
  requires an owner-authorized read-only root compatibility check and disposable
  live-write fixture, and every milestone has a verifiable checklist and the
  standard Rust/Swift gate. No production code or dependency was added.
- 2026-10-28 — Research — Compared how Codex and Claude Code attach to the
  server for context and token cost. Claude Code defers MCP tool schemas by
  default and caps results at 25,000 tokens; Codex compacts each tool schema to
  5,000 bytes and offers tool allowlists. The architecture gained a small,
  ranged agent surface, a fixed port and a separate agent token when `/mcp` is
  enabled, and a note that a shell-capable agent can read owner-only files.
  Codex's app server was assessed as a drafting backend: its thread, compaction,
  and token-usage features mostly go unused for a short two-pass draft, while
  its agent baseline, hardening needs, and experimental protocol add cost. See
  the [composition research](../research/agent-assisted-diary-composition.md#codex-app-server-as-the-drafting-backend).
- 2026-10-28 — Review — Fixed design gaps found in the second review: startup
  reconciliation no longer blocks the listener and gates per diary day; editor
  autosave has its own entry-buffer version instead of bumping the workflow
  revision; REST gained workflow list and discard, setup check, preferences,
  amendments, and conflict resolutions; "abandoned" now means no workflow change
  for 30 days; a save-under-a-different-day target with an unfinished workflow
  receives no carried text; the header date is the calendar date in the
  timestamp's own offset; and the Phase 1 plan now names the server, the Swift
  helper, drafting, and amendment. The P1-M1 documents moved from Archived to
  Active and their stale rows were corrected.
- 2026-10-28 — Owner interview — Reversed the on-device-only rule. Hosted
  drafting is now the default behind the `DraftAssistant` port, with Apple
  on-device optional and a hosted adapter in Phase 1. No extra consent step;
  readable note bodies, memories, drafts, and the edited entry may leave the
  Mac, while front matter, attachment paths, audio, and credentials never do.
  The UI labels the active path. A later agent client may read those classes
  and write draft proposals; confirming, approving, and publishing stay
  human-only. The provider, credential storage, and HTTP dependency stay
  Undecided. This supersedes "composition stays on the Mac" and the research
  statements that Phase 1 sends no composition text to a hosted model.
- 2026-10-28 — Owner interview — Decided: keybindings are focus navigation plus
  Ctrl chords (no bare-letter or `:` hotkeys), and the Command Line maps Hangul
  typed on the 2-Set Korean layout back to ASCII command tokens. A day with no
  notes may produce a manual entry through "continue without notes". A
  Completed entry can be amended (replace the app's own receipted output) with
  the editor seeded from the saved, possibly hand-edited, local file; Notion is
  replaced by recorded block IDs and a mismatch is a Conflict. A blocked Notion
  retry needs a new Notion-only approval; a pending one reuses its approval
  while the plan digest is unchanged. The Swift helper joins the gate with
  `swift build` and `swift test`, and the stack line reads Rust plus one Swift
  helper. This narrows the earlier "never overwrite" rule to content the app
  did not write.
- 2026-10-28 — Owner interview — Decided: the canonical project acronym is `nd`,
  always lowercase, and the application is the single binary `nd` (`nd` opens
  the TUI; `nd serve` runs the server by hand in its own terminal). The Control
  API Server's HTTP and event layer is async on Tokio, superseding the research
  note that Tokio is not required; the TUI still does not need it. The same
  Swift helper that reaches the Apple model also performs iCloud availability,
  download, and coordinated reads. The research notes under `docs/research/` are
  marked Active, and their stale statements were corrected: config location,
  Tokio, IME languages, and the ignored `tmp/` fixture. Specific crates still
  need approval during scaffolding.
- 2026-10-28 — Owner — Noted an MCP server (not an MCP CLI) as a Phase 2
  feature in the [roadmap](../roadmap.md). Phase 2 stays unplanned.
- 2026-10-28 — Review — Revised the [architecture](../architecture.md)
  and the UX documents ([terms](../ux-terms.md),
  [information architecture](../ux-information-architecture.md),
  [behavior](../ux-tui.md)). The architecture gained workflow states with
  voiding rules, a process lifecycle and startup order, an authorization trust
  boundary, write-ahead operation intents, a Swift helper contract, a files
  table, and a split of the catch-all `DraftStore` port into `WorkflowStore`
  and `OperationLog`. The UX documents gained step states, connection and error
  states, a Banner, terminal-size rules, inline inferred-reflection delimiters,
  and a confirm/approve/publish vocabulary.
- 2026-10-28 — Owner interview — Decided: Ayumi sources are en-US only and
  output is en-US or ko-KR; Japanese is out of scope for sources and for the
  IME indicator, which shows English and Korean only. The app does not detect
  language from a note body or block on it, and no known metadata field
  identifies language (recorded as Undecided). This replaces the research
  summary's pending `en ↔ ja` check; a live `en ↔ ko` switch was already
  observed in the probe. A human starts the server manually; the TUI never
  spawns it. A Swift helper process bridges Apple Foundation Models. The
  keybinding model stays open, with the Korean-IME constraint recorded. A new
  workflow defaults to today in the Mac's local time zone, unfinished drafts
  first. The Notion token moves to the repo-local, git-ignored
  `config/global_config.yaml` (owner-only), superseding the per-user global
  config file. Draft and source text is purged 7 days after verified output and
  30 days after abandonment. A conflict shows both versions and offers reread,
  discard, or a different diary day, never an overwrite. A blocked or failed
  Notion preflight still lets the local entry be approved and written.
- 2026-10-28 — Owner — Exempted P1-M2 from the standard milestone gate because
  it is documentation-only and the Cargo manifest arrives in P1-M3. See the
  [P1-M2 overview](./milestone-02-software-architecture-overview.md). The
  exemption does not extend to any later milestone.
- 2026-10-28 — Owner direction and architecture proposal — Planned P1-M2
  around a local Control API Server with the TUI as an HTTP client, extensible
  source and output connectors, REST and streaming interfaces, MVVM presentation,
  an explicit workflow pipeline, and a future MCP route. The
  [architecture proposal](../architecture.md) preserves the
  read-only runtime Notion preflight and introduces no production dependency.
- 2026-10-28 — Owner — Restored the completed P1-M1 milestone documents and
  their supporting research notes to `docs/` for P1-M2/P1-M3 preparation.
  Added only milestone titles and goals; no milestone plans or code were created.
- 2026-10-28 — Research — Closed P1-M1 after all checklist items gained
  evidence. Its research-only exit-gate exemption applies; no production Rust
  code, dependency, or later milestone was added. Archived the concluded
  research and milestone documents. See [P1-M1 overview](./milestone-01-research-overview.md).
- 2026-10-28 — Owner and research — The owner reaffirmed that the configured
  Notion root stays uninspected during P1-M1 and must pass the runtime read-only
  preflight. A twelve-scenario synthetic hierarchy fixture covered missing,
  existing, duplicate, incompatible, and ambiguous year/month paths. This
  closes the remaining P1-M1 evidence item while retaining live connector
  validation as later acceptance work. See [validation evidence](../research/p1-m1-validation-evidence.md).
- 2026-10-28 — Owner correction — The configured Notion page is the root of a
  page hierarchy: `Root → YYYY → YYYY-MM`. Create missing year and month
  subpages under their respective parents, then append each `YYYY-MM-DD` daily
  section at the bottom of its month page. This supersedes the earlier
  direct-to-root publication destination; the confirmed body, duplicate
  checks, and read-before-retry rule remain. See [Notion
  research](../research/notion-diary-section.md).
- 2026-10-28 — Owner correction — Place each new Notion daily section at the
  bottom of its destination page (later clarified above as the month page).
  This supersedes the page-top placement and
  deferred top-insertion design recorded in the interview below. The
  researched append operation and duplicate-safe read-back remain the
  connector direction. See [Notion research](../research/notion-diary-section.md).
- 2026-10-28 — Owner interview — Confirmed that an unverifiable source
  freshness check stops output until reread and reconfirmation, and that the
  app requests download of selected cloud-only notes through a coordinated
  macOS read. See [input research](../research/markdown-input-format.md) and
  [operational constraints](../research/mvp-operational-constraints.md).
- 2026-10-28 — Owner interview — Chose reviewed two-pass Apple on-device
  drafting and an always-available multiline manual path, including after
  model failure. Composition remains on device. Each entry offers US English
  or Korean and concise, standard, or reflective style; the last
  language is remembered and concise is the style default. Model-inferred
  reflection is allowed by default but visibly marked for review. The complete
  edited entry requires confirmation, and unfinished drafts persist locally
  for recovery. See [composition research](../research/agent-assisted-diary-composition.md).
- 2026-10-28 — Owner interview — Chose a configurable local Documents entry
  root with `YYYY/MM/YYYY-MM-DD.md` files, YAML front matter containing day
  and language, and no automatic overwrite of different content. A private
  per-user SQLite database in Application Support holds draft recovery and
  publication receipts, excluding the Notion token. See [operational
  constraints](../research/mvp-operational-constraints.md).
- 2026-10-28 — Owner interview — Chose the confirmed Markdown body without
  YAML front matter for Notion, under an ISO-date `heading_2`, with newest
  daily sections at the top of the configured page (superseded by the correction
  above). Conflicting same-day content stops
  publication; an ambiguous write is read back before retry. The configured
  page is not inspected in P1-M1; a fail-closed read-only runtime preflight is
  required. The exact safe top-insertion method is deferred to connector
  design. See [Notion research](../research/notion-diary-section.md).
- 2026-10-28 — Owner interview — Confirmed Apple Silicon, macOS 26.0+, Rust
  1.89+, bounded workers, Ratatui/Crossterm multiline editing, and TIS
  notifications with a 500 ms polling fallback and unknown-language label.
  Chose a plaintext Notion token in a per-user global config file requiring
  owner-only permissions instead of Keychain. See [TUI research](../research/rust-tui-stack.md)
  and [IME research](../research/macos-ime-language-status.md).
- 2026-10-28 — Owner — Confirmed best-effort source freshness checks rather
  than continuous monitoring. When metadata/digest checks detect a change, the
  app invalidates the draft and previews and requires reread/reconfirmation.
- 2026-10-28 — Owner — Confirmed that supported UTF-8 files may include an
  optional BOM and may use LF or CRLF line endings.
- 2026-10-28 — Owner — Confirmed that Phase 1 supports UTF-8 source text only.
  A non-UTF-8 file is warned about and is not treated as having a readable body.
- 2026-10-28 — Owner — Confirmed that unknown front-matter fields are ignored
  and missing non-date fields are tolerated. Only a valid offset-aware
  `created_at` contributes the header date; otherwise the TUI warns and keeps
  the directory date.
- 2026-10-28 — Owner — Confirmed that a note with missing or malformed front
  matter remains selectable when its Markdown body is readable. The TUI shows
  a warning and uses only valid date metadata for proposals.
- 2026-10-28 — Owner — Confirmed that automatic discovery for diary day `D`
  reads only `D-1`, `D`, and `D+1`. Notes farther away require explicit date
  selection; the TUI does not automatically scan a wider archive.
- 2026-10-28 — Owner — Reopened P1-M1 and requested an interview for every
  unresolved or previously inferred choice. The autonomous completion pass is
  retained as research evidence, but its recommendations are not owner
  decisions until individually confirmed here.
- 2026-10-28 — Owner — Completed P1-M1 under its research-only exit-gate
  exemption. The final decision/evidence map and bounded runtime uncertainties
  are in [the research summary](./milestone-01-research-summary.md); this status
  was superseded by the reopening entry above.
- 2026-10-28 — Research — Recommended Apple on-device guided two-pass drafting with
  source-linked review and a full multiline manual fallback. Phase 1 sends no
  composition text to a hosted model, uses source-language concise first-person
  defaults, and prohibits model-inferred reflections. A one-pass Japanese probe
  was rejected after inventing unsupported details; the two-pass synthetic
  probe was grounded.
- 2026-10-28 — Research — Recommended a Phase 1 baseline of Apple Silicon on macOS 26.0+
  and Rust 1.89+. Selected dedicated bounded worker threads, a Foundation
  iCloud availability/coordinated-read adapter, and direct
  `security-framework` generic-password storage; disposable interoperability
  and Keychain probes passed.
- 2026-10-28 — Research — Recommended process-global Text Input Source Services for
  the language-only IME label, with notifications first, a 500 ms polling
  fallback, and `IME: ?` for missing language. A terminal-hosted AppKit context
  was absent; the Rust probe observed live language changes without privacy
  permissions.
- 2026-10-28 — Research — Recommended source discovery at `D-1`, `D`, and `D+1`
  plus one explicitly selected day, accepted readable UTF-8 bodies with
  optional front matter and per-file diagnostics, and made source ID/content-
  digest snapshots invalidate drafts and previews when files change. The owner
  initially confirmed only the discovery bound; the later interview entries
  above resolve the remaining recommendations.
- 2026-10-28 — Research — Recommended local output at configurable
  `~/Documents/narihitos-diary/entries/YYYY/MM/YYYY-MM-DD.md` with atomic create,
  same-digest resume, and no automatic overwrite. Versioned private per-day
  Notion receipts live under Application Support and contain digests/status/
  block IDs but no credential or duplicate diary prose.
- 2026-10-28 — Research — Recommended that Notion receive the complete confirmed
  entry as a `heading_2` ISO diary day followed by paragraph blocks at page end.
  Same-day ambiguity stops publication; ambiguous writes are read before retry,
  and `Retry-After` is honored. The uninspected owner page requires a fail-closed
  first-run read-only preflight.
- 2026-10-28 — Owner — Added P1-M1 research to determine how the Rust TUI can
  show English/Japanese input-language status in Apple Terminal. Japanese
  conversion sub-modes, input-source switching, other terminals, and production
  implementation remain outside this research.
- 2026-10-28 — Owner — Decided that Phase 1 ignores Ayumi audio attachments.
  Audio is not played, copied, uploaded, decoded, transcribed, or used for
  composition; attachment paths are excluded and attachment presence does not
  affect source-note eligibility.
- 2026-10-28 — Owner — Decided that Ayumi diary-day source proposals use both
  the directory date and Markdown-header `created_at` date. The MVP TUI must
  show date disagreements and let the user include or exclude discovered notes
  regardless of metadata date; the confirmed selection controls composition.
- 2026-10-28 — Owner — Planned P1-M1 as a research-and-decisions milestone,
  left later Phase 1 milestones unplanned, and exempted P1-M1 alone from the
  standard milestone exit gate because it produces research documentation
  rather than Rust implementation.
