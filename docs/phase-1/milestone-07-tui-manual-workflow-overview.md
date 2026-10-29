# P1-M7: TUI and manual workflow overview

Status: Done

This milestone delivers the first human-usable vertical slice: start or resume
a workflow, select sources, write manually, and confirm the complete entry in
the TUI. It deliberately stops before output writes.

## Goal

Let a person use `nd` as an HTTP client of `nd serve` to reach a confirmed,
recoverable diary entry with or without source notes.

## Entry criteria

- P1-M12 is `Done`, the phase plan marks P1-M7 `Active`, and M12's revised
  contract, runtime-attachment loader, source-note names, explicit
  empty-selection transition, conditional-write behavior, and event semantics have
  been inspected. The client must not guess DTO shapes or compensate for a
  missing server operation.
- The owner approves the exact dependency and macOS FFI packet below before any
  manifest, workspace-member, feature, or crate-level unsafe-code-policy change.
- Use synthetic workflows and fake API/IME/terminal implementations in
  automated tests. The real-terminal check must use test text only; it must
  not read a private Ayumi directory or live Notion root.

## Pre-implementation decision and contract gate

P1-M12 owns and requalifies the three lower-layer corrections found by the
2026-10-29 M7 review: the safe runtime-advertisement loader, source-note
names, and direct explicit empty-selection confirmation. M7 consumes those
implemented contracts and must not add a TUI-side file probe, source-name
cache, fake discovery result, or alternate empty-selection transition.

The M7 packet below is **owner-approved as written** (2026-10-29):

Version requirements are exact (`=` in Cargo manifests), and default features
are off unless the table says otherwise.

| Owning package | Exact direct dependency or boundary | Purpose, features, and alternative |
|---|---|---|
| `diary-tui` | `ratatui = 0.30.2`, default features off, `crossterm_0_29` only | Rendering, layout, and in-memory test backend on Rust 1.89; MIT. Avoids calendar, macros, serialization, and unstable features. A handwritten renderer would duplicate tested width/layout machinery. |
| `diary-tui` | `crossterm = 0.29.0`, default features off, `events` and `bracketed-paste` only | Terminal ownership and input on the supported macOS terminals; MIT. Pin the same 0.29 family selected by Ratatui and verify one resolved Crossterm version. |
| `diary-tui` | `ratatui-textarea = 0.9.3`, default features off | Multiline buffer, cursor, selection, undo/redo, and wrapping without its backend or regex feature; MIT. M7 supplies its own key dispatch so editor defaults cannot consume global chords. |
| `diary-tui` | `ureq = 3.4.2`, default features off | Blocking HTTP/1.1 on bounded worker threads with no TLS, proxy, compression, cookie, JSON, or async-runtime feature; MIT OR Apache-2.0 and Rust 1.85 MSRV. Existing Serde/JSON dependencies do bounded encoding/decoding. A handwritten HTTP/SSE parser is security-sensitive; Reqwest would add an async runtime and unused loopback TLS surface. |
| `diary-macos` boundary crate | No third-party dependency; minimal Carbon/HIToolbox and Core Foundation linkage | Isolate the audited unsafe TIS query/notification/CF-ownership code from `diary-tui`, which keeps `#![forbid(unsafe_code)]`. The alternative is weakening that invariant inside the large TUI crate. No second helper process or build script is introduced. |

The same approval sets one ordinary HTTP worker with a 16-command queue, one
SSE worker with one active stream, and a 32-result channel. Connect, send, and
response-header phases are capped at 2, 5, and 5 seconds; ordinary response
bodies at 30 seconds; and each SSE body window at 45 seconds, after which it
reconnects with `Last-Event-ID`. Reconnect backoff starts at 250 ms and doubles
to a 4-second ceiling. All pins are compatible with Rust 1.89 and the repository
license policy based on their published metadata. Approval authorizes only
these manifest/workspace changes; immediately after resolution and before
implementation code, verify the graph, features, duplicate Crossterm count,
MSRVs, and licenses with Cargo metadata/tree inspection. Stop and return to the
owner if the lockfile does not match the packet.

## In scope

- Terminal ownership/restoration, MVVM client, typed API client, SSE reconnect,
  connection/version states, Setup check, and workflow list/discard. The client
  safely reads the owner-only runtime advertisement, verifies unauthenticated
  health and the matching instance/version before it attaches the bearer token
  to any request, and never starts the server.
- The six canonical stages (`Start`, `Sources`, `Compose`, `Review`, `Preview`,
  `Publish`) in the left Workflow Sidebar. M7 enables the manual flow through
  Review and renders later stages as locked; later milestones add behavior
  without renaming or inserting stages.
- A full-width Work Status pane above the Workflow Sidebar, Resources Sidebar,
  and Main Pane. Its four content rows explain current work and retain useful
  safe messages; the compact bottom Status Line remains and may repeat the most
  important state.
- Source selection/detail, selected-only preparation, empty-selection confirmation,
  Draft editor, entry review, and stage invalidation UX. The middle Resources
  Sidebar distinguishes the active source from the included set; activating a
  prepared or confirmed source renders its readable text in the Main Pane.
- Multiline autosave, recovery, per-entry language/style selection, remembered
  last language, concise style default, Korean/English display width, focus
  navigation, and Command Line. Any inferred-reflection delimiter received from
  a future assistant is rendered for review and blocks entry confirmation until
  removed; this milestone does not run an assistant.
- TIS notification-based IME state with polling fallback and real-terminal
  acceptance for Korean IME behavior.

## Out of scope

- Assistant extraction, drafting, paragraph review, output preview,
  publication, amendments, and MCP. P1-M8 adds assistant behavior inside the
  existing Compose stage and shared layout.
- Bare-letter shortcuts, a TUI-side database/source access, or a second durable
  workflow authority. The TUI may retain unsent editor text in memory only.

## Current implementation boundary

The live session covers session authority/recovery, operable Start, end-to-end
Sources, end-to-end manual Compose/Review, adaptive/accessibility presentation,
and Increment 6's notification-first IME path. Increment 7 added
restart/recovery evidence, reconciled the active user and UX docs, and passed
the dated full gate. The owner confirmed the Wave Terminal primary
qualification and secondary Apple Terminal regression set on 2026-10-29,
completing the milestone.

### Increment 5 completion checklist — adaptive and accessible presentation

- [x] Reserve a dedicated one-row Command Line above the Status Line without
  changing Work Status's four content rows, and render bounded input or fixed
  parse feedback there rather than in Work Status.
- [x] At 120 columns or wider render both sidebars inline; at 100–119 keep the
  Workflow Sidebar inline and open Resources over the body when focused; at
  80–99 keep the Main Pane visible and open either focused sidebar as an
  explicit overlay. Focus traversal and Enter actions remain available in
  every width band.
- [x] Keep the focused workflow/resource/control visible with bounded scrolling;
  render distinct text markers for focus, current stage, active resource, and
  included/confirmed state so meaning never depends on color.
- [x] Wrap and truncate visible text by terminal display width, retain safe
  rendering of untrusted controls, show weekday dates wherever workflow/source
  rows have room, and keep editor cursor/viewport behavior correct for Korean
  and other wide graphemes.
- [x] Centralize the 16-color presentation palette, honor `NO_COLOR`, and cover
  the dedicated row, all adaptive width bands, overlays, scrolling, state
  markers, display-width behavior, weekday fallback, and color-disabled output
  with in-memory terminal tests.

### Increment 6 completion checklist — IME and terminal qualification

- [x] Add a narrow `diary-macos` TIS notification monitor that reads the
  supported input-source language property, owns its Core Foundation observer
  on the main run loop where distributed notifications are delivered, and
  removes the observer before shutdown.
- [x] Prefer notification updates in the live TUI, fall back deterministically
  to the existing 500 ms query tracker when notification startup or delivery
  becomes unavailable, and keep editing enabled with `IME: ?` for missing,
  unknown, or failed language reads.
- [x] Cover notification startup/update/disconnect, duplicate/coalesced update,
  fallback timing, bounded delivery, and clean shutdown with deterministic
  tests; audit the narrow CF ownership paths and retain a bounded live
  query/monitor smoke check that never switches the person's input source.
- [x] In Wave Terminal, run `chord_probe`, `command_line_probe`, `editor_probe`,
  and `restoration_probe`, then exercise the integrated manual flow with Korean
  input, bracketed paste, overlays, autosave, disconnect/restart, stale
  preconditions, dirty quit, recovery, and live `EN`/`KO`/unknown changes.
  Repeat the documented secondary regression set in Apple Terminal.

### Increment 7 completion checklist — integration evidence and documentation

- [x] Real-process tests cover validated TUI attachment and the bounded REST/SSE
  path from Start through explicit empty-source confirmation, manual Compose,
  complete-entry Review confirmation, server restart, and canonical draft
  recovery without reading private sources or an external service.
- [x] README, contributor, product, and UX documentation describe the live
  manual slice, its exact Command Line grammar and focused-only confirmations,
  the still-locked Preview/Publish stages, and the completed Wave/Apple
  terminal qualification.
- [x] Focused acceptance checks and the complete Rust/Swift milestone gate pass;
  the dated evidence below records the observed commands and results.
- [x] The real-terminal checklist remained explicit and unchecked until the
  Wave Terminal primary run and Apple Terminal secondary regression run were
  observed and confirmed by the owner on 2026-10-29.

### Increment 7 tooling follow-up checklist — root Make aliases

- [x] A root `Makefile` exposes phony help, build, run, serve, formatting,
  linting, Rust/Swift test, and complete-gate targets that map directly to the
  repository's documented commands without changing dependencies or outputs.
- [x] Contributor and agent command documentation names the Make aliases while
  retaining the underlying Cargo and Swift commands as the explicit gate.
- [x] Dry-run/target inspection and representative execution prove the aliases
  invoke the intended commands; the existing full-gate evidence remains valid.

### Increment 7 server-startup diagnostic follow-up checklist

- [x] `nd serve` distinguishes missing, unsafe, malformed, and invalid source
  configuration with actionable, content-free stderr guidance instead of the
  single `configuration-invalid` code.
- [x] Process-level tests cover missing and unsafe configuration diagnostics,
  including the owner-only permission remedy, without creating server state.
- [x] Focused validation and the complete Rust/Swift gate pass after the
  diagnostic change.

### Increment 7 Workflow Sidebar width follow-up checklist

- [x] The inline and overlay Workflow Sidebar use the minimum width that keeps
  every canonical stage, progress label, focus marker, and border visible.
- [x] Adaptive-layout tests prove the fixed minimum width in wide, medium, and
  narrow focused-overlay bands while leaving Resources and Main accessible.
- [x] Focused validation and the complete Rust/Swift gate pass after the
  layout change.

## Completion checklist

### Client attachment and workflow authority

- [x] P1-M12 is `Done` with its lower-layer contract evidence, and no client
  workaround duplicates it. The owner-approved M7 dependency/FFI packet matches Cargo metadata, the
  lockfile, and the workspace structure before implementation begins.
- [x] `nd` attaches only through a validated runtime file and matching health
  response, never starts the server, never logs the token, and shows
  not-running, connecting, reconnecting, connected, and version-mismatch
  states. A new server instance causes a complete reattachment with a new token
  and event cursor. It fetches canonical state before re-enabling mutations
  after a reconnect, event reset/gap, unknown event, or local queue overflow.
- [x] A person can start, resume, or discard a workflow; discover and inspect
  sources; maintain a local prospective include/exclude set; prepare selected
  cloud-only notes; review their readable snapshots; and confirm a fresh
  selection through server commands. Starting with no diary-day value uses the
  server's Mac-local default. Conditional-write and job-conflict responses
  preserve the local intent, explain the conflict safely, and require a state
  reload rather than retrying blindly.
- [x] The explicit empty-selection path requires its confirmation, hides unavailable
  assistant actions, opens the Draft editor, and refuses a blank confirmed
  entry.

### Layout, editor, and interaction

- [x] The standard layout has a full-width Work Status pane above a left
  Workflow Sidebar, middle Resources Sidebar, and right Main Pane. Work Status
  has exactly four content rows excluding its border and title, shows current
  work plus useful safe messages, and may intentionally duplicate important
  state from the compact bottom Status Line. The Workflow Sidebar always uses
  the canonical `Start`, `Sources`, `Compose`, `Review`, `Preview`, and
  `Publish` order with server-derived progress state and one presentation-only
  current marker; Preview and Publish are visibly locked in M7.
  Both sidebars remain accessible through focus when terminal width collapses
  either one; resource state is explicit and trees retain their hierarchy. The
  minimum supported terminal size is 80 columns by 30 rows; below it the client
  shows only the terminal-too-small notice. The Command Line has its own
  visible row above the Status Line and never borrows a Work Status content row.
  At 120 columns or wider both sidebars are visible; from 100 through 119 the
  Resources Sidebar opens as an overlay; from 80 through 99 either sidebar can
  open as an overlay without hiding its actions.
- [x] In Sources, active focus and inclusion are independent: activating a
  prepared source renders its sanitized readable text in the Main Pane without
  changing inclusion, while include/exclude changes only the prospective set.
  In Compose, the Resources Sidebar lists the draft and confirmed sources;
  moving between a read-only source and the editable draft preserves unsent
  text and editor position.
- [x] The multiline editor autosaves with the server's separate buffer version,
  permits at most one save in flight, never lets an older response replace newer
  keystrokes, retains unsent text during a transient disconnect, warns before an
  unsafe quit, and recovers a server draft without representing stale or
  recovered text as confirmed. A reload or conflict keeps both local unsent and
  canonical server text visible for deliberate resolution.
- [x] Entry language and style are represented by the server-owned workflow;
  only the last language and optional assistant backend ID are remembered, while every new
  workflow defaults style to concise. The selected output language is visible
  in the editor and review. Reserved inferred-reflection delimiters, if present,
  identify the first unresolved span and refuse confirmation.
- [x] ViewModel and renderer tests cover canonical stage mapping and impact
  notices, Work Status priority and four-row rendering, intentional Status Line
  duplication, active-resource versus included-source behavior, prepared and
  confirmed source-body rendering, source warnings/date disagreements, server
  invalidation, conditional-save conflicts, `YYYY-MM-DD` and space-permitting
  `YYYY-MM-DD (Mon)` rendering, terminal-size fallback, grapheme/display width,
  safe visible rendering of terminal control characters, entry byte limits,
  and content-free errors using an in-memory terminal and fake API client. Stage
  mapping tests cover every core invalidation reason (`selection-changed`,
  `source-changed`, `freshness-unverifiable`, `entry-edited`, and
  `composition-choice-changed`), not only the one that happens to accompany a
  Sources- or Review-stale example.
- [x] Focus/Enter and Ctrl-G, Ctrl-S, Ctrl-D, and Ctrl-P/Ctrl-N (no
  function-key aliases; see AGENTS.md "Keybindings") expose every
  manual-flow action. Command Line ASCII tokens work under English and
  2-Set Korean input without forbidden chords, with a raw-jamo/precomposed-
  syllable test vector for each grammar token (`help`, `retry`, `day`,
  `stage`, `save`, `quit`), and every typed command has an equivalent
  focused control.
- [x] TIS notification and 500 ms fallback behavior is tested. A real-terminal
  check in Wave Terminal records terminal restoration after normal exit,
  handled failure, and panic unwind, plus Korean-IME navigation, exact chord
  recognition, Command Line mapping, bracketed paste, sidebar overlays, and
  `EN`/`KO`/unknown indicator evidence. Apple Terminal is a secondary
  regression check; its earlier observations do not substitute for Wave
  Terminal acceptance.

### Evidence and gate

- [x] User-facing, UX, and client-attachment documentation matches the
  implemented manual slice, including the actual chosen chords.
- [x] Acceptance evidence below records focused synthetic tests and a dated full
  gate: `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features -- -D warnings`,
  `cargo test --all-features`, `swift build --package-path helpers/apple-bridge`,
  and `swift test --package-path helpers/apple-bridge`.

## Acceptance evidence

Fill this in during implementation. Leave checklist items unchecked until their
results are observed; record concise synthetic evidence, not private diary text
or raw logs.

Correction (2026-10-29): the historical Sources evidence below incorrectly
claimed that source detail returned a preparation handle in a response header.
The executable contract returns it only as the succeeded preparation job's
typed `result_handle`; the client must send it in the
`X-ND-Preparation-Handle` request header for each prepared detail read. M7
Increment 3 supersedes that claim and adds the missing request-header flow.

The completion evidence immediately below is the final milestone record.
Numbered increment and component-history rows retain the state observed when
those components were first added; any “pending,” “next increment,” or
“deferred” wording in those dated rows is superseded by the completion record.

Increment 4 evidence amendment: two renderer/state tests were added during its
final review, so the final focused count is `nd`: 4 unit tests (not 2), plus
its integration targets.

| Evidence | Result |
|---|---|
| P1-M7 completion | Done (2026-10-29): after the automated manual slice, restart/recovery acceptance, documentation, and full Rust/Swift gate passed, the owner confirmed completion of the required Wave Terminal primary qualification and Apple Terminal secondary regression set. This covers the four interactive probes and integrated manual-flow cases listed in the checked terminal checklist above. No private diary text or external service evidence is recorded. |
| P1-M12 lower-layer readiness evidence and owner-approved M7 dependency/FFI packet | Done (2026-10-29): P1-M12 overview confirms `Status: Done`; `cargo tree -d`/`-i crossterm`/`-i ratatui` show exactly one of each, licenses are MIT/MIT OR Apache-2.0, MSRVs ≤1.89, resolved against the owner-approved packet in this overview. |
| Increment 1 — session authority and recovery | Done (2026-10-29): added typed capabilities, setup-check, preference read/write, and paged-workflow worker operations; strict duplicate/malformed `Content-Length` validation; a dedicated bounded SSE worker with heartbeat handling, per-event bounds, wire/payload validation, `Last-Event-ID`, reconnect backoff, and content-free failures; and a pure mutation gate for event/reset/unknown/decode/overflow/instance recovery. The live `nd` session now loads session metadata off-thread, consumes the event worker, disables actions until canonical workflow/job/list reloads succeed, and returns to full attachment on an incompatible stream or changed instance. Worker saturation tests were made deterministic after the review exposed an existing failure-unwind deadlock. Focused `diary-tui` tests pass (134), the `nd` worker/attachment tests pass, targeted Clippy passes with warnings denied, and `cargo test --all-features` passes. The real-process worker test exercises REST plus SSE when loopback binding is available; terminal interaction remains later evidence. |
| Increment 2 — operable Start | Done (2026-10-29): the live client now has one shared focus model for Workflow, Resources, and Main; classifies Tab, Shift-Tab, arrows, Enter, and editor-relevant keys; visibly marks focused panes/items; and exposes Setup/capability status, server-owned today or explicit day editing, paged workflow selection, start/resume, two-step discard, retry, help, unavailable-save feedback, quit, and accessible stage navigation. All mutations use worker intents and are gated by canonical session authority and advertised `workflow:write`/manual capability. Pagination refreshes duplicate summaries without reordering existing rows. Starting or resuming attaches the canonical workflow and advances to its server-derived stage; later-stage behavior remains locked to its planned increment. Focused Rust tests pass (`diary-tui`: 139; `nd` unit/integration targets pass) and targeted Clippy is clean with warnings denied. |
| Increment 3 — Sources end to end | Done (2026-10-29): the live Sources stage now drives discovery and selected-only preparation jobs through the bounded worker, polls typed progress, consumes string cursors for bounded candidate pages, offers explicit extra-date discovery, and keeps focused, active, and prospectively included sources independent. Candidate rows and the Main Pane expose safe name, availability, warning, and directory/header date evidence. A succeeded preparation contributes its typed `result_handle`; the client sends that handle in `X-ND-Preparation-Handle` on each included detail read, retains it only after server acceptance, requires every included body to be reviewed, and confirms the exact ID set with that handle. Inclusion changes or fresh discovery invalidate preparation/review state. `409`/`412`/transport/job failures retain local intent and show fixed safe errors; no mutation is blindly replayed. Empty selection uses a separate two-step confirmation and advances to Compose without source capability. Focused tests pass (`diary-tui`: 142; `nd`: 2 unit tests plus integration targets), the real-process worker test includes the empty-selection transition when loopback is available, and targeted Clippy is clean with warnings denied. |
| Increment 4 — Compose and Review end to end | Done (2026-10-29): the live Compose stage writes server-owned language/style settings and remembers only language, begins manual composition, renders the multiline Draft editor, and keeps confirmed sources inspectable without reconstructing the editor or losing its cursor/buffer. The loop issues roughly one-second debounced saves plus Ctrl-S/focus-leave flushes, permits one save in flight, and binds each response to the exact local generation so a late success cannot clear newer edits. A failed or uncertain save blocks replay and requests canonical reload; an exact canonical text match proves the uncertain write, while differing local/server texts remain separately visible until an explicit Keep local or Use server action. Unsafe quit requires a second confirmation. Review shows the full edited entry and output language, blocks blank or unresolved-reflection confirmation, and sends the workflow revision plus buffer version for complete-entry confirmation. The explicit empty-source confirmation opens manual composition automatically and exposes no assistant action. Focused tests pass (`diary-tui`: 146; `nd`: 2 unit tests plus integration targets), and targeted Clippy is clean with warnings denied. No dependency or server-contract change was needed. |
| Increment 5 — adaptive and accessible presentation | Done (2026-10-29): the live shell reserves independent Work Status, Command Line, and Status Line rows; renders both sidebars inline at 120+ columns, Resources as a focus-driven overlay at 100–119, and either focused sidebar as an overlay at 80–99; clears overlay backgrounds; and keeps focused resources and Main controls visible through bounded scrolling. Text markers distinguish focus/current/active/included state without color. Shared display-width helpers drive clipping, cursor-column and hit-test boundaries; Main text wraps safely; workflow and source dates add weekdays when space permits; the editor handles Korean/emoji cursor movement and rejects typed or pasted content beyond 512 KiB; and a centralized basic-color palette honors `NO_COLOR`. Command input is bounded and its input/parse feedback stays in its dedicated row. The real-process check exposed an intermittent SSE framing race: a fresh stream now emits an immediate comment, and the client announces Connected only after body bytes arrive and incrementally feeds bytes from Ureq's own buffered reader to the bounded parser. Focused `diary-tui` tests pass (161), `diary-control` tests pass (23), the complete `nd` package suite passed three consecutive runs including its real-process worker test, and targeted Clippy for all three packages is clean with warnings denied. No dependency or REST/event contract change was needed. |
| Increment 6 — notification-first IME implementation | Automated portion done; terminal qualification pending (2026-10-29): `diary-macos` now reads the first supported BCP 47 language property, owns a main-thread distributed-notification observer with bounded run-loop pumping and removal-before-drop, and coalesces notification bursts to one latest state. The live TUI prefers that path and permanently selects the 500 ms query fallback after startup or delivery failure without disabling editing. Fake notification/query tests cover updates, duplicates, failure, and exact fallback timing; focused tests pass (`diary-macos`: 6, `diary-tui`: 165, `diary-control`: 23), the read-only main-thread `ime_monitor_probe` reported `IME: EN`, the real REST+SSE worker integration passes, and targeted Clippy for `diary-control`, `diary-macos`, `diary-tui`, and `nd` is clean with warnings denied. Validation also exposed and fixed a pre-existing server omission: a newly created workflow now publishes the existing `workflow.changed` event; resumes/completed reads do not publish a false mutation. Wave Terminal and secondary Apple Terminal interactive evidence remain required. |
| Increment 7 — integration evidence and documentation | Automated portion done (2026-10-29): `p1_m7_tui_attach` proves the real default client validates a live server attachment before terminal acquisition, while `p1_m7_worker` drives the bounded REST/SSE path through workflow creation, explicit empty-source confirmation, writing settings, manual composition, synthetic entry save, complete-entry confirmation, clean server shutdown, restart with a new attachment instance, and canonical recovery of the confirmed text. The tests use a private scratch directory, fake/missing Apple helper, placeholder Notion configuration, and synthetic prose only. README, contributor, product, and UX docs now describe the live slice, exact bounded Command Line, focused-only confirmations, locked later stages, and pending terminal qualification. Focused acceptance passed (2 real-process tests), and the full Rust/Swift gate below passed. |
| Root Make aliases | Done (2026-10-29): all public targets are declared phony; plain `make` prints the complete help list; a combined dry run of `build`, `run`, `serve`, `fmt`, `fmt-check`, `clippy`, `test`, `rust-test`, `swift-build`, `swift-test`, and `check` maps to the documented Cargo/Swift commands; `make fmt-check` passed; and `make rust-test` passed every Rust unit and integration test. The Makefile adds no dependency and does not replace the explicit gate commands. |
| Server-startup diagnostics follow-up | Done (2026-10-29): `nd serve` now gives bounded, content-free remedies for a missing, unsafe, malformed, or invalid source configuration without printing configured values or private paths. The `command_shells` process suite passes 7 tests, including proof that missing and unsafe configuration creates no state. The delegated full gate passed: formatting, warnings-denied Clippy, every Rust test and doctest, Swift build, Swift test (10 cases), and `git diff --check`. |
| Workflow Sidebar width follow-up | Done (2026-10-29): inline and focused-overlay layouts now reserve exactly 24 columns for Workflow: 22 for the longest focused canonical stage/progress row and two for its border. Focused validation passed all 7 adaptive-layout tests and all 4 Workflow Sidebar tests, including exact-fit rendering. The delegated full gate passed: formatting, warnings-denied Clippy, every Rust test, Swift build, Swift test (10 cases), and `git diff --check`. |
| Runtime attachment, reconnect, and conditional-mutation tests | In progress: typed `ApiClient`/`Transport` boundary (5 tests), pure attachment/health state machine (5 tests: connected, instance-mismatch, version-mismatch, not-running, unsafe-attachment), and a hand-written bounded SSE frame parser (5 tests), all against fakes. **End-to-end against a real process (2026-10-29, `apps/nd/tests/p1_m7_tui_attach.rs`):** `nd` (no args) spawned against a real `nd serve` process reads the runtime file, matches unauthenticated health's instance ID and contract version, and only then stops — on the test harness's missing tty, not on attachment or health — proving the real read/health/version path works, not just the fakes. Reconnect worker threads, event-cursor replay, and conditional-mutation-disable-on-gap behavior not yet implemented. **Corrected (2026-10-29, owner-review feedback):** `nd` previously exited on a not-running/unsafe/version-mismatch server, contradicting the decided "Server lifecycle" item (the TUI must show that state, not exit, until it can attach). `tui_mode` now dispatches on whether stdout is a real tty: a real interactive session acquires the terminal once and loops, redrawing a phase screen and retrying attach with the approved 250ms→4s backoff, with Ctrl-D as the only way out; a non-tty (automated-test) context keeps the original one-shot exit behavior, since there is no terminal to render a persistent screen into. Also fixed: `LifecycleError::IncompatibleContract` was falling into the generic "unsafe" bucket instead of mapping to the version-mismatch phase. `TerminalGuard` now enables/disables bracketed paste; the blocking `next_event_is_quit` read is replaced by a non-blocking `poll_event(timeout)` inside this retry loop, which is also the prerequisite for wiring autosave/IME-poll/SSE-reload workers next. Both existing integration tests (`command_shells`, `p1_m7_tui_attach`) still pass unchanged, confirming the headless path is untouched. **Worker backbone completed and wired into the live render loop (2026-10-29):** `diary_tui::worker::Worker` is now generic over `Transport` (was hardcoded to `UreqTransport`, which made every intent beyond `RefreshWorkflows` untestable without a real server) and gained one `Intent`/`WorkerResult` pair per remaining M7 route — discard, canonical workflow refetch, discover, list sources, job polling, prepare sources, source detail, confirm selection, confirm empty selection, writing settings, begin manual composition, save entry, confirm entry — each exercised against a `Send`-safe fake transport on the real worker thread (new tests in `worker.rs`). `nd`'s `run_connected_session` previously called `StartViewModel::refresh` *directly* on the render thread, blocking it on real HTTP and contradicting the architecture doc's "the event/render loop never performs HTTP... work"; it now spawns a `Worker` and only `try_send`s `Intent::RefreshWorkflows`/drains `WorkerResult::Workflows` each tick (`StartViewModel` gained `apply_refresh`/`apply_start_or_resume`/`apply_discard` so the same outcome-application logic serves both the synchronous test methods and the async worker-driven path). The `retry` Command Line token now sends an intent instead of blocking, with an in-flight guard against duplicate sends. Sources/Compose/Review stage wiring onto this same Worker, and the dedicated SSE worker thread (reconnect, gap/cursor handling, forced reattachment), are the next increment. |
| Source-selection, empty-selection, autosave, recovery, and confirmation tests | In progress: `ApiClient` gained `discover`, `list_sources`, `job`, `prepare_sources`, `confirm_selection`, `confirm_empty_selection`, `set_writing_settings`, `begin_manual_composition`, `save_entry`, and `confirm_entry` operations matching the P1-M12 route/DTO contract (job-accepted discovery/preparation, `If-Match` on every mutating call; `save_entry` uses only the entry-buffer ETag, never the workflow ETag; `confirm_entry` sends both). A `SourcesViewModel` (7 tests) covers inclusion toggling, candidate reload, job-status mapping, and `confirm_empty_selection` as the one zero-note continuation. A `ComposeViewModel` (6 tests) implements the architecture doc's generation-tracking autosave rule: a successful save advances the canonical buffer version but only clears "dirty" when no newer local edit arrived while it was in flight; a `412` is recorded and never silently replayed; a canonical reload never discards unsent local text while dirty. A `DraftEditor` (8 tests) wraps `ratatui-textarea` with its own key dispatch — content edits via `input_without_shortcuts`, cursor movement via explicit `move_cursor` calls — and confirms a Ctrl chord is never consumed by a `ratatui-textarea` default binding (its own default Ctrl-D is delete-next-char); bracketed-paste insertion enforces the 512 KiB limit, rejecting an oversized payload whole rather than partially inserting it. Both ViewModels' error states now render through a `safe_label()` helper (added after an interview question caught the error display showing raw Rust `Debug` output instead of a fixed, content-free label) rather than the server/transport internals. Wiring the editor into the live render loop, and the ~1s autosave tick, are deliberately deferred together (owner decision) rather than wiring a timer with nothing yet to save. **The Command Line is now wired into the real `nd` binary's connected session** (`CommandLineState`, 4 new tests: submit-clears-buffer, backspace, whole-syllable backspace, clear-on-parse-failure): Ctrl-G opens it, typed characters (ASCII or Korean) buffer raw and normalize/parse together on Enter, Esc cancels, and Ctrl-D still quits unconditionally from any mode. `help`/`retry`/`save`/`day`/`stage` all have real (if minimal, since only Start exists) effects — `retry` re-calls `StartViewModel::refresh`, `day` sets a local field shown in the Status Line, `stage`/`save` report their not-yet-available state through the same `safe_label`-style fixed messages, never a raw error. `diary_tui::terminal` gained `poll_input_event`, a richer non-blocking poll (`InputEvent`/`InputPoll`) alongside the simpler `poll_event` used by the attach-retry loop. A `ReviewViewModel` (6 tests) adds `confirm_entry` wiring: blank text or an unresolved reflection delimiter (`⟪`/`⟫`, matching `diary_core::workflow::INFERRED_REFLECTION_*` — client-side presentation only, never a second authority) refuses confirmation without even calling the server; a real `412` is recorded through the same `safe_label()` path. Added a real-terminal `editor_probe` example so the Draft editor's Ctrl-chord isolation and Korean input can be verified interactively, the same way `chord_probe` and `command_line_probe` already are. **Advisor-review fixes (2026-10-29):** `source_detail` alone dropped the `X-ND-Preparation-Handle` response header, making a non-empty confirmed selection unreachable; added `source_detail_with_handle`/`SourceDetailResponse` and `SourcesViewModel::fetch_preparation_handle` (2 new tests) so the handle actually flows from a detail read to `confirm_selection`. Added a `sanitize_for_display` module (6 tests: newline/tab pass through, an ESC sequence and C1 controls and DEL are replaced one-for-one, wide text unaffected, length preserved) and wired it into `MainPaneWidget` and `ResourcesSidebarWidget` — sanitizing only the rendered copy, never the stored value, per the architecture doc. Wired `is_supported_size`/`TerminalTooSmallWidget` (1 new test) into both of `nd`'s render sites, which previously never checked terminal size at all. Fixed bracketed paste being silently dropped: `poll_input_event` discarded `Event::Paste` entirely; added `InputEvent::Paste(String)`, routed it into the live Command Line, and fixed `editor_probe` (which only matched `Event::Key`) to route it into `DraftEditor::paste`. |
| In-memory Work Status, rendering, accessibility, width, and Korean-command mapping tests | In progress: pure canonical-stage-mapping function (7 tests), the Command Line parser/Hangul-to-ASCII mapping (10 tests: direct ASCII tokens, a raw-jamo vector for every grammar token, unmappable-character/unknown-token/extra-argument/malformed-day rejection, and — added after a 2026-10-29 interactive real-terminal session via the new `command_line_probe` example surfaced the actual mixed delivery shape — precomposed-syllable decomposition with a simple tail, a compound vowel plus tail cluster, no tail, a mix of bare jamo and syllables in one line, and a bare compound-vowel/consonant-cluster jamo arriving standalone), the Work Status pane (4 tests via `ratatui::TestBackend`), the Standard Layout shell (5 tests), the Workflow Sidebar (3 tests), the Resources Sidebar (2 tests: empty placeholder, hierarchy indent), the Main Pane (1 render test), the Status Line (3 tests), and the Start-stage ViewModel (6 tests against a fake transport: list refresh/error retention, discard with the listed revision as `If-Match`, a `412` kept for deliberate resolution, discarded-workflow visibility, server-owned-default start) are implemented and tested. **`YYYY-MM-DD (Mon)` rendering (2026-10-29):** a new pure `diary_tui::date_display::format_diary_day` (7 tests: a known weekday, every day of one known week via Howard Hinnant's `days_from_civil`, the Unix epoch itself, narrow-width and exact-width fallback boundaries, malformed-input passthrough) computes the abbreviated weekday with plain integer arithmetic (no date crate dependency) and appends it only when the result still fits the caller's width budget. Wired into `StatusLineModel::render`, which now stores the raw `YYYY-MM-DD` and computes the budget left over after its other fixed segments at render time, rather than a pre-formatted string the caller had to get right; not yet wired into the Resources Sidebar's workflow-list day labels (still plain, pending Sources-stage live wiring). |
| Real-terminal restoration and IME check (Wave Terminal first, per 2026-10-29 owner decision — see AGENTS.md "Target terminal"; Apple Terminal secondary) | **Not yet re-checked in Wave Terminal.** All findings below were observed in Apple Terminal before the target-terminal decision; `chord_probe`, `command_line_probe`, `editor_probe`, and `restoration_probe` must be re-run in Wave Terminal before any of this is settled there. In progress, owner-observed (2026-10-29) in Apple Terminal: `cargo run -p diary-tui --example chord_probe` showed Ctrl-G, Ctrl-S, Ctrl-D, Ctrl-P, Ctrl-N all arrive directly as chords, and F10 (the originally planned Ctrl-D alias) does not arrive at all. Rather than confirm the remaining aliases (F2, F3, F6, F7) one at a time, the owner decided to drop function-key aliases entirely (AGENTS.md, both M7 docs, `docs/ux-gui.md`, and `docs/phase-1/changelog.md` revised); every action is reachable only by its Ctrl chord or by focus and Enter. A follow-up interactive session with `cargo run -p diary-tui --example command_line_probe` fed real Korean-IME keystrokes through the actual parser and found the delivery shape is a **mix** (bare simple jamo, bare compound jamo, and fully committed precomposed syllables, not only one of those), and separately caught and fixed a real decomposition bug (a bare compound-vowel jamo was rejected as unmappable) and a real table bug (the Compatibility Jamo consonant order was wrongly assumed to match the jongseong-only tail order, corrupting `retry`'s normalization) — both from this live owner-driven testing, not from unit tests alone; the owner also reviewed and explicitly approved both resulting design changes (Ctrl-D with no alias; the three-shape jamo/syllable mapping) as settled. Terminal restoration on panic/normal/error has a dedicated `cargo run -p diary-tui --example restoration_probe -- normal\|error\|panic` probe built, but is not yet owner-confirmed in a real terminal; the owner is running it. The production TIS notification/run-loop path and 500 ms fallback are now implemented and automated; the bounded read-only main-thread smoke probe reported `IME: EN`, but live `EN ↔ KO` notification delivery and indicator changes still require the primary Wave Terminal check and secondary Apple Terminal regression check. |
| Documentation inspection and full Rust/Swift gate, with date and tool versions | Passed 2026-10-29 after Increment 7: `cargo fmt --all -- --check`; `cargo clippy --all-targets --all-features -- -D warnings`; and `cargo test --all-features` (including `diary-tui` 165, `diary-control` 23, `nd` unit 5, and every integration target) on `rustc`/Cargo 1.98.0. `swift build --package-path helpers/apple-bridge` and `swift test --package-path helpers/apple-bridge` passed on Apple Swift 6.4 targeting arm64 macOS 26.0; XCTest ran 10 cases with 0 failures. The first sandboxed Swift attempt could not write Clang's module cache, so both commands were rerun with the required filesystem approval and then passed. Together with the later owner-confirmed terminal evidence, this is the completed P1-M7 gate. |
